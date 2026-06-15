# DESPACHO — F2-3-1 · Mover terminal (drag) · fatia `f2-3-1`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
Hoje o drag do canvas só faz PAN da câmera (main.rs ~5070-5110); cards NÃO se movem. A posição vive em
`NodeView.x/y` (bridge.rs); o evento `NodeMoved {node,x,y}` JÁ EXISTE e é aplicado (events.rs:202,
~1080) mas nunca é emitido. Z-order: `focus(node)` bomba ao topo (main.rs ~4880); hit_test resolve clique
(bridge.rs ~2958). `screen_to_world`/`world_to_screen` convertem (bridge.rs ~2877).

**FUNÇÃO** · Você é o dono da LÓGICA pura de arrastar-e-mover um card e da ordem-z. Frente de canvas.

**DIRECIONAMENTO** · Fronteira (SÓ estes): **crie `app/lina-gpui/src/canvas_drag.rs`** (módulo novo).
NÃO toque `main.rs` (os handlers on_mouse_down/move/up são costura do Maestro — você entrega o diff).
Entregue funções PURAS testáveis:
- `drag_delta_world(start_screen, now_screen, zoom) -> (dx,dy)`: delta de tela → delta de mundo (÷zoom).
- `apply_drag(orig_xy, delta) -> (x,y)`: nova posição (clamp opcional a bounds do mundo, se houver).
- **Z-order por fractional index** (invariante do épico/ADR 0029 §2): `fractional_between(a,b) -> key`
  gerando uma chave entre dois índices, p/ "trazer à frente" = 1 evento sem reindexar vizinhos. Modele a
  chave (string lexicográfica OU f64 — escolha e justifique; lexicográfica é o padrão Excalidraw/Figma).
- Decisão **mover-card vs pan-fundo**: helper puro que, dado hit_test(ponto)=Some(node)|None, decide o modo.
Invariantes (duros): **nada se move sem gesto**; gesto **cancelado (Esc) = ZERO `NodeMoved`**; o evento é
emitido **no FIM** do gesto com a posição FINAL (nunca por frame). Testes não-vacuosos (falham se o
mecanismo for removido), incluindo: delta correto sob zoom≠1; fractional_between sempre estritamente entre;
cancelamento não produz posição nova.

**OBJETIVO** · O fundador arrasta um terminal e ele vai pra onde soltou, fica na frente, e continua lá ao
reabrir (via NodeMoved no log) — sem nada se mexer sozinho.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-1.md`: arquivo:linha do
`canvas_drag.rs`, evidência (`cd app/lina-gpui && cargo test`/`clippy --all-targets -D warnings`/`fmt`
exit DIRETO + nº testes + suíte completa incl. token_ratchet), **diff de costura main.rs** (handlers de
mouse no card: down inicia drag-de-card se hit_test=Some; move atualiza nv.x/y via seu delta; up emite
NodeMoved + focus; Esc cancela), achados. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
