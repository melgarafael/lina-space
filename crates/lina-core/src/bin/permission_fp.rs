//! F1-1-6 · Harness do **protocolo de medição de FP** do fallback de grid (critério 5).
//!
//! ## Protocolo (definido nesta story)
//! Workspace de teste dedicado com **5 terminais reais** (PTY + grid VT via
//! `bench::LoadHarness`, o mesmo tijolo do W5-1) sob carga benigna ROTEIRIZADA — o
//! rótulo é conhecido de antemão (sabe-se quais terminais NUNCA pedem de verdade):
//!
//! | terminal      | carga                                                        | rótulo      |
//! |---------------|--------------------------------------------------------------|-------------|
//! | `build-ansi`  | saída de build com cores ANSI, contínua                      | nunca pede  |
//! | `verbose-log` | rajadas de log denso (`yes`-like)                            | nunca pede  |
//! | `trap-busy`   | IMPRIME `(y/n)` como texto e SEGUE rodando (#28174, viva)    | nunca pede  |
//! | `trap-idle`   | IMPRIME linha terminando em `(y/n)` e DORME (#28174, pior caso) | nunca pede |
//! | `real-asker`  | `read -p "Continue? (y/n) "` periódico (CLI sem hook)        | pede de verdade |
//!
//! Classificação: emissão do `real-asker` = **TP** (ele é o único que pede);
//! emissão de qualquer outro = **FP** (alimenta `record_false_positive`).
//! Métricas: **FP por hora-terminal** e **precisão** (TP ÷ emitidos).
//!
//! Rodada COMPLETA do protocolo: ≥10 rodadas × 15 min (agendada com o Maestro).
//! Esta execução default é a **rodada reduzida de calibração** (3 × 3 min).
//!
//! Uso: `permission_fp [rodadas] [segundos_por_rodada]` (defaults: 3, 180).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lina_core::bench::LoadHarness;
use lina_core::permission_detect::PermissionDetector;
use lina_core::{EventStore, PtyCommand, VtBackend};

/// Cadência de amostragem do grid (mesma ordem de grandeza do uso real no app).
const SAMPLE_MS: u64 = 250;
/// Sem mudança no grid por este tempo ⇒ `idle` (prompt parado esperando input).
const IDLE_MS: u64 = 1_500;

const COLS: u16 = 100;
const ROWS: u16 = 30;

/// Os 5 terminais do protocolo: (nome, script bash, pede-de-verdade?).
fn workload() -> Vec<(&'static str, String, bool)> {
    vec![
        (
            "build-ansi",
            // Build sintético com cores ANSI — fluxo contínuo, nunca interativo.
            r#"i=0; while true; do i=$((i+1)); for j in 1 2 3 4 5 6 7 8; do printf '\033[32m   Compiling\033[0m crate-%d-%d v0.%d.0 \033[33m(warnings: %d)\033[0m\n' "$i" "$j" "$((j%9))" "$((i%3))"; done; printf '\033[1;32m    Finished\033[0m dev profile round %d\n' "$i"; sleep 0.6; done"#
                .to_string(),
            false,
        ),
        (
            "verbose-log",
            // Log denso em rajada (família `yes`/`cat` da story).
            r#"while true; do yes 'verbose [INFO] pipeline tick ok latency=3ms queue=0' | head -n 250; sleep 0.8; done"#
                .to_string(),
            false,
        ),
        (
            "trap-busy",
            // Armadilha #28174 VIVA: imprime (y/n) como texto e segue produzindo
            // output — a condição de idle deve suprimir (contado na telemetria).
            r#"while true; do echo 'docs: interactive tools may print (y/n)'; echo 'still processing batch...'; sleep 0.4; done"#
                .to_string(),
            false,
        ),
        (
            "trap-idle",
            // Armadilha #28174 PIOR CASO: a ÚLTIMA linha termina no padrão e o
            // terminal fica ocioso — FP estrutural do mecanismo (texto varia por
            // ciclo para cada aparição contar como candidato novo).
            r#"i=0; while true; do i=$((i+1)); echo "batch $i summary follows"; echo "doc sample $i ends with marker Continue? (y/n)"; sleep 25; done"#
                .to_string(),
            false,
        ),
        (
            "real-asker",
            // CLI sem hook pedindo DE VERDADE (read y/n) — auto-timeout para o ciclo
            // seguir sem humano (ground truth: toda emissão daqui é TP).
            r#"i=0; while true; do i=$((i+1)); echo "task $i starting"; read -t 18 -p "Continue? (y/n) " ans; echo; echo "task $i resumed answer=${ans:-timeout}"; sleep 8; done"#
                .to_string(),
            true,
        ),
    ]
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Lock tolerante a poison (harness segue medindo mesmo se uma reader-thread panicar).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Fingerprint barato do viewport (linhas + cursor) — detector de "grid mudou?" para
/// derivar `idle` (sem mudança por `IDLE_MS`).
fn grid_fingerprint(vt: &dyn VtBackend) -> u64 {
    let (_, rows) = vt.dims();
    let mut h = DefaultHasher::new();
    for r in 0..rows {
        vt.row_text(r).hash(&mut h);
    }
    let screen = vt.screen();
    (screen.cursor.col, screen.cursor.line).hash(&mut h);
    // Output em scroll com viewport byte-idêntico (padrão alternado com tela cheia)
    // congelaria o hash de linhas — o ring de scrollback cresce a cada linha que sai
    // do viewport e desempata (achado da rodada de calibração; em produção o idle vem
    // do output do PTY/cycle_count, ADR 0019 — este fingerprint é o proxy do harness).
    vt.scrollback_len().hash(&mut h);
    h.finish()
}

#[derive(Default, Clone)]
struct RoundStats {
    tp: u64,
    fp: u64,
    fp_by_terminal: Vec<(String, u64)>,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let rounds: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let round_secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(180);

    let store_dir = std::env::temp_dir().join(format!("lina-fp-f116-{}", uuid::Uuid::now_v7()));
    let mut store = EventStore::open(&store_dir)?;
    println!("== F1-1-6 · protocolo de FP (fallback de grid) ==");
    println!(
        "rodadas={rounds} × {round_secs}s · terminais=5 · sample={SAMPLE_MS}ms · idle={IDLE_MS}ms"
    );
    println!("event store: {}\n", store_dir.display());

    let mut all = RoundStats::default();
    let mut emitted_total = 0_u64;

    for round in 1..=rounds {
        let mut harness = LoadHarness::new(COLS, ROWS);
        let specs = workload();
        let mut names = Vec::new();
        for (name, script, _) in &specs {
            let cmd = PtyCommand::new("/bin/bash").arg("-c").arg(script.clone());
            harness.spawn_terminal(name, "fp-load", cmd)?;
            names.push((*name).to_string());
        }

        let mut det = PermissionDetector::new();
        let mut last_fp: Vec<(u64, Instant)> = vec![(0, Instant::now()); harness.live().len()]; // (fingerprint, último-momento-de-mudança)
        let mut stats = RoundStats {
            fp_by_terminal: names.iter().map(|n| (n.clone(), 0)).collect(),
            ..RoundStats::default()
        };

        let t_end = Instant::now() + Duration::from_secs(round_secs);
        while Instant::now() < t_end {
            for (idx, term) in harness.live().iter().enumerate() {
                let guard = lock(term.grid());
                let vt: &dyn VtBackend = &**guard;
                let fp_now = grid_fingerprint(vt);
                if fp_now != last_fp[idx].0 {
                    last_fp[idx] = (fp_now, Instant::now());
                }
                let idle = last_fp[idx].1.elapsed() >= Duration::from_millis(IDLE_MS);
                if let Some(ask) = det.observe_grid(&names[idx], vt, idle, now_ms()) {
                    drop(guard);
                    store.append(&ask.to_event())?;
                    emitted_total += 1;
                    let is_real = specs[idx].2;
                    if is_real {
                        stats.tp += 1;
                    } else {
                        det.record_false_positive();
                        stats.fp += 1;
                        stats.fp_by_terminal[idx].1 += 1;
                    }
                    println!(
                        "[r{round}] {} {} — detail={:?} stable_id={}",
                        if is_real { "TP" } else { "FP" },
                        names[idx],
                        ask.detail.as_deref().unwrap_or(""),
                        ask.stable_id
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(SAMPLE_MS));
        }
        harness.shutdown();

        let tel = det.telemetry();
        println!(
            "\n— rodada {round}: emitidos={} TP={} FP={} suprimidos_busy={} precisão={}",
            tel.emitted_total(),
            stats.tp,
            stats.fp,
            tel.grid_suppressed_busy,
            tel.precision()
                .map_or("n/a".into(), |p| format!("{:.3}", p)),
        );
        for (name, n) in &stats.fp_by_terminal {
            if *n > 0 {
                println!("  FP em {name}: {n}");
            }
        }
        println!();
        all.tp += stats.tp;
        all.fp += stats.fp;
        if all.fp_by_terminal.is_empty() {
            all.fp_by_terminal = stats.fp_by_terminal.clone();
        } else {
            for (agg, cur) in all.fp_by_terminal.iter_mut().zip(&stats.fp_by_terminal) {
                agg.1 += cur.1;
            }
        }
    }

    // ── agregado + evidência de replay ──
    let terminal_hours = (f64::from(rounds) * round_secs as f64 * 5.0) / 3600.0;
    let fp_per_terminal_hour = if terminal_hours > 0.0 {
        all.fp as f64 / terminal_hours
    } else {
        0.0
    };
    let precision = if emitted_total > 0 {
        all.tp as f64 / emitted_total as f64
    } else {
        f64::NAN
    };
    println!("== AGREGADO ==");
    println!("hora-terminal: {terminal_hours:.3}");
    println!(
        "emitidos: {emitted_total} · TP: {} · FP: {}",
        all.tp, all.fp
    );
    println!("FP/hora-terminal: {fp_per_terminal_hour:.2}");
    println!("precisão: {precision:.3}");
    for (name, n) in &all.fp_by_terminal {
        println!("  FP {name}: {n}");
    }

    // Replay não duplica: 2 projeções do mesmo log = mesmo nº de PermissionAsked.
    let count = |s: &EventStore| -> anyhow::Result<usize> {
        Ok(s.events()?
            .into_iter()
            .filter(|r| r.kind == "PermissionAsked")
            .count())
    };
    let c1 = count(&store)?;
    store.project()?;
    store.project()?;
    let c2 = count(&store)?;
    println!("replay: PermissionAsked no log {c1} → após 2 projeções {c2} (sem duplicar)");
    // Sanidade: todos os stable_id são únicos no log.
    let ids: Vec<String> = store
        .events()?
        .into_iter()
        .filter(|r| r.kind == "PermissionAsked")
        .map(|r| r.payload["stable_id"].as_str().unwrap_or("").to_string())
        .collect();
    let mut dedup = ids.clone();
    dedup.sort();
    dedup.dedup();
    println!(
        "stable_id únicos: {}/{} {}",
        dedup.len(),
        ids.len(),
        if dedup.len() == ids.len() {
            "OK"
        } else {
            "DUPLICADO!"
        }
    );
    Ok(())
}
