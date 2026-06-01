//! `lina-gpui` — o **walking skeleton com render de terminal de verdade** (Onda 2). Canvas
//! gpui com 2 terminais reais do core; cada card pinta o **snapshot do grid VISÍVEL**
//! (`VtBackend::screen()`) a cada frame — cols×rows com **cores por célula + cursor** —
//! então TUIs ricas (Claude Code, vim) aparecem corretas. **Scroll** (scrollback),
//! **teclas especiais** (setas/Home/End/PageUp·Down/F-keys → CSI), **resize** (PTY+grid),
//! e o diferencial **A2A com pulso "sem fios"**. Tradução core↔UI em [`bridge`] (gpui-free).

mod bridge;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, rgb, size, text, App, Bounds, ClickEvent, Context, FocusHandle,
    FontWeight, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    Point, Render, Rgba, Role, ScrollDelta, ScrollWheelEvent, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;

use lina_core::{
    DomainEvent, EventStore, NodeStatus as CoreStatus, PtyManager, Supervisor, VtRgb, VtScreen,
};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use bridge::{
    card_visible, demo_profile, hit_test, lock, shell_cmd, spawn_pump, wire_terminal, A2aTrigger,
    Camera, CmdFactory, CoreInput, GpuiBridgeHost, Grid, Model, NodeManager, NodeView, SharedModel,
    CARD_H, CARD_W,
};

/// Métricas aproximadas da célula monoespaçada (Menlo 13px) — definem cols×rows do PTY.
const CELL_W: f32 = 7.84;
const CELL_H: f32 = 17.0;
const FONT_PX: f32 = 13.0;

/// Quantas cols×rows cabem na área de grid de um card (>= 80×24, "tamanho útil").
fn fit_dims() -> (u16, u16) {
    let pad = 8.0;
    let title = 34.0;
    let cols = (((CARD_W - 2.0 * pad) / CELL_W).floor() as u16).max(80);
    let rows = (((CARD_H - title - 2.0 * pad) / CELL_H).floor() as u16).max(24);
    (cols, rows)
}

/// Traduz uma `Keystroke` em bytes para o PTY. Imprimíveis via `key_char`; setas como
/// CSI (ou SS3 em `app_cursor`/DECCKM); Home/End/PageUp·Down/Insert/Delete/F1-F12; Ctrl+letra.
fn keystroke_to_bytes(ks: &Keystroke, app_cursor: bool) -> Vec<u8> {
    let key = ks.key.as_str();
    if ks.modifiers.control && key.len() == 1 {
        if let Some(c) = key.chars().next() {
            if c.is_ascii_alphabetic() {
                return vec![(c.to_ascii_uppercase() as u8) & 0x1f];
            }
        }
    }
    // Setas/Home/End: ESC O X em application-cursor-keys, ESC [ X em modo normal.
    let csi = |c: u8| -> Vec<u8> {
        if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    match key {
        "enter" | "return" => return vec![0x0D],
        "backspace" => return vec![0x7f],
        "tab" => return vec![0x09],
        "escape" => return vec![0x1b],
        "space" => return vec![b' '],
        "up" => return csi(b'A'),
        "down" => return csi(b'B'),
        "right" => return csi(b'C'),
        "left" => return csi(b'D'),
        "home" => return csi(b'H'),
        "end" => return csi(b'F'),
        "pageup" => return vec![0x1b, b'[', b'5', b'~'],
        "pagedown" => return vec![0x1b, b'[', b'6', b'~'],
        "insert" => return vec![0x1b, b'[', b'2', b'~'],
        "delete" => return vec![0x1b, b'[', b'3', b'~'],
        "f1" => return vec![0x1b, b'O', b'P'],
        "f2" => return vec![0x1b, b'O', b'Q'],
        "f3" => return vec![0x1b, b'O', b'R'],
        "f4" => return vec![0x1b, b'O', b'S'],
        "f5" => return vec![0x1b, b'[', b'1', b'5', b'~'],
        "f6" => return vec![0x1b, b'[', b'1', b'7', b'~'],
        "f7" => return vec![0x1b, b'[', b'1', b'8', b'~'],
        "f8" => return vec![0x1b, b'[', b'1', b'9', b'~'],
        "f9" => return vec![0x1b, b'[', b'2', b'0', b'~'],
        "f10" => return vec![0x1b, b'[', b'2', b'1', b'~'],
        "f11" => return vec![0x1b, b'[', b'2', b'3', b'~'],
        "f12" => return vec![0x1b, b'[', b'2', b'4', b'~'],
        _ => {}
    }
    if let Some(kc) = &ks.key_char {
        if !kc.is_empty() {
            return kc.clone().into_bytes();
        }
    }
    if key.chars().count() == 1 {
        return key.as_bytes().to_vec();
    }
    Vec::new()
}

#[inline]
fn rgba(c: VtRgb) -> Rgba {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
}

/// Pinta UMA linha do grid: agrupa células de mesmo estilo em runs (flex-row de spans),
/// com a célula do cursor como bloco invertido (accent). Monoespaçado → tudo alinha.
fn render_line(cells: &[lina_core::VtCell], cursor_col: Option<usize>) -> impl IntoElement {
    struct Run {
        fg: VtRgb,
        bg: VtRgb,
        bold: bool,
        text: String,
    }
    let mut runs: Vec<Run> = Vec::new();
    for (col, cell) in cells.iter().enumerate() {
        let (fg, bg, bold) = if cursor_col == Some(col) {
            // Bloco do cursor: bg accent, fg escuro.
            (
                VtRgb {
                    r: 0x0a,
                    g: 0x0e,
                    b: 0x27,
                },
                VtRgb {
                    r: 0x7a,
                    g: 0xa2,
                    b: 0xf7,
                },
                false,
            )
        } else {
            (cell.fg, cell.bg, cell.bold)
        };
        let ch = match cell.c {
            '\0' | '\t' => ' ',
            other => other,
        };
        match runs.last_mut() {
            Some(r) if r.fg == fg && r.bg == bg && r.bold == bold => r.text.push(ch),
            _ => runs.push(Run {
                fg,
                bg,
                bold,
                text: ch.to_string(),
            }),
        }
    }
    div().flex().flex_row().children(runs.into_iter().map(|r| {
        let span = div()
            .bg(rgba(r.bg))
            .text_color(rgba(r.fg))
            .child(text!(r.text));
        if r.bold {
            span.font_weight(FontWeight::BOLD)
        } else {
            span
        }
    }))
}

/// Fator de zoom para um evento de scroll (gentil; o zoom absoluto é clampado na [`Camera`]).
fn scroll_zoom_factor(delta: ScrollDelta) -> f32 {
    let dy: f32 = match delta {
        ScrollDelta::Lines(p) => p.y,
        ScrollDelta::Pixels(p) => p.y / px(40.0),
    };
    (1.0 + dy * 0.08).clamp(0.5, 1.5)
}

/// Pinta o GRID VISÍVEL inteiro (snapshot atual) — uma linha por linha do viewport. `scale` =
/// zoom do canvas (a fonte monoespaçada escala junto, mantendo o alinhamento do grid).
fn render_grid(screen: &VtScreen, scale: f32) -> impl IntoElement {
    let cur = screen.cursor;
    div()
        .flex()
        .flex_col()
        .font_family("Menlo")
        .text_size(px(FONT_PX * scale))
        .children((0..screen.rows).map(move |r| {
            let cursor_col = (cur.visible && cur.line == r).then_some(cur.col);
            render_line(screen.row(r), cursor_col)
        }))
}

/// O canvas gpui — a ÚNICA parte que conhece o toolkit. O estado dos nós (add/remove, model,
/// grids) vive no [`NodeManager`] gpui-free; a view só renderiza, roteia input e foca.
struct WorkspaceView {
    nodes: NodeManager,
    input: Arc<dyn InputSink>,
    a2a: Arc<A2aTrigger>,
    focused: NodeId,
    focus: FocusHandle,
    /// Câmera 2D do canvas (pan + zoom). Transform world↔screen, culling e hit-test usam ela.
    camera: Camera,
    /// Arrasto do FUNDO (pan): `(mouse_inicial_em_tela, pan_inicial)` em px de tela.
    drag: Option<((f32, f32), (f32, f32))>,
    /// Z-order (profundidade) por nó: maior = mais à frente. O focado é bombeado ao topo.
    z_order: BTreeMap<NodeId, u64>,
    z_next: u64,
}

impl WorkspaceView {
    fn new(
        nodes: NodeManager,
        input: Arc<dyn InputSink>,
        a2a: Arc<A2aTrigger>,
        focused: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            nodes,
            input,
            a2a,
            focused,
            focus,
            camera: Camera::default(),
            drag: None,
            z_order: BTreeMap::new(),
            z_next: 0,
        }
    }

    /// Marca `node` como focado e o traz para a FRENTE (z mais alto): clicar/abrir um card o põe
    /// no topo da pilha (z-order).
    fn focus(&mut self, node: NodeId) {
        self.focused = node;
        self.z_next = self.z_next.wrapping_add(1);
        self.z_order.insert(node, self.z_next);
    }

    /// z (profundidade) de um nó: maior = mais à frente; nós nunca focados ficam no fundo (0).
    fn z_of(&self, node: &NodeId) -> u64 {
        self.z_order.get(node).copied().unwrap_or(0)
    }

    /// Cards em ordem de z CRESCENTE (último = topo) — base do render (desenha o topo por
    /// último) e do hit-test (resolve sobreposição pelo maior z).
    fn cards_z_asc(&self) -> Vec<(NodeId, (f32, f32))> {
        let mut cz: Vec<(NodeId, (f32, f32))> = self
            .nodes
            .cards()
            .iter()
            .map(|(id, nv)| (*id, (nv.x, nv.y)))
            .collect();
        cz.sort_by_key(|(id, _)| self.z_of(id));
        cz
    }

    /// Auto-pan: traz o card do nó `node` INTEIRAMENTE para dentro da viewport visível. É o que
    /// faz o terminal recém-criado **aparecer** — o [`next_free_slot`] pode posicioná-lo além da
    /// borda da janela (3 cards de 680px não cabem numa janela de 1450px), e o pan o revela.
    fn reveal(&mut self, node: NodeId, window: &Window) {
        let card = lock(&self.nodes.model).nodes.get(&node).map(|v| (v.x, v.y));
        if let Some(card) = card {
            let vp = window.viewport_size();
            self.camera.reveal(
                card,
                (CARD_W, CARD_H),
                (f32::from(vp.width), f32::from(vp.height)),
            );
        }
    }

    /// Fecha o nó focado, reposiciona o foco num nó vivo (nunca tela em branco) e o revela.
    fn close_focused(&mut self, window: &Window) {
        let node = self.focused;
        if let Err(e) = self.nodes.remove_node(node) {
            eprintln!("lina-gpui: fechar nó focado: {e}");
            return;
        }
        self.z_order.remove(&node);
        // `first` num let SEPARADO: solta o lock do model ANTES do reveal (que re-locka o
        // model) — em edition 2021 o guard temporário de um `if let` viveria por todo o bloco,
        // causando re-lock na mesma thread (deadlock).
        let first = lock(&self.nodes.model).order.first().copied();
        if let Some(first) = first {
            self.focus(first);
            self.reveal(first, window);
        }
    }

    fn handle_key(&mut self, ev: &KeyDownEvent, window: &Window) {
        let ks = &ev.keystroke;
        // Atalhos do canvas (NÃO vão para o PTY): ⌘T adiciona (foca+revela); ⌘⌫ fecha o focado;
        // ⌘+ / ⌘- dão zoom in/out no centro da viewport.
        if ks.modifiers.platform && ks.key == "t" {
            match self.nodes.add_node() {
                Ok(node) => {
                    self.focus(node);
                    self.reveal(node, window);
                }
                Err(e) => eprintln!("lina-gpui: add_node (⌘T): {e}"),
            }
            return;
        }
        if ks.modifiers.platform && ks.key == "backspace" {
            self.close_focused(window);
            return;
        }
        if ks.modifiers.platform && (ks.key == "=" || ks.key == "+" || ks.key == "-") {
            let vp = window.viewport_size();
            let center = (f32::from(vp.width) / 2.0, f32::from(vp.height) / 2.0);
            let factor = if ks.key == "-" { 0.8 } else { 1.25 };
            self.camera.zoom_by(center, factor);
            return;
        }
        if ks.modifiers.platform && ks.key == "0" {
            self.camera.reset(); // ⌘0: volta ao home (resgate da vista).
            return;
        }
        let grid = lock(&self.nodes.grids).get(&self.focused).cloned();
        let app_cursor = grid.map(|g| lock(&g).mode().app_cursor).unwrap_or(false);
        let bytes = keystroke_to_bytes(ks, app_cursor);
        if !bytes.is_empty() {
            self.input.submit(self.focused, WriteOp::HumanKeys(bytes));
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        // Os cards desenhados DERIVAM do NodeManager (não de 2 fixos) — add/remove refletem aqui.
        let mut cards = self.nodes.cards();
        let (pulse, connected, event_count, recovering) = {
            let m = lock(&self.nodes.model);
            (m.pulse, m.connected, m.event_count, m.recovering)
        };
        // O foco deve apontar SEMPRE a um nó vivo: se o focado saiu (✕/⌘⌫/pump), o destaque
        // sumiria e as teclas iriam p/ um nó morto. Reaponta para o primeiro card vivo.
        if !cards.iter().any(|(id, _)| *id == self.focused) {
            if let Some((first, _)) = cards.first() {
                self.focused = *first;
            }
        }
        // Limpa z_order de nós mortos: remoções pelo pump (NodeDied) não passam pelos handlers
        // de UI, então a entrada ficaria órfã (vazamento ao longo de uma sessão longa).
        self.z_order
            .retain(|id, _| cards.iter().any(|(cid, _)| cid == id));

        let vp = window.viewport_size();
        let viewport = (f32::from(vp.width), f32::from(vp.height));
        // NUNCA tela em branco: se NADA está visível e o usuário NÃO está arrastando, recentra a
        // câmera (⌘0 / botão 🏠 fazem o mesmo sob demanda). Durante o arrasto, deixa rolar livre.
        if self.drag.is_none()
            && !cards.is_empty()
            && !cards
                .iter()
                .any(|(_, nv)| card_visible(&self.camera, (nv.x, nv.y), (CARD_W, CARD_H), viewport))
        {
            self.camera.reset();
        }
        // IDs de elemento ESTÁVEIS por nó (independem de cull/z-order): ordena por NodeId, que é
        // total e estável p/ o mesmo conjunto de nós — o gpui reusa o elemento certo entre frames.
        let eid: BTreeMap<NodeId, usize> = {
            let mut ids: Vec<NodeId> = cards.iter().map(|(id, _)| *id).collect();
            ids.sort();
            ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect()
        };
        // Z-ORDER: desenha do fundo p/ o topo — o focado/maior z por último, sobre os demais.
        cards.sort_by_key(|(id, _)| self.z_of(id));
        let cam = self.camera;
        let focused = self.focused;

        let aura_color = if connected {
            rgb(0x7aa2f7)
        } else {
            rgb(0x222a44)
        };
        let mut root = div()
            .id("canvas")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(rgb(0x0a0e27))
            .border_2()
            .border_dashed()
            .border_color(aura_color)
            .text_color(rgb(0xc8d3f5))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, ev: &MouseDownEvent, window, _cx| {
                    // PAN só ao arrastar o FUNDO (não sobre um card VISÍVEL → sem zona morta).
                    let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                    let v = window.viewport_size();
                    let vp = (f32::from(v.width), f32::from(v.height));
                    if hit_test(&view.camera, pos, &view.cards_z_asc(), (CARD_W, CARD_H), vp)
                        .is_none()
                    {
                        view.drag = Some((pos, view.camera.pan));
                    }
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _w, _cx| {
                if ev.dragging() {
                    if let Some((mouse0, pan0)) = view.drag {
                        let now = (f32::from(ev.position.x), f32::from(ev.position.y));
                        view.camera.pan = (pan0.0 + now.0 - mouse0.0, pan0.1 + now.1 - mouse0.1);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _ev, _w, _cx| {
                    view.drag = None;
                }),
            )
            .on_scroll_wheel(cx.listener(|view, ev: &ScrollWheelEvent, window, _cx| {
                // ZOOM ao rolar no FUNDO; sobre um card VISÍVEL, o handler do card cuida.
                let cursor = (f32::from(ev.position.x), f32::from(ev.position.y));
                let v = window.viewport_size();
                let vp = (f32::from(v.width), f32::from(v.height));
                if hit_test(
                    &view.camera,
                    cursor,
                    &view.cards_z_asc(),
                    (CARD_W, CARD_H),
                    vp,
                )
                .is_none()
                {
                    view.camera.zoom_by(cursor, scroll_zoom_factor(ev.delta));
                }
            }))
            .on_key_down(cx.listener(|view, ev: &KeyDownEvent, window, _cx| {
                view.handle_key(ev, window);
            }));

        for (idx, (id, nv)) in cards.iter().enumerate() {
            let node_id = *id;
            // ID de elemento ESTÁVEL por nó (não o idx do loop, que muda com cull/z-order).
            let card_eid = eid.get(&node_id).copied().unwrap_or(idx);
            // CULLING: cards fora da viewport não geram trabalho de render (grid pulado).
            if !card_visible(&cam, (nv.x, nv.y), (CARD_W, CARD_H), viewport) {
                continue;
            }
            let (sx, sy) = cam.world_to_screen((nv.x, nv.y));
            let z = cam.zoom;
            let card_border = if node_id == focused {
                rgb(0x7aa2f7)
            } else {
                rgb(0x2a3152)
            };
            let status_dot = match nv.status {
                NodeStatus::Idle => rgb(0x9ece6a),
                NodeStatus::Busy | NodeStatus::Running => rgb(0xe0af68),
                NodeStatus::Crashed | NodeStatus::Dead => rgb(0xf7768e),
                _ => rgb(0x7aa2f7),
            };

            let mut title = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(0x141a36))
                .text_size(px(13.0 * z))
                .text_color(rgb(0x7aa2f7))
                .child(div().size(px(9.0 * z)).rounded_full().bg(status_dot))
                .child(text!(nv.name.clone()))
                .child(
                    div()
                        .text_color(rgb(0x5b658f))
                        .child(text!(format!("· {:?} · {:?}", nv.kind, nv.status))),
                );
            // O ✕ só aparece quando há >1 card: o ÚLTIMO nó nunca pode ser fechado (o canvas
            // jamais fica em branco), então não mostramos um botão que recusaria a ação.
            if cards.len() > 1 {
                title = title.child(div().flex_1()).child(
                    div()
                        .id(("close", card_eid))
                        .px_2()
                        .rounded_md()
                        .bg(rgb(0x33202c))
                        .text_color(rgb(0xf7768e))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _ev: &ClickEvent, window, _cx| {
                            if let Err(e) = view.nodes.remove_node(node_id) {
                                eprintln!("lina-gpui: fechar terminal: {e}");
                                return;
                            }
                            view.z_order.remove(&node_id);
                            // Reposiciona o foco num nó vivo (nunca tela em branco) e o revela.
                            // `first` em let separado: solta o lock do model antes do reveal.
                            if view.focused == node_id {
                                let first = lock(&view.nodes.model).order.first().copied();
                                if let Some(first) = first {
                                    view.focus(first);
                                    view.reveal(first, window);
                                }
                            }
                        }))
                        .child(text!("✕")),
                );
            }

            // O conteúdo do terminal: o SNAPSHOT do grid lido AGORA (não deltas).
            let mut body = div()
                .id(("grid", card_eid))
                .role(Role::Terminal)
                .aria_label(nv.name.clone())
                .flex_1()
                .overflow_hidden()
                .p_1()
                .bg(rgb(0x0d1228));
            let grid = lock(&self.nodes.grids).get(&node_id).cloned();
            if let Some(g) = grid {
                let screen = lock(&g).screen();
                body = body.child(render_grid(&screen, z));
            }

            let card = div()
                .id(("card", card_eid))
                .absolute()
                .left(px(sx))
                .top(px(sy))
                .w(px(CARD_W * z))
                .h(px(CARD_H * z))
                .flex()
                .flex_col()
                .bg(rgb(0x0d1228))
                .border_2()
                .border_color(card_border)
                .rounded_md()
                .overflow_hidden()
                .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, _cx| {
                    // Só foca se o nó ainda existe (o ✕ pode tê-lo removido neste clique). Focar
                    // bombeia o card ao topo (z-order).
                    if lock(&view.nodes.model).nodes.contains_key(&node_id) {
                        view.focus(node_id);
                    }
                }))
                .on_scroll_wheel(cx.listener(move |view, ev: &ScrollWheelEvent, _w, _cx| {
                    // ⌘+scroll sobre o card = zoom do canvas; scroll normal = rolar o terminal.
                    if ev.modifiers.platform {
                        let cursor = (f32::from(ev.position.x), f32::from(ev.position.y));
                        view.camera.zoom_by(cursor, scroll_zoom_factor(ev.delta));
                        return;
                    }
                    let dy: f32 = match ev.delta {
                        ScrollDelta::Lines(p) => p.y,
                        ScrollDelta::Pixels(p) => p.y / px(CELL_H),
                    };
                    let delta = dy.round() as i32;
                    if delta != 0 {
                        let grid = lock(&view.nodes.grids).get(&node_id).cloned();
                        if let Some(g) = grid {
                            lock(&g).scroll(delta);
                        }
                    }
                }))
                .child(title)
                .child(body);

            root = root.child(card);
        }

        // PULSO efêmero A→B.
        if let Some(p) = pulse {
            if let Some(t) = p.progress() {
                let center = |id: NodeId| -> Option<Point<Pixels>> {
                    cards.iter().find(|(n, _)| *n == id).map(|(_, nv)| {
                        let (cx2, cy2) =
                            cam.world_to_screen((nv.x + CARD_W / 2.0, nv.y + CARD_H / 2.0));
                        point(px(cx2), px(cy2))
                    })
                };
                if let (Some(a), Some(b)) = (center(p.from), center(p.to)) {
                    let dot_x = a.x + (b.x - a.x) * t;
                    let dot_y = a.y + (b.y - a.y) * t;
                    let alpha = (1.0 - t).clamp(0.0, 1.0);
                    root = root.child(
                        div()
                            .absolute()
                            .left(dot_x - px(13.0))
                            .top(dot_y - px(13.0))
                            .size(px(26.0))
                            .rounded_full()
                            .bg(rgb(0x7aa2f7))
                            .opacity(alpha * 0.35),
                    );
                    root = root.child(
                        div()
                            .absolute()
                            .left(dot_x - px(6.0))
                            .top(dot_y - px(6.0))
                            .size(px(12.0))
                            .rounded_full()
                            .bg(rgb(0xbb9af7))
                            .opacity(alpha),
                    );
                }
            }
        }

        // Barra superior.
        let mut topbar = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .px_4()
            .py_2()
            .bg(rgb(0x0c1130))
            .text_color(rgb(0x7aa2f7))
            .child(text!(
                "Lina Space · terminais reais (clique p/ focar · digite · scroll)"
            ));

        if connected {
            topbar = topbar.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(8.0)).rounded_full().bg(rgb(0x9ece6a)))
                    .child(
                        div()
                            .text_color(rgb(0x9ece6a))
                            .child(text!("Time conectado · todos se falam")),
                    ),
            );
        }

        topbar = topbar.child(
            div()
                .text_color(rgb(0x5b658f))
                .child(text!(format!("log: {event_count} eventos"))),
        );

        // O ⚡ A2A (A→B) só aparece com AMBOS os alvos vivos: se o fundador fechar A ou B,
        // escondemos o botão em vez de deixá-lo falhar em silêncio.
        if self.a2a.ready() {
            topbar = topbar.child(
                div()
                    .id("a2a-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x3d59c9))
                    .text_color(rgb(0xeef1ff))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                        // O B é um shell real: injeta um COMANDO válido (roda limpo nele).
                        view.a2a.fire(
                            "echo '📨 A2A recebido de Terminal A · cooperacao sem fios'"
                                .to_string(),
                        );
                    }))
                    .child(text!("⚡ Enviar A2A (A→B)")),
            );
        }

        topbar = topbar.child(
            div()
                .id("add-terminal-btn")
                .px_3()
                .py_1()
                .rounded_md()
                .bg(rgb(0x2c7a4b))
                .text_color(rgb(0xeef1ff))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, window, _cx| {
                    // Cria um shell real novo, idêntico aos demais, foca e o TRAZ p/ a viewport.
                    match view.nodes.add_node() {
                        Ok(node) => {
                            view.focus(node);
                            view.reveal(node, window);
                        }
                        Err(e) => eprintln!("lina-gpui: add_node: {e}"),
                    }
                }))
                .child(text!("➕ Terminal")),
        );

        // 🏠 Centralizar: resgata a vista (pan/zoom → home). Sempre visível p/ o não-técnico
        // nunca ficar perdido no canvas (mesmo efeito do ⌘0).
        topbar = topbar.child(
            div()
                .id("home-btn")
                .px_3()
                .py_1()
                .rounded_md()
                .bg(rgb(0x2a3152))
                .text_color(rgb(0xc8d3f5))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                    view.camera.reset();
                }))
                .child(text!("🏠 Centralizar")),
        );

        if recovering {
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0xf7768e))
                    .text_color(rgb(0x11111b))
                    .child(text!("⟳ Recuperando…")),
            );
        }

        root.child(topbar)
    }
}

fn main() {
    let (cols, rows) = fit_dims();

    let dir = std::env::temp_dir().join("lina-space-ws2");
    let store = Arc::new(Mutex::new(
        EventStore::open(&dir).expect("abrir EventStore"),
    ));

    let pty = Arc::new(Mutex::new(PtyManager::new()));
    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    // Fábrica de nós: todo terminal é um SHELL INTERATIVO REAL e COMPLETO — idêntico em
    // capacidade (o B aceita teclado e roda claude/vim igual ao A). Add/remove em runtime
    // reusa EXATAMENTE esta fábrica via NodeManager.
    let cmd_factory: CmdFactory = Arc::new(shell_cmd);

    // Os 2 nós iniciais (A, B), pela MESMA fiação dos nós adicionados em runtime.
    let (node_a, grid_a) = {
        let mut p = lock(&pty);
        wire_terminal(
            &mut p,
            &sup,
            &delta_tx,
            "A",
            "Terminal A",
            (*cmd_factory)("Terminal A"),
            cols,
            rows,
        )
    }
    .expect("wire A");
    let (node_b, grid_b) = {
        let mut p = lock(&pty);
        wire_terminal(
            &mut p,
            &sup,
            &delta_tx,
            "B",
            "Terminal B",
            (*cmd_factory)("Terminal B"),
            cols,
            rows,
        )
    }
    .expect("wire B");
    let _ = sup.set_status(node_a, CoreStatus::Running);
    let _ = sup.set_status(node_b, CoreStatus::Running);

    {
        let mut s = lock(&store);
        let _ = s.append(&DomainEvent::WorkspaceCreated {
            name: "walking-skeleton".into(),
        });
        for (node, name, x, y) in [
            (node_a, "Terminal A", 30.0_f64, 96.0_f64),
            (node_b, "Terminal B", 740.0, 96.0),
        ] {
            let _ = s.append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x,
                y,
            });
            let _ = s.append(&DomainEvent::TerminalSpawned {
                node,
                cli: name.into(),
            });
        }
    }
    let event_count = lock(&store).event_count().unwrap_or(0);

    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    {
        let mut m = lock(&model);
        m.seed_node(
            node_a,
            NodeView::new("Terminal A", NodeKind::Terminal, 30.0, 96.0),
        );
        m.seed_node(
            node_b,
            NodeView::new("Terminal B", NodeKind::Terminal, 740.0, 96.0),
        );
        m.event_count = event_count;
    }

    let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
    {
        let mut g = lock(&grids);
        g.insert(node_a, Arc::clone(&grid_a));
        g.insert(node_b, Arc::clone(&grid_b));
    }
    let mut keys: BTreeMap<NodeId, String> = BTreeMap::new();
    keys.insert(node_a, "A".to_string());
    keys.insert(node_b, "B".to_string());

    let bridge = GpuiBridgeHost::new(Arc::clone(&model));
    let mut pump = spawn_pump(bridge, delta_rx, bus_rx).expect("subir a ponte core→UI");

    let input: Arc<dyn InputSink> = Arc::new(CoreInput::new(Arc::clone(&sup)));
    let a2a = Arc::new(A2aTrigger::new(
        Arc::clone(&sup),
        node_a,
        node_b,
        Arc::clone(&grid_b),
        demo_profile(),
        Arc::clone(&store),
        Arc::clone(&model),
    ));

    // O dono dos nós em runtime (add/remove): reusa pty/sup/store/model/grids. O `seq` inicia
    // em 2 porque A=0 e B=1 já consumiram os rótulos A/B.
    let nodes = NodeManager::new(
        Arc::clone(&pty),
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&model),
        Arc::clone(&grids),
        keys,
        delta_tx,
        cols,
        rows,
        2,
        cmd_factory,
    );

    eprintln!(
        "lina-gpui: render de terminal · grid {cols}x{rows} · log {} · {event_count} eventos",
        dir.display()
    );

    // DEMO: timer de auto-quit da janela. Antes ele só existia quando a env
    // LINA_AUTOQUIT_MS estava setada (caminho de smoke/CI: dispara um A2A no meio e
    // fecha); sem a env, a janela ficava aberta indefinidamente. Para o fundador
    // (não-técnico) validar o Terminal B COM CALMA, agora há um auto-quit PADRÃO de
    // 5 min quando a env não é dada — sem A2A surpresa.
    // DEMO: reverter para sem-auto-quit-padrão (timer só via env) após validação.
    const DEMO_AUTOQUIT_MS: u64 = 1_800_000; // 30 min (demo do fundador — reverter p/ 300_000 ou só-env após validar)
    match std::env::var("LINA_AUTOQUIT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
    {
        // Smoke/CI: a env (curta) dispara um A2A no meio e fecha — comportamento de teste.
        Some(ms) => {
            let a2a_auto = Arc::clone(&a2a);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms / 2));
                a2a_auto.fire("echo '📨 A2A automatico (smoke)'".to_string());
                thread::sleep(Duration::from_millis(ms / 2));
                std::process::exit(0);
            });
        }
        // DEMO: sem env → auto-quit calmo de 5 min (sem A2A surpresa) p/ validação visual.
        None => {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(DEMO_AUTOQUIT_MS));
                std::process::exit(0);
            });
        }
    }

    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1450.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lina Space — terminais reais (render de grid · sem fios)".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| WorkspaceView::new(nodes, input, a2a, node_a, window, cx)),
        )
        .expect("abrir a janela gpui");
        cx.activate(true);
    });

    pump.stop();
    let _ = grid_a;
    drop(sup);
    drop(pty);
}
