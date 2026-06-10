# ADR 0005 — teto de custo é event-sourced (`CostLedger` por replay) + pausa-com-gate, nunca kill

- **Status:** aceito (implementado em `9260416` — `W3-7c`; `lib` PASS, clippy0)
- **Onda/Story:** Onda 3 · W3-7c (teto de custo — §2.2 do `tasks/_design-w6-w7.md`)
- **Data:** 2026-06-02

## Contexto

Cada agente do Lina é um CLI de IA de terceiro (Claude Code, Codex…) que **consome
tokens pagos**. Sem um teto, uma cascata de auto-orquestração (delegações A→B→C, loops
recuperados, retries) pode queimar orçamento de forma **silenciosa e ilimitada** — o
oposto do que o público-alvo (não-técnico) tolera. O invariante **#6** ("não-técnico-first;
estado sempre salvo e visível") exige um **teto previsível** e uma parada **observável e
recuperável**, não um estouro de fatura descoberto depois.

O teto colide com três invariantes que precisam ser honrados ao mesmo tempo:

1. **#1 (zero LLM):** o app **não interpreta** o custo nem "decide" semanticamente parar.
   O teto tem de ser **contador determinístico** — soma de tokens vs. um número.
2. **#4 (event log = fonte da verdade):** o contador **não pode** ser estado mutável em
   memória (some no `kill -9`, diverge entre processos). Tem de ser **reconstruível por
   replay** do log, como o `plan.md` (ADR 0001).
3. **#6 (estado salvo/visível):** ao atingir o teto, a reação é **pausar com gate humano**,
   não **matar** o agente — matar perde estado e é invisível ao leigo.

Há ainda um conceito **já existente** que poderia ser confundido: `RouteOutcome::BudgetExceeded`
(orçamento de **delegação**, `delegation_budget=8`/turno por `root_cause_id`). Teto de custo
e orçamento de delegação são **eixos ortogonais** (um conta tokens/dia do workspace; o outro
conta delegações/turno por cadeia) — fundi-los apagaria a distinção que o operador precisa ver.

Por tocar o `RouterConfig` e o catálogo `DomainEvent` (âncora de continuidade), → ADR antes de codar.

## Decisão

### 1. O teto é um contador EVENT-SOURCED (`CostLedger` reconstruído por replay)

A fonte de uso é um evento novo **`TokenUsageReported { node, tokens }`**
(`events.rs:186`). O app o emitirá ao observar o **fim-de-resposta da CLI** (W0-10);
**hoje o teste semeia** esses eventos no store (a peça W0-10→app é pendência, ver
Consequências). O `CostLedger` (`router.rs:905`) **não guarda estado** — ele
**reconstrói** por replay do log via `CostLedger::replay(store, now_ms)`
(`router.rs:919`), exatamente como o `plan.md` é projeção (ADR 0001). Coerente com o
invariante #4: a soma de uso, o dia e até o flag de pausa são **derivados**, sobrevivem a
`kill -9` e não divergem entre leitores.

### 2. Granularidade `(workspace, dia-UTC)` + janela contábil que ZERA na retomada

`RouterConfig.token_budget_day` (`router.rs:67`; **`0` = desligado**, opt-in por
workspace, `router.rs:79`). **Nota de status (registrada 2026-06-10, conselho F1-3 item B1):**
em produção o default segue **OFF** (`0`) — a CONTABILIZAÇÃO (`TokenUsageReported`/`CostLedger`)
é sempre ativa, mas o GATE só morde quando o operador configura o teto. Consequência honesta:
em autonomia `autonomo` com teto OFF, ações de origem (ex.: `lina spawn`) ficam limitadas pelos
caps próprios + arbítrio do humano que origina os turnos. A decisão de ligar um teto default
pertence ao painel de custos/seam de Ajustes (candidata F1-4/F2) — não ligar silenciosamente
por trás do usuário. A **janela contábil** soma os
`TokenUsageReported` apendados **depois do último `CostCeilingResumed`** (`router.rs:900-902`):
a retomada humana **abre uma nova janela** (zera a soma), ela não "perdoa" retroativamente.
Sem esse zeramento, a soma anterior **re-pausaria** na próxima delegação e o `lina resume`
seria placebo.

### 3. Ao atingir o teto: `Paused` (projeção) + `CostCeilingHit` só na transição + `RouteOutcome::CostCeiling`

Na rota de uma **delegação** (`is_deleg && token_budget_day > 0`, `router.rs:360`), o
router reconstrói o ledger e:

- se **já pausado** (`ledger.paused`) → retorna `RouteOutcome::CostCeiling` **sem
  re-apender** (`router.rs:366-368`) — anti-amplificação A4 (não loga a cada delegação
  barrada);
- se a janela **atingiu o teto** (`ledger.tokens >= token_budget_day`, `router.rs:369`)
  → apenda **`CostCeilingHit { day, tokens }`** (`events.rs:194`, `router.rs:370`) **só
  nesta transição** para `Paused` e retorna `RouteOutcome::CostCeiling` (`router.rs:376`).

`RouteOutcome::CostCeiling` (`router.rs:111`) é **distinto** de `BudgetExceeded`: o
primeiro é o teto de tokens/dia do workspace; o segundo, o orçamento de delegação/turno.
**`Paused` não é um estado armazenado** — é a projeção `CostLedger.paused` (`router.rs:911`),
que segue o último evento de teto visto no replay (`CostCeilingHit`→pausado;
`CostCeilingResumed`→liberado, `router.rs:932-935`).

### 4. Retomada é GATE humano explícito — `lina resume --confirm` (nunca automático, nunca kill)

`lina resume` **sem `--confirm` é no-op** (`bin/lina.rs:367` — a retomada exige
confirmação explícita; invariante #6). Com `--confirm`: se o workspace está `Paused` (ledger
reconstruído), apenda **`CostCeilingResumed { day }`** (`events.rs:201`, `bin/lina.rs:390`)
→ sai de `Paused` e abre nova janela contábil. O agente **não é morto**: o estado fica salvo
e visível no log até o humano decidir retomar.

## Consequências

- **(+) Teto previsível e auditável:** o operador define `token_budget_day` por workspace;
  o estouro é um evento (`CostCeilingHit`) no log, não uma surpresa na fatura.
- **(+) Sobrevive a crash / não diverge:** sendo replay-reconstruído, o teto vale igual após
  `kill -9` e para qualquer leitor (invariante #4). Zero estado mutável em memória.
- **(+) Recuperável sem perda (invariante #6):** pausa preserva o agente e o trabalho; a
  retomada é um gate humano de uma linha, não um respawn que perde contexto.
- **(+) Conceitos não fundidos:** `CostCeiling` ≠ `BudgetExceeded` — o operador distingue
  "estourei tokens do dia" de "estourei delegações do turno".
- **(−) Depende do app emitir uso REAL (W0-10 → app):** hoje só o **teste** semeia
  `TokenUsageReported`; até o app instrumentar o fim-de-resposta da CLI, o teto **não morde
  em produção**. É a única peça que falta para o mecanismo (já completo no core) operar de fato.
- **(−) Janela diária em UTC pode não casar o fuso do usuário:** o "dia" reseta à meia-noite
  UTC, não no fuso local — um teto pode parecer reiniciar "no meio da tarde" para quem está
  longe de UTC. Aceitável no MVP (simplicidade, sem dependência de tz); candidato a refino
  futuro (fuso do workspace) se o leigo estranhar.
- **(−) `CostCeilingHit` só na transição** significa que o log **não** registra cada
  delegação barrada enquanto pausado (anti-amplificação A4) — para auditar "quantas vezes
  bateu na trava enquanto pausado" seria preciso outra fonte. Trade-off deliberado: log limpo
  > contabilidade de tentativas barradas.

## Alternativas rejeitadas

- **Matar (`kill`) o agente ao estourar o teto, em vez de pausar.** Rejeitada: fere o
  invariante #6 — perde o estado do agente, é invisível ao leigo e não-recuperável sem
  respawn. A parada tem de ser **salva e visível**, com humano no laço para retomar.
- **Contador em memória (campo mutável no router).** Rejeitada: fere o invariante #4 — some
  no `kill -9`, diverge entre processos/leitores e não é reconstruível por replay. O
  `CostLedger` é projeção do log, como o `plan.md` (ADR 0001), justamente por isto.
- **Reusar `RouteOutcome::BudgetExceeded` para o teto de custo.** Rejeitada: funde **dois
  conceitos ortogonais** (tokens/dia do workspace vs. delegações/turno por `root_cause_id`).
  O operador precisa ver **qual** trava bateu; um único outcome apagaria a distinção e a
  mensagem de retomada (`lina resume` reabre o teto, não o orçamento de delegação).
- **Resetar a janela por timer automático (meia-noite UTC) sem gate humano.** Rejeitada para
  a *retomada após estouro*: reabrir sozinho contradiz "gate humano" — o estouro é justamente
  o sinal de que o humano deve decidir. (O rótulo de dia-UTC existe para *contabilidade*, não
  para auto-retomar um workspace pausado.)

## Critério de verificação (headless, observável no log)

- **AC-0005.1 (antes do teto):** com `token_budget_day=100` e uso `0`, a delegação roteia
  normal e **nenhum** `CostCeilingHit` é apendado (`router.rs:2082-2092`).
- **AC-0005.2 (transição → Paused):** semear `TokenUsageReported` somando `>= 100` → a próxima
  delegação retorna `RouteOutcome::CostCeiling`, **não** entrega, e apenda **exatamente um**
  `CostCeilingHit{day, tokens}`; `CostLedger::replay(...).paused == true` (`router.rs:2111-2139`).
- **AC-0005.3 (anti-amplificação):** já pausado, nova delegação bloqueia **sem** apender um 2º
  `CostCeilingHit` (`router.rs:2144-2151`).
- **AC-0005.4 (retomada):** `lina resume --confirm` apenda `CostCeilingResumed{day}` → sai de
  `Paused` e zera a janela (`router.rs:2156-2162`); sem `--confirm`, no-op (`bin/lina.rs:367`).
- **Teste de cobertura:** `cost_ceiling_pauses_delegation_until_resume` (`router.rs:2060+`).
