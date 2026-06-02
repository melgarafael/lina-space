# ADR 0002 — `await/reply` lifecycle é event-log-sourced; `BusEvent` permanece enxuto

- **Status:** Proposto (será aceito quando W3-7 implementar os 2 eventos + a tabela `pending`)
- **Onda/Story:** Onda 3 · W3-7 (guardrails P0 — fecho §3 do `tasks/_design-w6-w7.md`)
- **Data:** 2026-06-02

## Contexto

O `BusEvent::Message { id, from, to }` é **enxuto** (só a notificação de roteamento). O
**payload** A2A viaja **fora da bus**, pela closure `deliver` → injeção faseada no PTY
(`deliver_a2a`). O `await` hoje é **manual e meia-boca**: o router faz `begin_await`/
`end_await` no `WaitForGraph` como **checagem pura com rollback imediato** (Guardrail 2
anti-deadlock do `router.rs`) — a aresta `waiter→target` **nunca persiste**, e **quando B responde,
nada casa a resposta ao `await` de A** nem chama `end_await`. Não há correlação de `reply`
modelada.

Isso colide com dois invariantes do `CLAUDE.md`:

1. **#4 — "O event log é a fonte da verdade; layout/buffers/tabelas são projeções
   reconstruíveis."** O ciclo `await→reply→close` precisa ser **observável headless** (via
   `.lina/events`) e reconstruível por replay — não pode viver só como estado efêmero da
   bus. O próprio `events.rs` documenta que o espelho `bus.jsonl` é **redundante**: a bus é
   transporte/projeção, **não** a fonte da verdade.
2. **Envelope A2A versionado (âncora de continuidade) — "não invente formatos por feature".**
   `BusEvent` (pub/sub interno) e `A2aEnvelope`/`lina/msg@1` (wire A2A) devem **versionar em
   separado**; fundi-los acopla dois contratos que evoluem por razões diferentes.

A auditoria A2A reforça: a ausência de correlação de `reply` é o que torna a pré-checagem
com rollback a única opção segura hoje (uma aresta persistente sem mecanismo de fechamento
viraria órfã e geraria **falsos deadlocks** depois). Modelar o lifecycle resolve a causa
raiz. Mexer aqui toca duas âncoras (`BusEvent` e o catálogo `DomainEvent`) → ADR antes de codar.

## Decisão

**Opção B — envelope completo fora da bus; bus enxuta; lifecycle no event log.**

### 1. `BusEvent::Message { id, from, to }` permanece como notificação/chave de correlação

Sem alargamento. Subscribers (UI/observabilidade) recebem a **chave** (`id`) e casam com o
envelope da origem quando precisam de contexto — a entrega **já** ocorre fora da bus, então
o plumbing é pequeno.

### 2. Dois novos `DomainEvent` (event_version=1, aditivos)

- `AwaitOpened { id, waiter, target, root_cause_id }` — apendado quando um `--await`
  `waiter→target` passa o pipeline (incluindo o Guardrail 2 anti-deadlock).
- `AwaitClosed { id, reason }` com `reason ∈ { Replied | Timeout | Deadlock }` — apendado
  quando o ciclo fecha.

Aditivos, mesma política do **ADR 0001**: o reducer `apply` ganha braços novos; os tipos
antigos não mudam de versão (sem upcasting); o único consumidor externo (`app/lina-gpui`)
só **constrói** variantes (`append`), nunca faz `match` exaustivo → adicionar variantes
**não quebra o app**.

### 3. Tabela tipada `pending: HashMap<id, AwaitTicket>` no supervisor

`AwaitTicket { waiter, target, opened_ms, root_cause_id }`. O lifecycle vira:

1. **Open:** rota `--await` `A→B` aprovada → `begin_await(A, B)` **persiste a aresta** (não
   mais rollback) + `append(AwaitOpened{...})` + insere em `pending[id]`.
2. **Reply:** B responde com um envelope `reply_to = id`. Ao drenar, o supervisor acha
   `pending[id]`, chama `end_await(A)`, `append(AwaitClosed{ reason: Replied })`, remove de
   `pending`.
3. **Timeout:** sweep periódico (relógio lógico, alinhado ao W0-10 fim-de-resposta) fecha
   tickets vencidos → `end_await(A)` + `append(AwaitClosed{ reason: Timeout })`.
4. **Deadlock:** a pré-checagem anti-deadlock continua **rejeitando antes** de abrir o
   ticket (nada é apendado; vira `RouteOutcome::Deadlock` + `RouteBlocked` por ADR 0003).

### 4. Payload NUNCA é duplicado na bus nem no log

Continua **só no envelope/transcript** (PTY). `reply_to` é a única correlação que trafega.
Nada de carregar texto de tarefa (até `MAX_MSG_BYTES` = 256 KiB) no `DomainEvent` nem no
`BusEvent`.

## Alternativas consideradas

- **Opção A — alargar `BusEvent::Message` para carregar o envelope inteiro** (e `reply` viraria
  `BusEvent::Reply{reply_to,from}`). **Rejeitada:** a bus é `broadcast::Sender<BusEvent>`, que
  **clona a TODOS os subscribers** a cada mensagem; o payload vai até **256 KiB**
  (`MAX_MSG_BYTES`) → broadcastar isso a N subscribers é **caro** e **vaza payload
  (possivelmente sensível) a todo subscriber/espelho** (fere invariante #2 local-first/minimização).
  Pior: **funde dois contratos que devem versionar separados** (`BusEvent` × `lina/msg@1`) —
  mudar um quebra o outro, fechando uma âncora de continuidade.
- **Modelar `reply` só em memória (sem `DomainEvent`).** Rejeitada: invisível headless (falha
  o AC do design §3 e o invariante #4) e perde-se em crash — exatamente a classe de problema
  que a auditoria A6 levanta.
- **Manter o rollback imediato e nunca persistir a aresta.** Rejeitada: é a meia-medida atual;
  o `--await` não bloqueia de fato, o anti-deadlock só enxerga arestas de outra origem, e o
  fan-out `--await` fica com detecção fraca (auditoria M5/H3). O lifecycle é o que torna a
  aresta segura de persistir.

## Consequências

- **(+)** `await/reply` 100% observável e reconstruível do log (invariante #4); teste de
  replay prova o estado de `pending` a partir de `AwaitOpened`/`AwaitClosed`.
- **(+)** `end_await` no `reply` elimina **arestas órfãs** no wait-for-graph → conserta a
  fonte de falsos deadlocks que hoje força o rollback (auditoria M5/H3): a aresta passa a
  viver de verdade até o fechamento.
- **(+)** Bus permanece **O(1)** e o payload sensível **não faz fan-out**; `BusEvent` e
  `lina/msg@1` seguem versionando em separado (âncora preservada).
- **(−)** Subscribers que querem o envelope completo casam `id ↔ origem` (plumbing pequeno —
  a entrega já é fora da bus).
- **(−)** O supervisor ganha estado de `await` (`pending`) e um **sweep de timeout** (timer
  lógico); é derivável do log, mas adiciona uma rotina periódica (compatível com o W0-10).
- **(−)** `ProjectedState` passa a refletir `await` aberto/fechado (campo `#[serde(default)]`,
  como em ADR 0001): snapshots antigos desserializam sem awaits; o fingerprint determinístico
  passa a incluí-los (esperado — é parte do estado).

## Critério de verificação (headless, via event log)

- **AC-0002.1 (open):** `A --await--> B` aprovado → `AwaitOpened{ id, waiter:A, target:B,
  root_cause_id }` no log e aresta `A→B` viva no wait-for-graph.
- **AC-0002.2 (reply fecha + libera):** envelope de B com `reply_to=id` → `AwaitClosed{ id,
  reason: Replied }` no log, `pending` sem `id`, e um **novo** `A --await--> B` volta a ser
  permitido (aresta zerada).
- **AC-0002.3 (timeout):** sem reply até o teto → `AwaitClosed{ id, reason: Timeout }`.
- **AC-0002.4 (bus enxuta):** o `BusEvent::Message` emitido **não** contém `payload`; o
  payload aparece apenas no transcript/PTY do alvo.
