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

    // Perfil de ENTREGA = espelho de produção. ANTES (baseline original): demo_profile()
    // hardcoded (bridge.rs:326, hoje só fallback). DEPOIS (F1-0-2, f130144): produção
    // carrega o claude-code.toml REAL (`production_profile()`) — o harness segue produção
    // (dever anti-drift da baseline). `--legacy-demo-delivery` reproduz o ANTES.
    let delivery_profile = if cfg.legacy_demo_delivery {
        demo_profile_replica()?
    } else {
        detect.clone()
    };
    eprintln!(
        "[baseline] perfil de ENTREGA ({}): regex={:?} submit_delay_ms={}",
        if cfg.legacy_demo_delivery {
            "LEGADO demo_profile — reproduz o ANTES"
        } else {
            "produção F1-0-2 — claude-code.toml real"
        },
        delivery_profile.prompt_ready_regex,
        delivery_profile.submit_delay_ms
    );
    // F1-0-4: janela de quietude que devolve um alvo Busy a Idle no harness (espelha o
    // fim-de-turno por idle do meter de produção — bridge.rs ~2598 — sincronizado no
    // ROSTER, que é a fonte da retenção).
    let idle_ms = detect.idle_ms.unwrap_or(1_500);

    // ── Sobe N claudes reais headless (cabeamento de produção) ──
    let store = Arc::new(Mutex::new(
        EventStore::open(ws.join(".lina/events")).map_err(|e| anyhow!("abrir EventStore: {e}"))?,
    ));

    // RÉPLICA do boot de produção (F1-0-8, `app/lina-gpui/src/main.rs:2464`): ao abrir o
    // workspace, fecha o ciclo de vida da geração ANTERIOR no log (`NodeStatusChanged{
    // Dead, reason:app_reopened}`) — em ws fresco é no-op. O cenário `reopen-check`
    // mede as projeções antes/depois (gate (e): replay = roster vivo, 0 nós-fantasma).
    let reopen_pre = if cfg.scenario == Scenario::ReopenCheck {
        Some(project_alive(&mut lock(&store))?)
    } else {
        None
    };
    let closed_prev = lina_core::lifecycle::close_previous_generation(&mut lock(&store))
        .map_err(|e| anyhow!("close_previous_generation: {e}"))?;
    eprintln!(
        "[baseline] boot F1-0-8 (réplica main.rs:2464): {} nó(s) da geração anterior fechados",
        closed_prev.len()
    );
    let reopen_post = if cfg.scenario == Scenario::ReopenCheck {
        Some(project_alive(&mut lock(&store))?)
    } else {
        None
    };

    let mut harness = LoadHarness::new(COLS, ROWS);
    let names = agent_names(cfg.claudes);
    let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
    let mut nodes: BTreeMap<String, NodeId> = BTreeMap::new();

    // Cenário `handoff`: identidade + permissão ANTES do spawn — espelha o onboarding
    // de produção (`write_terminal_files`, bridge.rs:1939) no MÍNIMO necessário ao
    // transcript: `.lina/bootstrap.json` (whoami/roster — contrato serde estável do
    // lina-bootstrap; o runner não pode depender da lib: ciclo bootstrap→core) e
    // `.claude/settings.json` com allow de `Bash(lina*)` (sem ele o claude REAL trava
    // no prompt de permissão ao responder — o toast de permissão é gate da F1-1, não
    // deste). Registrado como condição do transcript no artefato.
    let lina_bin_abs = if cfg.scenario == Scenario::Handoff {
        let p = cfg
            .lina_bin
            .canonicalize()
            .map_err(|e| anyhow!("--lina-bin {} inacessível: {e}", cfg.lina_bin.display()))?;
        Some(p)
    } else {
        None
    };

    for (i, name) in names.iter().enumerate() {
        let cwd = ws.join("agents").join(name);
        fs::create_dir_all(&cwd).with_context(|| format!("criar cwd de {name}"))?;
        let mut cmd = PtyCommand::new(&cfg.claude_bin)
            .cwd(&cwd)
            .env("TERM", "xterm-256color");
        if let Some(lina_bin) = &lina_bin_abs {
            write_handoff_agent_files(&cwd, name, &names)?;
            let path_env = format!(
                "{}:{}",
                lina_bin
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default()
                    .display(),
                std::env::var("PATH").unwrap_or_default()
            );
            cmd = cmd
                .env("LINA_HOME", ws.join(".lina").display().to_string())
                .env("PATH", path_env);
        }
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
                    requested_by: None,
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
                    cwd: None,
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
        idle_ms,
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
            cfg.busy_count,
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

    // ── Cenários do gate DEPOIS (F1-0) ──
    if cfg.scenario == Scenario::Dlq {
        eprintln!("\n[baseline] ━━ CENÁRIO DLQ (gate d): não-entregável → dead-letter visível ━━");
        run_dlq_scenario(
            &sup,
            &cli_mailbox,
            &log_path,
            &ws,
            &nodes,
            &names,
            &artifacts,
        )?;
    }
    if cfg.scenario == Scenario::ReopenCheck {
        eprintln!("\n[baseline] ━━ CENÁRIO REOPEN (gate e): replay = roster vivo ━━");
        run_reopen_check(
            &sup,
            &store,
            &log_path,
            reopen_pre.unwrap_or_default(),
            &closed_prev,
            reopen_post.unwrap_or_default(),
            &artifacts,
        )?;
    }
    if cfg.scenario == Scenario::Handoff {
        eprintln!("\n[baseline] ━━ CENÁRIO HANDOFF (F1-0-6 critério 1): golden transcript ━━");
        let lina_bin = lina_bin_abs.as_deref().expect("setado no scenario Handoff");
        run_handoff_scenario(&ws, lina_bin, &log_path, &nodes, &grids, &names, &artifacts)?;
    }

    // ── Relatório agregado (JSON) ──
    let summary = Summary {
        ws: ws.display().to_string(),
        claudes: cfg.claudes,
        delivery_profile: if cfg.legacy_demo_delivery {
            "demo_profile() réplica — regex='.', submit_delay=150ms (ANTES, ex-bridge.rs:327)"
                .into()
        } else {
            format!(
                "produção F1-0-2 — claude-code.toml real (regex={:?}, submit_delay_ms={}, ready_timeout_ms={:?}, busy_markers={:?})",
                detect.prompt_ready_regex, detect.submit_delay_ms, detect.ready_timeout_ms, detect.busy_markers
            )
        },
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
#[allow(clippy::too_many_arguments)]
fn spawn_pump(
    sup: Arc<Supervisor>,
    store: Arc<Mutex<EventStore>>,
    grids: BTreeMap<NodeId, Grid>,
    mailbox: Mailbox,
    profile: CliProfile,
    idle_ms: u64,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut router = Router::with_config(Arc::clone(&sup), mailbox, RouterConfig::default());
        let mut last_roster: Vec<AgentPresence> = Vec::new();
        // F1-0-4: detector de fim-de-turno por quietude do grid (espelha o meter de
        // produção, SINCRONIZADO NO ROSTER — a fonte da retenção). Um alvo `Busy` cujo
        // grid não muda por `idle_ms` volta a `Idle` pelo caminho canônico do lifecycle.
        let mut engine = lina_core::LifecycleEngine::new();
        let mut last_change: BTreeMap<NodeId, (u64, std::time::Instant)> = BTreeMap::new();
        while !stop.load(Ordering::SeqCst) {
            for (node, grid) in &grids {
                let h = rows_hash(&grid_rows(grid));
                let entry = last_change
                    .entry(*node)
                    .or_insert((h, std::time::Instant::now()));
                if entry.0 != h {
                    *entry = (h, std::time::Instant::now());
                } else if entry.1.elapsed() >= Duration::from_millis(idle_ms)
                    && sup.get(*node).map(|i| i.status) == Some(lina_core::NodeStatus::Busy)
                {
                    // PARIDADE COM O APP (fix de gate F1-0, bridge.rs `spawn_pump`):
                    // produção agora chama `on_end_of_response` no fim-de-turno do
                    // medidor (reason `end_of_response`, event-sourced no roster) —
                    // a réplica usa a MESMA API. Delta residual e aceito: o SINAL no
                    // app é o silêncio do CostMeter sobre `GridDelta.bytes`; aqui é o
                    // hash do grid estável — ambos "sem output novo por idle_ms do
                    // perfil". O guard `status == Busy` espelha o active-set do app
                    // (só nós em turno chegam ao `poll_finished_turns`).
                    let mut st = lock(&store);
                    if let Err(e) = engine.on_end_of_response(&sup, &mut st, *node) {
                        eprintln!("[pump] fim-de-resposta →Idle falhou: {e}");
                    }
                }
            }
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
            // Narração = PRODUÇÃO (bridge.rs `tick`): só Queued/Presence são silenciosos;
            // Retained/RetryBackoff/DeadLettered são narrados no stderr — é o "alerta
            // visível" do gate (d), além do "☠ DLQ" que o próprio core emite.
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
    busy_count: u32,
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
        let busy = ensure_busy(sup, target, grid, busy_count)?;

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
        // DEPOIS (F1-0-4/7): a entrega pode RETER (alvo Busy) e re-tentar com backoff —
        // o desfecho final demora até o alvo aquietar; 25s (ANTES) → 240s, e a DLQ
        // também é desfecho terminal (nada some em silêncio).
        let evt = tail.wait_for(Duration::from_secs(240), |rec| {
            let id = rec.payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            id == msg_id
                && (rec.kind == "MessageDelivered"
                    || rec.kind == "RouteBlocked"
                    || rec.kind == "MessageDeadLettered")
        })?;

        let (outcome, seq, ts, s1_rows, s2_rows) = match evt {
            Some(rec) if rec.kind == "MessageDelivered" => {
                // RECALIBRAÇÃO DO DEPOIS (ressalva 3 da baseline, prevista): no ANTES o
                // texto preso ficava no INPUT VIVO com o terminal ocioso. No DEPOIS a
                // mensagem é SUBMETIDA de verdade — enquanto o claude processa o turno,
                // o ECO `❯ [LINA::MSG]…` é a última linha-prompt do grid e a amostra
                // fixa t+3/t+8 confundia eco-em-processamento com preso (falso positivo
                // visto no smoke de 2026-06-06: grid final limpo + turno respondido).
                // Regra equivalente nas duas eras: espera o terminal AQUIETAR (prompt
                // presente, sem "esc to interrupt"; teto 90s — o estado preso do ANTES
                // já nasce quieto) e SÓ ENTÃO faz as 2 amostras (gap 3s) do marcador na
                // região de input vivo.
                wait_terminal_quiet(grid, Duration::from_secs(90));
                thread::sleep(STUCK_SAMPLE_1);
                let rows1 = grid_rows(grid);
                let s1 = marker_in_input_region(&rows1, &marker) || stuck_chip_signature(&rows1);
                thread::sleep(STUCK_SAMPLE_2);
                let rows2 = grid_rows(grid);
                let s2 = marker_in_input_region(&rows2, &marker) || stuck_chip_signature(&rows2);
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
                let label = if rec.kind == "MessageDeadLettered" {
                    "dead_lettered"
                } else {
                    "blocked"
                };
                (
                    format!("{label}:{reason}"),
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
/// `busy_count` parametriza a duração do streaming: 2500 = protocolo do ANTES;
/// o DEPOIS usa janela menor (a entrega agora ESPERA prontidão — busy infinito
/// estouraria o orçamento de retry por design, virando DLQ e não medição de P0).
fn ensure_busy(
    sup: &Arc<Supervisor>,
    target: NodeId,
    grid: &Grid,
    busy_count: u32,
) -> Result<bool> {
    if grid_changing(grid, Duration::from_millis(900)) {
        return Ok(true);
    }
    type_line(
        sup,
        target,
        &format!(
            "Conte de 1 até {busy_count} imprimindo um número por linha, sem usar nenhuma ferramenta e sem comentários."
        ),
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
/// RECALIBRAÇÃO 2026-06-06 (gate DEPOIS, evidência de bytes do smoke): o prompt VIVO
/// do Claude Code v2.1.166 desenha `❯` + **NBSP (U+00A0)**, enquanto o ECO de
/// mensagem submetida usa `❯` + espaço comum — sem normalizar, o prompt vivo nunca
/// casava e a região caía no eco (falso "stuck" em mensagem JÁ submetida). No ANTES
/// isso nunca apareceu: mensagem presa não gera eco, e o fallback das últimas 8
/// linhas dava o veredito certo.
fn is_prompt_row(row: &str) -> bool {
    let norm = row.replace('\u{a0}', " ");
    let mut t = norm.trim_start();
    if let Some(rest) = t.strip_prefix('│') {
        t = rest.trim_start();
    }
    t == ">" || t == "❯" || t.starts_with("> ") || t.starts_with("❯ ")
}

fn marker_in_input_region(rows: &[String], marker: &str) -> bool {
    input_region(rows).iter().any(|r| r.contains(marker))
}

/// Assinatura do estado PRESO-EM-CHIP (controle legacy 2026-06-06, evidência de grid):
/// no caminho do ANTES o paste colapsa em chip — o CONTEÚDO (e o marcador) ficam
/// INVISÍVEIS no grid e a detecção por marcador é cega. O que o grid mostra é o
/// rodapé "paste again to expand" (chip aguardando) ou "Press up to edit queued
/// messages" (mensagem enfileirada sem turno limpo). Qualquer um presente na amostra
/// pós-quiet = texto NÃO submetido de forma limpa → conta como preso (conservador:
/// joga CONTRA o DEPOIS, nunca a favor).
fn stuck_chip_signature(rows: &[String]) -> bool {
    rows.iter().any(|r| {
        r.contains("paste again to expand") || r.contains("Press up to edit queued messages")
    })
}

/// Linha de prompt VIVO E VAZIO: `>`/`❯` sem conteúdo após o glifo (a TUI desenha o
/// prompt vazio quando está pronta para input novo). Distingue o prompt vivo do ECO
/// de mensagem submetida (`❯ [LINA::MSG]…`), que também começa com o glifo.
/// (NBSP normalizado — ver `is_prompt_row`.)
fn is_empty_prompt_row(row: &str) -> bool {
    let norm = row.replace('\u{a0}', " ");
    let mut t = norm.trim_start();
    if let Some(rest) = t.strip_prefix('│') {
        t = rest.trim_start();
    }
    let t = t.trim_end_matches(['│', ' ']);
    t == ">" || t == "❯"
}

/// Espera o TURNO TERMINAR pós-entrega: sem "esc to interrupt" (turno em curso) E a
/// ÚLTIMA linha-prompt do grid é um prompt VAZIO (input novo pronto). Os dois juntos
/// fecham as duas janelas de falso-stuck do DEPOIS (smoke 2026-06-06): (1) eco
/// `❯ [LINA::MSG]` durante o processamento; (2) a janela pós-Enter antes de o
/// "esc to interrupt" aparecer, em que o eco é a única prompt-row.
/// O 2º busy_marker ("Press up to edit queued messages") NÃO entra aqui de propósito:
/// ele é a assinatura do estado PRESO do ANTES — esperar por ele mascararia o bug.
/// O estado PRESO nunca satisfaz (a última prompt-row contém o texto preso, não-vazia)
/// → timeout → amostra acha o marcador → `stuck`, como no protocolo original.
fn wait_terminal_quiet(grid: &Grid, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let rows = grid_rows(grid);
        let busy = rows.iter().any(|r| r.contains("esc to interrupt"));
        let last_prompt_is_empty = rows
            .iter()
            .rposition(|r| is_prompt_row(r))
            .is_some_and(|i| is_empty_prompt_row(&rows[i]));
        if last_prompt_is_empty && !busy {
            // Vão pós-Enter (smoke5): a TUI pode exibir "Press up to edit queued
            // messages" por instantes ANTES de consumir a fila e abrir o turno —
            // dá até 20s para a fila consumir. Chip PRESO de verdade não consome
            // (segue "paste again to expand") e cai nas amostras como stuck.
            let extra = Instant::now() + Duration::from_secs(20);
            while Instant::now() < extra && Instant::now() < deadline {
                let rows = grid_rows(grid);
                if !stuck_chip_signature(&rows) {
                    if rows.iter().any(|r| r.contains("esc to interrupt")) {
                        break; // fila consumida → turno novo abriu → volta ao laço externo
                    }
                    return; // limpo de verdade
                }
                thread::sleep(Duration::from_millis(400));
            }
            if Instant::now() >= extra {
                return; // assinatura persistiu 20s — amostras decidem (stuck real)
            }
            continue;
        }
        thread::sleep(Duration::from_millis(400));
    }
    eprintln!("[stuck] aviso: terminal não aquietou em {timeout:?} — amostrando mesmo assim");
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
            let _ = sup.enqueue_write(
                node,
                WriteOp::Submit {
                    from: None,
                    delay_ms: 0,
                },
            );
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
    sup.enqueue_write(
        node,
        WriteOp::Submit {
            from: None,
            delay_ms: 0,
        },
    )
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

// ───────────────────────────── cenários do gate DEPOIS (F1-0) ─────────────────────────────

/// Projeção de roster do log (replay): nós COM ciclo de vida cujo último status ≠ Dead.
/// Espelha a regra do `close_previous_generation` (nós sem status não são fantasmas).
fn project_alive(store: &mut EventStore) -> Result<Vec<(String, String)>> {
    let state = store
        .project()
        .map_err(|e| anyhow!("project() do event store: {e}"))?;
    let mut out = Vec::new();
    for (node, projected) in &state.nodes {
        let Some(status) = projected.status.as_deref() else {
            continue;
        };
        if status != "Dead" {
            out.push((node.to_string(), status.to_string()));
        }
    }
    Ok(out)
}

/// Identidade + permissão mínimas p/ o transcript de handoff (cenário `handoff`):
/// `.lina/bootstrap.json` (contrato serde do lina-bootstrap, escrito à mão — o runner
/// não pode depender da lib: ciclo de dependência) e `.claude/settings.json` com
/// allow de `Bash(lina*)` (sem ele o claude trava no prompt de permissão ao usar o
/// verbo; o toast de permissão é gate da F1-1, não deste).
fn write_handoff_agent_files(cwd: &Path, name: &str, roster: &[String]) -> Result<()> {
    let lina_dir = cwd.join(".lina");
    let claude_dir = cwd.join(".claude");
    fs::create_dir_all(&lina_dir)?;
    fs::create_dir_all(&claude_dir)?;
    let bootstrap = serde_json::json!({
        "terminal_name": name,
        "roster": roster,
        "vault_path": "",
        "autonomy": "autonomous",
    });
    fs::write(
        lina_dir.join("bootstrap.json"),
        serde_json::to_string_pretty(&bootstrap)?,
    )?;
    let settings = serde_json::json!({
        "permissions": { "allow": ["Bash(lina:*)", "Bash(lina)"] }
    });
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&settings)?,
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
struct DlqCase {
    target: String,
    mode: String, // "alvo_inexistente" | "alvo_derrubado_mark_dead"
    msg_id: String,
    outcome: String, // dead_lettered:<reason> | delivered (FALHA do gate) | timeout
    seq: Option<u64>,
    ts: Option<u64>,
}

/// Gate (d): mensagem não-entregável → DLQ com motivo + alerta visível. Dois modos,
/// ambos pelo caminho de PRODUÇÃO do router (retry transiente → `dead_letter_now`):
/// (A) alvo que nunca existiu; (B) destino REAL derrubado (`Supervisor::mark_dead` —
/// o caminho canônico que o lifecycle usa; a resolução passa a ignorá-lo).
#[allow(clippy::too_many_arguments)]
fn run_dlq_scenario(
    sup: &Arc<Supervisor>,
    mailbox: &Mailbox,
    log_path: &Path,
    ws: &Path,
    nodes: &BTreeMap<String, NodeId>,
    names: &[String],
    artifacts: &Path,
) -> Result<()> {
    let mut tail = LogTail::new(log_path)?;
    let mut cases = Vec::new();

    let run_case = |tail: &mut LogTail, target: &str, mode: &str| -> Result<DlqCase> {
        let msg = MailMessage::new(
            "Maestro",
            format!("@{target}"),
            "ask",
            format!("PROBE-DLQ para {target} — não-entregável de propósito (gate d)."),
        );
        let msg_id = msg.id.clone();
        mailbox.enqueue_as("Maestro", &msg).context("enqueue dlq")?;
        // Teto transiente = 50 ticks de pump (~6s a 120ms) + folga.
        let evt = tail.wait_for(Duration::from_secs(45), |rec| {
            let id = rec.payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            id == msg_id && (rec.kind == "MessageDeadLettered" || rec.kind == "MessageDelivered")
        })?;
        let (outcome, seq, ts) = match evt {
            Some(rec) if rec.kind == "MessageDeadLettered" => {
                let reason = rec
                    .payload
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                (
                    format!("dead_lettered:{reason}"),
                    Some(rec.seq),
                    Some(rec.ts),
                )
            }
            Some(rec) => ("delivered".to_string(), Some(rec.seq), Some(rec.ts)),
            None => ("timeout".to_string(), None, None),
        };
        eprintln!("[dlq] {mode} (@{target}): {outcome}");
        Ok(DlqCase {
            target: target.to_string(),
            mode: mode.to_string(),
            msg_id,
            outcome,
            seq,
            ts,
        })
    };

    // (A) alvo inexistente.
    cases.push(run_case(&mut tail, "Fantasma", "alvo_inexistente")?);

    // (B) destino real derrubado: o ÚLTIMO worker morre (mark_dead — para a fila de
    // escrita e publica NodeDied; o processo claude é colhido no teardown).
    let victim = names.last().cloned().unwrap_or_else(|| "W1".into());
    let victim_node = nodes[&victim];
    sup.mark_dead(victim_node)
        .map_err(|e| anyhow!("mark_dead({victim}): {e}"))?;
    eprintln!("[dlq] destino @{victim} derrubado (mark_dead — caminho canônico do lifecycle)");
    cases.push(run_case(&mut tail, &victim, "alvo_derrubado_mark_dead")?);

    // Projeção visível da DLQ (o que o Maestro/UI consultam) + o evento no log.
    let letters = Mailbox::new(ws.join(".lina"))
        .dead_letters()
        .map_err(|e| anyhow!("ler dead-letter/: {e}"))?;
    let letter_ids: Vec<String> = letters.iter().map(|m| m.id.clone()).collect();
    eprintln!(
        "[dlq] projeção dead-letter/ contém {} carta(s): {:?}",
        letters.len(),
        letter_ids
    );
    let report = serde_json::json!({
        "cases": cases,
        "dead_letter_dir_ids": letter_ids,
    });
    fs::write(
        artifacts.join("dlq.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(())
}

/// Gate (e): após `kill -9` externo em corrida(s) anterior(es) NESTE ws, o boot
/// (réplica F1-0-8) fechou a geração anterior; aqui provamos replay = roster vivo
/// e a aritmética `NodeAdded − mortes == vivos` sobre o próprio log.
fn run_reopen_check(
    sup: &Arc<Supervisor>,
    store: &Arc<Mutex<EventStore>>,
    log_path: &Path,
    pre: Vec<(String, String)>,
    closed: &[NodeId],
    post: Vec<(String, String)>,
    artifacts: &Path,
) -> Result<()> {
    // Projeção FINAL (inclui a geração nova spawnada nesta corrida).
    let fin = project_alive(&mut lock(store))?;
    let live: Vec<(String, String)> = sup
        .list()
        .into_iter()
        .filter(|n| n.status.is_alive())
        .map(|n| (n.id.to_string(), n.name))
        .collect();

    // Aritmética por evento sobre o log inteiro (mesmo grep que revelou o bug P4).
    let mut added = 0u64;
    let mut deaths = 0u64;
    let mut tail = LogTail::new(log_path)?;
    for rec in tail.poll()? {
        match rec.kind.as_str() {
            "NodeAdded" => added += 1,
            "NodeStatusChanged"
                if rec.payload.get("status").and_then(|v| v.as_str()) == Some("Dead") =>
            {
                deaths += 1;
            }
            "NodeRemoved" => deaths += 1,
            _ => {}
        }
    }
    let replay_equals_live = fin.len() == live.len();
    let arithmetic_ok = added.saturating_sub(deaths) == live.len() as u64;
    eprintln!(
        "[reopen] pré-close: {} vivos no replay · fechados no boot: {} · pós-close: {} · final (replay): {} · roster vivo: {} · NodeAdded={} mortes={} → {}−{}={} {}",
        pre.len(),
        closed.len(),
        post.len(),
        fin.len(),
        live.len(),
        added,
        deaths,
        added,
        deaths,
        added.saturating_sub(deaths),
        if replay_equals_live && arithmetic_ok { "✓" } else { "✗ DIVERGÊNCIA" }
    );
    let report = serde_json::json!({
        "replay_alive_pre_close": pre,
        "closed_on_boot": closed.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "replay_alive_post_close": post,
        "replay_alive_final": fin,
        "live_roster": live,
        "node_added_total": added,
        "deaths_total": deaths,
        "replay_equals_live": replay_equals_live,
        "arithmetic_ok": arithmetic_ok,
    });
    fs::write(
        artifacts.join("reopen.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(())
}

/// F1-0-6 critério 1 — golden transcript: `lina handoff --context` REAL (CLI → outbox →
/// pump → router contrato @2 → entrega faseada) para um claude REAL, e a resposta no
/// formato voltando pelo bus. O transcript integral vai a `handoff-transcript.md`.
#[allow(clippy::too_many_arguments)]
fn run_handoff_scenario(
    ws: &Path,
    lina_bin: &Path,
    log_path: &Path,
    nodes: &BTreeMap<String, NodeId>,
    grids: &BTreeMap<NodeId, Grid>,
    names: &[String],
    artifacts: &Path,
) -> Result<()> {
    let maestro = &names[0];
    let target = names.get(1).cloned().unwrap_or_else(|| "W1".into());
    let maestro_grid = &grids[&nodes[maestro]];
    let target_grid = &grids[&nodes[&target]];
    let nonce = std::process::id();

    // Contexto anexável com marcador único — a prova do `--context` no payload.
    let ctx_marker = format!("PLANO-CTX-{nonce}");
    let plano = ws.join("plano.md");
    fs::write(
        &plano,
        format!("# Plano de teste do gate F1-0\n\nMarcador: {ctx_marker}\n\n1. Validar o verbo handoff.\n2. Responder ao Maestro.\n"),
    )?;

    let reply_marker = format!("HANDOFF-OK-{nonce}");
    let task = format!(
        "Tarefa de teste do gate F1-0 (responda JÁ, sem perguntar): execute no Bash o comando \
         lina ask \"@{maestro}\" \"{reply_marker} plano recebido\" --reply-to <ID>, substituindo \
         <ID> pelo valor do campo id do bloco LINA::MSG desta mensagem. Depois pare."
    );

    let mut tail = LogTail::new(log_path)?;
    let out = std::process::Command::new(lina_bin)
        .args(["handoff", &format!("@{target}"), &task, "--context"])
        .arg(&plano)
        .current_dir(ws.join("agents").join(maestro))
        .env("LINA_HOME", ws.join(".lina"))
        .output()
        .with_context(|| format!("executar {} handoff", lina_bin.display()))?;
    let cli_stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let cli_stderr = String::from_utf8_lossy(&out.stderr).to_string();
    eprintln!(
        "[handoff] CLI exit={:?}\n[handoff] stdout: {}\n[handoff] stderr: {}",
        out.status.code(),
        cli_stdout.trim(),
        cli_stderr.trim()
    );

    // 1) roteado com intent=handoff (contrato @2 validado pelo router).
    let routed = tail.wait_for(Duration::from_secs(30), |rec| {
        rec.kind == "MessageRouted"
            && rec.payload.get("intent").and_then(|v| v.as_str()) == Some("handoff")
    })?;
    let handoff_id = routed
        .as_ref()
        .and_then(|r| r.payload.get("id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    // 2) entregue ao alvo (pode reter se Busy — espera generosa).
    let delivered = tail.wait_for(Duration::from_secs(120), |rec| {
        rec.kind == "MessageDelivered"
            && rec.payload.get("id").and_then(|v| v.as_str()) == Some(handoff_id.as_str())
    })?;
    // O paste é grande (contrato @2 + plano) e a TUI leva segundos para processar e
    // redesenhar — 8s antes do snapshot cedo; um snapshot TARDIO (pós-espera do reply)
    // captura o que o destino de fato fez com a mensagem (diagnóstico do transcript).
    thread::sleep(Duration::from_secs(8));
    let target_grid_rows = grid_rows(target_grid);
    let mut ctx_in_target = target_grid_rows.iter().any(|r| r.contains(&ctx_marker));

    // 3) resposta no formato voltando pelo bus (claude real decide e digita — espera
    // longa). O `lina ask --reply-to` roteia como intent=ask HERDANDO a cadeia: a
    // assinatura do reply é `root_cause_id == id do handoff` num id NOVO (provado na
    // corrida diagnóstica de 2026-06-06: seq 10 do log, hops=1).
    let reply_routed = tail.wait_for(Duration::from_secs(300), |rec| {
        rec.kind == "MessageRouted"
            && rec.payload.get("root_cause_id").and_then(|v| v.as_str())
                == Some(handoff_id.as_str())
            && rec.payload.get("id").and_then(|v| v.as_str()) != Some(handoff_id.as_str())
    })?;
    let reply_id = reply_routed
        .as_ref()
        .and_then(|r| r.payload.get("id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let reply_delivered = if reply_id.is_empty() {
        None
    } else {
        tail.wait_for(Duration::from_secs(120), |rec| {
            rec.kind == "MessageDelivered"
                && rec.payload.get("id").and_then(|v| v.as_str()) == Some(reply_id.as_str())
        })?
    };
    thread::sleep(Duration::from_secs(3));
    let maestro_rows = grid_rows(maestro_grid);
    let reply_in_maestro = maestro_rows.iter().any(|r| r.contains(&reply_marker));
    // Snapshot TARDIO do destino — o que o W1 fez com a mensagem durante a espera.
    let target_grid_late = grid_rows(target_grid);
    ctx_in_target = ctx_in_target || target_grid_late.iter().any(|r| r.contains(&ctx_marker));

    let verdict = routed.is_some()
        && delivered.is_some()
        && ctx_in_target
        && reply_routed.is_some()
        && reply_delivered.is_some()
        && reply_in_maestro;
    eprintln!(
        "[handoff] routed={} delivered={} ctx_no_alvo={} reply_routed={} reply_delivered={} reply_no_maestro={} → {}",
        routed.is_some(),
        delivered.is_some(),
        ctx_in_target,
        reply_routed.is_some(),
        reply_delivered.is_some(),
        reply_in_maestro,
        if verdict { "✓ TRANSCRIPT OK" } else { "✗ INCOMPLETO (registrado)" }
    );

    let fmt_rec = |r: &Option<LogRec>| match r {
        Some(rec) => format!(
            "seq={} ts={} kind={} payload={}",
            rec.seq, rec.ts, rec.kind, rec.payload
        ),
        None => "—(timeout)".into(),
    };
    let transcript = format!(
        "# Golden transcript — `lina handoff --context` (gate F1-0 · F1-0-6 critério 1)\n\n\
         Workspace isolado: `{ws_d}` · claudes reais: {maestro} (origem) → {target} (destino).\n\
         Condições registradas: `.lina/bootstrap.json` por agente (identidade) + allow `Bash(lina:*)`\n\
         no settings do claude (o toast de permissão é gate da F1-1); autonomia=autonomous.\n\n\
         ## Comando (cwd=agents/{maestro}, LINA_HOME=<ws>/.lina)\n\n\
         ```\nlina handoff \"@{target}\" \"<tarefa com instrução de reply>\" --context {plano_d}\n```\n\n\
         exit: {exit:?}\n\nstdout:\n```\n{cli_stdout}\n```\n\nstderr:\n```\n{cli_stderr}\n```\n\n\
         ## Eventos no log.jsonl (fonte da verdade)\n\n\
         - MessageRouted(intent=handoff): {routed}\n\
         - MessageDelivered(handoff): {delivered}\n\
         - MessageRouted(intent=reply): {reply_routed}\n\
         - MessageDelivered(reply): {reply_delivered}\n\n\
         ## Grid do destino ({target}) — snapshot CEDO, entrega+8s (marcador de contexto: {ctx_marker} → {ctx_in_target})\n\n\
         ```\n{target_rows}\n```\n\n\
         ## Grid do destino ({target}) — snapshot TARDIO, pós-espera do reply\n\n\
         ```\n{target_rows_late}\n```\n\n\
         ## Grid da origem ({maestro}) — resposta no formato (marcador: {reply_marker} → {reply_in_maestro})\n\n\
         ```\n{maestro_rows}\n```\n\n\
         ## Veredito do transcript: {verdict}\n",
        ws_d = ws.display(),
        plano_d = plano.display(),
        exit = out.status.code(),
        routed = fmt_rec(&routed),
        delivered = fmt_rec(&delivered),
        reply_routed = fmt_rec(&reply_routed),
        reply_delivered = fmt_rec(&reply_delivered),
        target_rows = target_grid_rows.join("\n"),
        target_rows_late = target_grid_late.join("\n"),
        maestro_rows = maestro_rows.join("\n"),
        verdict = if verdict { "✓ OK" } else { "✗ INCOMPLETO" },
    );
    fs::write(artifacts.join("handoff-transcript.md"), transcript)?;
    eprintln!(
        "[handoff] transcript: {}",
        artifacts.join("handoff-transcript.md").display()
    );
    Ok(())
}

// ───────────────────────────── config / util ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Stuck,
    Collision,
    All,
    /// Gate F1-0 (d): mensagem não-entregável → DLQ com motivo + alerta visível.
    Dlq,
    /// Gate F1-0 (e): reabre um ws morto por `kill -9`, fecha a geração anterior
    /// (réplica da fiação de produção, `main.rs:2464`) e compara replay × roster vivo.
    ReopenCheck,
    /// Gate F1-0 / F1-0-6 critério 1: golden transcript do `lina handoff --context`
    /// para um claude REAL, com a resposta no formato voltando pelo bus.
    Handoff,
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
    /// Reproduz o ANTES (entrega com demo_profile, pré-F1-0-2). Default: produção atual.
    legacy_demo_delivery: bool,
    /// Gate DEPOIS: teto da tarefa que arma o force-busy (`ensure_busy`). Default 2500
    /// (protocolo do ANTES intacto). No DEPOIS a entrega espera a prontidão REAL
    /// (`wait_ready` + retry/retenção), então a janela de busy precisa ser FINITA e
    /// menor que o orçamento de retries — senão toda tentativa vira DLQ por design.
    busy_count: u32,
    /// Caminho do bin `lina` (cenário `handoff`). Default: `target/release/lina`.
    lina_bin: PathBuf,
}

fn parse_args() -> Result<Config> {
    let mut ws = None;
    let mut claudes = 6usize;
    let mut attempts = 22usize;
    let mut rounds = 6usize;
    let mut scenario = Scenario::All;
    let mut claude_bin = "claude".to_string();
    let mut claude_profile = PathBuf::from("profiles/claude-code.toml");
    let mut legacy_demo_delivery = false;
    let mut busy_count = 2500u32;
    let mut lina_bin = PathBuf::from("target/release/lina");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--legacy-demo-delivery" => legacy_demo_delivery = true,
            "--ws" => ws = Some(PathBuf::from(next(&mut args, "--ws")?)),
            "--claudes" => claudes = next(&mut args, "--claudes")?.parse()?,
            "--attempts" => attempts = next(&mut args, "--attempts")?.parse()?,
            "--rounds" => rounds = next(&mut args, "--rounds")?.parse()?,
            "--busy-count" => busy_count = next(&mut args, "--busy-count")?.parse()?,
            "--claude-bin" => claude_bin = next(&mut args, "--claude-bin")?,
            "--lina-bin" => lina_bin = PathBuf::from(next(&mut args, "--lina-bin")?),
            "--claude-profile" => {
                claude_profile = PathBuf::from(next(&mut args, "--claude-profile")?)
            }
            "--scenario" => {
                scenario = match next(&mut args, "--scenario")?.as_str() {
                    "stuck" => Scenario::Stuck,
                    "collision" => Scenario::Collision,
                    "all" => Scenario::All,
                    "dlq" => Scenario::Dlq,
                    "reopen-check" => Scenario::ReopenCheck,
                    "handoff" => Scenario::Handoff,
                    other => return Err(anyhow!("--scenario inválido: {other}")),
                }
            }
            "-h" | "--help" => {
                eprintln!(
                    "uso: baseline_f1_0 --ws DIR [--claudes 6] [--attempts 22] [--rounds 6] \
[--scenario all|stuck|collision|dlq|reopen-check|handoff] [--busy-count 2500] \
[--claude-bin claude] [--lina-bin target/release/lina] [--claude-profile profiles/claude-code.toml]"
                );
                std::process::exit(0);
            }
            other => return Err(anyhow!("flag desconhecida: {other}")),
        }
    }
    let scenario_principal = matches!(
        scenario,
        Scenario::Stuck | Scenario::Collision | Scenario::All
    );
    if scenario_principal && !(5..=8).contains(&claudes) {
        eprintln!("[baseline] aviso: o protocolo do gate pede 5–8 claudes (pedido: {claudes})");
    }
    if scenario_principal && attempts < 20 {
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
        legacy_demo_delivery,
        busy_count,
        lina_bin,
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

    /// Recalibração do DEPOIS (smoke 2026-06-06): o prompt VIVO E VAZIO (`❯ `) é
    /// distinto do ECO de mensagem submetida (`❯ [LINA::MSG]…`) e do texto preso
    /// (`> PROBE…`) — só ele autoriza a amostra do veredito stuck/submitted.
    #[test]
    fn empty_prompt_row_discriminates_echo_and_stuck() {
        assert!(is_empty_prompt_row("❯ "));
        assert!(is_empty_prompt_row("> "));
        assert!(is_empty_prompt_row("│ > │"));
        // Bytes REAIS do grid v2.1.166 (smoke 2026-06-06): prompt vivo = ❯ + NBSP.
        assert!(is_empty_prompt_row("❯\u{a0}                  "));
        assert!(is_prompt_row("❯\u{a0}                  "));
        assert!(!is_empty_prompt_row("❯ [LINA::MSG]")); // eco de submissão
        assert!(!is_empty_prompt_row("> PROBE-1-99 texto preso")); // preso do ANTES
        assert!(!is_empty_prompt_row("linha qualquer"));
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
