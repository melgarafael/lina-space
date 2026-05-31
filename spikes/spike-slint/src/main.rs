//! Spike **W1-2 — Slint**: renderiza 1 nó-terminal REAL (lina-pty + lina-vt) numa
//! janela Slint e mede o que der (esforço, frame timing, AccessKit, IME).
//! Objetivo: **dado para a decisão de framework (W1-4)**, não produto.
//!
//! Pipeline: `PtyManager` abre um PTY rodando `sh` → o reader alimenta um
//! `AlacrittyBackend` (`VtBackend`) que parseia o grid → uma `slint::Timer` faz
//! poll do estado e atualiza a janela Slint (texto monoespaçado nativo).
//!
//! Limitação honesta (achado do spike): o `VtBackend` público só expõe
//! `last_nonempty_line()`; sem tocar o core, renderizamos uma **captura rolante**
//! dessa linha a cada `advance`. Render de grid completo pede um `renderable_rows()`
//! na trait (recomendação para a W2). Todo o texto exibido vem do parser do lina-vt.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lina_pty::{PtyCommand, PtyManager};
use lina_vt::{AlacrittyBackend, VtBackend};

slint::slint! {
    export component Main inherits Window {
        in property <string> grid-text;
        in property <string> telemetry;
        title: "Lina spike — Slint + lina-pty + lina-vt (W1-2)";
        preferred-width: 860px;
        preferred-height: 540px;
        background: #141414;
        VerticalLayout {
            padding: 10px;
            spacing: 6px;
            Text {
                text: root.telemetry;
                color: #9cdcfe;
                font-size: 13px;
                wrap: word-wrap;
            }
            Rectangle {
                background: #1e1e1e;
                border-radius: 4px;
                Text {
                    x: 10px;
                    y: 10px;
                    text: root.grid-text;
                    color: #d4d4d4;
                    font-family: "Menlo";
                    font-size: 14px;
                    horizontal-alignment: left;
                    vertical-alignment: top;
                }
            }
        }
    }
}

/// Estado compartilhado entre o thread leitor (escreve) e a UI (lê via poll).
#[derive(Default)]
struct Model {
    /// Captura rolante da `last_nonempty_line()` do lina-vt (cada item é uma linha
    /// distinta parseada pelo backend VT real).
    lines: Vec<String>,
    last_line: String,
    bytes: usize,
    advances: usize,
    max_damaged: usize,
    bracketed_paste: bool,
    eof: bool,
}

impl Model {
    fn telemetry(&self) -> String {
        format!(
            "PTY(sh) → lina-vt(AlacrittyBackend) → Slint  |  bytes={}  advances={}  max_damaged_rows={}  bracketed_paste={}  {}",
            self.bytes,
            self.advances,
            self.max_damaged,
            self.bracketed_paste,
            if self.eof { "[EOF]" } else { "[lendo]" }
        )
    }
    fn grid(&self) -> String {
        self.lines.join("\n")
    }
}

/// Script de terminal real: 14 linhas (com `sleep` para chegarem em reads separados,
/// exercitando a captura rolante) + uma linha-prompt final sem newline.
const SCRIPT: &str = "i=1; while [ $i -le 14 ]; do printf 'Lina linha %02d — grid parseado por lina-vt, render por Slint\\n' \"$i\"; i=$((i+1)); sleep 0.04; done; printf 'fim do spike (last_nonempty_line) > '";

/// Lê o master do PTY, alimenta o `VtBackend` e atualiza o `Model`. Roda fora da
/// thread de UI (jeito pty-host). Sem `unwrap()`: locks falíveis são ignorados.
fn read_loop(mut reader: Box<dyn Read + Send>, cols: u16, rows: u16, model: Arc<Mutex<Model>>) {
    let mut backend = AlacrittyBackend::new(cols, rows);
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                if let Ok(mut m) = model.lock() {
                    m.eof = true;
                }
                break;
            }
            Ok(n) => {
                backend.advance(&buf[..n]);
                let damaged = backend.damaged_rows().len();
                let line = backend.last_nonempty_line();
                let bracketed = backend.mode().bracketed_paste;
                backend.reset_damage();

                if let Ok(mut m) = model.lock() {
                    m.bytes += n;
                    m.advances += 1;
                    m.max_damaged = m.max_damaged.max(damaged);
                    m.bracketed_paste = bracketed;
                    if !line.is_empty() && line != m.last_line {
                        m.last_line.clone_from(&line);
                        m.lines.push(line);
                        let len = m.lines.len();
                        if len > 24 {
                            m.lines.drain(0..len - 24);
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn main() {
    let t0 = Instant::now();
    let (cols, rows) = (80u16, 24u16);

    // 1) PTY real (lina-pty) + parser real (lina-vt) num thread leitor.
    let mut mgr = PtyManager::new();
    let node = "spike-term";
    if let Err(e) = mgr.spawn(
        node,
        PtyCommand::new("sh").arg("-c").arg(SCRIPT),
        cols,
        rows,
    ) {
        eprintln!("spike-slint: falha ao abrir PTY: {e}");
        return;
    }
    let reader = match mgr.clone_reader(node) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("spike-slint: falha ao clonar reader: {e}");
            return;
        }
    };
    let model = Arc::new(Mutex::new(Model::default()));
    let model_reader = Arc::clone(&model);
    let reader_thread = thread::spawn(move || read_loop(reader, cols, rows, model_reader));

    // 2) Janela Slint. Se o ambiente for headless (sem display), o build é o gate;
    //    reportamos honestamente que a abertura de janela exige display.
    let ui = match Main::new() {
        Ok(ui) => ui,
        Err(e) => {
            eprintln!("spike-slint: Slint COMPILOU, mas a janela não pôde abrir aqui: {e}");
            eprintln!("(headless/sem display — esperado em CI; compilar é o gate desta etapa)");
            let _ = reader_thread.join();
            return;
        }
    };

    // metrics = (time_to_first_frame, n_ticks, soma_custo_update)
    let metrics = Arc::new(Mutex::new((Option::<Duration>::None, 0u64, Duration::ZERO)));
    let ui_weak = ui.as_weak();
    let model_poll = Arc::clone(&model);
    let metrics_poll = Arc::clone(&metrics);

    // Poll ~30 Hz: lê o Model e atualiza as propriedades da janela (UI thread).
    let poll = slint::Timer::default();
    poll.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let (grid, tele) = match model_poll.lock() {
                Ok(m) => (m.grid(), m.telemetry()),
                Err(_) => return,
            };
            let upd0 = Instant::now();
            ui.set_grid_text(grid.as_str().into());
            ui.set_telemetry(tele.as_str().into());
            let cost = upd0.elapsed();
            if let Ok(mut mm) = metrics_poll.lock() {
                if mm.0.is_none() {
                    mm.0 = Some(t0.elapsed());
                }
                mm.1 += 1;
                mm.2 += cost;
            }
        },
    );

    // 3) Auto-quit (spike): encerra o event loop sozinho para `cargo run` retornar.
    let quit = slint::Timer::default();
    quit.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(3500),
        || {
            let _ = slint::quit_event_loop();
        },
    );

    if let Err(e) = ui.run() {
        eprintln!("spike-slint: erro no event loop: {e}");
    }
    drop(poll);
    drop(quit);
    let _ = reader_thread.join();

    // 4) Relatório de métricas no stdout (alimenta o RESULTADO.md). Locks curtos:
    //    extraímos os valores Copy e soltamos os guards antes de imprimir.
    let (lines_n, bytes_n, max_dmg) = match model.lock() {
        Ok(m) => (m.lines.len(), m.bytes, m.max_damaged),
        Err(_) => (0, 0, 0),
    };
    let (ttf, ticks, total) = match metrics.lock() {
        Ok(mm) => *mm,
        Err(_) => (None, 0, Duration::ZERO),
    };
    let avg = if ticks > 0 {
        total / ticks as u32
    } else {
        Duration::ZERO
    };
    println!("--- METRICAS DO SPIKE ---");
    println!("time_to_first_frame: {ttf:?}");
    println!("ui_ticks (poll 30Hz): {ticks}");
    println!("avg_property_update: {avg:?}");
    println!("linhas capturadas (last_nonempty_line): {lines_n}");
    println!("bytes do PTY processados por lina-vt: {bytes_n}");
    println!("max damaged_rows observado: {max_dmg}");
}
