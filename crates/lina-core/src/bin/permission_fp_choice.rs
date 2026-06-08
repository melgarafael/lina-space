//! R2b · Harness de calibração das **traps NOVAS do padrão CHOICE** (âncora de chrome).
//!
//! CÓPIA adaptada de `permission_fp.rs` (o harness y/n do Analista roda com binário
//! congelado — este arquivo NÃO o toca nem compete com ele; workload e store próprios).
//!
//! ## Workload (rótulo conhecido de antemão)
//!
//! | terminal           | carga                                                          | rótulo |
//! |--------------------|----------------------------------------------------------------|--------|
//! | `build-ansi`       | build sintético colorido contínuo (regressão)                  | nunca pede |
//! | `md-list-idle`     | TRAP NOVA: imprime lista numerada markdown e DORME             | nunca pede |
//! | `chrome-cite-idle` | TRAP NOVA (pior caso): linha de doc CITANDO o chrome, e dorme  | nunca pede |
//! | `choice-asker`     | caixa de escolha real (pergunta+opções+rodapé) + `read`        | pede (choice) |
//! | `yn-asker`         | `read -p "Continue? (y/n) "` (regressão do padrão antigo)      | pede (yn) |
//!
//! `md-list-idle` prova a PROIBIÇÃO da story (lista numerada nunca é sinal — zero
//! emissão esperada). `chrome-cite-idle` é o FP estrutural do mecanismo: medido e
//! rotulado, nunca jurado zero (mesma doutrina da #28174 no y/n).
//!
//! Uso: `permission_fp_choice [rodadas] [segundos_por_rodada]` (defaults: 2, 60 —
//! a rodada REDUZIDA de calibração da R2b).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lina_core::bench::LoadHarness;
use lina_core::permission_detect::PermissionDetector;
use lina_core::{EventStore, PromptKind, PtyCommand, VtBackend};

/// Cadência de amostragem do grid (mesma do protocolo y/n).
const SAMPLE_MS: u64 = 250;
/// Sem mudança no grid por este tempo ⇒ `idle`.
const IDLE_MS: u64 = 1_500;

const COLS: u16 = 100;
const ROWS: u16 = 30;

/// Os 5 terminais da calibração: (nome, script bash, pede-de-verdade?).
fn workload() -> Vec<(&'static str, String, bool)> {
    vec![
        (
            "build-ansi",
            r#"i=0; while true; do i=$((i+1)); for j in 1 2 3 4; do printf '\033[32m   Compiling\033[0m crate-%d-%d v0.%d.0\n' "$i" "$j" "$((j%9))"; done; sleep 0.6; done"#
                .to_string(),
            false,
        ),
        (
            "md-list-idle",
            // TRAP NOVA (proibição da story): lista numerada markdown como última
            // coisa na tela, terminal ocioso. Texto varia por ciclo (cada aparição
            // seria candidato novo SE a âncora errada existisse).
            r#"i=0; while true; do i=$((i+1)); echo "Plano da iteracao $i:"; echo "  1. Revisar o modulo $i"; echo "  2. Rodar os testes"; echo "  3. Abrir o PR"; sleep 20; done"#
                .to_string(),
            false,
        ),
        (
            "chrome-cite-idle",
            // TRAP NOVA (pior caso estrutural): linha de DOC citando o chrome
            // COMPLETO (3 sinais, com separador) — indistinguível por texto do rodapé
            // real; mede o resíduo, não jura zero. (A citação de 2 sinais, que furava
            // antes do endurecimento da âncora, virou teste unitário negativo.)
            "i=0; while true; do i=$((i+1)); echo \"doc $i: the footer reads Enter to select \u{b7} arrows to navigate \u{b7} Esc to cancel\"; sleep 22; done"
                .to_string(),
            false,
        ),
        (
            "choice-asker",
            // Caixa de escolha REAL (chrome da tela do fundador) + read com timeout
            // (auto-segue sem humano; toda emissão daqui é TP de choice).
            "i=0; while true; do i=$((i+1)); printf 'Qual op\u{e7}\u{e3}o voc\u{ea} quer para a etapa %d?\\n  1. Op\u{e7}\u{e3}o A\\n  2. Op\u{e7}\u{e3}o B\\n  3. Chat about this\\nEnter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel ' \"$i\"; read -t 16 ans; echo; echo \"etapa $i seguiu (ans=${ans:-timeout})\"; sleep 6; done"
                .to_string(),
            true,
        ),
        (
            "yn-asker",
            // Regressão: o padrão y/n continua emitindo com o detector ampliado.
            r#"i=0; while true; do i=$((i+1)); echo "task $i starting"; read -t 16 -p "Continue? (y/n) " ans; echo; echo "task $i resumed answer=${ans:-timeout}"; sleep 6; done"#
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

/// Lock tolerante a poison (o harness segue medindo).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Fingerprint barato do viewport — proxy de `idle` do harness (mesmo do protocolo y/n).
fn grid_fingerprint(vt: &dyn VtBackend) -> u64 {
    let (_, rows) = vt.dims();
    let mut h = DefaultHasher::new();
    for r in 0..rows {
        vt.row_text(r).hash(&mut h);
    }
    let screen = vt.screen();
    (screen.cursor.col, screen.cursor.line).hash(&mut h);
    vt.scrollback_len().hash(&mut h);
    h.finish()
}

fn kind_str(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::Yn => "yn",
        PromptKind::Choice => "choice",
        PromptKind::Trust => "trust",
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let rounds: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);
    let round_secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);

    let store_dir = std::env::temp_dir().join(format!("lina-fp-r2b-{}", uuid::Uuid::now_v7()));
    let mut store = EventStore::open(&store_dir)?;
    println!("== R2b · calibração das traps de CHOICE (âncora de chrome) ==");
    println!(
        "rodadas={rounds} × {round_secs}s · terminais=5 · sample={SAMPLE_MS}ms · idle={IDLE_MS}ms"
    );
    println!("event store: {}\n", store_dir.display());

    let mut tp_choice = 0_u64;
    let mut tp_yn = 0_u64;
    let mut fp = 0_u64;
    let mut misclassified = 0_u64;
    let mut emitted_total = 0_u64;
    let mut fp_by_terminal: Vec<(String, u64)> = Vec::new();

    for round in 1..=rounds {
        let mut harness = LoadHarness::new(COLS, ROWS);
        let specs = workload();
        let mut names = Vec::new();
        for (name, script, _) in &specs {
            let cmd = PtyCommand::new("/bin/bash").arg("-c").arg(script.clone());
            harness.spawn_terminal(name, "fp-choice", cmd)?;
            names.push((*name).to_string());
        }
        if fp_by_terminal.is_empty() {
            fp_by_terminal = names.iter().map(|n| (n.clone(), 0)).collect();
        }

        let mut det = PermissionDetector::new();
        let mut last_fp: Vec<(u64, Instant)> = vec![(0, Instant::now()); harness.live().len()];

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
                    let name = names[idx].as_str();
                    let is_real = specs[idx].2;
                    let label = if !is_real {
                        det.record_false_positive();
                        fp += 1;
                        fp_by_terminal[idx].1 += 1;
                        "FP"
                    } else if name == "choice-asker" && ask.kind == PromptKind::Choice {
                        tp_choice += 1;
                        "TP-choice"
                    } else if name == "yn-asker" && ask.kind == PromptKind::Yn {
                        tp_yn += 1;
                        "TP-yn"
                    } else {
                        // Pediu de verdade mas o KIND veio errado — conta à parte
                        // (erro de classificação, não de detecção).
                        misclassified += 1;
                        "MISCLASS"
                    };
                    println!(
                        "[r{round}] {label} {name} kind={} — detail={:?}",
                        kind_str(ask.kind),
                        ask.detail.as_deref().unwrap_or("")
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(SAMPLE_MS));
        }
        harness.shutdown();

        let tel = det.telemetry();
        println!(
            "\n— rodada {round}: emitidos_total={} (yn={} choice={}) suprimidos_busy={}\n",
            tel.emitted_total(),
            tel.emitted_grid,
            tel.emitted_choice,
            tel.grid_suppressed_busy,
        );
    }

    let terminal_hours = (f64::from(rounds) * round_secs as f64 * 5.0) / 3600.0;
    let fp_per_terminal_hour = if terminal_hours > 0.0 {
        fp as f64 / terminal_hours
    } else {
        0.0
    };
    let precision = if emitted_total > 0 {
        (tp_choice + tp_yn) as f64 / emitted_total as f64
    } else {
        f64::NAN
    };
    println!("== AGREGADO (calibração R2b) ==");
    println!("hora-terminal: {terminal_hours:.3}");
    println!(
        "emitidos: {emitted_total} · TP-choice: {tp_choice} · TP-yn: {tp_yn} · FP: {fp} · misclass: {misclassified}"
    );
    println!("FP/hora-terminal: {fp_per_terminal_hour:.2}");
    println!("precisão: {precision:.3}");
    for (name, n) in &fp_by_terminal {
        println!("  FP {name}: {n}");
    }

    // Sanidade de contrato: todo PermissionAsked do choice-asker persistiu prompt_kind=choice.
    let recs = store.events()?;
    let bad_kind = recs
        .iter()
        .filter(|r| r.kind == "PermissionAsked" && r.payload["node_id"] == "choice-asker")
        .filter(|r| r.payload["prompt_kind"] != "choice")
        .count();
    println!(
        "contrato: PermissionAsked do choice-asker com prompt_kind=choice {} (divergentes: {bad_kind})",
        if bad_kind == 0 { "OK" } else { "FALHOU" }
    );
    Ok(())
}
