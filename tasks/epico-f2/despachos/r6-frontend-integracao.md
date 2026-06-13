# Despacho r6 — Terminal C (FRONTEND): integrações no main.rs — toast_stack + migração Badge (P0!) + montagem da toolbar

## CONTEXTO
Rodada 6. Você é DONO ÚNICO de main.rs de novo. Três integrações, com UMA prioridade soberana vinda da revisão de conformidade (PASS 90): **P0 = o card `needs_human` (main.rs:3774-3780) migra PRIMEIRO para Badge::needs_you()** — é o caso Assertive por excelência (gate humano) e hoje só fala ao focar; a r5 reforçou o visual e ampliou a assimetria justo nele. Specs: integração do toast_stack na entrega do E (r5, msg + ui/toast.rs docs) · migração chips/dots na mesma · toolbar em tasks/epico-f2/spec-f2-2-5-toolbar.md §anchor. Notas: tasks/epico-f2/notas-r6.md.

## FUNÇÃO
Frontend/integrador — fronteira: main.rs + attention_ui.rs (chips) + sidebar.rs SE a migração tocar (avise o E — ele está em palette.rs/ui/node_toolbar.rs).

## DIRECIONAMENTO
1. **P0 — needs_human→Badge::needs_you()** no card (dot/borda visuais da r5 FICAM; o Badge soma a VOZ): id estável por node-id (troca anuncia); o aria_label legado do W4-6 não pode duplicar o anúncio (um canal só — decida e documente).
2. **Migração dos demais chips/dots de estado** → ui::Badge (tons semânticos; texto+glifo+cor); remova o #[allow(unused_imports)] do ui.rs ao consumir.
3. **toast_stack**: campo no WorkspaceView + tick no heartbeat + render visible() empilhado + ArchiveToast migra com Desfazer (8s) + DROPA o announce() manual (o ToastView já anuncia — sem eco duplo).
4. **Montagem da toolbar** (quando o E entregar o componente — comece pelo 1-3): anchor screen-space no card focado conforme spec §anchor (Zone::Focus, flip sob topbar, sem ×zoom).
5. Validação: suíte + catraca (só desce) + clippy -D + fmt + WCAG gate. [PROF] sem regressão (badge/toast são poucos nós). Não commite. Reporte E continue.

## OBJETIVO / RESULTADO ESPERADO
O estado mais crítico do produto ganha VOZ sem foco; avisos integrados; toolbar montada. Roteiro de tela incremental da rodada. PRONTO:/BLOCKED:.

## Tentativas anteriores
Nenhuma.
