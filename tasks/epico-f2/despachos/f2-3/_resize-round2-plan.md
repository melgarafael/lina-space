# F2-3-2 · Resize — Plano de Costura Round 2

> Checklist do Maestro. Base: 4 veredictos céticos (px_to_grid ✅ · ResizeDebouncer ✅ ·
> event-additividade ✅ · costura-merge ❌). O core está pronto; o round 2 é SÓ a fiação
> em `bridge.rs` (B1–B3) + o merge de handlers em `main.rs` (B4). Nada de tocar `sizing.rs`
> nem `events.rs` — eles passaram.

---

## 1. VEREDITO CONSOLIDADO — o resize-core está sólido para fiar?

**SIM, sem ressalvas no core.** As três peças puras/duráveis foram refutadas com sucesso
pelos céticos (todas `holds:true`, edge_failures vazios):

- **`px_to_grid` / `grid_dim`** (`crates/lina-vt/src/sizing.rs:28-51`) — floor correto,
  pisos respeitados, entrada degenerada (NaN/∞/≤0) cai no piso sem pânico. **Fiar como está.**
- **`ResizeDebouncer`** (`sizing.rs:64-131`) — coalesce N→1 por janela, re-emite em janela
  nova, dedup de tamanho igual, `flush()` idempotente, `cancel()` zera o commit (ADR 0029 §1).
  **Fiar como está.** Use a instância com janela `Duration::from_millis(100)`.
- **Evento `TerminalRedimensionado { node, cols, rows, w, h }`** (`events.rs:214`, matcher 857,
  apply 1117) + `ProjectedNode.{cols,rows,w,h}: Option` com `serde(default)` (1007-1014) —
  100% aditivo, last-write-wins, replay de snapshot velho desserializa `None` sem quebrar.
  **Fiar como está.** `inspector.rs:137-141` já consome os `None` sem crash.

**Ressalva acionável única (vem do round 1):** o core NÃO inclui a costura de UI. O verdict
`costura-merge` está correto ao reprovar — B1–B4 não existem ainda. O round 2 é exatamente
fechar isso. Não há retrabalho no core; há trabalho de fiação novo.

---

## 2. PLANO DE COSTURA EXATO (ordem segura — compila a cada passo)

A ordem importa: **B-a antes de B-b** (o handler de borda em `main.rs` depende de `view.w/h`
e de `commit_resize` existirem em `bridge.rs`, senão não compila — exatamente o cenário 7 do
cético). Dono único de ambos os arquivos nesta rodada.

### (a) `app/lina-gpui/src/bridge.rs` — a API e o estado projetado

**B1 · `NodeView` ganha tamanho dinâmico** (struct em `bridge.rs:191-214`):
```rust
pub struct NodeView {
    // ...campos atuais...
    /// F2-3-2: tamanho do card em MUNDO (px). `None` = nunca redimensionado → o render
    /// usa CARD_W/CARD_H. Preenchido pela projeção (replay) e pelo preview vivo do gesto.
    pub w: Option<f32>,
    pub h: Option<f32>,
}
```
- Em **`NodeView::new`** (`bridge.rs:216+`): `w: None, h: None`.
- **Render lê daqui** — `main.rs:4081-4082` troca `px(CARD_W * z)` por
  `px(nv.w.unwrap_or(CARD_W) * z)` e idem `h`. **Todo** site que hoje pinta/hit-testa com
  `(CARD_W, CARD_H)` fixo no caminho do card (culling 3480/3733, hit_test 3527/3655,
  reveal 4199) passa a usar `nv.w.unwrap_or(CARD_W)` / `nv.h.unwrap_or(CARD_H)`. Se algum
  ficar fixo, a hot-zone de borda do B4 cai no lugar errado num card já redimensionado
  (risco materializado no verdict, item 7 e risco 3).

**B2 · Projeção escreve w/h no NodeView** (no ponto que sincroniza projeção→model;
`wire_terminal`/`admit_node_inner` em `bridge.rs:4960` e o caminho de replay):
ao construir/atualizar o `NodeView` a partir do `ProjectedNode`, copiar
`view.w = pnode.w.map(|v| v as f32); view.h = pnode.h.map(|v| v as f32);` — assim
**reabrir restaura o tamanho** (critério-chave do épico). O `None` herdado de snapshot
velho mantém CARD_W/CARD_H (aditivo).

**B3 · `resize_live` + `commit_resize` no `impl NodeManager`** (inserir após `admit_node`,
~`bridge.rs:4675+`):
```rust
/// Resize VIVO durante o gesto: repinta GPU contínuo + reflow do PTY/VT debounced.
/// Chamado pela costura a cada `offer()` que devolveu Some. NÃO emite evento.
pub fn resize_live(&self, node: NodeId, cols: u16, rows: u16) {
    if let Some(pty) = self.pty_of(node) { let _ = pty.lock().resize(node, cols, rows); }
    if let Some(grid) = self.grid_of(node) {
        let mut g = lock(&grid);
        g.resize(cols, rows);   // AlacrittyBackend::resize + damage_all (lina-vt:729)
        g.damage_all();         // synchronized output (DEC 2026) segura o flicker no reflow
    }
}
/// Commit final ao soltar: 1 resize real + 1 evento durável. w/h em MUNDO (px, pré-zoom).
pub fn commit_resize(&self, node: NodeId, cols: u16, rows: u16, w: f32, h: f32) {
    self.resize_live(node, cols, rows);
    if let Some(nv) = lock(&self.model).nodes.get_mut(&node) {
        nv.w = Some(w); nv.h = Some(h);
    }
    // append do TerminalRedimensionado via o MESMO funil de append_to_workspace
    // que o NodeMoved usa (main.rs faz o append; aqui só atualiza o model + devolve dims).
}
```
> Nota de fronteira: o `append` do evento pode ficar em `main.rs` (espelhando como o
> `NodeMoved` é apendado no `mouse_up`, `main.rs:3604`) para `commit_resize` não precisar do
> `active_root`/`runtimes`. Decida 1 dono do append e seja consistente com o mover.

**B-spawn · grade persistida no spawn** (`fit_dims` em `main.rs:113-117`): antes de cair no
default 80×24, checar `pnode.cols.zip(pnode.rows)` — se o nó já tem grade durável, o PTY
nasce já no tamanho salvo (sem um "salto" de reflow ao abrir). Fallback = a fórmula atual.

### (b) `app/lina-gpui/src/main.rs` — MERGE dos handlers de mouse

**B4-estado** · `WorkspaceView` (campo novo após `card_drag`, `main.rs:486`):
```rust
/// F2-3-2: gesto de RESIZE de card (arrasto da borda dir/inf). `None` = sem gesto.
resize_drag: Option<ResizeDrag>,   // { node, orig_world_size:(f32,f32),
                                   //   start_screen:(f32,f32), deb: ResizeDebouncer,
                                   //   cur_grid:(u16,u16), cur_size:(f32,f32) }
```
init `resize_drag: None` (`main.rs:677`). `ResizeDrag` espelha `CardDrag` (begin/update/
preview/cancel/finish) — mas `update` alimenta o `ResizeDebouncer` e devolve `Option` de
grade a aplicar vivo; `finish` chama `deb.flush()`.

**B4-hotzone** · a borda quente = faixa de **~8px em TELA** na aresta direita E/OU inferior
do card (cantos = ambos = resize bidimensional). Calculada SEMPRE em screen-space, já com zoom:
```
hot = 8.0  // px de tela, constante; NÃO multiplicar por zoom (é affordance de cursor)
sw = nv.w.unwrap_or(CARD_W) * zoom ; sh = nv.h.unwrap_or(CARD_H) * zoom
(sx, sy) = camera.world_to_screen((nv.x, nv.y))
on_right  = pos.0 >= sx + sw - hot && pos.0 <= sx + sw + hot
on_bottom = pos.1 >= sy + sh - hot && pos.1 <= sy + sh + hot
is_resize_edge = on_right || on_bottom
```

### Pseudocódigo FINAL mesclado (precedência: borda-resize > título-move > corpo-seleção > fundo-pan)

```
on_mouse_down(pos):
    hit = hit_test(camera, pos, cards_z_asc, (per-card w/h via nv), vp)
    if let Some(node) = hit:
        (nx, ny) = world pos do card ; (sx, sy) = world_to_screen((nx, ny))
        sw, sh = nv.w.unwrap_or(CARD_W)*zoom, nv.h.unwrap_or(CARD_H)*zoom
        # (1) BORDA → RESIZE (precede tudo; checa ANTES de drag_mode — verdict cenário 1)
        if is_resize_edge(pos, sx, sy, sw, sh):
            focus(node)                                  # z-bump
            resize_drag = Some(ResizeDrag::begin(node,(nx,ny),
                               (nv.w.unwrap_or(CARD_W), nv.h.unwrap_or(CARD_H)), pos))
            notify(); return                              # RETURN — não entra em drag_mode
        # (2) TÍTULO → MOVE (lógica atual intacta: header_px = 34.0*zoom)
        (_, top_y) = world_to_screen((nx, ny)); header_px = 34.0*zoom
        if pos.1 >= top_y && pos.1 <= top_y + header_px:
            focus(node); card_drag = Some(CardDrag::begin(node,(nx,ny),pos)); notify(); return
        # (3) CORPO → seleção/mouse-report do grid (NÃO pan — corpo é janela)
        #     fluxo atual do grid trata; cair aqui = começar seleção, return implícito
        return                                            # corpo: nem move nem pan
    # (4) FUNDO vazio → PAN (hit==None)
    drag = Some((pos, camera.pan))

on_mouse_move(pos):
    if let Some(rd) = resize_drag.as_mut():               # RESIZE precede card_drag
        if ev.dragging():
            (w, h) = rd.update(pos, zoom)                 # tamanho-mundo de preview, com pisos
            nv.w = Some(w); nv.h = Some(h)                # preview vivo (só memória, sem evento)
            (cols, rows) = px_to_grid(w-chrome_w, h-chrome_h, cell_w(), cell_h(), 80, 24)
            if let Some((c, r)) = rd.deb.offer(Instant::now(), cols, rows):
                nodes.resize_live(node, c, r)             # PTY/VT debounced ~100ms (vivo)
            notify()
    elif let Some(g) = card_drag.as_mut():  ...           # MOVE (bloco atual, 3561)
    elif let Some((m0,p0)) = drag:          ...           # PAN  (bloco atual, 3572)
    elif dragging_sel:                      ...           # SELEÇÃO (3582)
    elif report_node:                       ...           # mouse report (3588)

on_mouse_up(pos):
    if let Some(rd) = resize_drag.take():                 # RESIZE precede card_drag.take()
        if let Some((cols, rows)) = rd.finish():          # = deb.flush(); None se cancelado/igual
            (w, h) = rd.cur_size()
            nodes.commit_resize(node, cols, rows, w, h)   # 1 resize real + 1 evento
            append_to_workspace(&active_root, TerminalRedimensionado{node,cols,rows,w,h})
            notify()
        # cancelado → nv.w/h revertem à origem do gesto (rd.orig_size)
    if let Some(g) = card_drag.take(): ...                # MOVE (bloco atual, 3601)
    # ...resto do mouse_up atual (drag=None, dragging_sel, report_node)...

on_key(Escape):
    if resize_drag.is_some(): rd.cancel(); reverte nv.w/h à origem; resize_drag=None; notify()
    # (espelha o Esc do card_drag em main.rs:3320)
```

> **`chrome_w`/`chrome_h`:** o `px_to_grid` recebe a ÁREA DE TERMINAL, não o card inteiro —
> desconte o chrome (header ~34 + padding) como `fit_dims` já faz (`main.rs:116-117`). Mantenha
> a MESMA conta dos dois lados (spawn e resize) para o card não "pular" de grade ao reabrir.

---

## 3. RISCOS DE TELA + checkpoint do fundador

- **Flicker no reflow (DEC 2026):** o synchronized output já existe (`lina-vt/src/lib.rs:239`,
  teto ~150ms) — o `damage_all()` no `resize_live` precisa rodar DENTRO desse frame sincronizado,
  senão o reflow aparece a meio-caminho. Risco baixo (já feito), mas é o que o fundador deve
  observar: **o conteúdo reflui sem piscar** ao puxar a borda com o Claude rodando.
- **Duplicação observável:** sintoma de re-emissão por frame. O debouncer (offer→Some só na
  borda da janela) + commit único no flush previnem; observar que **NÃO aparecem linhas
  repetidas** durante o arrasto rápido (gesto que cruza várias janelas de 100ms).
- **Precedência (o ponto que reprovou no round 1):** confirmar que (a) clicar no CANTO
  redimensiona — nunca move nem pana; (b) clicar no TÍTULO move; (c) clicar no CORPO
  seleciona texto — **nunca** pana (corpo é janela); (d) clicar no FUNDO pana. O `return` após
  cada caso é o que impede `resize_drag` E `card_drag` ativos no mesmo gesto (cenários 1–2).
- **Hot-zone em card já redimensionado:** se algum site de render/hit-test ficou em CARD_W/H
  fixo (B1 incompleto), a borda fica no lugar errado depois do 1º resize. Checkpoint:
  redimensionar, soltar, **reabrir a borda de novo** no novo tamanho — tem de pegar.
- **Persistência:** fechar e reabrir o Espaço → o card volta no tamanho salvo (não no default).
  É o critério durável do épico; observar diretamente.

> O fundador puxa a borda inferior-direita de um terminal com o Claude vivo dentro: cresce/encolhe
> liso, o texto reflui sem piscar nem duplicar, e ao reabrir o Espaço o tamanho continua lá.
