# Despacho F3-1 · CORE-Plan → Terminal H  [ADR 0032]

## CONTEXTO
Rodada F3-1, épico 39. O **contrato** (Terminal B) já adicionou os 4 campos ao `PlanItem` (`goal_id`/`parents`/`acceptance`/`budget_tokens`) e o `apply_item_attributed`. Sua fatia é a **GUARDA anti-race** e o **round-trip do serializer**. **Fonte:** spec 52 §2 (l.160-228). LEIA §2. ADR 0032 (`docs/adr/0032-parents-no-plano.md`) — **aceito**.

## FUNÇÃO
Dev Core. effort **Medium**.

## FRONTEIRA (só este)
`crates/lina-core/src/plan.rs` — `try_claim` (guarda) + serializer/parser de linha (round-trip) + `PlanError`. **NÃO toque** `events.rs` (congelado pelo contrato), nem o struct `PlanItem`/`apply_item_attributed` (o contrato já fez), nem `router.rs`/`goal.rs` (CORE-Goal).

## DIRECIONAMENTO

**`PlanError::ParentsNotDone { id: String, pending: Vec<String> }`** (spec 52 §2 l.222-226) — novo erro, estilo `thiserror` das vizinhas (l.110+).

**`try_claim` (l.185):** ANTES da mutação, se o item tem `parents` não-todos-`Done` → recusa com `ParentsNotDone{ pending }` (mesmo local/estilo da guarda `AlreadyOwned` l.189-197). É o que torna `parents:` **dado que bloqueia despacho**, não prosa. Mede-se: `RouteOutcome::PlanRejected` ao tentar claim/despachar item prematuro.

**Serializer/parser rígido (l.261+, `PLAN_SCHEMA_V1`):** os 4 campos novos entram como **sufixos opcionais** na linha do item, na gramática `FIELD_SEP = " :: "` (l.27): `@goal:<id>`, `@parents:T1,T2`, `@accept:<...>`, `@budget:N`. **Ausentes → defaults; a linha legada faz round-trip byte-a-byte IDÊNTICO** (round-trip é critério, não opção). `acceptance` (lista de `AcceptanceCriterion`) — serialize compacto e legível; na dúvida do formato, escale ao Maestro (não invente um que o parser não releia).

## OBJETIVO
`parents:` bloqueia o despacho até os pais ficarem `Done`; o plano serializado faz round-trip byte-a-byte com o log legado.

## RESULTADO ESPERADO (gate F3-1 f)
- `parents:["T1"]` bloqueia `claim T2` até `T1` virar `Done` → `PlanError::ParentsNotDone` (teste).
- Round-trip: linha legada (sem os campos) → parse → serialize → **idêntica**; linha com `@parents`/`@goal`/`@accept`/`@budget` → round-trip exato.
- `cargo test -p lina-core` verde + `cargo clippy -p lina-core -- -D warnings` + `cargo fmt -p` (só `plan.rs`). **NÃO commite** — reporte `PRONTO`/`BLOCKED` ao Maestro @Terminal A.
