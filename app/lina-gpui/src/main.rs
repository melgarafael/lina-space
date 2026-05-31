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
    DomainEvent, EventStore, NodeStatus as CoreStatus, PtyCommand, PtyManager, Supervisor, VtRgb,
    VtScreen,
};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use bridge::{
    demo_profile, lock, spawn_pump, wire_terminal, A2aTrigger, CoreInput, GpuiBridgeHost, Grid,
    Model, NodeView, SharedModel,
};

const CARD_W: f32 = 680.0;
const CARD_H: f32 = 500.0;
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

/// Comando de um terminal: um **SHELL INTERATIVO REAL** — o do usuário (`$SHELL`) com
/// fallback `/bin/sh`. Idêntico para TODOS os nós: aceita teclado, roda claude/codex/vim,
/// etc. Mostra um banner neutro e dá `exec` no shell (sem mock/`cat`).
fn shell_cmd(name: &str) -> PtyCommand {
    PtyCommand::new("sh")
        .arg("-c")
        .arg(format!(
            "printf '{name} — shell interativo (digite comandos; rode claude/vim/…).\\r\\n'; \
             exec \"${{SHELL:-/bin/sh}}\" -i"
        ))
        .env("TERM", "xterm-256color")
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

/// Pinta o GRID VISÍVEL inteiro (snapshot atual) — uma linha por linha do viewport.
fn render_grid(screen: &VtScreen) -> impl IntoElement {
    let cur = screen.cursor;
    div()
        .flex()
        .flex_col()
        .font_family("Menlo")
        .text_size(px(FONT_PX))
        .children((0..screen.rows).map(move |r| {
            let cursor_col = (cur.visible && cur.line == r).then_some(cur.col);
            render_line(screen.row(r), cursor_col)
        }))
}

/// O canvas gpui — a ÚNICA parte que conhece o toolkit.
struct WorkspaceView {
    model: Model,
    grids: BTreeMap<NodeId, Grid>,
    input: Arc<dyn InputSink>,
    a2a: Arc<A2aTrigger>,
    focused: NodeId,
    focus: FocusHandle,
    pan: Point<Pixels>,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
}

impl WorkspaceView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        model: Model,
        grids: BTreeMap<NodeId, Grid>,
        input: Arc<dyn InputSink>,
        a2a: Arc<A2aTrigger>,
        focused: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            model,
            grids,
            input,
            a2a,
            focused,
            focus,
            pan: point(px(0.0), px(0.0)),
            drag: None,
        }
    }

    fn handle_key(&mut self, ev: &KeyDownEvent) {
        let app_cursor = self
            .grids
            .get(&self.focused)
            .map(|g| lock(g).mode().app_cursor)
            .unwrap_or(false);
        let bytes = keystroke_to_bytes(&ev.keystroke, app_cursor);
        if !bytes.is_empty() {
            self.input.submit(self.focused, WriteOp::HumanKeys(bytes));
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        let (cards, pulse, connected, event_count, recovering) = {
            let m = lock(&self.model);
            let cards: Vec<(NodeId, NodeView)> = m
                .order
                .iter()
                .filter_map(|n| m.nodes.get(n).map(|v| (*n, v.clone())))
                .collect();
            (cards, m.pulse, m.connected, m.event_count, m.recovering)
        };
        let pan = self.pan;
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
                cx.listener(|view, ev: &MouseDownEvent, _w, _cx| {
                    view.drag = Some((ev.position, view.pan));
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _w, _cx| {
                if ev.dragging() {
                    if let Some((mouse0, pan0)) = view.drag {
                        view.pan = pan0 + (ev.position - mouse0);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _ev, _w, _cx| {
                    view.drag = None;
                }),
            )
            .on_key_down(cx.listener(|view, ev: &KeyDownEvent, _w, _cx| {
                view.handle_key(ev);
            }));

        for (idx, (id, nv)) in cards.iter().enumerate() {
            let x = px(nv.x) + pan.x;
            let y = px(nv.y) + pan.y;
            let node_id = *id;
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

            let title = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(0x141a36))
                .text_color(rgb(0x7aa2f7))
                .child(div().size(px(9.0)).rounded_full().bg(status_dot))
                .child(text!(nv.name.clone()))
                .child(
                    div()
                        .text_color(rgb(0x5b658f))
                        .child(text!(format!("· {:?} · {:?}", nv.kind, nv.status))),
                );

            // O conteúdo do terminal: o SNAPSHOT do grid lido AGORA (não deltas).
            let mut body = div()
                .id(("grid", idx))
                .role(Role::Terminal)
                .aria_label(nv.name.clone())
                .flex_1()
                .overflow_hidden()
                .p_1()
                .bg(rgb(0x0d1228));
            if let Some(g) = self.grids.get(&node_id) {
                let screen = lock(g).screen();
                body = body.child(render_grid(&screen));
            }

            let card = div()
                .id(("card", idx))
                .absolute()
                .left(x)
                .top(y)
                .w(px(CARD_W))
                .h(px(CARD_H))
                .flex()
                .flex_col()
                .bg(rgb(0x0d1228))
                .border_2()
                .border_color(card_border)
                .rounded_md()
                .overflow_hidden()
                .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, _cx| {
                    view.focused = node_id;
                }))
                .on_scroll_wheel(cx.listener(move |view, ev: &ScrollWheelEvent, _w, _cx| {
                    let dy: f32 = match ev.delta {
                        ScrollDelta::Lines(p) => p.y,
                        ScrollDelta::Pixels(p) => p.y / px(CELL_H),
                    };
                    let delta = dy.round() as i32;
                    if delta != 0 {
                        if let Some(g) = view.grids.get(&node_id) {
                            lock(g).scroll(delta);
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
                        point(
                            px(nv.x + CARD_W / 2.0) + pan.x,
                            px(nv.y + CARD_H / 2.0) + pan.y,
                        )
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

        topbar = topbar
            .child(
                div()
                    .text_color(rgb(0x5b658f))
                    .child(text!(format!("log: {event_count} eventos"))),
            )
            .child(
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

    let mut pty = PtyManager::new();
    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    // Todo terminal é um SHELL INTERATIVO REAL e COMPLETO — idêntico em capacidade.
    // Não há terminal mock/receptor: o B aceita teclado e roda claude/vim igual ao A.
    let cmd_a = shell_cmd("Terminal A");
    let cmd_b = shell_cmd("Terminal B");

    let (node_a, grid_a) = wire_terminal(
        &mut pty,
        &sup,
        &delta_tx,
        "A",
        "Terminal A",
        cmd_a,
        cols,
        rows,
    )
    .expect("wire A");
    let (node_b, grid_b) = wire_terminal(
        &mut pty,
        &sup,
        &delta_tx,
        "B",
        "Terminal B",
        cmd_b,
        cols,
        rows,
    )
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

    let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
    grids.insert(node_a, Arc::clone(&grid_a));
    grids.insert(node_b, Arc::clone(&grid_b));

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

    eprintln!(
        "lina-gpui: render de terminal · grid {cols}x{rows} · log {} · {event_count} eventos",
        dir.display()
    );

    if let Ok(raw) = std::env::var("LINA_AUTOQUIT_MS") {
        if let Ok(ms) = raw.parse::<u64>() {
            let a2a_auto = Arc::clone(&a2a);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms / 2));
                a2a_auto.fire("echo '📨 A2A automatico (smoke)'".to_string());
                thread::sleep(Duration::from_millis(ms / 2));
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
            |window, cx| {
                cx.new(|cx| WorkspaceView::new(model, grids, input, a2a, node_a, window, cx))
            },
        )
        .expect("abrir a janela gpui");
        cx.activate(true);
    });

    pump.stop();
    let _ = grid_a;
    drop(sup);
    drop(pty);
}
