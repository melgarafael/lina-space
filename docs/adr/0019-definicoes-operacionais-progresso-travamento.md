# ADR 0019 — Definições operacionais: "progresso", "travamento", spawn caps e direção estética default

- **Status:** Aceito (fundador: spawn caps + gate humano; arquiteto/Maestro: definições mensuráveis — 2026-06-06)
- **Onda/Story:** F1-0 (lifecycle/heartbeat) · F1-1 (fila de atenção, `lina check`) · F1-3 (agente-cria-terminal, doutrina)
- **Data:** 2026-06-06
- **Fontes:** pesquisa `13.11` (Sol Framework: cycle count + hash; STORY 1/3/6) · `13.12` (failure modes do Maestri v0.29) · `13.4` (anti-slop) · `13.2` §portas · ADRs 0003/0005/0007 · `tasks/epico-f1/arquitetura.md` §c.2

## Contexto

A fila de atenção, o `lina check` e o agente-cria-terminal precisam de definições
**mensuráveis** — não impressões de LLM — para "este terminal está progredindo?" e "este
terminal travou?". A pesquisa fundamenta: heartbeat por timestamp puro **não distingue**
*alive+working* de *alive+hung* (13.11 🟢 — exige cycle count + hash); auto-report do agente
é rejeitado (agente pode mentir — 13.11 STORY 6; invariante #1); o freeze do Maestri (UI
viva, terminais sem progresso — 13.12 🟢, feedback de mercado real) é o falso-negativo a não
reproduzir, e a delegação instável dele é o falso-positivo a não reproduzir.

## Decisão

1. **Amostragem determinística por nó** (telemetria efêmera, NÃO evento): a cada
   `HEARTBEAT_SAMPLE_MS = 120_000` (2 min — 13.11: hash a cada 2–3 min), capturar
   `(cycle_count, tail_hash)` — `tail_hash` = SHA-256 dos últimos ~80 chars do tail do PTY
   (13.11 STORY 6); `cycle_count` incrementa a cada advance com output novo.
2. **PROGRESSO** (definição mensurável): houve progresso na janela se **(a)** `tail_hash`
   mudou entre amostras consecutivas, **ou (b)** ≥1 `DomainEvent` novo atribuível ao nó no
   período (`RouteDelivered` de/para o nó, `TokenUsageReported`, `PlanClaimed`/`PlanChecked`,
   `HandoffOpened`/`HandoffClosed`, …). Qualquer um basta.
3. **TRAVAMENTO** (default conservador): nó com status **Busy** acumulando
   `STALL_WARN_SAMPLES = 3` amostras consecutivas SEM progresso (~6 min) → emitir
   **`NodeStalled` uma única vez, na transição** (anti-amplificação — ADRs 0003/0005) →
   entra na fila de atenção como WARN. Persistindo até `STALL_BREAKER_SAMPLES = 6` (~12 min)
   → **circuit breaker: pausa-com-gate (`lina resume --confirm`), nunca kill** (padrão
   ADR 0005). Constantes em `RouterConfig` (tunáveis; defaults conservadores para não
   reproduzir a delegação instável do Maestri — 13.12).
4. **O relógio de stall só corre em `Busy`.** `Blocked` (aguardando permissão/custódia — já
   está na fila de atenção por outro caminho) e `Idle` **não acumulam** — elimina o
   falso-positivo nº 1 (terminal esperando y/n não está travado).
5. **Invariante #4:** o VEREDITO (progredindo/travado) é **projeção do event log** — toda
   transição vira evento (`NodeStatusChanged`, `NodeStalled`); as amostras cruas são
   efêmeras e **nunca apendadas** (alto volume/baixo sinal). `lina check @X` responde da
   projeção, nunca de view cacheada (13.11 STORY 3: agentes leem log recente).
6. **Spawn caps (agente-cria-terminal, F1-3 — decisão do fundador):**
   `max_spawns_per_turn = 2` por nó solicitante. A distinção **origem vs cascata** reusa o
   binding NÃO-forjável do ADR 0007 (`derive_root_hops`). **Texto alinhado ao código em
   2026-06-10 (conselho F1-3, achado R2 — o implementado é MAIS restritivo que a redação
   original):** pedido de spawn nascido em cascata (`hops ≥ 1` efetivo) é **gateado SEMPRE**
   (`SpawnGated{cascade}`, `router.rs:1805` — pede aval humano), **ANTES** de consultar o
   cap — a defesa anti-fork-bomb não depende do contador; o cap `(root, sender) = 2` morde a
   **origem**. Criação direta pelo humano (UI/comando) não passa pelo cap de agente. Spawn é
   sempre via Supervisor (**NodeId = autoridade única**); cada spawn conta no teto de custo
   (ADR 0005); evento: `NodeAdded { requested_by }` (campo aditivo, `serde(default)`).
   **Gap aceito (Maestro, 2026-06-10 — red-team do spawn INV3/R1-L2):** o cap é por
   `root_cause_id`; uma ORIGEM que inicia turnos novos ganha root fresco e renova o cap
   (origem-burst). Aceito porque a origem é ação a mando do humano, a cascata segue gateada,
   e os backstops (teto de custo quando configurado + gate humano) permanecem; o caveat da
   janela de liveness do binding está documentado no ADR 0007. Re-avaliar se o retro (F1-3-7)
   mostrar spawns de origem em rajada sem pedido humano correspondente.
7. **Direção estética default da doutrina (F1-3, conforme 13.4):** a skill estética da Lina
   carrega **opinião explícita como default** — banir os genéricos (Inter/Roboto/Arial como
   tipografia default, gradiente branco-roxo, layout-padrão-de-template); adotar JetBrains
   Mono para código/terminal e tipografia display com personalidade **via tokens do design
   system da F1-2** (nunca fonte/cor hard-coded); motion de alto impacto SEMPRE subordinado
   a reduce-motion; paradigma prescritivo + encorajamento explícito de risco criativo
   (13.4 §3, Anthropic Cookbook). Vale para o que a Lina **produz** (código/design/
   artefatos) e para a UI do app. Customizável pelo usuário; o default é a opinião.

## Limite explícito

- Stall detection mede o **PTY e o log**, não a UI — o freeze de UI do Maestri é classe
  diferente de falha, coberta por test-suite própria que replica os failure modes dele
  (freeze, foco, delegação, command-relay — 13.12).
- Os thresholds (2 min / 3 amostras / 6 amostras) são **hipóteses calibráveis**: validar
  contra a baseline real do gate formal da F1-0 (5–8 Claudes, P0/P1 reproduzidos antes dos
  fixes) antes de considerá-los definitivos.

## Alternativas rejeitadas

- **Auto-report do agente** ("estou progredindo") — agente pode mentir/alucinar (13.11
  STORY 6); campo escrito por agente nunca decide (família ADR 0007).
- **Timeout de wall-clock puro** — não distingue *hung* de *long-task* (13.11 🟢).
- **LLM julgando o scrollback** — viola o invariante #1 e introduz não-determinismo no
  guardrail (13.2 §portas).
- **Kill no breaker** — perde contexto/trabalho do agente; pausa-com-gate preserva
  (ADR 0005: "pausa-com-gate, nunca kill").
