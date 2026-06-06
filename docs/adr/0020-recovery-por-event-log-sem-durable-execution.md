# ADR 0020 — Recovery por event log + idempotência durável; SEM durable execution full (estilo Temporal)

- **Status:** Aceito (F1-0-7, 2026-06-06)
- **Contexto:** story F1-0-7 (tasks/epico-f1/ondas-0-1.md) · pesquisa 13.11 (recomendação 3: "não perseguir [durable execution] — event log recovery + idempotência bastam no local-first"; "circuit breaker é MANDATÓRIO: sem ele, retry storm consome quota e propaga erro") · a decisão era IMPLÍCITA no código desde W0-5/ADR 0003 — a verificação adversarial flagou a ausência de registro formal.

## Decisão

O Lina **não** adota um motor de *durable execution* completo (estilo Temporal/Restate: replay de workflow determinístico com side-effect journaling por atividade). A confiabilidade de entrega A2A é construída sobre **quatro mecanismos próprios, locais e event-sourced**:

1. **Idempotência durável por `id` (`msg_`+UUIDv7)** — o *ledger de entrega* é derivado do próprio event log (`MessageRouted`/`MessageDelivered`/`MessageDeliveryFailed`/`MessageDeadLettered`): após um crash/restart, o pump distingue **entregue** (re-apresentação ⇒ no-op com resultado conhecido), **falha de entrega pendente** (retoma o retry do attempt/agendamento DURÁVEIS gravados no log) e **não-roteada** (segue o pipeline). Replay + retry da mesma operação ⇒ no-op. Janela A6 residual (crash entre rotear e o desfecho da injeção, sem evento de entrega/falha): mantém-se a postura **conservadora** histórica do D1 — assume entregue (re-injetar num PTY é irreversível; perder é recuperável pela DLQ/humano, duplicar não é).
2. **Retry com backoff exponencial + jitter determinístico** — `base·2^(attempt−1)` + `hash(id,attempt) % (exp/4+1)`. Determinístico ⇒ reprodutível em teste e coerente com replay (invariante #4); ainda assim anti-thundering-herd (fase própria por id). Agendamentos persistidos em `MessageDeliveryFailed{next_retry_ms}` — os timestamps do log SÃO a prova do backoff.
3. **Dead-letter queue durável** (`<.lina>/dead-letter/<id>.json` + `MessageDeadLettered{reason}`) — destino de: estouro do teto de retenção (F1-0-4), esgotamento de `delivery_max_attempts` (5), destino inexistente persistente (transiente esgotado — antes era ack+drop SILENCIOSO; agora nada some). **Retry manual, nunca automático**: re-enfileirar = mover o arquivo de volta ao outbox por-nó. Ordem durável: o arquivo entra na DLQ ANTES do ack do `.inflight` (a mensagem nunca fica sem casa).
4. **Circuit breaker por nó** — `breaker_threshold` (5) falhas CONSECUTIVAS de entrega ⇒ `CircuitOpened` + nó `Blocked` (`NodeStatusChanged{reason:"circuit_breaker"}`); novas mensagens **nem tentam** (retidas com `circuit_open`) até **reset externo** (lifecycle/humano tirar o nó de `Blocked`). Anti retry-storm (13.11).

## Por que NÃO durable execution full

- **Local-first, processo único**: não há infra distribuída a coordenar; o event log (SQLite WAL + JSONL, W0-5) já é a única fonte da verdade e sobrevive a `kill -9` (gate W0-6). Um runtime de workflow adicionaria um segundo "log de verdade" (journal de atividades) — exatamente a tabela-autoridade paralela que o doc 01 §3 proíbe.
- **Os efeitos do Lina são injeções em PTY — irreversíveis e não-transacionais**: nenhum motor de replay torna um Enter "des-apertável". O ponto certo de idempotência é ANTES do efeito (ledger + dedupe), não o replay do efeito.
- **Custo/complexidade**: Temporal-like exige versionamento de workflow code, non-determinism guards e um runtime próprio — desproporcional para um broker single-thread cuja unidade de trabalho é 1 mensagem.

## Consequências

- O caminho de entrega tem **zero perda silenciosa**: todo desfecho é `Delivered`, `Retained`, `RetryBackoff`, `DeadLettered` ou recusa logada (`RouteBlocked`) — auditável por replay.
- A janela A6 residual aceita **perda-com-registro** (conservador anti-duplicação); o humano recupera pela DLQ/log.
- Revisitar ESTE ADR se: (a) surgirem efeitos multi-passo que precisem de saga/compensação; (b) o Lina ganhar execução distribuída (multi-máquina); (c) a taxa real de janela-A6 observada em produção justificar journaling por atividade.

## Calibração do teto de retenção (`retention_timeout_ms`) — 2026-06-06

**Achado (MÉDIO, medidor do gate):** o default original de **30 s** dead-letterou 1 retorno LEGÍTIMO sob rajada saturada. Análise de produto (Maestro, validada pelos dados): reter atrás de um turno real é o comportamento **desejado** ("entrega quando o alvo ficar livre") — e um turno de claude leva **minutos**, não segundos.

**Dados que ancoram a faixa:**
- Turnos REAIS (observação de 14h, DIRECIONAMENTO §P1): retornos espaçados em **200–600 s**.
- Turnos TRIVIAIS (probes "não responda", corrida DEPOIS da F1-0-4): **3,2–15,9 s** (mediano 9,6 s).
- Com 30 s, qualquer espera atrás de um turno real (≥ ~3 min) estouraria o teto → mensagem legítima na DLQ → o propósito da retenção morre.

**Decisão:** default = **600 000 ms (10 min)** — cobre o teto observado dos turnos reais, fica ~40× acima do turno trivial mediano, e **mantém o papel anti-deadlock** (finito; estouro → DLQ **visível** com retry manual, nunca espera infinita — 13.11 §P0). Tunável por workspace (`RouterConfig::retention_timeout_ms`). Trava anti-drift: teste `retention_default_e_da_ordem_de_minutos` (banda [600 s, 1 h]).

**Limitações conhecidas (aceitas):** (a) cauda de turnos >10 min ainda dead-lettera — aceitável porque é VISÍVEL e recuperável; (b) sob fila profunda, o relógio conta desde a PRIMEIRA retenção da mensagem — profundidade k × turno médio pode exceder o teto antes de chegar a vez dela.

**Gatilho de revisita:** quando o dashboard F1-1-5 der **tempos reais de resposta por agente** em produção, re-medir a distribuição (p50/p95/p99 de turno + profundidade típica de fila) e re-calibrar — idealmente p95(turno) × profundidade típica + margem.

## Verificação

`crates/lina-core/tests/gate_f1_0_7.rs` (exactly-once sob crash; backoff+jitter por timestamps do log; DLQ com motivo + projeção visível; breaker abre/bloqueia/reseta; backoff durável a restart; append concorrente de 2 escritores) + `retention_default_e_da_ordem_de_minutos` (calibração) + suíte do router + bench N=50.
