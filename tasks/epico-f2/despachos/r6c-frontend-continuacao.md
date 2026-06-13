# Despacho r6c — Terminal C (FRONTEND): continuação da r6 (você caiu em app_reopened ~70%)

## CONTEXTO
A fatia estável da r6 está COMMITADA (`3c42a56`): seu P0 (needs_human→Badge) + render_toast migração + wiring da toolbar entraram e estão verdes. Trabalhe SOBRE essa árvore (dê pull mental). Faltam 3 itens que você não chegou a fazer antes de cair. Régua de completude do Arquiteto-revisor em tasks/epico-f2/notas-r6.md (ele VAI verificar). Strings em tasks/epico-f2/regua/strings-r6.md. Componente da toolbar pronto: src/ui/node_toolbar.rs (E).

## FUNÇÃO
Frontend/integrador — dono único de main.rs + attention_ui.rs.

## DIRECIONAMENTO (3 itens restantes)
1. **render_badge/sino do attention_ui (l.~454)** — item nº1.2 da régua: a CONTAGEM e a ESCALAÇÃO (espera-você-há-N-min) hoje são aria_label sem live → não auto-anunciam. Migrue: compõe live_region; escalação→Assertive, contagem nova→Polite. Critério: zero só-aria_label para estado que muda sem foco.
2. **toast_stack** (spec na sua entrega r5 + ui/toast.rs docs): campo no WorkspaceView + tick(now_ms) no heartbeat existente (padrão do dashboard) + render dos visible() empilhados num canto + ArchiveToast migra para Toast::with_action("Desfazer", 8s) DROPANDO o announce() manual (o ToastView já anuncia — sem eco duplo).
3. **Montagem da toolbar** (spec-f2-2-5 §anchor): overlay screen-space no card FOCADO (Zone::Focus, flip sob a topbar, sem ×zoom); consome ui::node_toolbar + palette::node_commands(NodeCtx); F1-2-6 (canvas por teclado) INTOCADO.

## VALIDAÇÃO / RESULTADO
Suíte + catraca (só desce) + guard a11y_conformance verde + clippy -D + fmt + [PROF] sem regressão. Roteiro de tela incremental. NÃO commite. Reporte E continue; PRONTO:/BLOCKED:.

## Tentativas anteriores
1ª (commit 3c42a56): P0 + render_toast + wiring entregues e commitados ANTES da queda em app_reopened. Esta retoma do que faltou — NÃO refaça o que já está na árvore.
