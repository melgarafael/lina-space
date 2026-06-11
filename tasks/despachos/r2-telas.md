# DESPACHO r2-f1-2-6 — Especialista em Telas
**id:** `f1-2-6` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md`

## F1-2-6 · Canvas navegável por teclado (fonte: `tasks/epico-f1/ondas-2-4.md` linhas 120-137)
Focus manager do canvas: Tab entra como widget composto; setas navegam em ordem espacial estável; Enter entra no terminal, Esc volta ao canvas; zoom-to-focus respeitando `reduce_motion_effective()` (fonte única W4-6); focus ring com token do design system (contraste ≥3:1 nos 2 temas); roving tabindex no AccessKit (1 nó focável por vez). Critérios: teste headless da ordem de navegação + lógica do node AccessKit; reduce-motion = salto sem animação; o percurso só-teclado GRAVADO fica p/ a tela do fundador (escreva o roteiro em `tasks/epico-f1/roteiro-f1-2-6.md`).

## Fronteira (sua)
`app/lina-gpui/src/canvas.rs` · NOVO `app/lina-gpui/src/canvas_focus.rs` (o grosso aqui) · `a11y.rs` toques mínimos (você já é o dono dele).
**⛔ NÃO toque:** `main.rs`/`agent_modal.rs`/`attention_ui.rs` (time EXTERNO está NELES agora) — wire de main.rs vira PEDIDO DE COSTURA na entrega · bridge.rs (Bug Finder) · crates/.
Entrega: `tasks/epico-f1/.entrega-f1-2-6.md` · Marcador: `.iniciado-f1-2-6`.
