# Despacho F3-1 · CLI (verbos da Goal) → Terminal I

## CONTEXTO
Rodada F3-1, épico 39. Hoje `lina plan` **não tem escritor** (`lina.rs:1026-1036`: só `read|claim|check`). Esta fatia cria a **superfície CLI da Goal** + o escritor de plano que falta. **Fonte:** spec 52 §Superfície (l.315-325) + §Confirmação por autonomia (l.327-337). LEIA. Padrão de enfileirar intent: `run_plan_intent` (`lina.rs:1060`).

## FUNÇÃO
Dev Core + LLM Eng. effort **Medium**.

## FRONTEIRA (só estes)
`crates/lina-bootstrap/src/bin/lina.rs` (verbos) + `crates/lina-bootstrap/tests/doctrine_verbs.rs` (consistência). **NÃO toque** `router.rs`/`events.rs` — o **supervisor é o escritor único**; o bin SÓ enfileira o envelope/intent (como `run_plan_intent`). O HANDLER dos intents vive no router (fatia CORE-Goal/Terminal B).

## DIRECIONAMENTO [F3-1-6]
Verbos novos (todos enfileiram intent; o supervisor cunha ids e emite os eventos):
- `lina goal define "<statement>" [--budget <tokens>] [--accept "<criterio>"]...` → intent **`goal.define`** → `GoalDefined` (supervisor cunha `goal_id`; NÃO decompõe — espera interpretação).
- `lina goal interpret <goal_id> --understanding "<...>" --strategy "<...>" [--team A,B,C] [--accept ...]` → intent **`goal.interpret`** → `GoalInterpreted` (o Maestro/Tradutor devolve o entendimento ANTES de executar — doc-fonte 11).
- `lina goal status <goal_id> [--json]` → leitura pura da projeção Goal (estilo `lina retro`/`lina check`): ciclo, itens (parents/acceptance), vereditos, iteração, budget.
- `lina plan add <id> "<desc>" [--goal <goal_id>] [--parents T1,T2] [--accept "<>"] [--budget N]` → intent **`plan.add`** → `PlanItemAdded` + `PlanItemAttributed` (promove `seed_plan_item` `router.rs:1961`, hoje só-teste, a verbo real).
- `lina plan seed <goal_id>` → intent **`plan.seed`** → `GoalDecomposed` + N `PlanItemAttributed` (decomposição do `GoalInterpreted` confirmado).

**Gate por autonomia (spec 52 §Confirmação l.327-337):** em `manual` exige gate por passo; em `assistido` propõe→confirma; em `autonomo` auto. **Consistência verbos × `lina --help` (ZERO verbo fantasma)** — atualize `doctrine_verbs.rs` (memória: a catraca conta o que existe). `lina plan read|claim|check` seguem intactos.

## OBJETIVO
A superfície CLI da Goal — o escritor de plano + os verbos `goal` — testável pelo ENFILEIRAMENTO correto do intent.

## RESULTADO ESPERADO
- `lina goal define/interpret/status` + `lina plan add/seed` enfileiram o intent certo (assertável isolado, sem depender do handler).
- `doctrine_verbs.rs` verde (sem verbo fantasma); `cargo test -p lina-bootstrap` verde + `cargo clippy -p lina-bootstrap -- -D warnings` + `fmt`. **NÃO commite** — reporte `PRONTO`/`BLOCKED` ao Maestro @Terminal A.
- **DEP:** os nomes de intent (`goal.define`/`goal.interpret`/`plan.add`/`plan.seed`) são contrato comum com a fatia CORE-Goal (Terminal B) — use EXATAMENTE esses; o e2e o Maestro valida na integração.
