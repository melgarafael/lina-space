# ADR 0003 — toda recusa de guardrail apenda um `DomainEvent` (o log é o livro-razão das recusas)

- **Status:** Proposto (será aceito quando W3-7 emitir os eventos de bloqueio e os ACs assertarem do log)
- **Onda/Story:** Onda 3 · W3-7 (decisão transversal §4.1 do `tasks/_design-w6-w7.md`)
- **Data:** 2026-06-02

## Contexto

Os bloqueios de guardrail existem hoje **só como `RouteOutcome` em memória**
(`router.rs`): `Duplicate`, `HopLimit`, `UnknownSender`, `NoTarget`, `BlockedByAutonomy`,
`FanoutGated`, `BudgetExceeded`, `Deadlock` — e os futuros `LoopDetected` (W3-7 §2.1) e teto
de custo (§2.2). A auditoria A2A (achado A6) confirmou no código real que **todo `return`
por guardrail ocorre ANTES** do primeiro `store.append(DomainEvent::MessageRouted)`
no `route_message`: o caminho **negativo** não persiste **nada**.

Isso quebra duas coisas:

1. **Critério de aceite da Onda 3 — "validável headless via `.lina/events/log.jsonl`".** Os
   caminhos negativos (a maior parte dos guardrails é justamente recusar) **não podem ser
   assertados do log** porque não há evento; o teste só consegue ler um `enum` de retorno que
   um harness in-process enxerga. Os ACs de bloqueio do design (W3-6/W3-7) ficam impossíveis
   de cumprir de verdade.
2. **Agrava a janela de perda (A6).** Como o `drain` já **removeu** o arquivo do outbox
   ao consumi-lo, antes do roteamento, uma mensagem recusada por guardrail **desaparece**
   do outbox **e** não deixa rastro durável — não há registro nem de que ela existiu, nem de
   por que foi recusada.

Invariante **#4** (event log = fonte da verdade) exige que o log seja o **livro-razão das
decisões — inclusive das recusas**. Isso toca o catálogo `DomainEvent` (âncora de
continuidade) → ADR antes de codar.

## Decisão

**Toda recusa de guardrail apenda um `DomainEvent` antes de retornar o `RouteOutcome`.** Três
variantes novas (event_version=1, aditivas — mesma política do ADR 0001):

### 1. `RouteBlocked { id, reason, from, to }` — recusas do roteamento (router)

`reason` é um enum tipado (projetável e assertável):

| `reason` | `RouteOutcome` correspondente |
|---|---|
| `duplicate` | `Duplicate` |
| `hop_limit` | `HopLimit` |
| `unknown_sender` | `UnknownSender` |
| `no_target` | `NoTarget` |
| `blocked_by_autonomy` | `BlockedByAutonomy` |
| `fanout_gated` | `FanoutGated{count}` (carrega `count`) |
| `budget_exceeded` | `BudgetExceeded` |
| `deadlock` | `Deadlock` |
| `loop_detected` | `LoopDetected` (W3-7 §2.1) |

> **Exceção explícita — `PersistFailed`:** este outcome é a **falha do próprio `append`**;
> não há como persistir um evento que descreve a impossibilidade de persistir. Mantém-se o
> tratamento atual (visível em `stderr`, nunca silencioso) — documentado aqui como o único
> bloqueio que **não** vira `RouteBlocked`.

### 2. `ActionGated { cmd, class, decision }` — gate de execução (W3-6)

`class ∈ { gated-soft | gated-hard | gated-hard-external }`; `decision ∈ { allow | ask | deny }`.
Emitido pelo verbo `lina guard --check-action` e pelos wrappers do PATH-shim (ver ADR 0004).
`allow` de ação `routine`/`local-reversible` **não** emite evento (ruído desnecessário).

### 3. `CostCeilingHit { workspace, day, tokens }` — teto de custo (W3-7 §2.2)

Apendado quando o uso ultrapassa `token_budget_day` e o workspace entra em `Paused`.

Todas aditivas: o reducer ganha braços; o app só constrói (`append`). A ordem é **apendar o
evento de bloqueio e então retornar** o `RouteOutcome`.

## Alternativas consideradas

- **Manter só o `RouteOutcome` em memória.** Rejeitada: caminhos negativos inobserváveis
  headless → falha os ACs da onda e o invariante #4.
- **Logar bloqueios só em `tracing`/`stderr`.** Rejeitada: `stderr` é para **visibilidade do
  operador** (e a auditoria já exige "nunca silencioso" para inconsistências críticas), mas
  **não é o event log** — não é replayável, não é a fonte da verdade, não reconstrói estado.
  O livro-razão das recusas precisa estar no `DomainEvent`.
- **Um único `GuardrailHit { reason: String }` genérico.** Rejeitada como forma primária: um
  `reason` livre dificulta projeção e asserção tipada. Opta-se por `reason` **enum** em
  `RouteBlocked`, e por eventos **separados** (`ActionGated`/`CostCeilingHit`) porque carregam
  campos diferentes e vêm de **camadas diferentes** (gate de execução vs roteamento vs custo).

## Consequências

- **(+)** Caminhos negativos **validáveis headless**; o log vira o **livro-razão completo das
  recusas** (invariante #4). Cada AC de bloqueio assere o `DomainEvent`, não só o `enum`.
- **(+)** Habilita a UX não-técnica (invariante #6): o app pode **narrar** "não fiz X porque
  Y" a partir do log, em vez de o motivo morrer num retorno interno.
- **(+)** Fecha **parcialmente** a A6 do lado da observabilidade da recusa (a mensagem recusada
  passa a deixar rastro). **Necessário, mas não suficiente** para a A6 completa — ver dependência.
- **(−)** Mais volume de eventos. **Atenção (liga-se à auditoria A4 DoS por quantidade):**
  `duplicate` é alto-volume/baixo-sinal; logar um `RouteBlocked{duplicate}` por mensagem sob um
  flood transformaria o livro-razão num **amplificador de escrita**. Mitigação recomendada:
  **não** emitir evento para `duplicate` (ou rate-limitar), já que a duplicata é benigna por
  definição; os demais `reason` são de baixo volume e alto sinal.
- **(−)** O caminho de recusa ganha uma dependência de persistência (se o `append` do
  bloqueio falhar, recai no tratamento de `PersistFailed`/`stderr`).

## Dependência (não-decidida aqui, sinalizada)

Logar a recusa **não** recupera a mensagem perdida em crash. Para fechar a A6 por inteiro é
preciso, **em conjunto**, tornar o `drain` **não-destrutivo** (mover para `outbox/.inflight/`
e só remover após persistir; ou persistir um `MessageReceived` antes do `remove`). Essa
mudança vive em `mailbox.rs`/`router.rs` (outra cadeia, em obras) e deve ser registrada como
endurecimento de W3-7 — este ADR cobre apenas a **observabilidade das recusas**.

## Critério de verificação (headless, via event log)

- **AC-0003.1:** cada teste de bloqueio (dedupe — se mantido —, hops, autonomia, fanout,
  budget, deadlock, loop) assere o `RouteBlocked{reason}` **correspondente no log**, além do
  `RouteOutcome`.
- **AC-0003.2:** `lina guard --check-action --cmd "git push --force …"` → `ActionGated{ class:
  "gated-hard", decision: "ask" }` no log; `--cmd "cargo test"` → `allow` **sem** evento.
- **AC-0003.3:** uso > `token_budget_day` → `CostCeilingHit{ workspace, day, tokens }` no log e
  workspace `Paused` (projeção observável).
- **AC-0003.4 (anti-amplificação):** sob N mensagens duplicadas, o nº de eventos de bloqueio
  **não** cresce com N (duplicata não loga / é rate-limitada).
