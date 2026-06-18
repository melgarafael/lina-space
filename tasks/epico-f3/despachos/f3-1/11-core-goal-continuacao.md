# Re-despacho F3-1 · CORE-Goal continuação (ReviewVerdict{Fail} informado) → Terminal B

> **Veredito do Maestro (juiz ≠ executor — o próprio loop da fase, aplicado a nós).** Validei o core de fora: `lina-core` compila, **379/380 testes verdes**. Motor EXCELENTE (projeção `goal.rs` + juiz estrutural zero-LLM `run_acceptance` + laço `GoalAchieved`/`GoalEscalated`/turn-budget). Mas a fatia **não está pronta** — 3 itens para o PRONTO. É a SUA fatia (`router.rs`/`goal.rs`); não toquei.

## (1) BUG — enforcement desligado · `router.rs:2490` · `defect_class: criterio_nao_cumprido`
`emit_review_verdict` tem **`if false && reviewer == target`** → o enforcement `reviewer != target` está **DESLIGADO**. O teste `gate_d_review_verdict_recusa_auto_avaliacao` (`router.rs:6727`) **FALHA** (espera `None` para `(dev, dev)`).
- Entendo o raciocínio (doc l.69: `STRUCTURAL_JUDGE` sentinela `!= target` por construção). **Mas** defense-in-depth + gate (d) + meu adendo exigem a recusa **EXPLÍCITA**: cobre o caso de **nó QA real** (reviewer = nó QA, não o sentinela) e a **forja**. Não confiar só na construção é a doutrina da casa.
- **FIX:** `if reviewer == target {` (remova o `false && `). `evidence`: a saída do teste gate_d.

## (2) FALTA — handlers de intent · `router.rs` (dispatch, espelhe `plan.claim` em `:1851`) · `defect_class: criterio_nao_cumprido`
A CLI (Terminal I, **já PRONTA**) enfileira `goal.define`/`goal.interpret`/`goal.confirm`/`plan.add`/`plan.seed`, mas o router **NÃO os processa** (só existem `plan.claim`/`plan.check`/`params.set`/`effort.assign`). Sem os handlers, o e2e não fecha. Implemente (escritor único = supervisor; cunhe/carimbe SERVER-SIDE):
- **`goal.define`** → cunha `goal_id` (supervisor, jamais do payload) + emite `GoalDefined`.
- **`goal.interpret`** → `GoalInterpreted`.
- **`goal.confirm`** → **CARIMBA `by` via `derive_root_hops`** (`human` se `hops==0`, senão `sender`; JAMAIS do payload) + `GoalConfirmed`. ⚠️ gate de segurança — a QA vai testar forja de `by`.
- **`plan.add`** → `PlanItemAdded` + `PlanItemAttributed` (promove `seed_plan_item`).
- **`plan.seed`** → `GoalDecomposed` + N `PlanItemAttributed`.

## (3) FALTA — gate a-e em testes de integração · `defect_class: criterio_nao_cumprido`
O e2e do gate de saída F3-1: `define → interpret → confirm → seed → loop`:
- **(a)** replay byte-a-byte após matar/reabrir (Goal + Plan idênticos).
- **(b)** 2 itens, critério exit 0 → exatamente um `GoalAchieved`, zero despacho depois.
- **(c)** critério exit 1 → após 3 `Fail` → `GoalEscalated{turn_budget_exhausted}`.
- **(d)** `reviewer != target` (o teste já existe — só ativar o fix (1)).
- **(e)** re-spawn no Fail com effort estritamente maior (`Effort: Ord`).

## CRITÉRIO DE ACEITE (o PRONTO)
`cargo test -p lina-core` **100% VERDE (0 falhas)** + `clippy -p lina-core -- -D warnings` + `cargo fmt -p` (só seus arquivos). Reporte `PRONTO` + os comandos rodados. Quando você fechar (1) e (2), eu sinalizo a QA (Terminal R, em standby) para a suíte adversarial de segurança.
