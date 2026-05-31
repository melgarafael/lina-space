//! `lina-gpui` — o **SHELL gpui** do Lina Space (Onda 2): a janela que projeta um
//! **terminal real do CORE** pela ponte `lina_host::UiHost`/`InputSink`.
//!
//! Esta é a ÚNICA camada que conhece o toolkit (gpui). O core (Supervisor + pty-host +
//! Event Store) fica atrás de `UiHost`; toda a tradução vive em [`bridge`] (livre de gpui).
//! Output → render: `PTY → pty-host → GridDelta → UiHost → SharedModel → gpui`. O shell
//! **nunca** lê o PTY direto — só projeta o grid que o core parseou.

mod bridge;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpui::{
    div, prelude::*, px, rgb, size, text, App, Bounds, Context, FocusHandle, KeyDownEvent,
    Keystroke, Render, Role, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

use lina_core::{NodeStatus as CoreStatus, PtyCommand, PtyHost, Supervisor};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use bridge::{lock, spawn_pump, CoreInput, GpuiBridgeHost, Model, SharedModel, COLS, ROWS};

/// Traduz uma `Keystroke` do gpui em bytes para o PTY (imprimíveis via `key_char`,
/// Enter/Backspace/Tab/Esc/setas e Ctrl+letra). É a ÚNICA peça gpui-acoplada do input;
/// o `CoreInput` da ponte recebe só `Vec<u8>` (fica gpui-free).
fn keystroke_to_bytes(ks: &Keystroke) -> Vec<u8> {
    let key = ks.key.as_str();
    // Ctrl + letra → byte de controle (Ctrl-C = 0x03, Ctrl-D = 0x04, …).
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
        "home" => return vec![0x1b, b'[', b'H'],
        "end" => return vec![0x1b, b'[', b'F'],
        "delete" => return vec![0x1b, b'[', b'3', b'~'],
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

/// A view gpui — a ÚNICA parte que conhece o toolkit. Lê o `SharedModel` (projetado
/// pela ponte) a cada frame e desenha o grid; encaminha teclas via `InputSink`.
struct WorkspaceView {
    model: Model,
    input: Arc<dyn InputSink>,
    node: NodeId,
    focus: FocusHandle,
}

impl WorkspaceView {
    fn new(
        model: Model,
        input: Arc<dyn InputSink>,
        node: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            model,
            input,
            node,
            focus,
        }
    }

    fn handle_key(&mut self, ev: &KeyDownEvent) {
        let bytes = keystroke_to_bytes(&ev.keystroke);
        if !bytes.is_empty() {
            self.input.submit(self.node, WriteOp::HumanKeys(bytes));
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Poll do SharedModel a cada frame (padrão provado no spike W1-1).
        window.request_animation_frame();

        let (rows, status, kind, recovering) = {
            let m = lock(&self.model);
            let nv = m.nodes.get(&self.node);
            (
                nv.map(|n| n.rows.clone())
                    .unwrap_or_else(|| vec![String::new(); ROWS as usize]),
                nv.map(|n| n.status).unwrap_or(NodeStatus::Starting),
                nv.map(|n| n.kind).unwrap_or(NodeKind::Terminal),
                m.recovering,
            )
        };

        let status_color = match status {
            NodeStatus::Idle => rgb(0x9ece6a),
            NodeStatus::Busy | NodeStatus::Running => rgb(0xe0af68),
            NodeStatus::Crashed | NodeStatus::Dead => rgb(0xf7768e),
            _ => rgb(0x7aa2f7),
        };

        let header = div()
            .id("hdr")
            .flex()
            .flex_row()
            .gap_3()
            .px_3()
            .py_2()
            .bg(rgb(0x16161e))
            .text_color(rgb(0x7aa2f7))
            .child(text!("Lina Space · shell gpui ⟶ core · 1 terminal real"))
            .child(
                div()
                    .text_color(status_color)
                    .child(text!(format!("{kind:?} ● {status:?}"))),
            );

        let mut root = div()
            .id("workspace")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, ev: &KeyDownEvent, _window, _cx| {
                view.handle_key(ev);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0b0b12))
            .text_color(rgb(0xc8d3f5))
            .font_family("Menlo")
            .text_size(px(13.0))
            .child(header);

        if recovering {
            root = root.child(
                div()
                    .id("recovery")
                    .px_3()
                    .py_2()
                    .bg(rgb(0xf7768e))
                    .text_color(rgb(0x11111b))
                    .child(text!("⟳ Recuperando estado pós-crash do event log…")),
            );
        }

        root.child(
            div()
                .id("terminal")
                .role(Role::Terminal)
                .aria_label("Terminal do core (lina-core pty-host)")
                .flex()
                .flex_col()
                .p_3()
                .children(rows.into_iter().enumerate().map(|(i, line)| {
                    let line = line.trim_end().to_string();
                    div().id(("row", i)).child(text!(if line.is_empty() {
                        " ".to_string()
                    } else {
                        line
                    }))
                })),
        )
    }
}

fn main() {
    // 1) CORE — pty-host: 1 PTY real (sh) vira o pipeline de render (grid → GridDelta).
    let mut pty_host = PtyHost::new();
    let term_id = pty_host
        .spawn(
            PtyCommand::new("sh")
                .arg("-c")
                .arg("printf 'Lina Space - terminal real do core (gpui). Digite; Enter executa.\\r\\n'; exec sh -i")
                .env("TERM", "xterm-256color"),
            COLS,
            ROWS,
        )
        .expect("spawn do PTY real (sh) no pty-host");
    let delta_rx = pty_host
        .take_delta_receiver()
        .expect("delta receiver do pty-host (uma vez)");

    // 2) CORE — Supervisor (roster + bus). Assina ANTES de registrar p/ capturar o
    //    NodeSpawned. O pty-host é dono do writer (input via PtyHost::write); o
    //    Supervisor só precisa de presença/status no bus → writer `io::sink` (seam).
    let sup = Supervisor::new();
    let bus_rx = sup.subscribe();
    let sup_id = sup.register(
        "terminal-1",
        Some("terminal".into()),
        Box::new(std::io::sink()),
    );
    let _ = sup.set_status(sup_id, CoreStatus::Running);

    // 3) Estado projetado (SÓ dados — sem tipos gpui) + handles compartilhados.
    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    let host = Arc::new(Mutex::new(pty_host));

    // 4) PONTE — a thread-bomba projeta deltas/eventos do core no model (livre de gpui).
    let bridge = GpuiBridgeHost::new(Arc::clone(&model), Arc::clone(&host));
    let remap = move |id: NodeId| if id == sup_id { term_id } else { id };
    let mut pump = spawn_pump(bridge, Arc::clone(&host), delta_rx, bus_rx, remap)
        .expect("subir a ponte core→UI (thread-bomba)");

    // 5) Input — o shell chama InputSink::submit; roteia p/ o writer do pty-host.
    let input: Arc<dyn InputSink> = Arc::new(CoreInput::new(Arc::clone(&host)));

    // Watchdog opcional p/ verificação headless: LINA_AUTOQUIT_MS encerra o processo.
    if let Ok(raw) = std::env::var("LINA_AUTOQUIT_MS") {
        if let Ok(ms) = raw.parse::<u64>() {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms));
                std::process::exit(0);
            });
        }
    }

    // 6) SHELL gpui — 1 janela projetando o terminal real do core.
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.0), px(560.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lina Space — shell gpui (terminal real do core)".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| WorkspaceView::new(model, input, term_id, window, cx)),
        )
        .expect("abrir a janela gpui");
        cx.activate(true);
    });

    pump.stop();
    drop(host);
}
