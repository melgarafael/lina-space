# Despacho F3-1 · Rα-0 — CONTRATO de eventos da Goal  →  Terminal B (Ultra Code)

> **Esta é a fatia-fundação. As outras 4 frentes esperam ela commitada.** Você é o **DONO ÚNICO de `events.rs` nesta rodada** (precedente cravado em `events.rs:869` — F1-3-7: dono único batcheia TODAS as variantes da onda, eliminando colisão de merge). Depois deste contrato, **ninguém mais toca `events.rs`** até o fim da rodada.

## CONTEXTO
Rodada F3-1 (Goal-and-Loop) do épico `39 - Epico Fase 3` (vault). É onde o Lina deixa de **executar pedidos** e passa a **perseguir metas**. Este contrato materializa a Goal como entidade event-sourced. **Fonte da verdade:** vault `52 - SPEC Goal-and-Loop - A Meta como Primitiva`. **LEIA, antes de codar:** §1 (eventos da Goal, l.40-148), §2 (PlanItem estendido + PlanItemAttributed, l.160-228), §3 (ReviewVerdict + Verdict, l.230-264), §4 (goal_id no spawn, l.282-301). Comando: `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/52 - SPEC Goal-and-Loop - A Meta como Primitiva.md"`.

**Regra que atravessa tudo (ADR 0007/0033):** `interpretation`/`evidence`/`by`/`reviewer`/`effort` são **DADO transportado, JAMAIS autoridade**. Você só DEFINE o schema aqui; a garantia server-side é das fatias de lógica/segurança. Mas os doc-comments devem registrar isso onde a spec marca.

## FUNÇÃO
Dev Core. O ARQUITETO (Terminal A) delegou o contrato; você materializa o schema EXATO da spec 52. effort recomendado: **High** (decisão de tipos que tudo herda).

## FRONTEIRA DE ARQUIVO (só estes — território disjunto)
`crates/lina-core/src/events.rs` · `crates/lina-core/src/plan.rs` (só struct `PlanItem` + novo `apply_item_attributed`; **NÃO** mexa em `try_claim`/serializer — é da fatia CORE-Plan) · `crates/lina-core/src/goal.rs` **(novo, esqueleto)** · `crates/lina-core/src/lib.rs` (mod + re-export).

## DIRECIONAMENTO

**1) `events.rs` — TIPOS novos** (após `EffortOrigin`, ~l.122):
- `pub struct AcceptanceCriterion { pub desc: String, #[serde(default)] pub check_kind: CheckKind, #[serde(default)] pub check_arg: Option<String> }` — derive `Clone, Serialize, Deserialize, PartialEq, Eq`.
- `pub enum CheckKind { #[default] HumanReview, Command, FileExists, TestPass }` — derive `Clone, Default, Serialize, Deserialize, PartialEq, Eq`.
- `pub enum Verdict { Pass, Fail }` — derive `Clone, Copy, Serialize, Deserialize, PartialEq, Eq`.
- Schema EXATO: spec 52 §1 l.124-148 e §3 l.262-263. Doc-comments no estilo das vizinhas (`Effort`/`EffortOrigin`).

**2) `events.rs` — 8 VARIANTES no enum `DomainEvent`** (junto aos eventos de plano, após `PlanChecked` ~l.533). Variante nova = **sem `serde(default)` nos campos** (replay antigo nunca a encontra — precedente `SpawnRequested`), EXCETO os campos que a spec marca `#[serde(default)]`:
- `GoalDefined { goal_id: String, statement: String, root_cause_id: String, origin: String, budget_tokens: u64 }`
- `GoalInterpreted { goal_id: String, interpretation: String, strategy: String, proposed_team: Vec<String>, acceptance_criteria: Vec<AcceptanceCriterion> }`
- `GoalConfirmed { goal_id: String, by: NodeId, #[serde(default)] amended_statement: Option<String> }`
- `GoalDecomposed { goal_id: String, plan_items: Vec<String> }`
- `GoalAchieved { goal_id: String, iterations: u32, verdict_event_id: String }`
- `GoalEscalated { goal_id: String, reason: String }`
- `PlanItemAttributed { item: String, goal_id: Option<String>, parents: Vec<String>, acceptance: Vec<AcceptanceCriterion>, budget_tokens: u64 }`
- `ReviewVerdict { goal_id: String, plan_item: String, target: NodeId, reviewer: NodeId, verdict: Verdict, #[serde(default)] evidence: String, #[serde(default)] defect_class: String, #[serde(default)] iteration: u32 }`
- Schema EXATO: spec 52 §1 l.44-111, §2 l.192-202, §3 l.240-260. Cada variante com doc-comment estilo-da-casa (fase P3, fonte, e "dado jamais autoridade" em `GoalConfirmed.by`/`ReviewVerdict.reviewer` — onde a spec crava).

**3) `events.rs` — `SpawnRequested` ganha** `#[serde(default)] goal_id: Option<String>` (após `effort`, ~l.840): "a Goal que JUSTIFICA o recurso — auditoria" (spec 52 §4 l.295). ADITIVO.

**4) `events.rs` — `fn kind`** (~l.996, após o braço `PlanChecked`): 8 braços `DomainEvent::X { .. } => "X"`.

**5) `events.rs` — `apply`** (~l.1291+):
- `PlanItemAttributed` → **EFETIVO** (junto aos `PlanItem*`, ~l.1296): `state.plan.apply_item_attributed(item.clone(), goal_id.clone(), parents.clone(), acceptance.clone(), *budget_tokens);`
- As 6 variantes Goal + `ReviewVerdict` → **META (no-op)**: adicione ao bloco de no-op da projeção do canvas (~l.1358, junto a `SystemParamsChanged`/`EffortAssigned`), com comentário: *"F3-1: o ciclo da Goal é META no apply do canvas; a projeção `Goal` (goal.rs) reconstrói por replay varrendo por `goal_id` — padrão CostLedger/ParamsLedger; sem efeito na projeção do canvas."*

**6) `plan.rs` — struct `PlanItem`** (l.89) ganha 4 campos `#[serde(default)]`: `goal_id: Option<String>`, `parents: Vec<String>`, `acceptance: Vec<AcceptanceCriterion>`, `budget_tokens: u64` (importe `AcceptanceCriterion` de `crate::events` ou `super`). `PlanItem` **NÃO tem `Default`** e há **5 construções** (l.230, 421, 487, 493, 499, 505) — atualize cada uma com os campos no equivalente-default (`None`/`Vec::new()`/`0`). Adicione `pub(crate) fn apply_item_attributed(&mut self, item: &str, goal_id: Option<String>, parents: Vec<String>, acceptance: Vec<AcceptanceCriterion>, budget_tokens: u64)` espelhando `apply_item_added` (l.228): acha o item por id (`find_mut`) e seta os 4 campos (spec 52 §2 l.204-218). **NÃO** toque `try_claim`/serializer (fatia CORE-Plan).

**7) `goal.rs` — NOVO, ESQUELETO da projeção** (a lógica vem na sua fatia seguinte): `pub struct Goal { pub goal_id: String, pub statement: String, pub phase: GoalPhase, pub interpretation: Option<String>, pub acceptance: Vec<AcceptanceCriterion>, pub items: Vec<String>, pub iterations: u32 }` + `pub enum GoalPhase { Defined, Interpreted, Confirmed, Decomposed, InLoop, Achieved, Escalated }` + `pub fn project_goals(events: &[EventRecord]) -> Vec<Goal>` com corpo mínimo `Vec::new()` + comentário *"F3-1-1: projeção real (varrer por goal_id) na fatia CORE-Goal-lógica."*. Derives/docs no estilo da casa (use o tipo `EventRecord` de `crate::events`).

**8) `lib.rs`** — `mod goal;` + `pub use goal::{Goal, GoalPhase};` (após `plan`, ~l.83); adicione `AcceptanceCriterion, CheckKind, Verdict` (e as 8 variantes não — variantes vivem em `DomainEvent`) ao `pub use events::{...}` (~l.35) se ainda não exportados via `DomainEvent`.

## OBJETIVO
O **contrato-base** da Goal: todas as variantes + tipos + projeção-esqueleto, **compilando verde**, que as 4 frentes (CORE-Plan, CLI, UI, QA) consomem sem tocar `events.rs`.

## RESULTADO ESPERADO (roda e se mede)
- `cargo build -p lina-core` **verde**; `cargo test -p lina-core` **verde** (replay de log F0/F1/F2 carrega — campos `serde(default)`; nenhum teste existente regride).
- `cargo clippy -p lina-core -- -D warnings` **limpo**; `cargo fmt -p lina-core` (só seus arquivos — memória: *fmt em árvore compartilhada*).
- **NÃO commite.** Reporte ao Maestro: `PRONTO: contrato Rα-0 — <N> testes, clippy 0, <arquivos>`. O Maestro valida de fora (exit codes diretos) e commita por fatia.
- Conflito spec↔código? **`BLOCKED: <o quê>`** e escale ao Maestro — **NÃO adapte em silêncio** (a responsabilidade de fidelidade à spec é compartilhada; na dúvida, pergunte).
