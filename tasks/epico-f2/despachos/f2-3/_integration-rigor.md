# F2-3 — Rigor de Integração (checklist do Maestro)

> Arquiteto de integração da onda. As 4 features (drag de card · borda de resize · tecla zoom-to-fit · LOD no render) compartilham `main.rs` e `bridge.rs`. Costuras têm **dono único por rodada**: o Maestro aplica e valida de fora (exit codes diretos), workers não commitam. Fontes: mapas de costura + crivos adversariais consolidados abaixo.

---

## 1. Plano de costura ordenado (ordem segura de aplicar)

Aplicar **de baixo para cima na pilha** (core → bridge puro → render → input), para que cada camada só dependa de costura já verde. Validar `cargo test -p <pkg>` + `clippy -D warnings` por pacote **antes** de subir um degrau.

### Degrau 0 — Núcleo de eventos (sem UI) — `crates/lina-core/src/events.rs`
Nada de UI depende disto, mas a UI vai emitir; entra primeiro.
- **`events.rs:202-206`** — adicionar variant `TerminalRedimensionado { node, cols, rows, w, h }` com `#[serde(default)]` nos 4 campos (NodeMoved já existe e serve de molde para `NodeMoved`/drag, que NÃO precisa de variant novo).
- **`events.rs:838-909`** — arm em `kind()` → `"TerminalRedimensionado"` (match exaustivo, o compilador cobra).
- **`events.rs:914-920`** — `current_version()`: cai no `_ => 1` (lança v1, sem upcast).
- **`events.rs:1080-1130`** — arm em `apply()`. **Decisão de arquitetura (fecha porta): resize NÃO projeta** em `ProjectedNode` (é metadado operacional, como `PermissionAsked`). `apply()` é no-op/log-only; quem guarda dims é a camada PTY/VT. Drag (`NodeMoved`) **projeta** (`n.x=*x; n.y=*y`) — já existe.

### Degrau 1 — Cálculo puro (gpui-free) — `bridge.rs` + `canvas_zoom.rs`
Testável 100% sem janela; é o coração das 3 features visuais.
- **`canvas_zoom.rs:1-119`** — `bounds_of` / `zoom_to_fit` / `zoom_to_selection` já existem e são puros. Confirmar contrato 0-card: `bounds_of([]) → None`.
- **`canvas_zoom.rs:121-148`** — `lod_for(zoom, last)` já existe com histerese `[0.6, 0.75]`. Falta SÓ o fio para o render (degrau 3). Remover `#![allow(dead_code)]` só quando o consumidor entrar (clippy `-D` quebra costura meia — pub sem chamador = dead-code error).
- **`bridge.rs`** — função pura `px_to_grid(w_px, h_px, cell_w, cell_h, min_cols, min_rows) -> (u16,u16)` para a borda de resize: guarda `cell_w==0||cell_h==0 → (min_cols,min_rows)`, `floor` com piso `max(min_cols)`/`max(min_rows)`. Espelhar a aritmética do VT (`cols.max(1)`).

### Degrau 2 — Estado de gesto na struct — `main.rs`
- **`main.rs:483-484`** — trocar `drag: Option<((f32,f32),(f32,f32))>` por enum discriminado `DragState { Pan{mouse,pan0}, Card{node,mouse,card_world0}, Resize{node,edge,start_dims} }`. Migrar os 3 handlers de mouse **na mesma costura** (não deixar `Pan` legado e `Card` novo coexistindo meio-fio).

### Degrau 3 — Render (consome degraus 1-2) — `main.rs` + `bridge.rs`
- **`main.rs:3429,3609-3641`** — loop de cards: após `cards.sort_by_key(z_of)` e o gate `card_visible` (3618), calcular `lod = lod_for(cam.zoom, self.lod_last)` **uma vez por frame**, persistir `self.lod_last` (histerese precisa de estado frame-a-frame). Passar `lod` ao corpo do card.
- **`main.rs:3863-3975`** (corpo) e **`3694-3861`** (título) — `Lod::Compact` simplifica conteúdo; **nunca** remove `div()` do slot nem o portal (invariante de composição). `world_to_screen` (3641) continua a ÚNICA fonte de `(sx,sy)` — handles de borda usam o mesmo transform.
- **Borda de resize**: zona de hit nas 8px de borda do rect de tela do card, testada **antes** do corpo (hit_test do card resolve z; borda é sub-zona do topo). Só em card `card_visible` (culled não tem DOM → sem handle).

### Degrau 4 — Input/dispatcher (topo da pilha) — `main.rs`
A ordem dentro de `handle_key` é **estritamente precedência**; inserir nos lugares exatos:
- **`main.rs:127-192`** `keystroke_to_bytes` — **REMOVER/neutralizar** os arms `f5`→`[1b,5b,31,35,7e]` e `f6`→`[1b,5b,31,37,7e]` (linhas ~181-182) **OU** garantir return antes. Senão a tecla DUPLICA (app + CSI ao PTY).
- **`main.rs:3258-3268`** (bloco zoom ⌘±/⌘0) — **logo após**, **antes** do Escape (3276): inserir arms `F5`→zoom-to-fit e `F6`→cycle-attention. Cada um: limpar `self.drag = None` (estado de pan estável antes de aplicar câmera) → `bounds_of(cards) → Some(b)` → `self.camera = zoom_to_fit(b, vp, PAD, ZOOM_MIN, ZOOM_MAX)` → `cx.notify()` → `return`. Se `bounds_of → None` (0 card): no-op + `return` (não mexe na câmera).
- **`main.rs:3276`** (Escape) — **ANTES** da lógica de toast/painel: se `self.drag.is_some()` (qualquer variante, inclusive Resize), `self.drag = None; cx.notify(); return` (cancela gesto, **0 eventos**). Só então cai no snooze/painel/PTY. Ordem: **gesto > toast/painel > PTY**.
- **`main.rs:3413-3522`** (handlers de mouse) — wire de drag de card e arrasto de borda:
  - `on_mouse_down` (3457-3469): `hit_test.is_some()` → `Card{node}` (ou `Resize` se sobre a borda); `is_none()` → `Pan`. **`focus(node)` JÁ no down** (z-bump contínuo: card arrastado vai ao topo durante o gesto, não só no up).
  - `on_mouse_move` (3471-3495): `delta_mundo = delta_tela / cam.zoom`; atualizar `nv.x/nv.y` no model OU emitir `PtyManager::resize` **debounced** (~100ms) para resize. Sem evento durável aqui.
  - `on_mouse_up` (3497-3522): emitir **1** evento (`NodeMoved` no fim do drag / `TerminalRedimensionado` no fim do resize) via `append_to_workspace` (1645-1667), **só se** o gesto passou o threshold; `focus(node)` para selar z-order.

---

## 2. Riscos transversais de integração

| Risco | Sintoma | Mitigação (obrigatória) |
|---|---|---|
| **Drag-vs-pan (zona morta / exclusão)** | Hoje `hit_test.is_none() → pan`. Novo: `is_some() → card-drag`. Gap entre cards vira pan ou card errado. | Decisão única no `on_mouse_down` via `hit_test` (z-DESC, topo vence). Threshold de **~5px** distingue clique de drag — abaixo disso: nem move card nem emite `NodeMoved` (é clique). Clique no header = mover; clique no grid = seleção de texto (`dragging_sel` de W2-4) — **borda/título arrastável separada do corpo**. |
| **delta não-invertido pelo zoom** | A 100px de arrasto em zoom 0.4 o card anda 40px no mundo (deveria 250px). | `delta_mundo = delta_tela / cam.zoom` SEMPRE. Teste com zoom 0.4 **e** 2.0 (não só 1.0 — vacuoso). |
| **Resize durante PTY ativo (reflow race)** | PTY escreve com cols novo; VT ainda no cols velho → reflow garbled / frame rasgado. | `PtyManager::resize` (pty/lib.rs:274) **antes** de `VtBackend::resize` (vt/lib.rs:729), no mesmo tick do event-loop (mesma call-stack no pty-host). Grid SIZE é fonte da verdade do próximo damage scan. Durante DEC-2026 (sync output) o `flush_sync` (vt/lib.rs:803) tem de rodar `accumulate_damage` no mesmo frame — emitir evento ANTES do flush garante causalidade. SLA <150ms é best-effort sob drag lento (documentar, não prometer). |
| **Resize com PTY morto** | `PtyManager::resize` retorna `Err`. | Não emitir `TerminalRedimensionado` se a syscall falhou (auditoria nunca diverge do OS). Piso `cols≥1,rows≥1` validado **antes** da syscall — `resize(0,0)` é bug, não feature. |
| **Debounce sem commit final** | Drag solto, último `PtyResize` pendente nunca enviado → grid trava no tamanho anterior. | Debounce coalesce N→1 na janela ~100ms, **mas** o `on_mouse_up` força o flush do resize final (commit garantido). Emitir evento só se `(cols,rows) != dims_anteriores` (evita log inchado de movimentos de 1-2px). |
| **Zoom-to-fit com 0/1 cards** | `bounds_of([]) → None` → divisão por zero ou câmera sem gesto. | 0 cards: no-op, câmera persiste (contrato no call-site). 1 card: `bounds_of` retorna Rect com `w,h = max(1.0, dim)` (vt/zoom já clampa linha 96); zoom satura em ZOOM_MIN/MAX sem cortar. |
| **Câmera estado-velho (drag + zoom simultâneos)** | F5 disparado no meio de um drag de mouse usa `cam.pan` stale. | `self.drag = None` **antes** de aplicar `zoom_to_fit`. `cam` é `f32` não-travado; nenhuma thread de animação roda — mas o gesto pendente tem de ser zerado. |
| **Z-order na persistência (replay)** | Log de `NodeMoved` não carrega `z` — só `(node,x,y)`. Ordem de replay define z final. Arrasto antes do sync de outro agente inverte z esperado. | Z derivado **da ordem do log** (last applied = topo), determinístico no replay. Drag emite `NodeMoved` no up; `focus()` re-eleva localmente. Documentar: z é projeção da ordem, não campo persistido. |
| **Mouse-up fora da janela** | Arrasta card pro canto, solta fora da app → `on_mouse_up` pode não chegar → drag travado. | Verificar contrato gpui de captura. Fallback: Escape cancela (`drag=None`) e blur/foco-perdido limpa estado efêmero. |
| **Conversão f32↔f64** | `NodeView.x/y` é f32 (mundo); `NodeMoved.x/y` é f64. | f32→f64 lossless; log mantém f64; replay f64→f32 arredonda — aceitável (sub-px). Não acumular delta em float ao longo de centenas de mousemove: recalcular de `card_world0 + delta`, **não** `nv.x += delta` por frame (evita drift cumulativo). |
| **Aditividade do log antigo** | Log sem `TerminalRedimensionado` ou sem campos novos recusa deserialização. | `#[serde(default)]` nos 4 campos; replay de log velho projeta degradado, **nunca** crasha (ADR 0001 §2). |

---

## 3. Crivo de revisão por módulo (invariantes · edge cases · armadilhas de teste vacuoso)

### `canvas_drag` (drag de card)
**Invariantes a provar (teste não-vacuoso):**
- `screen_delta / zoom == world_delta` — testar em zoom ≠ 1.0 (0.4 e 2.0), **não** `==` em float (usar `abs() < 1e-3`).
- Posição em MUNDO só muda dentro de `on_mouse_move` com `ev.dragging()==true`; nunca mid-frame.
- `NodeMoved` emitido **1×** por gesto completo (no up), **0×** se cancelado (Esc) ou se `dist < threshold` (clique).
- Z-order monotônico: cada `focus()` faz `z_next += 1`; sem colisão/inversão; idempotente no mesmo card.
- `hit_test` usa câmera atual + só cards visíveis (culled excluído).
**Armadilhas a rejeitar:** `assert_ne!(before, after)` sem checar direção/magnitude; testar delta só em zoom 1.0 (nunca exercita o `/zoom`); `view.drag=None` manual em vez do handler de cancel; verificar `hit_test` foi chamado sem checar se o resultado é usado; setar drag sem chamar `on_mouse_move`.

### `resize` (borda + PTY/VT/evento)
**Invariantes:** `px_to_grid` SEMPRE retorna `cols≥min_cols, rows≥min_rows` (nunca `(0,X)`/`(X,0)`); guarda `cell_w==0`. Debounce N→1 na janela ~100ms (observado via spy de `PtyManager::resize` com **tempo real passando**, não mock). 1 drag = 1 `TerminalRedimensionado`; cancel = 0. Aditividade: replay sem campos não quebra. Synch output honrado: `sync_deadline()→Some(t)`, `flush_sync` antes de `t+150ms`, grid nunca rasgado.
**Armadilhas:** testar `px_to_grid` só no happy-path (`(1000,800,...)`) sem o piso `(1,1,...)`; debounce com tempo mockado (não prova coalesce real); emitir evento mas nunca chamar `apply()`; `ProjectedNode.cols=0` inicial confundido com 0-resultado; cancel testado editando log direto, não via Esc/UI; arredondamento com "ambos corretos" sem veredito de floor-vs-ceil.

### `canvas_zoom` (tecla zoom-to-fit + LOD)
**Invariantes:** após `zoom_to_fit`, **todos** os 4 cantos do bounds caem na área útil (`x_min≥0, x_max≤vp.0, y_min≥TOPBAR, y_max≤vp.1`). `zoom ∈ [0.4, 2.0]` sempre. Histerese não oscila na faixa `[0.6, 0.75]` (mantém `last`). `bounds_of([])→None` = no-op. Centro do bounds ≈ centro da área útil (erro < 1e-2). `zoom_to_fit` é função pura (mesmo input → mesmo Camera; só calcula, não muta).
**Armadilhas:** fit sem checar que zoom NÃO saturou em MIN/MAX (fixture que cabe torna o teste vácuo — usar `zoom > ZOOM_MIN && zoom < ZOOM_MAX`); LOD testado longe da fronteira (1.8 vs 0.3 — óbvio); histerese com 2 passos (precisa da rampa inteira up↔down); `bounds.center()==area_center` (coincidência que esconde pan enviesado); fronteira LOD só de um lado (testar `==LOD_UP` e `==LOD_DOWN`, `>=`/`<=` incluem a fronteira no novo estado).

### `card-render` (LOD no render)
**Invariantes:** `world_to_screen` é a ÚNICA fonte de `(sx,sy)` — handle de borda usa o mesmo transform (senão desalinha sob pan/zoom). `Lod::Compact` simplifica conteúdo, **nunca** remove o slot/portal (camada de composição sempre desenhada). Culled (off-viewport) não entra no loop → sem DOM → sem hit. LOD calculado 1×/frame, `lod_last` threadado frame-a-frame.
**Armadilhas:** testar que LOD foi calculado sem checar que o slot ainda renderiza em Compact; hit-test de borda em coords de tela mas zona definida em card-local sem converter; integrar `lod_for` sem persistir `last` (histerese vira flicker).

### Doutrina de segurança (router — sempre verde)
Nenhum campo escrito por agente decide identidade/ordem/autorização. `TerminalRedimensionado`/`NodeMoved` são **dado transportado, jamais autoridade**. Toda story que toca `deliver_a2a`/`Router` mantém a suíte de segurança do router verde como critério implícito.

---

## 4. Definição de pronto da onda F2-3

**Gates de engenharia (exit code direto, por pacote — pipe mascara, `touch` fura cache de lint):**
- [ ] `cargo test -p lina-core` verde — inclui replay de log antigo SEM `TerminalRedimensionado` (aditividade), `kind()` exaustivo, apply de `NodeMoved` idempotente.
- [ ] `cargo test -p lina-vt` + `-p lina-pty` verde — piso `cols/rows≥1`, ordem PTY-antes-VT, `flush_sync` no frame.
- [ ] `cargo test -p lina-gpui` verde — **suíte completa** (o filtro de módulo não casa `token_ratchet`, que é teste global; gate de UI roda tudo). Inclui: `delta/zoom` em 0.4 e 2.0; fit-encloses-every-card; histerese-na-rampa; debounce-coalesce com tempo real; cancel-via-Esc = 0 eventos; `px_to_grid` no piso.
- [ ] `cargo fmt` + `cargo clippy -D warnings` limpos **por pacote** (formatar só os próprios arquivos; `allow(dead_code)` do `canvas_zoom` removido só com o consumidor wired).
- [ ] `esc_snooze_never_emits_decision_event` e a suíte de segurança do router **verdes**.
- [ ] Token ratchet: apertar o snapshot se a dívida de literais caiu (catraca bidirecional).

**Gate de TELA (o que separa "compila" de "pronto") — leigo sem ajuda, sem jank:**
- [ ] **Organiza 5 terminais**: abre 5 cards, **arrasta** cada um para onde quiser. Movimento acompanha o mouse 1:1 em qualquer zoom; card arrastado vem ao topo; soltar persiste a posição (sobrevive a reabrir o Espaço — replay restaura).
- [ ] **Redimensiona 2 com CLI vivo**: dois terminais rodando saída ativa; arrasta a borda de ambos. O conteúdo **reflua sem piscar/rasgar** (synch output segura o frame), o CLI continua rodando, o novo tamanho persiste.
- [ ] **Acha o que pede aprovação**: aperta a tecla de zoom-to-fit (F5) e o canvas **enquadra todos os cards** centralizados, sem cortar nenhum; com 0/1 card não dá erro nem salto. Acha o card que pede gate humano e navega até ele.
- [ ] **Zero jank**: pan e drag não congelam (`cx.notify()` em todo wiring point); afastar o zoom degrada para Compact **sem flicker** na fronteira; nenhum card some atrás da topbar.
- [ ] **Sem jargão na superfície**: nenhuma das ações exige instrução técnica; o leigo organiza, redimensiona e acha sozinho.

> **Regra de fechamento:** worker entrega `PRONTO:`/`BLOCKED:`; o Maestro valida de fora (exit codes), roda o gate de tela, e só então commita **por fatia**. Se a implementação contradiz a fonte 13.x da story, **pare e escale** — não "adapte" em silêncio. Antes de fechar: `isto fecha uma porta de continuidade?` → se sim, ADR em `docs/adr/`.
