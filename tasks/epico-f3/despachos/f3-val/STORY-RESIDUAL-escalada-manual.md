# Story residual F3-2 · Recuperação manual pós-escalada (GAP-1 de F3-VAL)

> **Origem:** achado do Terminal G na rodada F3-VAL, validado de fora pelo Maestro. **Decisão do fundador (2026-06-20):** porta aberta — registrar, não fechar agora (o critério central F3-1/F3-2 está 100%; o fix cruza `main.rs` com WIP não-commitado do Terminal A + exige design de intent novo no core).

## O gap (preciso, com evidência)

Quando uma Goal atinge `GoalEscalated{turn_budget_exhausted}` (3 `Fail`), o card mostra os CTAs **"Reforçar o time"** / **"Olhar com você"** (`goal_card.rs::cta_labels`, fase `Escalated`). Mas o call-site `main.rs::render_goals_panel` (linhas ~2178-2183) liga **os mesmos** `on_confirm`/`on_correct` da fase `Interpreted`:
- **"Reforçar o time"** → `confirm_goal` → `push_human_intent("goal.confirm")`. Em goal `Escalated`, `goal.confirm` re-dispara a montagem do time (router.rs:116) mas **não sobe o effort** — não é o "reforço" prometido (effort↑).
- **"Olhar com você"** → `correct_goal` → abre o editor de re-interpretação. Tangencial à label.

**Resultado:** label ≠ ação. O loop automático e a escala de effort por `Fail` (o critério central) estão 100% e fora deste gap.

## O que a story precisa decidir/entregar

1. **Design (decisão de produto):** o que "Reforçar o time" faz exatamente? Proposta: intent novo `goal.reinforce` no core → re-arma o OUTER loop concedendo +N iterações **com effort de partida elevado** (topo da `EFFORT_LADDER`), registrando um evento (ex.: `GoalReinforced{by, extra_budget, effort_floor}`) — gesto humano, `by` server-side (ADR 0036/0007), nunca autoridade forjável. E "Olhar com você": pausa explícita / co-trabalho (ou reusar re-interpretação com label honesta).
2. **CORE (router.rs/goal.rs):** handler de `goal.reinforce`; a projeção reflete o re-arme; replay idempotente; turn-budget volta a valer.
3. **UI (goal_card.rs + main.rs):** handlers dedicados `on_reinforce`/`on_look_together` para a fase `Escalated` (parar de reusar confirm/correct). ⚠️ **costura em `main.rs` — coordenar com o dono do WIP (Terminal A) antes de tocar.**
4. **QA:** teste de aceite — na fase `Escalated`, "Reforçar o time" sobe o effort (Ord no log) e re-arma o budget; segurança (`by` server-side; reforço não vira autoridade).

## Fix intermediário barato (se priorizar honestidade antes do design completo)
Só `goal_card.rs` (território do G, **não** toca `main.rs`/core): ajustar `cta_labels(Escalated)` para refletir o que as ações fazem HOJE — exige antes **confirmar empiricamente** o comportamento de `goal.confirm` numa goal `Escalated` (re-arma de verdade ou é no-op?). Não fazer no escuro.
