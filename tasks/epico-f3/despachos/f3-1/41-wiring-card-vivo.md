# Despacho F3-1 · WIRING do card da Goal (projeção viva) → Terminal G

## CONTEXTO
O core (projeção `project_goals` + handlers de intent) e a CLI (verbos) estão **COMMITADOS** (core: `0b3cc13`; CLI: `541e9bc`). Agora ligue o seu `GoalCard` (PRONTO, hoje com `mock_goal()`) à projeção **VIVA** — é o 1º call-site real que derruba o `allow(dead_code)` e habilita o **gate (h)** na tela do fundador. Exports confirmados (lib.rs:88, params.rs:513).

## FUNÇÃO
Dev Frontend (G). effort Medium. **FRONTEIRA:** `app/lina-gpui/` (bridge/canvas/main) — só app, ZERO `crates/`.

## DIRECIONAMENTO
1. **Goals vivas:** `lina_core::project_goals(&store.events()?)` → `Vec<Goal>` (já exportado em lib.rs:88). O bridge já tem o `EventStore`.
2. **Render:** um `GoalCard` para cada Goal a exibir no canvas — decida o critério de exibição com gosto (ex.: Goals em andamento; o card já humaniza a fase e narra a escalada). Onde aparece (painel/overlay no canvas vs card no canvas) é sua decisão de design — identidade F2, sem jargão (já garantido no componente).
3. **`iteration_budget`** = `goal_max_iterations` do `SystemParams` vivo: `lina_core::resolve_from_store(&store)…router_config.goal_max_iterations` (params.rs:513).
4. **`on_confirm`** → enfileira o intent **`goal.confirm`** pelo mesmo funil que a CLI usa (`run_plan_intent`/o supervisor carimba `by` server-side — você só dispara o intent). **`on_correct`** → mínimo: re-abrir o fluxo de `goal.interpret`.
5. Remova o `allow(dead_code)` (agora há call-site real).

## RESULTADO ESPERADO
- O card renderiza uma Goal **VIVA** (não `mock_goal`) no app rodando.
- `cargo test --manifest-path app/lina-gpui/Cargo.toml` verde (catraca token_ratchet **intacta**) + `clippy --all-targets -D warnings` + `fmt`.
- **Screen-ready** para a sessão de tela do fundador (gate h). **NÃO commite** — reporte PRONTO; eu valido, commito e conduzo o e2e na tela.
- Faltou um getter/export do core? Me avise (Maestro) que eu abro a costura — não toque `crates/`.
