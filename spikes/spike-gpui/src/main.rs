//! Spike **W1-1 — gpui**: renderiza 1 nó-terminal **real** (lina-pty + lina-vt) numa
//! janela gpui e mede o que der (frame timing, integração AccessKit), documentando
//! esforço/obstáculos. Objetivo: dado para a decisão de framework (W1-4), não produto.
//!
//! Fluxo: um PTY real roda `cat`; uma thread produtora injeta linhas vivas no master;
//! uma thread leitora drena o PTY e alimenta o grid `alacritty_terminal` via `VtBackend`
//! (lina-vt). A cada frame, o gpui amostra a saída **parseada** (`last_nonempty_line`),
//! mantém um scrollback e desenha como texto monoespaçado dentro de um nó acessível
//! `Role::Terminal` (AccessKit). Mede intervalos de frame (p50/p99) e encerra sozinho.
//!
//! Resultado factual do esforço: `RESULTADO.md`.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, rgb, size, text, App, Bounds, Context, Role, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

use lina_pty::{PtyCommand, PtyManager};
use lina_vt::{AlacrittyBackend, VtBackend};

const COLS: u16 = 80;
const ROWS: u16 = 24;
const SCROLLBACK: usize = 22;
const MAX_FRAMES: u64 = 1_000_000;
const WALL_CAP: Duration = Duration::from_secs(300);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Estado da view: o grid VT compartilhado com a thread leitora + métricas de frame.
struct TermView {
    grid: Arc<Mutex<AlacrittyBackend>>,
    stop: Arc<AtomicBool>,
    _pty: PtyManager, // mantém o PTY vivo enquanto a janela existe
    lines: Vec<String>,
    start: Instant,
    last_frame: Option<Instant>,
    intervals_ms: Vec<f64>,
    frames: u64,
    done: bool,
}

impl TermView {
    /// Empurra a linha viva parseada para o scrollback (dedup de linha repetida).
    fn sample(&mut self) {
        let live = lock(&self.grid).last_nonempty_line();
        let live = live.trim_end().to_string();
        if !live.is_empty() && self.lines.last().map(String::as_str) != Some(live.as_str()) {
            self.lines.push(live);
            if self.lines.len() > SCROLLBACK {
                self.lines.remove(0);
            }
        }
    }

    fn metrics(&self) -> String {
        let mut s = self.intervals_ms.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pct = |p: f64| -> f64 {
            if s.is_empty() {
                return 0.0;
            }
            let idx = ((p / 100.0) * (s.len() as f64 - 1.0)).round() as usize;
            s[idx.min(s.len() - 1)]
        };
        let elapsed = self.start.elapsed().as_secs_f64();
        let fps = if elapsed > 0.0 {
            (self.frames.saturating_sub(1)) as f64 / elapsed
        } else {
            0.0
        };
        format!(
            "SPIKE_METRICS{{frames={}, elapsed_s={:.2}, fps={:.1}, frame_ms_p50={:.2}, \
             frame_ms_p99={:.2}, frame_ms_min={:.2}, frame_ms_max={:.2}, scrollback_lines={}, \
             last_line={:?}}}",
            self.frames,
            elapsed,
            fps,
            pct(50.0),
            pct(99.0),
            s.first().copied().unwrap_or(0.0),
            s.last().copied().unwrap_or(0.0),
            self.lines.len(),
            self.lines.last().cloned().unwrap_or_default(),
        )
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ── frame timing ────────────────────────────────────────────────
        let now = Instant::now();
        if let Some(prev) = self.last_frame {
            self.intervals_ms.push((now - prev).as_secs_f64() * 1000.0);
        }
        self.last_frame = Some(now);
        self.frames += 1;

        // ── amostra a saída PARSEADA por lina-vt ────────────────────────
        self.sample();

        // ── encerra sozinho (frames ou wall-clock) ──────────────────────
        let finishing = self.frames >= MAX_FRAMES || self.start.elapsed() >= WALL_CAP;
        if finishing && !self.done {
            self.done = true;
            self.stop.store(true, Ordering::Relaxed);
            println!("{}", self.metrics());
            cx.quit();
        } else if !self.done {
            // dirige o redraw contínuo (loop de animação no refresh do display)
            window.request_animation_frame();
        }

        // ── UI: um nó AccessKit Role::Terminal real + grid monoespaçado ──
        let aria = format!(
            "Terminal Lina (spike gpui) — {} linhas vivas",
            self.lines.len()
        );
        let rows = self.lines.clone();
        div()
            .id("lina-terminal")
            .role(Role::Terminal)
            .aria_label(SharedString::from(aria))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x11111b))
            .text_color(rgb(0xa6e3a1))
            .font_family("Menlo")
            .text_size(px(13.0))
            .p_3()
            .child(
                div()
                    .id("hdr")
                    .text_color(rgb(0x89b4fa))
                    .pb_2()
                    .child(text!("lina-space · spike gpui · PTY real → lina-vt → gpui")),
            )
            .children(rows.into_iter().enumerate().map(|(i, line)| {
                div()
                    .id(("line", i))
                    .child(text!(if line.is_empty() { " ".to_string() } else { line }))
            }))
    }
}

fn main() {
    // ── PTY REAL: `cat` ecoa o que escrevemos no master ─────────────────
    let mut pty = PtyManager::new();
    pty.spawn("spike", PtyCommand::new("cat"), COLS, ROWS)
        .expect("spawn do PTY real (cat)");
    let mut writer = pty.take_writer("spike").expect("take_writer (1 dono)");
    let mut reader = pty.clone_reader("spike").expect("clone_reader");

    let grid = Arc::new(Mutex::new(AlacrittyBackend::new(COLS, ROWS)));
    let stop = Arc::new(AtomicBool::new(false));

    // thread leitora: PTY → VtBackend (grid), como o pty-host faz no core.
    {
        let grid = Arc::clone(&grid);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => lock(&grid).advance(&buf[..n]),
                }
            }
        });
    }

    // thread produtora: injeta linhas vivas (~12ms) sem grandchild (throttle na thread).
    {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut i: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                let _ = write!(
                    writer,
                    "frame vivo {i:>6} · cooperacao sem fios · estado salvo\r\n"
                );
                let _ = writer.flush();
                i += 1;
                thread::sleep(Duration::from_millis(12));
            }
        });
    }

    // watchdog: garante que o processo encerra mesmo se a janela não fechar.
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(330));
        std::process::exit(0);
    });

    // ── gpui: abre 1 janela e roda o loop até o encerramento automático ─
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(760.0), px(460.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lina Spike — gpui + PTY real".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| {
                cx.new(|_cx| TermView {
                    grid: Arc::clone(&grid),
                    stop: Arc::clone(&stop),
                    _pty: pty,
                    lines: Vec::new(),
                    start: Instant::now(),
                    last_frame: None,
                    intervals_ms: Vec::new(),
                    frames: 0,
                    done: false,
                })
            },
        )
        .expect("open_window");
        cx.activate(true);
    });

    println!("spike-gpui: run() retornou (janela encerrada).");
}
