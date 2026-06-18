# Despacho F3-1 · CORE-Goal (lógica) → Terminal B (Ultra Code) — fatia 2, após o contrato

> Continua de Rα-0 (você fez o contrato). Agora a **LÓGICA**: a projeção da Goal e o **OUTER loop** (juiz≠executor). É o coração da fase. `events.rs` está **congelado** (você não o toca mais).

## CONTEXTO
Rodada F3-1, épico 39. **Fonte:** spec 52 §1 (projeção, l.40-148) + §3 (o loop-até-aceite, l.230-280) + §4 (effort, l.282-301). RELEIA §3 — é o miolo. **Invariante #1: o Lina roda só o OUTER loop** (observa idle, roda o juiz, re-injeta ou fecha); o INNER loop é o CLI de terceiro — **zero LLM no core** (risco #1 da fase). O juiz é (a) `CheckKind` determinístico OU (b) nó QA distinto — **nunca um "motor de avaliação" próprio**.

## FUNÇÃO
Dev Core (A2A). effort **High**.

## FRONTEIRA (só estes)
`crates/lina-core/src/goal.rs` (preencher `project_goals` + estado do ciclo) · `crates/lina-core/src/router.rs` (OUTER loop + handlers `goal.define`/`goal.interpret` + re-despacho informado + escala effort). **NÃO toque** `events.rs` (congelado), `plan.rs` (CORE-Plan), `lina.rs` (CLI).

## DIRECIONAMENTO

**F3-1-1 (projeção real em `goal.rs`):** `project_goals(events)` varre o log por `goal_id` e reconstrói `Defined → Interpreted → Confirmed → Decomposed → (ReviewVerdict)* → Achieved|Escalated` (padrão `CostLedger`/`ParamsLedger`). `goal_id`/`root_cause_id` vêm do supervisor (`derive_root_hops`), **nunca** de `msg.*`. Replay reconstrói idêntico (teste).

**F3-1-2 (executar `acceptance[]` no `PlanChecked`):** ao `PlanChecked(item)`, rodar `item.acceptance[]` por `CheckKind`: `Command`=PASS sse exit 0 **rodando sob `guard::decide`** (`GatedHard` → pede `Ask`; o juiz NÃO roda ação irreversível para "verificar" — spec 52 §1 l.142, §Seg); `FileExists`; `TestPass`. `HumanReview`=degrada honesto (exige gate). **ZERO LLM.**

**F3-1-4 (`ReviewVerdict` + laço):** `reviewer != target` **enforçado** (executor nunca se auto-avalia). Pass → se TODOS os itens da Goal Pass → exatamente um `GoalAchieved`, **nenhum** despacho depois. Fail → `ReviewVerdict{Fail, evidence(arquivo:linha OBSERVADA), defect_class, iteration:N}` → **re-despacho INFORMADO** ao mesmo nó (payload carrega `evidence`+`defect_class` — padrão `lina-dispatch`, não "tente de novo"). N atinge teto do degrau → **escala effort**: re-spawn com effort do próximo degrau de `effort_ladder` (`Effort: Ord` → "estritamente maior", doc-fonte 12).

**F3-1-5 (turn-budget + fail-open + breaker):** `goal_max_iterations` (default 3, faixa 1..=20) em `RouterConfig` = teto duro do OUTER loop → esgotou sem Pass → `GoalEscalated{reason:"turn_budget_exhausted"}` (PAUSA resumível, **nunca** mais um turno). Erro de transporte do juiz = CONTINUE (fail-open). `judge_max_parse_failures` (3) vereditos não-parseáveis seguidos → `GoalEscalated{reason:"judge_unreliable"}`.

**Handlers no router:** `goal.define` (cunha `goal_id`, emite `GoalDefined`), `goal.interpret` (`GoalInterpreted`) — enfileirados pela CLI (fatia I). Confirme comigo (Maestro) os nomes de intent: `goal.define`/`goal.interpret`/`plan.add`/`plan.seed`.

## OBJETIVO
A Goal projeta por replay; o loop fecha **só** com `Pass`; escala effort no Fail; turn-budget freia em N. O ponto [8] de Meadows mecanizado.

## RESULTADO ESPERADO (gate F3-1 a-e)
- (a) replay reconstrói o estado da Goal byte-a-byte após matar/reabrir. (b) 2 itens, critério exit 0 → um `GoalAchieved`, zero despacho depois. (c) critério exit 1 → após 3 `Fail` → `GoalEscalated{turn_budget_exhausted}`, zero iteração a mais. (d) todo `ReviewVerdict` tem `reviewer != target`; forjado recusado. (e) re-spawn no Fail carrega effort estritamente maior (`Ord` no log).
- `cargo test -p lina-core` verde + `cargo clippy -p lina-core -- -D warnings` + `cargo fmt -p` (só seus arquivos). **NÃO commite** — reporte `PRONTO`/`BLOCKED` ao Maestro @Terminal A.
