# Despacho F3-1 · QA/Red-team → Terminal R

## CONTEXTO
Rodada F3-1, épico 39. Duas missões: **(1)** a suíte adversarial que prova que **crença/interpretação/veredito/effort nunca vira autoridade** (a regra que atravessa a fase); **(2)** validar o **gate de F3-0** (badge + teste C + suíte), que está na árvore mas não foi auditado. **Fonte:** spec 52 §Segurança (resumo no épico, story F3-1-8) + família **ADR 0007** (campo de agente nunca decide identidade/ordem/autorização) + `guard.rs:209` (autônomo nunca afrouxa `GatedHard`).

## FUNÇÃO
QA/Red-team. effort **High** (carimbo server-side é sutil — re-derive no código, não confie no relato).

## FRONTEIRA (só testes)
`crates/lina-core/tests/` + `#[cfg(test)]` de `router.rs`/`goal.rs`. **LÊ produção, não altera lógica.**

## DIRECIONAMENTO

**(1) F3-1-8 — segurança da Goal (testes adversariais, TDD: falham contra forja, passam com o carimbo):**
- `GoalConfirmed.by` é carimbado pelo **dir-dono do outbox autenticado**, NUNCA do campo `from`/payload → `GoalConfirmed` forjado com `by` de outro nó é **IGNORADO**.
- `interpretation`/`evidence`/`defect_class` são **dado, jamais autoridade**: um `ReviewVerdict{Pass}` **NÃO** libera ação `GatedHard` (`guard.rs:209` segue pedindo `Ask`).
- `ReviewVerdict` com `reviewer == target` é **RECUSADO** (executor não se auto-aprova).
- A **suíte de segurança do Router permanece VERDE** (`delegation_budget_is_enforced_per_root_cause`, `spawn_cascade_requires_human_gate`) — critério implícito de toda story que toca `deliver_a2a`/`Router`/`handle_plan`/spawn.

**(2) Validação do gate F3-0 (a-e, read-only):** rode e reporte PASS/FAIL **por critério**:
- (a) `lina params set fanout_gate 8` → cascata p/ 7 alvos entrega sem gate; reverter p/ 3 → volta a gatear (conte `RouteBlocked{fanout_gated}` antes/depois).
- (b) `resolve_from_store(&store).router_config.fanout_gate == 8` por **replay** (sem reler arquivo).
- (c) badge `modelo·effort` na tela + `LINA_EFFORT` no PTY filho. (d) `grep -rniE '\beffort\b'` acha código real. (e) suíte de segurança verde com qualquer `params.json`. Teste C `weakening_a_balancing_loop_is_gated_hard` (`router.rs:5293`) verde.

## OBJETIVO
Provar empiricamente que parâmetro/crença/veredito/effort **nunca** vira autoridade; e dar ao Maestro o veredito do gate F3-0 por critério.

## RESULTADO ESPERADO
- Testes adversariais que **falham** contra a forja e **passam** com o carimbo server-side; suíte do router verde.
- Relatório do gate F3-0 (a-e + teste C): PASS/FAIL por item, com evidência (comando + saída observada — não "ok").
- `cargo test -p lina-core` verde + `cargo clippy` + `fmt`. **NÃO commite** — reporte `PRONTO`/`BLOCKED` + o relatório ao Maestro @Terminal A.
