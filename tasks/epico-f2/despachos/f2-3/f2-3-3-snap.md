# DESPACHO — F2-3-3 · Encaixe (snap passivo) · fatia `f2-3-3`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
O mover (F2-3-1) já existe: `app/lina-gpui/src/canvas_drag.rs` (puro — `drag_delta_world`, `apply_drag`,
`CardDrag` máquina de gesto, `fractional_between`). Posição do card em `NodeView.x/y` (mundo);
câmera `screen = world*zoom + pan`; `CARD_W=680, CARD_H=500, CARD_GAP=30` (bridge.rs ~140). **ATENÇÃO:
o bin do app está temporariamente QUEBRADO** por outra fatia em voo (resize, struct do core +
`inspector.rs`) — valide seu módulo EM ISOLAMENTO (harness `rustc` standalone, como a fatia drag fez:
`rustc --test --edition 2021 ...`), não dependa de `cargo test` do bin linkar agora.

**FUNÇÃO** · Você é o dono da lógica pura de ENCAIXE (snap) ao arrastar/soltar um card.

**DIRECIONAMENTO** · Fronteira (SÓ estes): **crie `app/lina-gpui/src/canvas_snap.rs`** (módulo puro) +
1 linha de registro `#[path] pub mod snap;` em `canvas.rs` (precedente de `focus`/`zoom`). NÃO toque
`main.rs`, `bridge.rs`, `canvas_drag.rs` (a fiação no `mouse_move`/`up` é costura do Maestro — entregue
diff). Entregue funções PURAS testáveis:
- `snap_position(dragged_xy, others: &[(f32,f32)], card_w, card_h, gap, threshold_world) -> (f32,f32)`:
  dado o card arrastado e os demais, encaixa por proximidade de BORDA/CENTRO quando dentro do
  `threshold` (alinhamento de bordas esquerda/direita/topo/base e gap canônico `CARD_GAP` entre cards).
  **viewport-only** (só considera cards relevantes que a costura passa). Retorna a posição ajustada (ou
  a original se nada perto). Determinístico e total.
- O **threshold é em TELA (8px) escalado por zoom** → a costura passa `threshold_world = 8.0 / zoom`
  (documente isso; a função recebe o valor de mundo, fica pura). Snap é PASSIVO: só sugere ao soltar/
  durante o move, **nunca move o card sozinho** (invariante da onda).
Testes não-vacuosos: encaixa quando dentro do threshold (borda alinha exatamente, gap = CARD_GAP); NÃO
encaixa fora do threshold; threshold escala (8/zoom); sem `others` = no-op (retorna original).

**OBJETIVO** · Ao arrastar um terminal para perto de outro, ele "imanta" num alinhamento limpo (bordas
e respiro canônico), sem nunca pular sozinho — organização que parece de Figma.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-3.md`: arquivo:linha do
`canvas_snap.rs`, evidência (testes em isolamento — nº + comandos exit DIRETO; nota de que o bin está
quebrado por peer), **diff de costura main.rs** (aplicar `snap_position` no preview do `CardDrag` antes do
`finish`/no `mouse_up`), achados. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
