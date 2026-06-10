//! GATE DE SAÍDA — Onda 2 (walking skeleton ROBUSTO), num fluxo único e HEADLESS (sem gpui).
//!
//! Prova, ponta a ponta e determinístico, o portão da W2 (épico `32 - Epico Fase 0`,
//! story W2-5). Reusa o substrato REAL da Onda 0 (nada de mock) e exercita o caminho
//! que a casca gpui percorreria — mas headless, para rodar em CI nos 3 SOs:
//!
//! 1. **2 nós-terminal REAIS** — dois PTYs com shell de verdade (`sh`/`cat`),
//!    registrados como nós no Workspace Bus (Supervisor).
//! 2. **A2A A→B FASEADA** — bracketed-paste sanitizado (sem `ESC[201~`) → `submit_delay`
//!    → Enter `0x0D` **separado**; o Enter de fato SUBMETE em B e **B responde** com a
//!    saída do seu próprio programa (`BREPLY_OK`), não um eco morto.
//! 3. **O pulso A→B é um EVENTO REAL do pub/sub in-process** — `BusEvent::Message`
//!    capturado AO VIVO do `broadcast` do Supervisor (mesmo `id` do envelope). Não é
//!    um mock: o evento trafega pelo barramento e a "casca" headless o traduz p/ o `UiHost`.
//! 4. **Fim-de-resposta** — o turno de B fecha por `idle (damage estável + resposta
//!    assentada) > timeout`, pelo contrato do `EndDetector` (relógio lógico). Sem
//!    truncamento silencioso (`truncated == false`).
//! 5. **Sobrevive a `kill -9` MEDIDO** — `SIGKILL` no PID REAL do filho de B; a morte
//!    é MEDIDA (o master vê EOF na hora; o PID some da tabela após o reap). O estado é
//!    então reconstruído do **event log (JSONL)** e o fingerprint bate (como o gate_onda0).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lina_core::{
    deliver_a2a, A2aEnvelope, AlacrittyBackend, BusEvent, BusTarget, CliProfile, DeliveryOutcome,
    DomainEvent, EndDetector, EndOutcome, EventStore, HeadlessUiHost, HostEvent, InjectPolicy,
    NodeId, NodeKind, PtyCommand, PtyManager, Recipient, RolePolicy, Supervisor, UiHost, VtBackend,
    WriteOp,
};

/// Marcador do payload A2A (o texto que A injeta em B).
const MARKER: &str = "LINA_GATE_OK";
/// Resposta que o PROGRAMA de B emite ao receber uma linha submetida (não um eco).
const BREPLY: &str = "BREPLY_OK";
/// Início/fim de uma colagem bracketed-paste (xterm) — para inspecionar o payload.
const PASTE_BEGIN: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
/// Timeout generoso p/ toda espera-por-condição — robusto a contenção de CPU em CI.
const T: Duration = Duration::from_secs(5);

// ───────────────────────────── helpers ─────────────────────────────

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Espera-POR-CONDIÇÃO com timeout (poll) — nunca um sleep fixo de asserção.
fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// `kill <signal> <pid>` via o binário do SO. `-0` testa existência (medição), `-9` é
/// o SIGKILL. Devolve `true` se o `kill` saiu com sucesso (processo existia / foi morto).
fn run_kill(signal: &str, pid: u32) -> bool {
    Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        // Só o exit status importa; silencia o "No such process" do kill no ESRCH.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `true` se o processo `pid` ainda existe (e podemos sinalizá-lo) — `kill -0`.
fn process_alive(pid: u32) -> bool {
    run_kill("-0", pid)
}

/// Diretório temporário único; removido no `Drop`.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lina-gatew2-{}", uuid_like()));
        std::fs::create_dir_all(&p).expect("criar tempdir");
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Id único sem depender de `uuid` no test (nanos + endereço de pilha).
fn uuid_like() -> u128 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let salt = &now as *const _ as u128;
    now ^ salt
}

/// Um nó cabeado: o grid alimentado pela thread de leitura do PTY real + o controle
/// dessa thread (o "core headless" mínimo: PTY → VtBackend, como o pty-host faz).
struct Wired {
    grid: Arc<Mutex<Box<dyn VtBackend>>>,
    stop: Arc<AtomicBool>,
    /// Vira `true` quando o master vê EOF/erro — i.e., o processo do PTY morreu.
    eof: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pty_key: String,
    /// PID REAL do filho do PTY (para o `kill -9` medido).
    pid: u32,
}

/// Texto da tela inteira (sem ANSI), lendo o grid PARSEADO — não OCR de pixel.
fn screen_text(grid: &Arc<Mutex<Box<dyn VtBackend>>>) -> String {
    let g = lock(grid);
    let (_cols, rows) = g.dims();
    (0..rows)
        .map(|r| g.row_text(r))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Abre um PTY REAL, registra o writer do master na fila serial do Bus (o nó) e sobe a
/// thread de leitura que aplica a saída ao `VtBackend` (grid). Captura o PID e um flag
/// de EOF. Devolve o `NodeId` do Bus e o cabeamento.
fn wire_node(
    pty: &mut PtyManager,
    sup: &Supervisor,
    pty_key: &str,
    name: &str,
    role: Option<String>,
    cmd: PtyCommand,
) -> (NodeId, Wired) {
    let pid = pty.spawn(pty_key, cmd, 80, 24).expect("spawn PTY real");
    let writer = pty.take_writer(pty_key).expect("take_writer (1 dono)");
    let mut reader = pty.clone_reader(pty_key).expect("clone_reader");

    let grid: Arc<Mutex<Box<dyn VtBackend>>> =
        Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(80, 24))));
    let stop = Arc::new(AtomicBool::new(false));
    let eof = Arc::new(AtomicBool::new(false));

    let join = {
        let grid = Arc::clone(&grid);
        let stop = Arc::clone(&stop);
        let eof = Arc::clone(&eof);
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        // master fechou: o processo do PTY morreu (EOF) — medição da morte.
                        eof.store(true, Ordering::Relaxed);
                        break;
                    }
                    Ok(n) => lock(&grid).advance(&buf[..n]),
                }
            }
        })
    };

    let node = sup.register(name, role, writer);
    (
        node,
        Wired {
            grid,
            stop,
            eof,
            join: Some(join),
            pty_key: pty_key.to_string(),
            pid,
        },
    )
}

/// Encerra o nó cabeado: para a thread de leitura, mata o PTY (SIGTERM→SIGKILL, reapa
/// o filho) e dá join.
fn teardown(pty: &mut PtyManager, mut wired: Wired) {
    wired.stop.store(true, Ordering::Relaxed);
    let _ = pty.kill(wired.pty_key.as_str(), Duration::from_secs(2));
    if let Some(j) = wired.join.take() {
        let _ = j.join();
    }
}

/// "Shell" headless: traduz eventos REAIS do Bus (`subscribe`) em `HostEvent`s para o
/// `UiHost` — demonstra o contrato core→UI sobre o pub/sub real.
fn bus_to_host(ev: &BusEvent) -> Option<HostEvent> {
    match ev {
        BusEvent::NodeSpawned { node } => Some(HostEvent::NodeAdded {
            node: *node,
            kind: NodeKind::Terminal,
        }),
        BusEvent::NodeDied { node } => Some(HostEvent::NodeRemoved { node: *node }),
        BusEvent::Message { from, to, .. } => Some(HostEvent::BusMessage {
            from: *from,
            to: match to {
                Recipient::Node(n) => BusTarget::Node(*n),
                Recipient::Role(r) => BusTarget::Role(r.clone()),
                Recipient::Broadcast => BusTarget::Broadcast,
            },
        }),
        BusEvent::NodeStatus { .. } => None,
    }
}

fn fake_profile() -> CliProfile {
    // pty_inject; prompt do alvo = "GATEPROMPT"; Enter separado após 10ms; fim por idle.
    let src = r#"
        id = "gate-fake"
        program = "fake"
        delivery = "pty_inject"
        submit_delay_ms = 10
        prompt_ready_regex = 'GATEPROMPT'
        idle_ms = 100
        ask_timeout_ms = 120000
        [end_signal]
        kind = "idle"
    "#;
    CliProfile::from_toml_str(src, "<gate-w2>").expect("profile do gate deve parsear")
}

/// Sobrescreve o meio do `.db` com lixo (lição Codex #21750) — corrupção localizada
/// que `integrity_check` pode não pegar mas a varredura da tabela pega.
fn corrupt_middle(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};
    let len = std::fs::metadata(path).expect("metadata").len();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("abrir p/ corromper");
    let start = len / 4;
    let span = (len / 2).max(1);
    f.seek(SeekFrom::Start(start)).expect("seek");
    f.write_all(&vec![0xEE_u8; span as usize])
        .expect("escrever lixo");
    f.flush().expect("flush");
}

// ───────────────────────────────── o gate ─────────────────────────────────

#[test]
fn onda2_exit_gate_end_to_end() {
    // Casca headless + Supervisor; assina o pub/sub REAL ANTES de qualquer evento.
    let mut ui = HeadlessUiHost::new();
    let sup = Supervisor::new();
    let mut bus_rx = sup.subscribe();

    let tmp = TempDir::new();
    let mut store = EventStore::open(tmp.path()).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: "Gate Onda 2".into(),
            focus_preset: String::new(),
        })
        .expect("append WorkspaceCreated");

    // (1) 2 PTYs REAIS registrados como nós do Bus.
    //   A = remetente (cat, fica vivo lendo stdin).
    //   B = alvo REAL: imprime o prompt, entra em bracketed-paste e roda um laço que,
    //       a CADA linha submetida, emite a SUA resposta (BREPLY_OK) — não um eco morto.
    let mut pty = PtyManager::new();
    let (node_a, wired_a) = wire_node(&mut pty, &sup, "pty-A", "@A", None, PtyCommand::new("cat"));
    let (node_b, wired_b) = wire_node(
        &mut pty,
        &sup,
        "pty-B",
        "@B",
        Some("dev".into()),
        PtyCommand::new("sh").arg("-c").arg(
            "printf 'GATEPROMPT '; printf '\\033[?2004h'; \
             while IFS= read -r _; do printf 'BREPLY_OK\\n'; done",
        ),
    );

    // Persiste a topologia no event log (a fonte da verdade).
    for (node, x, cli) in [(node_a, 0.0_f64, "cat"), (node_b, 300.0, "sh")] {
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x,
                y: 50.0,
                requested_by: None,
            })
            .expect("append NodeAdded");
        store
            .append(&DomainEvent::TerminalSpawned {
                node,
                cli: cli.into(),
                cwd: None,
            })
            .expect("append TerminalSpawned");
    }

    // B sobe com o prompt e em bracketed-paste mode (pré-condição do A2A faseado).
    assert!(
        poll_until(T, || lock(&wired_b.grid).last_nonempty_line()
            == "GATEPROMPT"),
        "B deveria mostrar o prompt 'GATEPROMPT' no grid"
    );
    // B emite `GATEPROMPT` e a sequência `ESC[?2004h` em printfs SEPARADOS; o prompt pode chegar ao
    // grid um flush ANTES de o modo ser parseado. Espera-POR-CONDIÇÃO (não assert imediato) elimina
    // a corrida — mesmo padrão do poll do prompt acima.
    assert!(
        poll_until(T, || lock(&wired_b.grid).mode().bracketed_paste),
        "B deve estar em bracketed-paste mode"
    );

    // (3-pré) O pulso A→B passa pelo ROUTER do Bus, que publica `BusEvent::Message`.
    let env = A2aEnvelope::new(node_a, Recipient::Node(node_b), Some("ask".into()));
    let targets = sup.route(&env, RolePolicy::FirstIdle);
    assert_eq!(targets, vec![node_b], "route deve resolver para B");
    store
        .append(&DomainEvent::BusMessageSent {
            id: env.id.clone(),
            from: node_a,
            to: format!("node:{node_b}"),
        })
        .expect("append BusMessageSent");

    // (2) A2A A→B FASEADA. O payload carrega um vetor de injeção `ESC[201~` para provar
    //     a sanitização. B está em bracketed mode → wrapper + payload limpo, sem `\r`.
    let deliver_text = format!("{MARKER}\x1b[201~ tail");
    let out = deliver_a2a(
        &sup,
        node_b,
        node_a,
        &deliver_text,
        &fake_profile(),
        &wired_b.grid,
        InjectPolicy::AllowAll,
    )
    .expect("deliver_a2a");
    assert_eq!(
        out,
        DeliveryOutcome::Injected {
            ready: true,
            bracketed: true
        }
    );

    // (2a) Sequência FASEADA correta na fila serial: AgentText(bracketed, sanitizado,
    //      sem CR) e DEPOIS Submit(0x0D) separado — nunca texto+\r monolítico.
    assert!(poll_until(T, || sup.applied_ops(node_b).len() == 2));
    let ops = sup.applied_ops(node_b);
    match (&ops[0], &ops[1]) {
        (WriteOp::AgentText { from, bytes }, WriteOp::Submit { from: Some(s) }) => {
            assert_eq!((from, s), (&node_a, &node_a));
            assert!(
                bytes.starts_with(PASTE_BEGIN) && bytes.ends_with(PASTE_END),
                "deve estar embrulhado em bracketed-paste"
            );
            assert!(!bytes.contains(&b'\r'), "o texto colado nunca carrega CR");
            // O `ESC[201~` malicioso do payload foi sanitizado: sobra só o do wrapper.
            let n_end = bytes
                .windows(PASTE_END.len())
                .filter(|w| *w == PASTE_END)
                .count();
            assert_eq!(
                n_end, 1,
                "só o terminador do wrapper deve sobrar (sanitizado)"
            );
        }
        other => panic!("sequência faseada inesperada: {other:?}"),
    }

    // (2b) O Enter de fato SUBMETE e B RESPONDE: o texto chega ao grid de B (round-trip
    //      via PTY real) E o programa de B emite a SUA própria resposta `BREPLY_OK`.
    assert!(
        poll_until(T, || {
            let s = screen_text(&wired_b.grid);
            s.contains(MARKER) && s.contains(BREPLY)
        }),
        "B deveria receber o texto submetido e responder com {BREPLY}; tela = {:?}",
        screen_text(&wired_b.grid)
    );

    // (4) Fim-de-resposta: o turno fecha por IDLE (damage estável + resposta assentada),
    //     pelo contrato `result > idle > timeout` (relógio lógico). Sem truncar.
    let mut det = EndDetector::new(&fake_profile(), 0);
    let mut now_ms = 0_u64;
    let mut closed = None;
    for _ in 0..200 {
        let (changed, ready) = {
            let mut g = lock(&wired_b.grid);
            let changed = !g.damaged_rows().is_empty();
            g.reset_damage();
            let (_c, rows) = g.dims();
            let text = (0..rows)
                .map(|r| g.row_text(r))
                .collect::<Vec<_>>()
                .join("\n");
            (changed, text.contains(BREPLY)) // resposta de B assentada
        };
        if let Some(r) = det.on_tick(now_ms, changed, ready) {
            closed = Some(r);
            break;
        }
        now_ms += 60; // relógio lógico: > idle_ms (100) em ~2 ticks quietos
        thread::sleep(Duration::from_millis(10));
    }
    let closed = closed.expect("o contrato de fim-de-resposta deveria fechar o turno");
    assert_eq!(closed.outcome, EndOutcome::IdleSettled);
    assert!(
        !closed.truncated,
        "fechou por idle, não por timeout — sem truncamento silencioso"
    );

    // (3) O pulso A→B foi um EVENTO REAL do pub/sub: drena o `broadcast` e prova que a
    //     `BusEvent::Message` trafegou pelo barramento (mesmo `id` do envelope, from=A).
    let mut bus_events = Vec::new();
    while let Ok(ev) = bus_rx.try_recv() {
        bus_events.push(ev);
    }
    assert!(
        bus_events.iter().any(|e| matches!(
            e,
            BusEvent::Message { id, from, to: Recipient::Node(n) }
                if *from == node_a && *n == node_b && *id == env.id
        )),
        "o pulso A→B deveria ter trafegado pelo bus como BusEvent::Message (id={}); veio {:?}",
        env.id,
        bus_events
    );

    // A casca headless traduz os eventos REAIS do bus para o UiHost (core→UI sobre pub/sub).
    for ev in &bus_events {
        if let Some(host_ev) = bus_to_host(ev) {
            ui.on_event(host_ev);
        }
    }
    assert!(
        ui.events()
            .iter()
            .filter(|e| matches!(e, HostEvent::NodeAdded { .. }))
            .count()
            >= 2,
        "a UI deveria ter recebido os 2 NodeAdded do bus real"
    );
    assert!(
        ui.events()
            .iter()
            .any(|e| matches!(e, HostEvent::BusMessage { .. })),
        "a UI deveria ter recebido o BusMessage A→B do bus real"
    );

    // (5) `kill -9` MEDIDO no PID REAL de B (crash abrupto do processo).
    let pid_b = wired_b.pid;
    assert!(process_alive(pid_b), "B deve estar vivo antes do SIGKILL");
    assert!(run_kill("-9", pid_b), "o `kill -9` deve ter sucesso");
    // Medição da morte (sem depender de reap, à prova de zumbi): o master vê EOF.
    assert!(
        poll_until(T, || wired_b.eof.load(Ordering::Relaxed)),
        "o PTY de B deveria ver EOF após o SIGKILL (morte medida)"
    );

    // A morte é registrada no event log; capturamos o fingerprint pré-crash (+ snapshot).
    store
        .append(&DomainEvent::TerminalExited { node: node_b })
        .expect("append TerminalExited");
    store.take_snapshot().expect("snapshot");
    let pre_fp = store.project().expect("project").fingerprint();

    // CRASH: perde o estado in-memory (mata PTYs/reapa, dropa supervisor + store) e
    // corrompe o `.db` — forçando a reconstrução a vir 100% do event log (JSONL).
    teardown(&mut pty, wired_a);
    teardown(&mut pty, wired_b);
    drop(pty); // reapa quaisquer zumbis
    drop(sup);
    drop(store); // fecha a conexão SQLite (auto-checkpoint do WAL no último close)

    // Confirmação extra da morte: após o reap, o PID de B sumiu da tabela (ESRCH).
    assert!(
        poll_until(T, || !process_alive(pid_b)),
        "após o reap, o PID de B não deve mais existir (SIGKILL medido)"
    );

    let db = tmp.path().join("lina.db");
    corrupt_middle(&db);
    for ext in ["-wal", "-shm"] {
        let mut p = db.as_os_str().to_os_string();
        p.push(ext);
        let _ = std::fs::remove_file(std::path::PathBuf::from(p));
    }

    // RECOVERY VISÍVEL: preserva o corrompido, reconstrói do JSONL, emite Recovering→Recovered.
    let store2 = EventStore::open_or_recover(tmp.path(), &mut ui).expect("open_or_recover");

    // o corrompido foi PRESERVADO (não apagado).
    let corrupt = std::fs::read_dir(tmp.path())
        .expect("readdir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
        .count();
    assert_eq!(
        corrupt, 1,
        "o .db corrompido deve ser preservado como *.corrupt-*"
    );

    // estado reconstruído do event log IDÊNTICO ao pré-crash (zero perda lógica).
    let recovered = store2.project().expect("project pós-recovery");
    assert_eq!(
        recovered.fingerprint(),
        pre_fp,
        "a reconstrução do event log deve ser idêntica ao estado pré-crash"
    );
    // e a morte de B (kill -9) está refletida no estado recuperado.
    assert_eq!(
        recovered
            .nodes
            .get(&node_b)
            .and_then(|n| n.status.as_deref()),
        Some("Dead"),
        "o estado recuperado deve mostrar B como Dead (o SIGKILL foi persistido)"
    );

    // recuperação VISÍVEL: a UI terminou com o par Recovering → Recovered.
    let evs = ui.events();
    assert!(
        matches!(
            &evs[evs.len() - 2..],
            [HostEvent::Recovering, HostEvent::Recovered]
        ),
        "a UI deveria terminar com Recovering → Recovered; veio {:?}",
        &evs[evs.len().saturating_sub(3)..]
    );
}
