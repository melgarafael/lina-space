# DESPACHO — F2-3-5 · Zoom inteligente (fit / selection / LOD) · fatia `f2-3-5`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
A câmera existe (`app/lina-gpui/src/bridge.rs` ~2860): `Camera{pan,zoom}`, `world_to_screen`/
`screen_to_world` (zoom*w+pan), `zoom_by(cursor,factor)` clampa `[0.4, 2.0]`, `reveal` auto-pan. Cards
têm `x/y` (mundo) e tamanho `CARD_W=680 × CARD_H=500`. Hoje só há zoom sob cursor (scroll) e Centralizar
(reset). Falta enquadrar tudo e focar a seleção.

**FUNÇÃO** · Você é o dono da matemática de enquadramento e dos níveis de detalhe (LOD) do canvas.

**DIRECIONAMENTO** · Fronteira (SÓ estes): **crie `app/lina-gpui/src/canvas_zoom.rs`** (módulo novo).
NÃO toque `main.rs` nem `bridge.rs` (a tecla e a aplicação no Camera são costura do Maestro — entregue o
diff; se precisar de um método novo no Camera, descreva-o como pedido de costura, NÃO edite bridge.rs).
Entregue funções PURAS testáveis:
- `zoom_to_fit(cards_bounds: Rect, viewport: (f32,f32), padding, zoom_min, zoom_max) -> (pan,zoom)`:
  dado o bounding box de TODOS os cards (em mundo) + a viewport, calcula pan+zoom que enquadra tudo com
  respiro, clampado em [0.4, 2.0]. Caso 0 cards = no-op (retorna câmera atual/home).
- `zoom_to_selection(sel_bounds: Rect, viewport, ...) -> (pan,zoom)`: idem para a seleção (1+ cards).
- `bounds_of(cards: &[(x,y)], card_w, card_h) -> Rect`: union dos retângulos dos cards.
- **LOD de 2 degraus com histerese:** `lod_for(zoom, last_lod) -> Lod` (ex.: Full | Compact) com
  HISTERESE (limiares de subida ≠ descida) para não piscar o nível perto da fronteira. LOD **degrada
  fidelidade, NUNCA remove a camada de composição** (critério do épico: R3/slot portal preservado).
Invariantes: **a câmera nunca anda sem gesto** (suas funções só CALCULAM; quem aplica é o handler de
tecla, costura do Maestro). Premium=latência: zoom-to-fit é gesto pontual (ok animar suave), LOD é
estado derivado (sem animação por frame de zoom). Testes não-vacuosos: fit enquadra (todos os cards caem
dentro da viewport pós-transform); clamp respeitado; histerese não oscila num zoom de fronteira.

**OBJETIVO** · O fundador aperta uma tecla e vê TODOS os terminais enquadrados (nunca perdido no
vazio), ou foca a seleção; e ao afastar o zoom os cards simplificam sem sumir.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-5.md`: arquivo:linha do
`canvas_zoom.rs`, evidência (`cd app/lina-gpui && cargo test`/`clippy --all-targets -D warnings`/`fmt`
exit DIRETO + nº testes + suíte completa incl. token_ratchet), **diff de costura main.rs** (tecla
zoom-to-fit / zoom-to-selection aplicando seu cálculo no self.camera + cx.notify; uso do LOD no render do
card) e eventual pedido de método novo no Camera (bridge.rs, dono = Maestro), achados. Última linha
`PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
