# DESPACHO r3-f1-2-5 — Especialista em Telas
**id:** `f1-2-5` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md` (fronteiras v2!)

## F1-2-5 · Faxina: resíduos de UI de dev/teste fora da superfície (fonte: `ondas-2-4.md` linhas 102-118)
Inventário de TODO elemento visível de dev/teste no shell (comece por `dev_tools.rs` e irmãos + grep por labels técnicos/UUIDs/métricas cruas em strings renderizadas); para cada item UMA decisão: remover · esconder atrás de `LINA_DEV=1` · promover a feature legível. Teste que o painel dev NÃO monta sem a flag (não-vacuoso: com a flag, monta). Inventário item-a-item em `tasks/epico-f1/inventario-f1-2-5.md` (o fundador percorre no gate). A observabilidade INTERNA (logs/eprintln) fica — só sai da SUPERFÍCIE.

## Fronteira (sua)
`app/lina-gpui/src/dev_tools.rs` · `canvas.rs`/`canvas_focus.rs` (seus) · `dashboard.rs` · `gallery.rs`/`inspector.rs`/`persistence_ui.rs` se tiverem resíduo · testes.
**⛔ NÃO toque:** `main.rs`/`agent_modal.rs`/`attention_ui.rs` (EXTERNO — se o resíduo morar lá, liste no inventário como COSTURA com o hunk proposto, não edite) · bridge.rs (Bug Finder) · crates/.
Entrega: `tasks/epico-f1/.entrega-f1-2-5.md` · Marcador: `.iniciado-f1-2-5`.
