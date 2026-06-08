//! GATE DE SAÍDA — Onda 0 (core framework-agnóstico), num fluxo único e headless.
//!
//! Prova, ponta a ponta e determinístico, o portão da W0 (ver `32 - Epico Fase 0`):
//! 1. **Core headless** com `HeadlessUiHost` recebendo eventos.
//! 2. **2 PTYs REAIS** abertos (`lina-pty`) e registrados como nós no **Workspace Bus**.
//! 3. **A2A FASEADA** de A→B (bracketed-paste sanitizado → `submit_delay` → Enter `0x0D`
//!    separado, fila SERIAL): o texto **chega ao grid de B** (`lina-vt`) via o PTY real.
//! 4. **Fim-de-resposta** fecha o turno pelo contrato (idle do grid).
//! 5. **Eventos persistidos** no Event Store (SQLite WAL + JSONL).
//! 6. **Crash**: perde-se o estado in-memory (mata PTYs, dropa supervisor/store) e o
//!    `.db` é corrompido; `open_or_recover` PRESERVA o corrompido, **reconstrói do
//!    JSONL** (estado IDÊNTICO) e emite `Recovering`→`Recovered`.

use std::io::Read;
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

const MARKER: &str = "LINA_GATE_OK";

// ───────────────────────────── helpers ─────────────────────────────

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

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

/// Diretório temporário único; removido no `Drop`.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("lina-gate-{}", uuid_like()));
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

/// Id único sem depender de `uuid` no test (usa nanos + endereço de pilha).
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
    join: Option<JoinHandle<()>>,
    pty_key: String,
}

impl Wired {
    fn last_line(&self) -> String {
        lock(&self.grid).last_nonempty_line()
    }
}

/// Abre um PTY REAL, liga o **writer do master à fila serial do Bus** (o nó) e sobe a
/// **thread de leitura** que aplica a saída ao `VtBackend` (grid). Devolve o `NodeId`
/// do Bus e o cabeamento.
fn wire_node(
    pty: &mut PtyManager,
    sup: &Supervisor,
    pty_key: &str,
    name: &str,
    role: Option<String>,
    cmd: PtyCommand,
) -> (NodeId, Wired) {
    pty.spawn(pty_key, cmd, 80, 24).expect("spawn PTY real");
    let writer = pty.take_writer(pty_key).expect("take_writer (1 dono)");
    let mut reader = pty.clone_reader(pty_key).expect("clone_reader");

    let grid: Arc<Mutex<Box<dyn VtBackend>>> =
        Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(80, 24))));
    let stop = Arc::new(AtomicBool::new(false));

    let join = {
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
        })
    };

    // O writer do master vira o sink da MailQueue serial do nó (humano/agente nunca
    // interleavam bytes no master).
    let node = sup.register(name, role, writer);
    (
        node,
        Wired {
            grid,
            stop,
            join: Some(join),
            pty_key: pty_key.to_string(),
        },
    )
}

/// Encerra o nó cabeado: para a thread de leitura, mata o PTY (SIGTERM→SIGKILL) e dá join.
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
        // status: enum de status do Bus ≠ do host; fora do escopo do gate.
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
    CliProfile::from_toml_str(src, "<gate>").expect("profile do gate deve parsear")
}

/// Sobrescreve o meio do `.db` com lixo (lição Codex #21750).
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
fn onda0_exit_gate_end_to_end() {
    // (1) Core headless + UI recebendo eventos; assina o pub/sub real ANTES de tudo.
    let mut ui = HeadlessUiHost::new();
    let sup = Supervisor::new();
    let mut bus_rx = sup.subscribe();

    let tmp = TempDir::new();
    let mut store = EventStore::open(tmp.path()).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: "Gate Onda 0".into(),
            focus_preset: String::new(),
        })
        .expect("append");

    // (2) 2 PTYs REAIS registrados como nós do Bus.
    let mut pty = PtyManager::new();
    // A = remetente (cat simples). B = alvo: habilita bracketed-paste + imprime prompt + cat.
    let (node_a, wired_a) = wire_node(&mut pty, &sup, "pty-A", "@A", None, PtyCommand::new("cat"));
    let (node_b, wired_b) = wire_node(
        &mut pty,
        &sup,
        "pty-B",
        "@B",
        Some("dev".into()),
        PtyCommand::new("sh")
            .arg("-c")
            .arg("printf 'GATEPROMPT '; printf '\\033[?2004h'; exec cat"),
    );
    for (node, x) in [(node_a, 0.0_f64), (node_b, 300.0)] {
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x,
                y: 50.0,
            })
            .expect("append NodeAdded");
        store
            .append(&DomainEvent::TerminalSpawned {
                node,
                cli: "cat".into(),
                cwd: None,
            })
            .expect("append TerminalSpawned");
    }

    // B sobe com o prompt e em bracketed-paste mode.
    assert!(
        poll_until(Duration::from_secs(5), || wired_b.last_line()
            == "GATEPROMPT"),
        "B deveria mostrar o prompt 'GATEPROMPT' no grid"
    );
    assert!(
        lock(&wired_b.grid).mode().bracketed_paste,
        "B deve estar em bracketed-paste mode"
    );

    // (3) A2A FASEADA A→B. route resolve o alvo e publica o evento de barramento.
    let env = A2aEnvelope::new(node_a, Recipient::Node(node_b), Some("ask".into()));
    let targets = sup.route(&env, RolePolicy::FirstIdle);
    assert_eq!(targets, vec![node_b], "route deve resolver para B");
    store
        .append(&DomainEvent::BusMessageSent {
            id: env.id.clone(),
            from: node_a,
            to: "node:B".into(),
        })
        .expect("append BusMessageSent");

    let out = deliver_a2a(
        &sup,
        node_b,
        node_a,
        MARKER,
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

    // (3a) sequência FASEADA correta na fila serial: AgentText(bracketed, sanitizado,
    //      sem CR) e DEPOIS Submit(0x0D) separado — nunca texto+\r monolítico.
    assert!(poll_until(Duration::from_secs(5), || sup
        .applied_ops(node_b)
        .len()
        == 2));
    let ops = sup.applied_ops(node_b);
    match (&ops[0], &ops[1]) {
        (WriteOp::AgentText { from, bytes }, WriteOp::Submit { from: Some(s) }) => {
            assert_eq!((from, s), (&node_a, &node_a));
            assert!(
                bytes.starts_with(b"\x1b[200~") && bytes.ends_with(b"\x1b[201~"),
                "bracketed-paste"
            );
            assert!(!bytes.contains(&b'\r'), "o texto colado nunca carrega CR");
        }
        other => panic!("sequência faseada inesperada: {other:?}"),
    }

    // (3b) o texto CHEGA ao grid de B via o PTY real (round-trip).
    assert!(
        poll_until(Duration::from_secs(5), || wired_b
            .last_line()
            .contains(MARKER)),
        "o texto A2A deveria chegar ao grid de B; última linha = {:?}",
        wired_b.last_line()
    );

    // (4) Fim-de-resposta: o turno fecha por IDLE do grid (damage estável + resposta
    //     presente), pelo contrato `result > idle > timeout` (relógio lógico).
    let mut det = EndDetector::new(&fake_profile(), 0);
    let mut now_ms = 0_u64;
    let mut closed = None;
    for _ in 0..200 {
        let (changed, ready) = {
            let mut g = lock(&wired_b.grid);
            let changed = !g.damaged_rows().is_empty();
            g.reset_damage();
            let ready = g.last_nonempty_line().contains(MARKER); // resposta assentada
            (changed, ready)
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
        "fechou por idle, não por timeout — sem truncamento"
    );

    // "Shell" headless: traduz os eventos REAIS do Bus para o UiHost.
    while let Ok(ev) = bus_rx.try_recv() {
        if let Some(host_ev) = bus_to_host(&ev) {
            ui.on_event(host_ev);
        }
    }
    assert!(
        ui.events()
            .iter()
            .filter(|e| matches!(e, HostEvent::NodeAdded { .. }))
            .count()
            >= 2,
        "a UI deveria ter recebido os 2 NodeAdded do Bus real"
    );
    assert!(
        ui.events()
            .iter()
            .any(|e| matches!(e, HostEvent::BusMessage { .. })),
        "a UI deveria ter recebido o BusMessage A→B"
    );

    // (5) fingerprint do estado projetado ANTES do crash (com snapshot).
    store.take_snapshot().expect("snapshot");
    let pre_fp = store.project().expect("project").fingerprint();

    // (6) CRASH: perde o estado in-memory (mata PTYs, dropa supervisor + store) e
    //     corrompe o `.db`.
    teardown(&mut pty, wired_a);
    teardown(&mut pty, wired_b);
    drop(pty);
    drop(sup);
    drop(store); // fecha a conexão SQLite (auto-checkpoint do WAL no último close)
    let db = tmp.path().join("lina.db");
    corrupt_middle(&db);
    // remove WAL/SHM órfãos para o lixo do .db não ser mascarado na detecção.
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

    // estado reconstruído IDÊNTICO ao pré-crash (zero perda lógica).
    let post_fp = store2
        .project()
        .expect("project pós-recovery")
        .fingerprint();
    assert_eq!(
        post_fp, pre_fp,
        "reconstrução do event log deve ser idêntica"
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
