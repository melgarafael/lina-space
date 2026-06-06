//! F1-0-1 — runner headless da BASELINE dos bugs P0/P1 (Coordenação Confiável).
//!
//! Reproduz, ANTES de qualquer fix, os 2 sintomas da observação de 14h
//! (`docs/DIRECIONAMENTO-coordenacao-a2a.md` §2 P0/P1 e §4) num workspace
//! ISOLADO, com N claudes REAIS rodando headless via o core (PtyManager +
//! Supervisor + reader-thread → grid — o mesmo cabeamento do `bench_load`/W5-1):
//!
//! - **texto-colado (P0):** deposita `lina ask` (via o MESMO outbox por-nó que a
//!   CLI usa) para um claude OCUPADO, ≥20 tentativas, e mede a TAXA X/N de
//!   pastes que ficam presos no input sem submeter (bug intermitente —
//!   DIRECIONAMENTO §4.4; nunca single-shot).
//! - **atropelamento (P1):** N "retornos" simultâneos ao mesmo alvo (fan-out de
//!   tarefa curta terminando junto — evidência original seq 1853→1860 com Δ0.6s)
//!   e mede os intervalos entre `MessageDelivered` consecutivos ao mesmo alvo
//!   (<2s = colisão).
//!
//! **Fidelidade ao caminho de produção (o que este harness REPLICA, não muda):**
//! o tick de 120 ms do `MailboxPump` (`app/lina-gpui/src/bridge.rs:443-528`), o
//! `Router::pump` → `deliver_a2a` com `WorkspaceTrust::from_members` (idem
//! `deliver_fn`, bridge.rs:406-440) e — crucialmente — o **`demo_profile()` de
//! produção** (bridge.rs:327: `prompt_ready_regex='.'`, `submit_delay_ms=150`),
//! que é a causa-raiz hipotetizada do P0. A réplica é testada em
//! `tests::demo_profile_replica_fields` para não derivar silenciosamente.
//!
//! **100% leitor do código de produção:** este binário NÃO altera supervisor/
//! router/events — só usa as APIs públicas, como o app faz. O log da verdade é o
//! `<ws>/.lina/events/log.jsonl` (mesmo espelho que `tools/lina_watch.py` lê).
//!
//! Uso:
//!   cargo run --release -p lina-core --bin baseline_f1_0 -- \
//!     --ws /tmp/lina-baseline-ws --claudes 6 --attempts 22 --rounds 6
//!
//! Critérios atendidos (story F1-0-1, tasks/epico-f1/ondas-0-1.md):
//! (1) eventos legíveis no log.jsonl vivo (acompanhe com tools/lina_watch.py);
//! (2) taxa X/N de texto-colado em ≥20 tentativas; (3) retornos ao mesmo alvo
//! com <2s citando seq+timestamps; (4) artefatos p/ tasks/epico-f1/baseline-f1-0.md.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use lina_core::bench::LoadHarness;
use lina_core::{
    deliver_a2a, AgentPresence, CliProfile, DomainEvent, EventStore, MailMessage, Mailbox, NodeId,
    PtyCommand, RouteOutcome, Router, RouterConfig, Supervisor, VtBackend, WorkspaceTrust, WriteOp,
};

/// Grid VT compartilhado (mesmo alias do `bench_load`/`app::bridge::Grid`).
type Grid = Arc<Mutex<Box<dyn VtBackend>>>;

/// Geometria de cada terminal — igual ao bench W5-1 (painel típico de canvas).
const COLS: u16 = 120;
const ROWS: u16 = 40;

/// Tick do pump — RÉPLICA do `MailboxPump::spawn` (bridge.rs:447: ~120 ms).
const PUMP_TICK: Duration = Duration::from_millis(120);

/// Limiar de colisão do DIRECIONAMENTO §4.5 ("retornos a A com < 2s de intervalo").
const COLLISION_THRESHOLD_MS: u64 = 2000;

/// Amostras da detecção de texto-preso: após `MessageDelivered`, o marcador ainda
/// está no INPUT do alvo em t+3s E t+8s? (duas amostras = estabilidade, não flash).
const STUCK_SAMPLE_1: Duration = Duration::from_secs(3);
const STUCK_SAMPLE_2: Duration = Duration::from_secs(5); // +3s → +8s

fn main() -> Result<()> {
    let cfg = parse_args()?;
    let ws = cfg.ws.clone();
    fs::create_dir_all(ws.join(".lina")).context("criar <ws>/.lina")?;
    let artifacts = ws.join("baseline-artifacts");
    fs::create_dir_all(&artifacts).context("criar baseline-artifacts")?;

    eprintln!("[baseline] workspace ISOLADO: {}", ws.display());
    eprintln!(
        "[baseline] log vivo: {}",
        ws.join(".lina/events/log.jsonl").display()
    );
    eprintln!(
        "[baseline] acompanhe: python3 tools/lina_watch.py --follow '{}'",
        ws.join(".lina/events/log.jsonl").display()
    );

    // ── Condição de ambiente (aviso do Maestro 2026-06-06): o claude-code.toml REAL,
    //    pós-fix F1-1-1 (idle_ms/ask_timeout_ms antes de [end_signal]). Imprime os
    //    valores PARSEADOS em runtime — prova de vigência para o baseline-f1-0.md.
    let detect = CliProfile::load_file(&cfg.claude_profile)
        .map_err(|e| anyhow!("carregar {} falhou: {e}", cfg.claude_profile.display()))?;
    eprintln!(
        "[baseline] claude-code.toml vigente: prompt_ready_regex={:?} submit_delay_ms={} idle_ms={:?} ask_timeout_ms={:?}",
        detect.prompt_ready_regex, detect.submit_delay_ms, detect.idle_ms, detect.ask_timeout_ms
    );
    let prompt_re = regex::Regex::new(&detect.prompt_ready_regex)
        .map_err(|e| anyhow!("prompt_ready_regex do claude-code.toml não compila: {e}"))?;

    // Perfil de ENTREGA = réplica do demo_profile() que o app usa em produção HOJE.
    let delivery_profile = demo_profile_replica()?;
    eprintln!(
        "[baseline] perfil de ENTREGA (produção hoje, bridge.rs:327): regex={:?} submit_delay_ms={}",
        delivery_profile.prompt_ready_regex, delivery_profile.submit_delay_ms
    );

    // ── Sobe N claudes reais headless (cabeamento de produção) ──
    let store = Arc::new(Mutex::new(
        EventStore::open(ws.join(".lina/events")).map_err(|e| anyhow!("abrir EventStore: {e}"))?,
    ));
    let mut harness = LoadHarness::new(COLS, ROWS);
    let names = agent_names(cfg.claudes);
    let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
    let mut nodes: BTreeMap<String, NodeId> = BTreeMap::new();

    for (i, name) in names.iter().enumerate() {
        let cwd = ws.join("agents").join(name);
        fs::create_dir_all(&cwd).with_context(|| format!("criar cwd de {name}"))?;
        let cmd = PtyCommand::new(&cfg.claude_bin)
            .cwd(&cwd)
            .env("TERM", "xterm-256color");
        let role = if i == 0 { "maestro" } else { "worker" };
        let node = harness
            .spawn_terminal(name, role, cmd)
            .with_context(|| format!("spawn do claude {name}"))?;
        // Espelha o que o app emite ao criar um nó-terminal — alimenta o mapa
        // NodeId→nome do lina_watch (NodeRenamed) e a auditoria (TerminalSpawned).
        {
            let mut st = lock(&store);
            append(
                &mut st,
                &DomainEvent::NodeAdded {
                    node,
                    kind: "terminal".into(),
                    x: f64::from(u32::try_from(i).unwrap_or(0)) * 100.0,
                    y: 0.0,
                },
            )?;
            append(
                &mut st,
                &DomainEvent::NodeRenamed {
                    node,
                    name: name.clone(),
                },
            )?;
            append(
                &mut st,
                &DomainEvent::TerminalSpawned {
                    node,
                    cli: "claude-code".into(),
                },
            )?;
        }
        let grid = Arc::clone(harness.live().last().expect("recém-spawnado").grid());
        grids.insert(node, grid);
        nodes.insert(name.clone(), node);
        eprintln!("[baseline] spawned {name} ({node})");
    }
    let sup = Arc::clone(harness.supervisor());

    // ── Pump: RÉPLICA do MailboxPump (1 thread, tick 120 ms, mesmo deliver_fn) ──
    let stop = Arc::new(AtomicBool::new(false));
    let pump_handle = spawn_pump(
        Arc::clone(&sup),
        Arc::clone(&store),
        grids.clone(),
        Mailbox::new(ws.join(".lina")),
        delivery_profile,
        Arc::clone(&stop),
    );

    // ── Espera cada claude chegar ao prompt (responde diálogo de trust com Enter) ──
    for (name, node) in &nodes {
        let grid = &grids[node];
        wait_claude_ready(&sup, *node, grid, &prompt_re, Duration::from_secs(120))
            .with_context(|| format!("claude {name} não ficou pronto"))?;
        eprintln!("[baseline] {name} pronto (prompt real casou com o regex do claude-code.toml)");
    }

    // Mailbox "lado CLI" — o mesmo canal por-nó que `lina ask` usa (from autenticado
    // pelo diretório-dono no drain; mailbox.rs `enqueue_as`).
    let cli_mailbox = Mailbox::new(ws.join(".lina"));
    let log_path = ws.join(".lina/events/log.jsonl");

    let mut stuck_report = None;
    let mut collision_report = None;

    if cfg.scenario.runs_stuck() {
        eprintln!(
            "\n[baseline] ━━ CENÁRIO 1: texto-colado (P0) — {} tentativas ━━",
            cfg.attempts
        );
        let target_name = names.get(1).cloned().unwrap_or_else(|| "W1".into());
        let r = run_stuck_scenario(
            &sup,
            &cli_mailbox,
            &log_path,
            &nodes,
            &grids,
            &target_name,
            cfg.attempts,
            &artifacts,
        )?;
        eprintln!(
            "[baseline] RESULTADO texto-colado: {}/{} presos ({} submetidos, {} tardios, {} não-entregues)",
            r.stuck, r.attempts, r.submitted, r.late_clear, r.undelivered
        );
        stuck_report = Some(r);
    }

    if cfg.scenario.runs_collision() {
        eprintln!(
            "\n[baseline] ━━ CENÁRIO 2: atropelamento (P1) — {} rodadas ━━",
            cfg.rounds
        );
        let r = run_collision_scenario(
            &sup,
            &cli_mailbox,
            &log_path,
            &nodes,
            &grids,
            &names,
            cfg.rounds,
            &artifacts,
        )?;
        eprintln!(
            "[baseline] RESULTADO atropelamento: {}/{} pares consecutivos <2s (Δ mínimo {} ms, mediano {} ms)",
            r.colliding_pairs, r.total_pairs, r.min_delta_ms, r.median_delta_ms
        );
        collision_report = Some(r);
    }

    // ── Relatório agregado (JSON) ──
    let summary = Summary {
        ws: ws.display().to_string(),
        claudes: cfg.claudes,
        delivery_profile: "demo_profile() réplica — regex='.', submit_delay=150ms (bridge.rs:327)"
            .into(),
        claude_toml_idle_ms: detect.idle_ms,
        claude_toml_ask_timeout_ms: detect.ask_timeout_ms,
        stuck: stuck_report,
        collision: collision_report,
    };
    let summary_path = artifacts.join("summary.json");
    fs::write(&summary_path, serde_json::to_string_pretty(&summary)?)
        .context("escrever summary.json")?;
    eprintln!("\n[baseline] artefatos: {}", artifacts.display());
    eprintln!("[baseline] resumo: {}", summary_path.display());

    // ── Teardown: derruba o pump e TODOS os claudes (nada vivo sobra) ──
    stop.store(true, Ordering::SeqCst);
    let _ = pump_handle.join();
    drop(grids);
    drop(harness); // Drop → kill dos PTYs + join das reader-threads
    eprintln!("[baseline] teardown completo.");
    Ok(())
}

// ───────────────────────────── pump (réplica de produção) ─────────────────────────────

/// Réplica fiel do laço do `MailboxPump` do app (bridge.rs:443-528): a cada ~120 ms,
/// reescreve `agents.json` quando o roster muda e roda `Router::pump` com o MESMO
/// `deliver_fn` (grid do alvo + `WorkspaceTrust::from_members` + `deliver_a2a`).
fn spawn_pump(
    sup: Arc<Supervisor>,
    store: Arc<Mutex<EventStore>>,
    grids: BTreeMap<NodeId, Grid>,
    mailbox: Mailbox,
    profile: CliProfile,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut router = Router::with_config(Arc::clone(&sup), mailbox, RouterConfig::default());
        let mut last_roster: Vec<AgentPresence> = Vec::new();
        while !stop.load(Ordering::SeqCst) {
            // refresh_agents (bridge.rs:453-470)
            let roster: Vec<AgentPresence> = sup
                .list()
                .into_iter()
                .map(|n| AgentPresence {
                    name: n.name,
                    role: n.role,
                    status: format!("{:?}", n.status),
                })
                .collect();
            if roster != last_roster {
                if let Err(e) = router.mailbox().write_agents(&roster) {
                    eprintln!("[pump] falha ao escrever agents.json: {e}");
                }
                last_roster = roster;
            }

            // tick (bridge.rs:501-518) — deliver_fn idêntico ao de produção.
            let mut deliver = |target: NodeId, from: NodeId, text: &str| {
                let Some(g) = grids.get(&target).cloned() else {
                    return Err(format!("sem grid para o alvo {target}"));
                };
                let members: Vec<NodeId> = sup.list().into_iter().map(|n| n.id).collect();
                let trust = WorkspaceTrust::from_members(&members);
                deliver_a2a(&sup, target, from, text, &profile, &g, trust.policy())
                    .map_err(|e| e.to_string())
            };
            let results = {
                let mut st = lock(&store);
                router.pump(&mut st, now_ms(), &mut deliver)
            };
            for (id, outcome) in &results {
                match outcome {
                    RouteOutcome::Delivered { .. }
                    | RouteOutcome::Duplicate
                    | RouteOutcome::Queued
                    | RouteOutcome::Presence => {}
                    other => eprintln!("[pump] mensagem {id} não roteada: {other:?}"),
                }
            }
            thread::sleep(PUMP_TICK);
        }
    })
}

// ───────────────────────────── cenário 1: texto-colado ─────────────────────────────

#[derive(Debug, Serialize)]
struct StuckAttempt {
    k: usize,
    marker: String,
    msg_id: String,
    outcome: String, // stuck | submitted | late_clear | undelivered | blocked:<reason>
    delivered_seq: Option<u64>,
    delivered_ts: Option<u64>,
    target_busy_at_send: bool,
    input_region_at_s1: Vec<String>,
    input_region_at_s2: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StuckReport {
    target: String,
    attempts: usize,
    stuck: usize,
    submitted: usize,
    late_clear: usize,
    undelivered: usize,
    rate: String,
    attempts_detail: Vec<StuckAttempt>,
}

#[allow(clippy::too_many_arguments)]
fn run_stuck_scenario(
    sup: &Arc<Supervisor>,
    mailbox: &Mailbox,
    log_path: &Path,
    nodes: &BTreeMap<String, NodeId>,
    grids: &BTreeMap<NodeId, Grid>,
    target_name: &str,
    attempts: usize,
    artifacts: &Path,
) -> Result<StuckReport> {
    let target = *nodes
        .get(target_name)
        .ok_or_else(|| anyhow!("alvo {target_name} não existe"))?;
    let grid = &grids[&target];
    let nonce = std::process::id();
    let mut detail = Vec::new();
    let mut tail = LogTail::new(log_path)?;

    for k in 1..=attempts {
        // Garante o alvo OCUPADO (streaming) — condição do bug (DIRECIONAMENTO §4.4).
        let busy = ensure_busy(sup, target, grid)?;

        let marker = format!("PROBE-{k}-{nonce}");
        let msg = MailMessage::new(
            "Maestro",
            format!("@{target_name}"),
            "ask",
            format!("{marker} — mensagem de teste do harness; NÃO responda, ignore."),
        );
        let msg_id = msg.id.clone();
        mailbox
            .enqueue_as("Maestro", &msg)
            .context("enqueue_as no outbox por-nó (mesmo canal do lina ask)")?;

        // Espera o desfecho do roteamento no PRÓPRIO log (fonte da verdade).
        let evt = tail.wait_for(Duration::from_secs(25), |rec| {
            let id = rec.payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            id == msg_id && (rec.kind == "MessageDelivered" || rec.kind == "RouteBlocked")
        })?;

        let (outcome, seq, ts, s1_rows, s2_rows) = match evt {
            Some(rec) if rec.kind == "MessageDelivered" => {
                thread::sleep(STUCK_SAMPLE_1);
                let rows1 = grid_rows(grid);
                let s1 = marker_in_input_region(&rows1, &marker);
                thread::sleep(STUCK_SAMPLE_2);
                let rows2 = grid_rows(grid);
                let s2 = marker_in_input_region(&rows2, &marker);
                let outcome = match (s1, s2) {
                    (true, true) => "stuck",
                    (true, false) => "late_clear",
                    (false, true) => "stuck", // reapareceu/persistiu no input → preso
                    (false, false) => "submitted",
                };
                (
                    outcome.to_string(),
                    Some(rec.seq),
                    Some(rec.ts),
                    input_region(&rows1),
                    input_region(&rows2),
                )
            }
            Some(rec) => {
                let reason = rec
                    .payload
                    .get("reason")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into());
                (
                    format!("blocked:{reason}"),
                    Some(rec.seq),
                    Some(rec.ts),
                    vec![],
                    vec![],
                )
            }
            None => ("undelivered".to_string(), None, None, vec![], vec![]),
        };

        eprintln!("[stuck] tentativa {k}/{attempts}: {outcome} (busy_no_envio={busy})");
        if outcome == "stuck" {
            // Evidência completa: grid inteiro do alvo no momento do "preso".
            let dir = artifacts.join("grids");
            let _ = fs::create_dir_all(&dir);
            let _ = fs::write(
                dir.join(format!("stuck-{k}.txt")),
                grid_rows(grid).join("\n"),
            );
            clear_input(sup, target, grid, &marker);
        }
        detail.push(StuckAttempt {
            k,
            marker,
            msg_id,
            outcome,
            delivered_seq: seq,
            delivered_ts: ts,
            target_busy_at_send: busy,
            input_region_at_s1: s1_rows,
            input_region_at_s2: s2_rows,
        });
        thread::sleep(Duration::from_secs(3));
    }

    let stuck = detail.iter().filter(|a| a.outcome == "stuck").count();
    let submitted = detail.iter().filter(|a| a.outcome == "submitted").count();
    let late = detail.iter().filter(|a| a.outcome == "late_clear").count();
    let undelivered = detail.len() - stuck - submitted - late;
    let report = StuckReport {
        target: target_name.to_string(),
        attempts,
        stuck,
        submitted,
        late_clear: late,
        undelivered,
        rate: format!("{stuck}/{attempts}"),
        attempts_detail: detail,
    };
    fs::write(
        artifacts.join("stuck.json"),
        serde_json::to_string_pretty(&report)?,
    )
    .context("escrever stuck.json")?;
    Ok(report)
}

/// Alvo ocupado = grid mudando (claude streamando). Se ocioso, injeta (como HUMANO,
/// pela mesma fila serial) uma tarefa longa de baixo custo e espera streamar.
fn ensure_busy(sup: &Arc<Supervisor>, target: NodeId, grid: &Grid) -> Result<bool> {
    if grid_changing(grid, Duration::from_millis(900)) {
        return Ok(true);
    }
    type_line(
        sup,
        target,
        "Conte de 1 até 2500 imprimindo um número por linha, sem usar nenhuma ferramenta e sem comentários.",
    )?;
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if grid_changing(grid, Duration::from_millis(900)) {
            return Ok(true);
        }
    }
    // Segue mesmo assim (registrado como busy=false na tentativa) — taxa honesta.
    Ok(false)
}

/// Se um paste ficou preso, limpa o input do alvo (Ctrl+U na linha) para a PRÓXIMA
/// tentativa começar limpa — intervenção do protocolo, registrada no doc da baseline.
fn clear_input(sup: &Arc<Supervisor>, target: NodeId, grid: &Grid, marker: &str) {
    for _ in 0..3 {
        let _ = sup.enqueue_write(target, WriteOp::HumanKeys(vec![0x15])); // Ctrl+U
        thread::sleep(Duration::from_millis(600));
        if !marker_in_input_region(&grid_rows(grid), marker) {
            return;
        }
    }
    eprintln!("[stuck] aviso: input do alvo seguiu sujo após 3×Ctrl+U (registrado)");
}

// ───────────────────────────── cenário 2: atropelamento ─────────────────────────────

#[derive(Debug, Serialize)]
struct CollisionDelivery {
    seq: u64,
    ts: u64,
    msg_id: String,
    from: String,
    round: usize,
}

#[derive(Debug, Serialize)]
struct CollisionPair {
    prev_seq: u64,
    cur_seq: u64,
    delta_ms: u64,
    colliding: bool,
}

#[derive(Debug, Serialize)]
struct CollisionReport {
    target: String,
    rounds: usize,
    senders_per_round: usize,
    deliveries: Vec<CollisionDelivery>,
    pairs: Vec<CollisionPair>,
    total_pairs: usize,
    colliding_pairs: usize,
    min_delta_ms: u64,
    median_delta_ms: u64,
    maestro_grid_snapshots: Vec<Vec<String>>,
}

#[allow(clippy::too_many_arguments)]
fn run_collision_scenario(
    sup: &Arc<Supervisor>,
    mailbox: &Mailbox,
    log_path: &Path,
    nodes: &BTreeMap<String, NodeId>,
    grids: &BTreeMap<NodeId, Grid>,
    names: &[String],
    rounds: usize,
    artifacts: &Path,
) -> Result<CollisionReport> {
    let maestro_name = &names[0];
    let maestro = nodes[maestro_name];
    let maestro_grid = &grids[&maestro];
    let workers: Vec<&String> = names.iter().skip(1).collect();
    let mut tail = LogTail::new(log_path)?;
    let mut deliveries: Vec<CollisionDelivery> = Vec::new();
    let mut snapshots = Vec::new();

    for r in 1..=rounds {
        // "Fan-out de tarefa curta que termina junto": os retornos dos N workers são
        // depositados no MESMO instante (a sincronia é a condição do bug — §P1; a
        // simultaneidade real foi vista ao vivo em seq 1853→1860 com Δ0.6s).
        let mut ids: BTreeMap<String, String> = BTreeMap::new();
        for w in &workers {
            let msg = MailMessage::new(
                (*w).clone(),
                format!("@{maestro_name}"),
                "ask",
                format!("RESULTADO rodada {r} de {w}: tarefa concluída — probe do harness, NÃO responda."),
            );
            ids.insert(msg.id.clone(), (*w).clone());
            mailbox
                .enqueue_as(w, &msg)
                .with_context(|| format!("enqueue_as de {w}"))?;
        }

        // Colhe os MessageDelivered (ao Maestro) desta rodada no log.
        let deadline = Instant::now() + Duration::from_secs(40);
        let mut got = 0usize;
        while got < ids.len() && Instant::now() < deadline {
            for rec in tail.poll()? {
                if rec.kind != "MessageDelivered" {
                    continue;
                }
                let id = rec.payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(from) = ids.get(id) {
                    deliveries.push(CollisionDelivery {
                        seq: rec.seq,
                        ts: rec.ts,
                        msg_id: id.to_string(),
                        from: from.clone(),
                        round: r,
                    });
                    got += 1;
                }
            }
            thread::sleep(Duration::from_millis(60));
        }
        eprintln!(
            "[collision] rodada {r}/{rounds}: {got}/{} entregas ao {maestro_name}",
            ids.len()
        );

        // Evidência visual: o grid COMPLETO do Maestro logo após a rajada.
        thread::sleep(Duration::from_secs(2));
        snapshots.push(grid_rows(maestro_grid));
        // Limpa o input do Maestro entre rodadas (mesma intervenção do cenário 1).
        let _ = sup.enqueue_write(maestro, WriteOp::HumanKeys(vec![0x15]));
        thread::sleep(Duration::from_secs(5));
    }

    deliveries.sort_by_key(|d| d.seq);
    let pairs = collision_pairs(&deliveries, COLLISION_THRESHOLD_MS);
    let deltas: Vec<u64> = pairs.iter().map(|p| p.delta_ms).collect();
    let colliding = pairs.iter().filter(|p| p.colliding).count();
    let report = CollisionReport {
        target: maestro_name.clone(),
        rounds,
        senders_per_round: workers.len(),
        total_pairs: pairs.len(),
        colliding_pairs: colliding,
        min_delta_ms: deltas.iter().copied().min().unwrap_or(0),
        median_delta_ms: median(&deltas),
        deliveries,
        pairs,
        maestro_grid_snapshots: snapshots,
    };
    fs::write(
        artifacts.join("collision.json"),
        serde_json::to_string_pretty(&report)?,
    )
    .context("escrever collision.json")?;
    Ok(report)
}

/// Pares consecutivos de entrega ao mesmo alvo (já filtrado) com seus Δt.
/// Compara APENAS dentro da mesma rodada — entre rodadas há settle proposital.
fn collision_pairs(deliveries: &[CollisionDelivery], threshold_ms: u64) -> Vec<CollisionPair> {
    let mut out = Vec::new();
    for w in deliveries.windows(2) {
        if w[0].round != w[1].round {
            continue;
        }
        let delta = w[1].ts.saturating_sub(w[0].ts);
        out.push(CollisionPair {
            prev_seq: w[0].seq,
            cur_seq: w[1].seq,
            delta_ms: delta,
            colliding: delta < threshold_ms,
        });
    }
    out
}

fn median(sorted_src: &[u64]) -> u64 {
    if sorted_src.is_empty() {
        return 0;
    }
    let mut v = sorted_src.to_vec();
    v.sort_unstable();
    v[v.len() / 2]
}

// ───────────────────────────── leitura do grid (detecções) ─────────────────────────────

fn grid_rows(grid: &Grid) -> Vec<String> {
    let g = lock(grid);
    (0..usize::from(ROWS)).map(|i| g.row_text(i)).collect()
}

fn rows_hash(rows: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    rows.hash(&mut h);
    h.finish()
}

/// O grid mudou dentro da janela? (proxy de "claude streamando/ocupado").
fn grid_changing(grid: &Grid, window: Duration) -> bool {
    let h0 = rows_hash(&grid_rows(grid));
    let steps = 3u32;
    let step = window / steps;
    for _ in 0..steps {
        thread::sleep(step);
        if rows_hash(&grid_rows(grid)) != h0 {
            return true;
        }
    }
    false
}

/// Região do INPUT vivo da TUI do Claude Code (calibrada contra o grid REAL no
/// smoke de 2026-06-06, Claude Code v2.1.166): o prompt vivo é a ÚLTIMA linha do
/// grid que começa com `>`/`❯` (com ou sem borda `│` de caixa) — echo de mensagens
/// já submetidas aparece ACIMA dele; a região vai do prompt vivo até o fim do grid
/// (cobre paste multi-linha preso). Fallback: últimas 8 linhas. A regra é
/// DETERMINÍSTICA e testada (`tests::input_region_*`) — o doc da baseline a cita
/// como o critério de "texto preso".
fn input_region(rows: &[String]) -> Vec<String> {
    match rows.iter().rposition(|r| is_prompt_row(r)) {
        Some(i) => rows[i..].to_vec(),
        None => {
            let start = rows.len().saturating_sub(8);
            rows[start..].to_vec()
        }
    }
}

/// Linha de prompt vivo da TUI: `> ` ou `❯ ` no início (opcionalmente atrás da
/// borda `│` nas versões que desenham caixa em volta do input).
fn is_prompt_row(row: &str) -> bool {
    let mut t = row.trim_start();
    if let Some(rest) = t.strip_prefix('│') {
        t = rest.trim_start();
    }
    t == ">" || t == "❯" || t.starts_with("> ") || t.starts_with("❯ ")
}

fn marker_in_input_region(rows: &[String], marker: &str) -> bool {
    input_region(rows).iter().any(|r| r.contains(marker))
}

// ───────────────────────────── claude: boot + input humano ─────────────────────────────

/// Espera o claude do nó chegar ao prompt REAL (regex do claude-code.toml). Diálogos
/// de primeira execução (trust da pasta etc.) são respondidos com Enter (default).
fn wait_claude_ready(
    sup: &Arc<Supervisor>,
    node: NodeId,
    grid: &Grid,
    prompt_re: &regex::Regex,
    timeout: Duration,
) -> Result<()> {
    const DIALOG_MARKERS: &[&str] = &[
        "trust the files",
        "Do you trust",
        "Yes, proceed",
        "Press Enter",
        "to continue",
    ];
    let deadline = Instant::now() + timeout;
    let mut last_enter = Instant::now() - Duration::from_secs(10);
    let mut first_match: Option<Instant> = None;
    while Instant::now() < deadline {
        let rows = grid_rows(grid);
        let joined = rows.join("\n");
        if prompt_re.is_match(&joined) {
            // Settle anti-race: o prompt precisa SEGUIR visível por 2s contínuos
            // antes de declararmos pronto (a TUI pode ainda estar montando — visto
            // no smoke: match em ~1.3s pós-spawn).
            match first_match {
                Some(t0) if t0.elapsed() >= Duration::from_secs(2) => return Ok(()),
                Some(_) => {}
                None => first_match = Some(Instant::now()),
            }
            thread::sleep(Duration::from_millis(400));
            continue;
        }
        first_match = None;
        let dialog = DIALOG_MARKERS.iter().any(|m| joined.contains(m));
        if dialog && last_enter.elapsed() > Duration::from_secs(2) {
            let _ = sup.enqueue_write(node, WriteOp::Submit { from: None });
            last_enter = Instant::now();
        }
        thread::sleep(Duration::from_millis(400));
    }
    let tail: Vec<String> = grid_rows(grid).into_iter().rev().take(12).collect();
    Err(anyhow!(
        "timeout esperando o prompt do claude; últimas linhas do grid: {tail:?}"
    ))
}

/// Digita uma linha como HUMANO (mesma fila serial `WriteOp::HumanKeys` + `Submit`
/// que o teclado do app usa) — é assim que armamos a tarefa longa no alvo.
fn type_line(sup: &Arc<Supervisor>, node: NodeId, text: &str) -> Result<()> {
    sup.enqueue_write(node, WriteOp::HumanKeys(text.as_bytes().to_vec()))
        .map_err(|e| anyhow!("HumanKeys: {e}"))?;
    thread::sleep(Duration::from_millis(250));
    sup.enqueue_write(node, WriteOp::Submit { from: None })
        .map_err(|e| anyhow!("Submit: {e}"))?;
    Ok(())
}

// ───────────────────────────── log.jsonl: leitor incremental ─────────────────────────────

/// Registro mínimo do espelho `log.jsonl` (EventRecord do events.rs).
#[derive(Debug, Clone)]
struct LogRec {
    seq: u64,
    ts: u64,
    kind: String,
    payload: serde_json::Value,
}

/// Leitor incremental do `log.jsonl` — o harness consome a MESMA fonte da verdade
/// que o `lina_watch` (âncora Event Store; nenhum canal paralelo).
struct LogTail {
    path: PathBuf,
    offset: u64,
}

impl LogTail {
    fn new(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            offset: 0,
        })
    }

    fn poll(&mut self) -> Result<Vec<LogRec>> {
        let Ok(f) = fs::File::open(&self.path) else {
            return Ok(Vec::new()); // log ainda não existe — sem eventos
        };
        let mut reader = BufReader::new(f);
        reader
            .seek(SeekFrom::Start(self.offset))
            .context("seek no log.jsonl")?;
        let mut out = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).context("ler log.jsonl")?;
            if n == 0 {
                break;
            }
            if !line.ends_with('\n') {
                break; // linha parcial (escrita em curso): re-lê no próximo poll
            }
            self.offset += n as u64;
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let (Some(seq), Some(ts), Some(kind)) = (
                v.get("seq").and_then(serde_json::Value::as_u64),
                v.get("ts").and_then(serde_json::Value::as_u64),
                v.get("kind").and_then(|k| k.as_str()),
            ) else {
                continue;
            };
            out.push(LogRec {
                seq,
                ts,
                kind: kind.to_string(),
                payload: v.get("payload").cloned().unwrap_or(serde_json::Value::Null),
            });
        }
        Ok(out)
    }

    fn wait_for(
        &mut self,
        timeout: Duration,
        mut pred: impl FnMut(&LogRec) -> bool,
    ) -> Result<Option<LogRec>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            for rec in self.poll()? {
                if pred(&rec) {
                    return Ok(Some(rec));
                }
            }
            thread::sleep(Duration::from_millis(60));
        }
        Ok(None)
    }
}

// ───────────────────────────── config / util ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Stuck,
    Collision,
    All,
}

impl Scenario {
    fn runs_stuck(self) -> bool {
        matches!(self, Scenario::Stuck | Scenario::All)
    }
    fn runs_collision(self) -> bool {
        matches!(self, Scenario::Collision | Scenario::All)
    }
}

struct Config {
    ws: PathBuf,
    claudes: usize,
    attempts: usize,
    rounds: usize,
    scenario: Scenario,
    claude_bin: String,
    claude_profile: PathBuf,
}

fn parse_args() -> Result<Config> {
    let mut ws = None;
    let mut claudes = 6usize;
    let mut attempts = 22usize;
    let mut rounds = 6usize;
    let mut scenario = Scenario::All;
    let mut claude_bin = "claude".to_string();
    let mut claude_profile = PathBuf::from("profiles/claude-code.toml");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--ws" => ws = Some(PathBuf::from(next(&mut args, "--ws")?)),
            "--claudes" => claudes = next(&mut args, "--claudes")?.parse()?,
            "--attempts" => attempts = next(&mut args, "--attempts")?.parse()?,
            "--rounds" => rounds = next(&mut args, "--rounds")?.parse()?,
            "--claude-bin" => claude_bin = next(&mut args, "--claude-bin")?,
            "--claude-profile" => {
                claude_profile = PathBuf::from(next(&mut args, "--claude-profile")?)
            }
            "--scenario" => {
                scenario = match next(&mut args, "--scenario")?.as_str() {
                    "stuck" => Scenario::Stuck,
                    "collision" => Scenario::Collision,
                    "all" => Scenario::All,
                    other => return Err(anyhow!("--scenario inválido: {other}")),
                }
            }
            "-h" | "--help" => {
                eprintln!(
                    "uso: baseline_f1_0 --ws DIR [--claudes 6] [--attempts 22] [--rounds 6] \
[--scenario all|stuck|collision] [--claude-bin claude] [--claude-profile profiles/claude-code.toml]"
                );
                std::process::exit(0);
            }
            other => return Err(anyhow!("flag desconhecida: {other}")),
        }
    }
    if !(5..=8).contains(&claudes) {
        eprintln!("[baseline] aviso: o protocolo do gate pede 5–8 claudes (pedido: {claudes})");
    }
    if attempts < 20 {
        eprintln!("[baseline] aviso: o critério (2) pede ≥20 tentativas (pedido: {attempts})");
    }
    let ws = ws.ok_or_else(|| {
        anyhow!("--ws é obrigatório (workspace ISOLADO; nunca aponte para um workspace vivo)")
    })?;
    Ok(Config {
        ws,
        claudes,
        attempts,
        rounds,
        scenario,
        claude_bin,
        claude_profile,
    })
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next().ok_or_else(|| anyhow!("{flag} requer um valor"))
}

fn agent_names(n: usize) -> Vec<String> {
    let mut v = vec!["Maestro".to_string()];
    for i in 1..n {
        v.push(format!("W{i}"));
    }
    v
}

/// Réplica TESTADA do `demo_profile()` que o app usa na entrega em produção hoje
/// (`app/lina-gpui/src/bridge.rs:327` — fora do workspace do core, daí a réplica).
/// Se produção mudar, o teste `demo_profile_replica_fields` é o lembrete de re-checar.
fn demo_profile_replica() -> Result<CliProfile> {
    let src = r#"
        id = "lina-demo"
        program = "sh"
        delivery = "pty_inject"
        submit_delay_ms = 150
        prompt_ready_regex = '.'
        idle_ms = 200
        ask_timeout_ms = 120000
        [end_signal]
        kind = "idle"
    "#;
    CliProfile::from_toml_str(src, "<demo-replica>")
        .map_err(|e| anyhow!("réplica do demo_profile não parseia: {e}"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn append(store: &mut EventStore, event: &DomainEvent) -> Result<()> {
    store
        .append(event)
        .map(|_| ())
        .map_err(|e| anyhow!("append no event store: {e}"))
}

// ───────────────────────────── testes ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A réplica do demo_profile DEVE espelhar bridge.rs:327 (regex '.', 150 ms).
    /// Se este teste falhar, produção mudou → re-verificar a fidelidade da baseline.
    #[test]
    fn demo_profile_replica_fields() {
        let p = demo_profile_replica().expect("parseia");
        assert_eq!(p.id, "lina-demo");
        assert_eq!(p.prompt_ready_regex, ".");
        assert_eq!(p.submit_delay_ms, 150);
        assert_eq!(p.idle_ms, Some(200));
        assert_eq!(p.ask_timeout_ms, Some(120_000));
    }

    /// Regra calibrada no grid REAL (smoke 2026-06-06, Claude Code v2.1.166): o
    /// input vivo é a ÚLTIMA linha-prompt (`> `/`❯ `, com ou sem borda `│`) até o
    /// fim do grid. Marcador nessa região = preso; echo ACIMA do prompt vivo (msg
    /// já submetida) NÃO conta.
    #[test]
    fn input_region_finds_live_prompt() {
        // (a) marcador preso no input vivo (prompt sem borda — layout v2.1.166).
        let mut rows: Vec<String> = (0..36).map(|i| format!("linha de saída {i}")).collect();
        rows.push("> PROBE-7-123 texto preso".into());
        rows.push("  ? for shortcuts".into());
        assert!(marker_in_input_region(&rows, "PROBE-7-123"));

        // (b) o MESMO marcador como echo de mensagem submetida, input vivo VAZIO.
        let mut rows2: Vec<String> = (0..34).map(|i| format!("linha {i}")).collect();
        rows2.push("> PROBE-7-123 (echo da mensagem já submetida)".into());
        rows2.push("resposta do claude em andamento…".into());
        rows2.push("> ".into()); // prompt vivo, vazio
        rows2.push(String::new());
        assert!(!marker_in_input_region(&rows2, "PROBE-7-123"));

        // (c) variante com caixa em volta do input (`│ > … │`) também é prompt.
        let mut rows3: Vec<String> = (0..36).map(|i| format!("l{i}")).collect();
        rows3.push("╭──────────────╮".into());
        rows3.push("│ > PROBE-9-77 │".into());
        rows3.push("╰──────────────╯".into());
        assert!(marker_in_input_region(&rows3, "PROBE-9-77"));

        // (d) a caixa de BOAS-VINDAS (sem linha-prompt dentro) não engana a regra:
        // com um prompt vivo abaixo dela, a região começa no prompt.
        let mut rows4: Vec<String> = vec!["╭─── Claude Code v2.1.166 ───╮".into()];
        rows4.push("│  Welcome back!             │".into());
        rows4.push("╰────────────────────────────╯".into());
        rows4.push("> ".into());
        let region = input_region(&rows4);
        assert_eq!(region.len(), 1);
        assert!(is_prompt_row(&region[0]));
    }

    /// Sem caixa detectável, o fallback são as últimas 8 linhas (regra documentada).
    #[test]
    fn input_region_fallback_last_8() {
        let rows: Vec<String> = (0..40).map(|i| format!("r{i}")).collect();
        let region = input_region(&rows);
        assert_eq!(region.len(), 8);
        assert_eq!(region[0], "r32");
        assert_eq!(region[7], "r39");
    }

    /// Δt entre entregas consecutivas ao mesmo alvo: <2000 ms = colisão; pares de
    /// rodadas diferentes NÃO comparam (settle proposital entre rodadas).
    #[test]
    fn collision_pairs_rule() {
        let d = |seq: u64, ts: u64, round: usize| CollisionDelivery {
            seq,
            ts,
            msg_id: format!("msg_{seq}"),
            from: "W1".into(),
            round,
        };
        let deliveries = vec![
            d(10, 1000, 1),
            d(11, 1600, 1), // Δ600 → colisão (padrão seq 1853→1860 da observação)
            d(12, 4200, 1), // Δ2600 → ok
            d(20, 9000, 2), // rodada nova: NÃO compara com seq 12
            d(21, 9100, 2), // Δ100 → colisão
        ];
        let pairs = collision_pairs(&deliveries, COLLISION_THRESHOLD_MS);
        assert_eq!(pairs.len(), 3);
        assert!(pairs[0].colliding && pairs[0].delta_ms == 600);
        assert!(!pairs[1].colliding && pairs[1].delta_ms == 2600);
        assert!(pairs[2].colliding && pairs[2].delta_ms == 100);
    }

    #[test]
    fn median_of_deltas() {
        assert_eq!(median(&[]), 0);
        assert_eq!(median(&[5]), 5);
        assert_eq!(median(&[900, 100, 500]), 500);
    }

    #[test]
    fn agent_names_shape() {
        let n = agent_names(6);
        assert_eq!(n[0], "Maestro");
        assert_eq!(n.len(), 6);
        assert_eq!(n[5], "W5");
    }
}

#[derive(Debug, Serialize)]
struct Summary {
    ws: String,
    claudes: usize,
    delivery_profile: String,
    claude_toml_idle_ms: Option<u64>,
    claude_toml_ask_timeout_ms: Option<u64>,
    stuck: Option<StuckReport>,
    collision: Option<CollisionReport>,
}
