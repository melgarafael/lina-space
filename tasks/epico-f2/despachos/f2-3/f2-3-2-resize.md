# DESPACHO — F2-3-2 · Redimensionar terminal vivo · fatia `f2-3-2`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
Esta é a fatia mais CORE da onda — por isso é sua (Backend/Ultra Code). Hoje o card é de tamanho FIXO
(CARD_W=680, CARD_H=500, bridge.rs ~140). O resize do motor já existe: `PtyManager::resize(node,cols,rows)`
(`crates/lina-pty/src/lib.rs:274`) + `AlacrittyBackend::resize(cols,rows)`+`damage_all()`
(`crates/lina-vt/src/lib.rs:729`). O **synchronized output (DEC 2026 / F2-0-6) JÁ ESTÁ FEITO**
(`crates/lina-vt/src/lib.rs:239`, `sync_deadline`, `flush_sync`, teto ~150ms) — é o que segura o flicker
durante o reflow. NÃO existe evento de resize persistido nem conversão px→cols/rows.

**FUNÇÃO** · Você é o dono do redimensionamento durável do terminal vivo (px → cols/rows → PTY/VT).

**DIRECIONAMENTO** · Fronteira (SÓ estes):
- `crates/lina-core/src/events.rs` — **evento aditivo** `TerminalRedimensionado { node, cols, rows, w, h }`
  (`serde(default)`; padrão dos eventos existentes; aplique em `apply()` atualizando o tamanho do nó;
  registre no matcher de nome ~842 e no apply ~1080). Emitido **no FIM do gesto**, nunca por frame;
  gesto cancelado = ZERO evento (espelhe a doutrina do ADR 0029 §1 que o `NodeMoved` segue).
- **crie um módulo de conversão pura** (ex.: `crates/lina-vt/src/sizing.rs` ou em lina-core): função
  `px_to_grid(w_px, h_px, cell_w, cell_h, min_cols, min_rows) -> (cols, rows)` — converte o tamanho do
  card em px para a grade do PTY, com pisos (um terminal nunca vira 0×0). Pura, testável.
- Fiação **PtyResize debounced ~100ms** + commit ao soltar: o resize VIVO repinta contínuo (GPU), mas o
  `PtyManager::resize` real é debounced (~100ms) para não floodar o PTY; ao soltar, 1 commit final +
  emite o evento. Decida ONDE o debounce vive (pty-host em lina-core, fora de main.rs) e entregue-o
  testável. Use o synchronized output para o reflow sem flicker (cite onde).
- NÃO toque `main.rs` (o handle de borda arrastável do card é costura do Maestro — entregue o diff).
Critério-chave do épico: CLIs reais (Claude Code) redimensionados **sem flicker (DEC 2026) nem
duplicação observável** no teste padrão; reflow soft-wrap com cap. Persistência via o evento novo
(restaura cols/rows ao reabrir). Testes não-vacuosos: px_to_grid respeita pisos e arredonda certo; evento
aditivo (replay de log SEM o campo não quebra); debounce coalesce N chamadas em 1 dentro da janela.

**OBJETIVO** · O fundador puxa a borda de um terminal com o Claude rodando dentro e ele cresce/encolhe
liso, o conteúdo reflui sem piscar nem duplicar, e o tamanho persiste ao reabrir.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-2.md`: arquivo:linha (evento +
módulo de sizing + debounce), evidência (`cargo test -p lina-core`/`-p lina-vt`/`-p lina-pty`
--test-threads=1 exit DIRETO + nº testes; `cd app/lina-gpui && cargo test` se tocar algo do app), **diff
de costura main.rs** (handle de borda: detecta drag na borda do card → resize vivo → debounced PtyResize →
commit+evento ao soltar), achados/riscos. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
