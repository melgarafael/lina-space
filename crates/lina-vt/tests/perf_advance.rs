//! F1-6-3 (critério c) — **custo por-advance do caminho de CAPTURA, medido**.
//!
//! Micro-bench manual (`#[ignore]`: timing não vira flake de CI). Como rodar:
//! `cargo test -p lina-vt --release --test perf_advance -- --ignored --nocapture --test-threads=1`
//! com a máquina QUIETA (load < nº de cores — lição da rodada 1: load ~12 distorce timing).
//!
//! Protocolo da story: rodar ANTES da mudança (baseline no código intocado) e DEPOIS
//! (com a detecção `after==ceiling`); registrar os dois números na entrega. A carga é o
//! caso REAL de scrollback (linhas via LF), o caminho quente que não pode regredir.

use lina_vt::{AlacrittyBackend, VtBackend};
use std::time::Instant;

#[test]
#[ignore = "micro-bench manual de timing (critério c da F1-6-3) — rode com --release e máquina quieta"]
fn perf_advance_capture_lf_torrential() {
    let cap = 10_000usize;
    // Linha de 64 bytes (63 'x' + \n) — output denso típico; 64 KiB por advance; 4 MiB no total.
    let line = format!("{}\n", "x".repeat(63));
    let chunk = line.repeat(1024);
    const ADVANCES: usize = 64;

    let mut b = AlacrittyBackend::with_scrollback_capture(80, 24, cap);
    // Warm-up: enche o ring até o regime estável (cap atingido → todo advance subsequente
    // exercita colheita + trim, o pior caso constante).
    b.advance(chunk.as_bytes());
    let _ = b.take_scrollback();

    let t0 = Instant::now();
    let mut harvested = 0usize;
    for _ in 0..ADVANCES {
        b.advance(chunk.as_bytes());
        harvested += b.take_scrollback().len(); // drena como o pty-host drena
    }
    let dt = t0.elapsed();

    let bytes = chunk.len() * ADVANCES;
    println!(
        "[PERF] capture LF: {} MiB / {} linhas colhidas em {:?} → {:.2} ns/byte · {:.2} ms/MiB",
        bytes / (1024 * 1024),
        harvested,
        dt,
        dt.as_nanos() as f64 / bytes as f64,
        dt.as_secs_f64() * 1000.0 / (bytes as f64 / (1024.0 * 1024.0)),
    );

    // Sanity não-temporal: o regime estável colhe ~1 linha por linha enviada e o ring fica no cap.
    assert!(
        harvested >= ADVANCES * 1024 - 24,
        "colheita furada: {harvested}"
    );
    assert_eq!(b.scrollback_len(), cap, "ring fora do regime estável");
}
