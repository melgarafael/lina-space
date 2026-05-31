//! `lina-gpui` — o **walking skeleton** do Lina Space (gate da Onda 2): o primeiro Lina
//! que se VÊ e se USA. Um canvas gpui com **2 terminais reais do core**, **pan**, e o
//! diferencial #1 — a metáfora **"sem fios"**: um A2A A→B dispara um **pulso efêmero**
//! visível (ao escutar `BusEvent::Message` do pub/sub do core) e o texto **chega no grid
//! de B** (round-trip faseado real, via `deliver_a2a`). Eventos persistidos no EventStore.
//!
//! Toda a tradução core↔UI vive em [`bridge`] (livre de gpui). Esta camada só desenha.

mod bridge;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, rgb, size, text, App, Bounds, ClickEvent, Context, FocusHandle,
    KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render,
    Role, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

use lina_core::{
    DomainEvent, EventStore, NodeStatus as CoreStatus, PtyCommand, PtyManager, Supervisor,
};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use bridge::{
    demo_profile, lock, spawn_pump, wire_terminal, A2aTrigger, CoreInput, GpuiBridgeHost, Grid,
    Model, NodeView, SharedModel,
};

const CARD_W: f32 = 420.0;
const CARD_H: f32 = 380.0;

/// Traduz uma `Keystroke` do gpui em bytes para o PTY (igual à story 1).
fn keystroke_to_bytes(ks: &Keystroke) -> Vec<u8> {
    let key = ks.key.as_str();
    if ks.modifiers.control && key.len() == 1 {
        if let Some(c) = key.chars().next() {
            if c.is_ascii_alphabetic() {
                return vec![(c.to_ascii_uppercase() as u8) & 0x1f];
            }
        }
    }
    match key {
        "enter" | "return" => return vec![0x0D],
        "backspace" => return vec![0x7f],
        "tab" => return vec![0x09],
        "escape" => return vec![0x1b],
        "space" => return vec![b' '],
        "up" => return vec![0x1b, b'[', b'A'],
        "down" => return vec![0x1b, b'[', b'B'],
        "right" => return vec![0x1b, b'[', b'C'],
        "left" => return vec![0x1b, b'[', b'D'],
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

/// O canvas gpui — a ÚNICA parte que conhece o toolkit.
struct WorkspaceView {
    model: Model,
    input: Arc<dyn InputSink>,
    a2a: Arc<A2aTrigger>,
    focused: NodeId,
    focus: FocusHandle,
    /// Offset de pan (arrastar o fundo). Tudo em `Pixels` (soma direta com `px(pos)`).
    pan: Point<Pixels>,
    /// `(mouse_inicial, pan_inicial)` enquanto arrasta o fundo.
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
}

impl WorkspaceView {
    fn new(
        model: Model,
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
            input,
            a2a,
            focused,
            focus,
            pan: point(px(0.0), px(0.0)),
            drag: None,
        }
    }

    fn handle_key(&mut self, ev: &KeyDownEvent) {
        let bytes = keystroke_to_bytes(&ev.keystroke);
        if !bytes.is_empty() {
            self.input.submit(self.focused, WriteOp::HumanKeys(bytes));
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Poll do SharedModel a cada frame (também anima o pulso efêmero).
        window.request_animation_frame();

        // Snapshot do estado projetado.
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

        // ── canvas (fundo escuro OLED + AURA pontilhada "sem fios") ──────────────
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
            .font_family("Menlo")
            // PAN: arrastar o fundo.
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

        // ── os 2 cards-terminal (posicionados; pan move todos juntos) ────────────
        for (idx, (id, nv)) in cards.iter().enumerate() {
            let x = px(nv.x) + pan.x;
            let y = px(nv.y) + pan.y;
            let card_border = if *id == focused {
                rgb(0x7aa2f7)
            } else {
                rgb(0x2a3152)
            };
            let node_id = *id;
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
                .child(div().size(px(9.0)).rounded_full().bg(status_dot))
                .child(text!(nv.name.clone()))
                .child(
                    div()
                        .text_color(rgb(0x5b658f))
                        .child(text!(format!("· {:?} · {:?}", nv.kind, nv.status))),
                );

            let grid = div()
                .id(("grid", idx))
                .role(Role::Terminal)
                .aria_label(nv.name.clone())
                .flex()
                .flex_col()
                .flex_1()
                .p_2()
                .text_size(px(12.0))
                .children(nv.rows.iter().enumerate().map(|(i, line)| {
                    let line = line.trim_end().to_string();
                    div()
                        .id(("row", idx * 1000 + i))
                        .child(text!(if line.is_empty() {
                            " ".to_string()
                        } else {
                            line
                        }))
                }));

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
                .child(title)
                .child(grid);

            root = root.child(card);
        }

        // ── PULSO efêmero A→B (a metáfora "sem fios": aparece e some em ~1s) ──────
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
                    // Pacote viajando de A→B (interpolação componente-a-componente).
                    let dot_x = a.x + (b.x - a.x) * t;
                    let dot_y = a.y + (b.y - a.y) * t;
                    let alpha = (1.0 - t).clamp(0.0, 1.0);
                    // Halo (ripple) + núcleo do pacote.
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

        // ── barra superior: título · selo "Time conectado" · log · botão A2A ─────
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
            .child(text!("Lina Space · walking skeleton"));

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
                        view.a2a
                            .fire("📨 A2A de A→B · time conectado (sem fios)".to_string());
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
    // ── PERSISTÊNCIA: EventStore num diretório estável (sobrevive entre execuções) ──
    let dir = std::env::temp_dir().join("lina-space-ws2");
    let store = Arc::new(Mutex::new(
        EventStore::open(&dir).expect("abrir EventStore"),
    ));

    // ── CORE: 2 PTYs reais, writers ao Supervisor (input + A2A pela MailQueue) ──────
    let mut pty = PtyManager::new();
    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    let cmd_a = PtyCommand::new("sh")
        .arg("-c")
        .arg("printf 'Terminal A — seu shell. Digite comandos (Enter executa).\\r\\n'; exec sh -i")
        .env("TERM", "xterm-256color");
    let cmd_b = PtyCommand::new("sh")
        .arg("-c")
        .arg("printf 'Terminal B — recebe mensagens A2A (sem fios).\\r\\n'; exec cat")
        .env("TERM", "xterm-256color");

    let (node_a, grid_a) =
        wire_terminal(&mut pty, &sup, &delta_tx, "A", "Terminal A", cmd_a).expect("wire A");
    let (node_b, grid_b) =
        wire_terminal(&mut pty, &sup, &delta_tx, "B", "Terminal B", cmd_b).expect("wire B");
    let _ = sup.set_status(node_a, CoreStatus::Running);
    let _ = sup.set_status(node_b, CoreStatus::Running);

    // ── PERSISTÊNCIA: grava a criação do workspace + nós + terminais ───────────────
    {
        let mut s = lock(&store);
        let _ = s.append(&DomainEvent::WorkspaceCreated {
            name: "walking-skeleton".into(),
        });
        for (node, name, x, y) in [
            (node_a, "Terminal A", 60.0_f64, 130.0_f64),
            (node_b, "Terminal B", 540.0, 130.0),
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

    // ── Estado projetado: semeia os 2 cards (nome + posição) ───────────────────────
    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    {
        let mut m = lock(&model);
        m.seed_node(
            node_a,
            NodeView::new("Terminal A", NodeKind::Terminal, 60.0, 130.0),
        );
        m.seed_node(
            node_b,
            NodeView::new("Terminal B", NodeKind::Terminal, 540.0, 130.0),
        );
        m.event_count = event_count;
    }

    // ── PONTE: grids → bridge; thread-bomba projeta deltas + Bus (pulso) ───────────
    let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
    grids.insert(node_a, Arc::clone(&grid_a));
    grids.insert(node_b, Arc::clone(&grid_b));
    let bridge = GpuiBridgeHost::new(Arc::clone(&model), grids);
    let mut pump = spawn_pump(bridge, delta_rx, bus_rx).expect("subir a ponte core→UI");

    // ── Input + gatilho A2A (A→B), reusando deliver_a2a + EventStore ───────────────
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
        "lina-gpui: walking skeleton · log em {} · {event_count} eventos",
        dir.display()
    );

    // Watchdog opcional só p/ verificação headless (NÃO liga por padrão → janela fica aberta).
    if let Ok(raw) = std::env::var("LINA_AUTOQUIT_MS") {
        if let Ok(ms) = raw.parse::<u64>() {
            // Dispara um A2A automático antes de encerrar, p/ o smoke exercitar o pulso.
            let a2a_auto = Arc::clone(&a2a);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms / 2));
                a2a_auto.fire("📨 A2A automatico (smoke)".to_string());
                thread::sleep(Duration::from_millis(ms / 2));
                std::process::exit(0);
            });
        }
    }

    // ── SHELL gpui: o canvas. Janela fica ABERTA até o usuário fechar. ─────────────
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1040.0), px(660.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lina Space — walking skeleton (2 terminais · sem fios)".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| WorkspaceView::new(model, input, a2a, node_a, window, cx)),
        )
        .expect("abrir a janela gpui");
        cx.activate(true);
    });

    pump.stop();
    drop(sup);
    drop(pty);
}
