# ONDA F2-3 — Canvas: organizar, redimensionar, mover (contexto compartilhado)

> Leia este doc + `tasks/despachos/_regras-comuns.md` ANTES de tocar código.
> O Maestro (Terminal A / Arquiteto) é **dono único de `app/lina-gpui/src/main.rs`** nesta onda —
> os handlers de mouse/tecla/render vivem lá. Você **não toca main.rs**: entrega seu MÓDULO PRÓPRIO
> + testes, e passa a costura (call-site) ao Maestro na sua entrega (diff textual preciso).

## Norte (por que esta onda existe)
Debriefing do fundador, item 2 da Fase 2: **"Organizar, redimensionar e mover os terminais no canvas."**
Épico `38 - Epico Fase 2`, onda F2-3 — "o pedido explícito do fundador". Invariantes da onda
(protegem janelas VIVAS): **nada se move sozinho · nada se redimensiona sozinho · a câmera nunca
anda sem gesto**. Premium = latência (nunca animar ação iniciada por input frequente; 120Hz é
orçamento de resposta, não espetáculo). A tela nunca mente. Régua D0: medir antes de prometer.

## Estado do canvas HOJE (do mapeamento — confirme antes de mudar)
- **Posição do nó:** `NodeView { x: f32, y: f32, ... }` em `app/lina-gpui/src/bridge.rs` (~176, ~460).
  Espaço de MUNDO. `CARD_W=680, CARD_H=500, CARD_GAP=30` (bridge.rs ~140).
- **Evento `NodeMoved { node, x, y }`** JÁ EXISTE e é aplicado (`crates/lina-core/src/events.rs:202`,
  apply ~1080; teste ~1958) — mas **nunca é emitido** (não há drag de card). Emitir no FIM do gesto.
- **Câmera** (`app/lina-gpui/src/bridge.rs` ~2860): `Camera { pan:(f32,f32), zoom:f32 }`;
  `world_to_screen`/`screen_to_world` (zoom*w+pan); `reset`; `zoom_by(cursor,factor)` clampa
  `[ZOOM_MIN=0.4, ZOOM_MAX=2.0]`; `reveal(card,size,viewport)` auto-pan. **ADR 0029: câmera FORA do
  event log** (estado de sessão local).
- **Z-order:** `z_order: BTreeMap<NodeId,u64>` + `focus(node)` bomba ao topo (main.rs ~400, ~4880);
  `cards_z_asc()` ordena p/ render; `hit_test` resolve clique por z decrescente (bridge.rs ~2958).
- **Drag HOJE:** só faz PAN da câmera (main.rs ~5070-5110) — NÃO move cards. `on_mouse_down` no fundo
  (hit_test vazio) → `self.drag = Some((pos, pan))`; `on_mouse_move` atualiza `camera.pan`.
- **Resize:** `PtyManager::resize(node, cols, rows)` (`crates/lina-pty/src/lib.rs:274`) +
  `AlacrittyBackend::resize(cols, rows)` + `damage_all()` (`crates/lina-vt/src/lib.rs:729`). Tamanho do
  card é FIXO hoje (CARD_W/H); não há resize pelo usuário; cols/rows NÃO persistem.
- **Synchronized output (DEC 2026 / F2-0-6 — FEITO):** `crates/lina-vt/src/lib.rs:239` (`sync_deadline`,
  `flush_sync`, teto ~150ms) — segura o flicker durante redraw. Use-o no resize.
- **Persistência:** replay + snapshots (`crates/lina-core/src/events.rs:1520` project / 1563 snapshot).
  `NodeMoved` restaura posição. Câmera = snapshot de sessão local (ADR 0029, ainda não implementado).

## Doutrina de FRONTEIRAS desta onda (a lição da árvore compartilhada)
Quase toda a inteligência de canvas é **geometria PURA testável**. Cada frente entrega um **módulo
próprio** `app/lina-gpui/src/canvas_*.rs` (ou crate core) com funções puras + testes não-vacuosos. O
Maestro costura os handlers em main.rs consumindo seu módulo. **NÃO pré-monte metade de costura** (item
pub sem chamador = dead-code que o `clippy -D` do bin reprova). Se sua função pura ainda não tem
consumidor, marque-a `#[cfg(test)]`-exercitada por teste OU entregue o consumidor junto na costura.

## Gate de saída da onda (o que prova "pronto")
**Cenário na tela com leigo (camada b da régua):** organizar 5 terminais, redimensionar 2 com CLI
vivo (Claude rodando), achar o que pede aprovação — **sem ajuda e sem jank** (camada d [PROF] na cena
de estresse). gpui NÃO roda headless → a validação FINAL é na tela do fundador. Seu trabalho: lógica
pura provada por teste + costura que compila + rebuild; o Maestro recompila o app (release) e o fundador
valida na tela. **O ciclo só fecha na tela** (lição cravada nesta sessão).

## Protocolo (além das REGRAS COMUNS)
1. 1º ato: `touch .iniciado-<sua-fatia>` (ex.: `.iniciado-f2-3-1`).
2. Fronteira = LEI. `main.rs` é do Maestro. Eventos aditivos (`serde(default)`) — replay antigo nunca quebra.
3. Reporte status: `lina ask "@Terminal A" "<status>" --intent status` ao começar/terminar/travar.
4. Valide POR PACOTE, exit DIRETO (sem pipe). App: `cd app/lina-gpui`; rode a SUÍTE COMPLETA (inclui o
   `token_ratchet` — teste global que o filtro de módulo NÃO casa). Catraca é bidirecional (aperte o
   snapshot se a dívida cair: `LINA_RATCHET_UPDATE=1`).
5. Entrega: `tasks/epico-f2/despachos/f2-3/.entrega-<fatia>.md` — o que mudou (arquivo:linha), evidência
   (comandos+exit+nº testes), **costura para main.rs (diff textual preciso)**, achados/riscos. Última
   linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
6. NÃO commite. O Maestro valida de fora e commita por fatia.
