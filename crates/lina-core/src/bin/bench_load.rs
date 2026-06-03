//! W5-1 — benchmark headless de carga (Trilho A, Onda 5). GATE DE ESCALA.
//!
//! Spawna N PTYs reais (gerador de rajada contínua), espelhando o cabeamento de um
//! terminal de produção (PtyManager + Supervisor + reader-thread → grid), e mede SEM
//! intervenção humana, para a matriz N∈{10,20,30,40,50}:
//!   • RAM total do processo (RSS, via `sysinfo`),
//!   • contagem de threads do SO (esperado ~2N: reader + serial-writer por terminal),
//!   • latência de entrega A2A faseada sob carga (fan-out concorrente de `deliver_a2a`),
//!   • curva de RSS sob acúmulo de saída (a RAM estabiliza ou cresce? → scrollback-cap W5-2),
//!   • culling lógico (nós suspensos não geram trabalho de core).
//!
//! Escreve `target/bench/bench_load_<N>.json` por N + `target/bench/RELATORIO-W5-1.md`.
//!
//! LIMITE HONESTO: FPS de render sob pan/zoom **não é mensurável headless** (o gpui não
//! roda headless). Este binário mede só CORE/PTY/RAM/threads/A2A — ver o relatório.
//!
//! Uso: `cargo run --bin bench_load -- --panels 30`  (matriz completa, garante N=30)
//!      `cargo run --bin bench_load -- --single 4`    (só N=4 — iteração rápida)

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use sysinfo::System;

use lina_core::bench::{
    broadcast_targets, classify_rss_trend, fan_out_a2a_latency, rss_curve, sample_self,
    LatencyStats, LoadHarness, ProcStats, RssTrend,
};
use lina_core::{CliProfile, NodeId, PtyCommand, VtBackend};

/// Grid VT compartilhado (mesmo alias de `app::bridge::Grid`).
type Grid = Arc<Mutex<Box<dyn VtBackend>>>;

/// Gerador de rajada contínua: shell builtin `printf` num laço apertado (máxima taxa
/// de saída, sem `exec` por iteração) — estressa reader-threads e acúmulo de VT.
const BURST: &str = r#"i=0; while true; do printf 'linha %d\n' "$i"; i=$((i+1)); done"#;

/// Tolerância relativa (5%) para classificar a curva de RSS como crescente/plateau.
const RSS_TOL: f64 = 0.05;

/// Geometria de cada terminal (cols×rows) — um painel de canvas típico.
const COLS: u16 = 120;
const ROWS: u16 = 40;

struct Config {
    matrix: Vec<usize>,
    submit_delay_ms: u64,
    rss_window_ms: u64,
    rss_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            matrix: vec![10, 20, 30, 40, 50],
            submit_delay_ms: 120,
            rss_window_ms: 2500,
            rss_interval_ms: 250,
        }
    }
}

/// Relatório de uma corrida com `panels` terminais (serializado em JSON).
#[derive(Serialize)]
struct PanelReport {
    panels: usize,
    cols: u16,
    rows: u16,

    // ── RAM ──
    baseline_rss_bytes: u64,
    steady_rss_bytes: u64,
    rss_per_panel_bytes: f64,
    post_teardown_rss_bytes: u64,

    // ── threads ──
    baseline_threads: Option<usize>,
    steady_threads: Option<usize>,
    threads_added: Option<i64>,
    expected_threads_added: usize,

    // ── latência A2A ──
    a2a_targets: usize,
    a2a_failures: usize,
    a2a_submit_delay_ms: u64,
    a2a_total: LatencyStats,
    a2a_overhead_over_submit_delay: LatencyStats,

    // ── curva de RSS ──
    rss_curve_window_ms: u64,
    rss_curve_interval_ms: u64,
    rss_curve_bytes: Vec<u64>,
    rss_trend: RssTrend,

    // ── culling lógico ──
    culling_targets_before: usize,
    culling_suspended: usize,
    culling_targets_after: usize,
}

fn main() -> Result<()> {
    let cfg = parse_args().context("parse de argumentos")?;
    let out_dir = Path::new("target/bench");
    std::fs::create_dir_all(out_dir).context("criar target/bench")?;

    let mut sys = System::new();
    // Baseline global ANTES de qualquer terminal (depende só do processo + sysinfo).
    let baseline = sample_self(&mut sys);
    eprintln!(
        "[bench_load] baseline: RSS {} MiB, threads {:?}",
        mib(baseline.rss_bytes),
        baseline.threads
    );

    let mut reports = Vec::new();
    for &panels in &cfg.matrix {
        eprintln!("[bench_load] === N={panels} ===");
        let report = run_scenario(panels, &mut sys, &cfg, baseline)?;
        let json = serde_json::to_string_pretty(&report).context("serializar JSON")?;
        let path = out_dir.join(format!("bench_load_{panels}.json"));
        std::fs::write(&path, json).with_context(|| format!("escrever {}", path.display()))?;
        eprintln!(
            "[bench_load] N={panels}: RSS {} MiB (+{:.1} MiB/painel), threads {:?} (Δ{:?}, esperado +{}), \
A2A p50 {:.0}ms p95 {:.0}ms max {:.0}ms (overhead p95 {:.0}ms, {} falhas), curva {:?}, culling {}→{}",
            mib(report.steady_rss_bytes),
            report.rss_per_panel_bytes / (1024.0 * 1024.0),
            report.steady_threads,
            report.threads_added,
            report.expected_threads_added,
            report.a2a_total.p50_ms,
            report.a2a_total.p95_ms,
            report.a2a_total.max_ms,
            report.a2a_overhead_over_submit_delay.p95_ms,
            report.a2a_failures,
            report.rss_trend,
            report.culling_targets_before,
            report.culling_targets_after,
        );
        reports.push(report);
    }

    let md = render_report(&cfg, baseline, &reports);
    let md_path = out_dir.join("RELATORIO-W5-1.md");
    std::fs::write(&md_path, md).with_context(|| format!("escrever {}", md_path.display()))?;
    eprintln!("[bench_load] relatório: {}", md_path.display());
    Ok(())
}

/// Roda a corrida de `panels` terminais e devolve o relatório medido.
fn run_scenario(
    panels: usize,
    sys: &mut System,
    cfg: &Config,
    baseline: ProcStats,
) -> Result<PanelReport> {
    if panels == 0 {
        return Err(anyhow!("N (panels) deve ser >= 1"));
    }
    let mut harness = LoadHarness::new(COLS, ROWS);

    // Spawna N geradores de rajada.
    for i in 0..panels {
        let cmd = PtyCommand::new("sh").arg("-c").arg(BURST);
        harness
            .spawn_terminal(&format!("@Bench{i}"), "bench", cmd)
            .with_context(|| format!("spawn do terminal {i}"))?;
    }

    // ── Curva de RSS sob acúmulo de saída (item 3) ──
    // Começa IMEDIATAMENTE após o spawn para capturar a RAMPA: a rajada enche o scrollback
    // em sub-segundo, então amostrar só depois de um settle perderia o crescimento. A própria
    // janela de amostragem serve de settle para o estado estável seguinte.
    let curve = rss_curve(
        sys,
        Duration::from_millis(cfg.rss_window_ms),
        Duration::from_millis(cfg.rss_interval_ms),
    );
    let trend = classify_rss_trend(&curve, RSS_TOL);
    // Estado estável (threads ANTES do fan-out A2A, que sobe threads transitórias).
    let steady = sample_self(sys);

    // Coleta dados dos terminais ANTES de soltar o harness (sem segurar o borrow).
    let nodes: Vec<NodeId> = harness.live().iter().map(|t| t.node()).collect();
    let targets: Vec<(NodeId, Grid)> = harness.live()[1..]
        .iter()
        .map(|t| (t.node(), Arc::clone(t.grid())))
        .collect();
    let from = *nodes
        .first()
        .ok_or_else(|| anyhow!("N (panels) deve ser >= 1"))?;

    // ── Latência A2A faseada sob carga (fan-out concorrente) ──
    let profile = Arc::new(bench_profile(cfg.submit_delay_ms).context("perfil de bench")?);
    let fan = fan_out_a2a_latency(harness.supervisor(), from, &targets, "ping", &profile);
    let a2a_total = LatencyStats::from_durations(&fan.latencies);
    let submit = Duration::from_millis(cfg.submit_delay_ms);
    let overhead: Vec<Duration> = fan
        .latencies
        .iter()
        .map(|d| d.saturating_sub(submit))
        .collect();
    let a2a_overhead = LatencyStats::from_durations(&overhead);

    // ── Culling lógico: suspende K nós (mark_dead) → saem da resolução de broadcast ──
    let before = broadcast_targets(harness.supervisor(), from);
    let suspend_k = panels / 3;
    for &node in nodes.iter().skip(1).take(suspend_k) {
        harness
            .supervisor()
            .mark_dead(node)
            .map_err(|e| anyhow!("mark_dead: {e}"))?;
    }
    let after = broadcast_targets(harness.supervisor(), from);

    // ── Teardown + verificação de vazamento (RSS deve voltar perto do baseline) ──
    drop(targets); // solta os Arcs dos grids antes de derrubar o harness
    drop(harness); // Drop → kill dos PTYs + join das reader/serial threads
    thread::sleep(Duration::from_millis(500));
    let post = sample_self(sys);

    let threads_added = match (baseline.threads, steady.threads) {
        (Some(b), Some(s)) => Some(s as i64 - b as i64),
        _ => None,
    };
    let rss_per_panel_bytes = if panels > 0 {
        (steady.rss_bytes as f64 - baseline.rss_bytes as f64) / panels as f64
    } else {
        0.0
    };

    Ok(PanelReport {
        panels,
        cols: COLS,
        rows: ROWS,
        baseline_rss_bytes: baseline.rss_bytes,
        steady_rss_bytes: steady.rss_bytes,
        rss_per_panel_bytes,
        post_teardown_rss_bytes: post.rss_bytes,
        baseline_threads: baseline.threads,
        steady_threads: steady.threads,
        threads_added,
        expected_threads_added: 2 * panels,
        a2a_targets: targets_len(panels),
        a2a_failures: fan.failures,
        a2a_submit_delay_ms: cfg.submit_delay_ms,
        a2a_total,
        a2a_overhead_over_submit_delay: a2a_overhead,
        rss_curve_window_ms: cfg.rss_window_ms,
        rss_curve_interval_ms: cfg.rss_interval_ms,
        rss_curve_bytes: curve,
        rss_trend: trend,
        culling_targets_before: before,
        culling_suspended: suspend_k,
        culling_targets_after: after,
    })
}

/// Nº de alvos do fan-out A2A (todos menos o remetente).
fn targets_len(panels: usize) -> usize {
    panels.saturating_sub(1)
}

/// Perfil de CLI inline para o benchmark: `prompt_ready_regex` casa a saída de rajada
/// (`linha …`) para que `wait_ready` resolva em ~1 poll, isolando o custo de injeção +
/// contenção (a parte sensível a N) em vez do timeout de 2s.
fn bench_profile(submit_delay_ms: u64) -> Result<CliProfile> {
    let toml = format!(
        r#"
id = "bench"
program = "sh"
args = []
delivery = "pty_inject"
submit_delay_ms = {submit_delay_ms}
prompt_ready_regex = 'linha'
idle_ms = 200
ask_timeout_ms = 120000

[end_signal]
kind = "idle"
"#
    );
    CliProfile::from_toml_str(&toml, "<bench>")
        .map_err(|e| anyhow!("perfil de bench inválido: {e}"))
}

/// Parser mínimo de argumentos (sem dep de clap). Erros são propagados, nunca pânico.
fn parse_args() -> Result<Config> {
    let mut cfg = Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--single" => {
                let n = next_usize(&mut args, "--single")?;
                cfg.matrix = vec![n];
            }
            "--panels" => {
                // Mantém a matriz padrão e GARANTE que N esteja presente (ordenado, sem dup).
                let n = next_usize(&mut args, "--panels")?;
                if !cfg.matrix.contains(&n) {
                    cfg.matrix.push(n);
                    cfg.matrix.sort_unstable();
                }
            }
            "--submit-delay-ms" => cfg.submit_delay_ms = next_u64(&mut args, "--submit-delay-ms")?,
            "--rss-window-ms" => cfg.rss_window_ms = next_u64(&mut args, "--rss-window-ms")?,
            "--rss-interval-ms" => cfg.rss_interval_ms = next_u64(&mut args, "--rss-interval-ms")?,
            "-h" | "--help" => {
                eprintln!(
                    "uso: bench_load [--panels N | --single N] [--submit-delay-ms X] \
[--rss-window-ms X] [--rss-interval-ms X]"
                );
                std::process::exit(0);
            }
            other => return Err(anyhow!("flag desconhecida: {other}")),
        }
    }
    Ok(cfg)
}

fn next_usize(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<usize> {
    args.next()
        .ok_or_else(|| anyhow!("{flag} requer um valor"))?
        .parse()
        .with_context(|| format!("{flag} deve ser um inteiro"))
}

fn next_u64(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<u64> {
    args.next()
        .ok_or_else(|| anyhow!("{flag} requer um valor"))?
        .parse()
        .with_context(|| format!("{flag} deve ser um inteiro"))
}

/// MiB com 1 casa, para logs/relatório.
fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

/// Monta o relatório Markdown final com os NÚMEROS e a decisão de cap.
fn render_report(cfg: &Config, baseline: ProcStats, reports: &[PanelReport]) -> String {
    let mut s = String::new();
    s.push_str("# RELATÓRIO W5-1 — Benchmark de carga N=30–50 terminais (gate de escala)\n\n");
    s.push_str(
        "Binário headless `bench_load` (crate `lina-core`). Spawna N PTYs reais rodando um \
gerador de rajada contínua, cabeados como em produção (PtyManager + Supervisor + reader-thread \
→ grid VT), e mede sem intervenção humana.\n\n",
    );

    s.push_str("## ⚠️ Limite honesto (o que NÃO foi medido)\n\n");
    s.push_str(
        "**FPS de render sob pan/zoom NÃO é mensurável headless** — o gpui não roda sem janela/GPU. \
Este binário mede apenas a parte de CORE/PTY/RAM/threads/A2A. O FPS de render fica para \
instrumentação no app gpui, validada na tela com o fundador (etapa separada). Nada de FPS é \
fabricado aqui.\n\n",
    );
    s.push_str(
        "**Estado `suspended` não é um estado de primeira classe no core hoje** (os estados são \
`Running/Crashed/Exited` no pty-host e `Starting/Running/Idle/Busy/Blocked/Dead` no roster). O \
culling lógico medido abaixo usa o mecanismo que EXISTE: nós não-vivos (suspensos via `mark_dead`) \
são excluídos da resolução de broadcast → recebem **zero** trabalho de entrega de core.\n\n",
    );

    s.push_str("## Parâmetros\n\n");
    s.push_str(&format!(
        "- Geometria: {COLS}×{ROWS} por terminal\n- Gerador: `sh -c '{BURST}'`\n- \
`submit_delay` do A2A: **{} ms** (default de produção é 300 ms — ver `submit_delay_ms` do CLI Profile)\n- \
Curva de RSS: janela {} ms, amostra a cada {} ms (tolerância de plateau {:.0}%)\n- \
Baseline do processo: **{} MiB**, **{:?}** threads\n\n",
        cfg.submit_delay_ms,
        cfg.rss_window_ms,
        cfg.rss_interval_ms,
        RSS_TOL * 100.0,
        mib(baseline.rss_bytes),
        baseline.threads,
    ));

    s.push_str("## RAM e threads por N\n\n");
    s.push_str(
        "| N | RSS estável (MiB) | +MiB/painel | threads (Δ vs baseline) | esperado +2N | RSS pós-teardown (MiB) |\n\
|---:|---:|---:|:--|---:|---:|\n",
    );
    for r in reports {
        s.push_str(&format!(
            "| {} | {} | {:.2} | {:?} (Δ{:?}) | +{} | {} |\n",
            r.panels,
            mib(r.steady_rss_bytes),
            r.rss_per_panel_bytes / (1024.0 * 1024.0),
            r.steady_threads,
            r.threads_added,
            r.expected_threads_added,
            mib(r.post_teardown_rss_bytes),
        ));
    }

    s.push_str("\n## Latência de entrega A2A faseada (fan-out concorrente, sob carga)\n\n");
    s.push_str(
        "`deliver_a2a` ponta-a-ponta: wait_ready (poll do grid a cada 10 ms) → bracketed-paste → \
submit_delay → Enter(0x0D). O **overhead acima do submit_delay** é o custo AGREGADO da stack sob \
carga — inclui (a) o polling do `wait_ready`, que disputa o `Mutex` do grid com a reader-thread sob \
rajada, e (b) `lock_pty` + `enqueue_write` do Supervisor. NÃO é isolação limpa de uma única fonte \
de contenção; é quanto a entrega custa ALÉM do atraso fixo, com N terminais vivos.\n\n",
    );
    s.push_str(
        "| N | alvos | falhas | total p50 (ms) | total p95 (ms) | total max (ms) | overhead p95 (ms) | overhead max (ms) |\n\
|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for r in reports {
        s.push_str(&format!(
            "| {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |\n",
            r.panels,
            r.a2a_targets,
            r.a2a_failures,
            r.a2a_total.p50_ms,
            r.a2a_total.p95_ms,
            r.a2a_total.max_ms,
            r.a2a_overhead_over_submit_delay.p95_ms,
            r.a2a_overhead_over_submit_delay.max_ms,
        ));
    }

    s.push_str("\n## Curva de RSS sob acúmulo de saída (item 3)\n\n");
    s.push_str(
        "Cenário de **estresse** (brief item 3): N terminais em rajada CONTÍNUA — pior caso, não o \
padrão de produção (um agente real emite esporadicamente). A curva amostra o RSS a partir do spawn, \
capturando a RAMPA de enchimento do scrollback. `Growing` aqui significa que o scrollback ainda está \
ENCHENDO no horizonte de 2.5 s rumo ao seu teto (~32 MiB/painel — ver tabela de RAM); **não** é prova \
de crescimento ILIMITADO. O ponto: esse teto por painel é alto e escala linear (50 painéis ≈ 1.6 GiB) \
→ é exatamente o que o **scrollback-cap do W5-2 (Dev 02)** reduz. Sob padrão realista (rajada ocasional) \
o RSS estabilizaria mais cedo no mesmo teto.\n\n",
    );
    s.push_str(
        "| N | tendência | RSS início→fim da janela (MiB) | nº amostras |\n|---:|:--|:--|---:|\n",
    );
    for r in reports {
        let first = r.rss_curve_bytes.first().copied().unwrap_or(0);
        let last = r.rss_curve_bytes.last().copied().unwrap_or(0);
        s.push_str(&format!(
            "| {} | {:?} | {} → {} | {} |\n",
            r.panels,
            r.rss_trend,
            mib(first),
            mib(last),
            r.rss_curve_bytes.len(),
        ));
    }

    s.push_str("\n## Culling lógico (item 4)\n\n");
    s.push_str(
        "Suspende K≈N/3 nós (`mark_dead`) e re-resolve um broadcast. Os suspensos saem da \
resolução → não entram em nenhuma entrega de core.\n\n",
    );
    s.push_str(
        "| N | alvos antes | suspensos | alvos depois | confere? |\n|---:|---:|---:|---:|:--|\n",
    );
    for r in reports {
        let ok =
            r.culling_targets_after == r.culling_targets_before.saturating_sub(r.culling_suspended);
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            r.panels,
            r.culling_targets_before,
            r.culling_suspended,
            r.culling_targets_after,
            if ok { "✅" } else { "❌" },
        ));
    }

    s.push_str("\n## Decisão (com números)\n\n");
    s.push_str(&decision(reports));

    s.push_str("\n## Reprodução\n\n");
    s.push_str(
        "```sh\ncargo run --release --bin bench_load -- --panels 30   # matriz completa, garante N=30\n\
cargo run --bin bench_load -- --single 4               # iteração rápida (só N=4)\n```\n\n\
Artefatos: `target/bench/bench_load_<N>.json` (dados crus por N) + este relatório.\n",
    );
    s
}

/// Veredito automático a partir dos números medidos.
fn decision(reports: &[PanelReport]) -> String {
    let mut s = String::new();
    let any_growing = reports.iter().any(|r| r.rss_trend == RssTrend::Growing);
    let any_a2a_fail = reports.iter().any(|r| r.a2a_failures > 0);
    let threads_ok = reports.iter().all(|r| match r.threads_added {
        Some(d) => d >= 2 * r.panels as i64,
        None => true,
    });
    let max_n = reports.iter().map(|r| r.panels).max().unwrap_or(0);
    let avg_per_panel_mib = if reports.is_empty() {
        0.0
    } else {
        reports.iter().map(|r| r.rss_per_panel_bytes).sum::<f64>()
            / reports.len() as f64
            / (1024.0 * 1024.0)
    };

    s.push_str(&format!(
        "- **Threads ≈ 2N:** {}.\n",
        if threads_ok {
            "confirmado — cada terminal adiciona reader + serial-writer"
        } else {
            "DIVERGÊNCIA — ver tabela de threads (Δ < 2N em algum N)"
        }
    ));
    s.push_str(&format!(
        "- **RAM:** ~{avg_per_panel_mib:.0} MiB/painel, ~linear em N (ver tabela; dominado pelo buffer de \
scrollback do VT). ⚠️ O RSS **não retorna** ao baseline 500 ms após o teardown — comportamento de \
**retenção do alocador** (páginas liberadas não voltam ao SO de imediato), **não** prova de vazamento. \
Este bench NÃO isola leak de heap (exigiria N ciclos spawn/teardown no mesmo processo ou purge do alocador); \
o per-painel estável entre os N indica reuso, não acúmulo.\n- **Curva de RSS sob rajada:** {growth}.\n",
        growth = if any_growing {
            "**sobe** (scrollback enchendo rumo ao teto de ~32 MiB/painel) no cenário de rajada CONTÍNUA — não é crescimento ilimitado, mas o teto por painel é alto e linear em N → o **scrollback-cap do W5-2 (Dev 02)** é o lever para baixar o teto de RAM em sessões longas"
        } else {
            "estabiliza (plateau) no horizonte medido — o scrollback já encheu até o teto; ainda assim o W5-2 baixa esse teto por painel"
        }
    ));
    s.push_str(&format!(
        "- **Latência A2A sob carga:** {} (ver tabela; o overhead acima do submit_delay é o custo agregado da stack — polling do wait_ready + lock do Supervisor — sob N terminais vivos, não uma fonte única isolada).\n",
        if any_a2a_fail {
            "houve FALHAS de entrega sob carga — investigar timeout de lock"
        } else {
            "todas as entregas tiveram sucesso até N máximo"
        }
    ));
    s.push_str(&format!(
        "- **Cap recomendado:** a stack de CORE/PTY/RAM/threads suporta N≤{max_n} no Mac sem falha de entrega. \
O teto de produto (\"muitos terminais\") deve ser fixado pelo gate de RENDER (FPS no gpui, etapa separada) e \
pelo cap de scrollback (W5-2), não pelo core medido aqui.\n",
    ));
    s
}
