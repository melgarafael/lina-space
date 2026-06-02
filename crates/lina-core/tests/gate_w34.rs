//! GATE W3-4 (HEADLESS, sem gpui) — **o caso do fundador "manda oi pro Terminal B"**.
//!
//! Prova, ponta a ponta e determinístico, o pipeline de entrega A2A da Onda 3:
//!
//! 1. **2 PTYs REAIS** registrados como nós do Bus (A = remetente `cat`; B = alvo que entra em
//!    bracketed-paste, mostra `GATEPROMPT` e ECOA/responde cada linha submetida).
//! 2. **`lina ask @B "oi"` no contexto de A** = uma `MailMessage` (`lina/msg@1`) depositada na
//!    **mailbox** (`.lina/outbox/`) — exatamente o que o bin `lina` faz.
//! 3. **O supervisor (Router) drena a mailbox**, aplica os guardrails 0-4 (alvo existe → autonomia
//!    → orçamento → anti-deadlock), **publica `BusEvent::Message`** (pulso A→B do Bus), persiste
//!    `MessageRouted`/`MessageDelivered` no event log (`log.jsonl`) e **injeta** o bloco
//!    `[LINA::MSG]` no PTY de B via a entrega faseada (`deliver_a2a`).
//! 4. **B RECEBE o bloco**: o `screen()` de B mostra `[LINA::MSG]`, `from: @A` e o payload `oi`.
//! 5. **Dedupe**: re-depositar a MESMA mensagem (mesmo `id`) na janela é ignorado (não re-injeta).
//!
//! Os demais guardrails — anti-loop (`hops`), orçamento, autonomia (`manual`), gate de fan-out
//! (`broadcast` > N) e anti-deadlock (wait-for) — são exercitados um a um nos testes unitários de
//! `router` (mesma crate, `cargo test -p lina-core`). Aqui focamos o caminho REAL ponta-a-ponta
//! (PTY → tela) + dedupe.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lina_core::{
    deliver_a2a, now_ms, AlacrittyBackend, BusEvent, CliProfile, DomainEvent, EventStore,
    InjectPolicy, MailMessage, Mailbox, NodeId, PtyCommand, PtyManager, Recipient, RouteOutcome,
    Router, Supervisor, VtBackend, WriteOp,
};

const T: Duration = Duration::from_secs(5);

// ───────────────────────────── helpers (molde do gate_onda2) ─────────────────────────────

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

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let p =
            std::env::temp_dir().join(format!("lina-gatew34-{}-{}", std::process::id(), now_ms()));
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

struct Wired {
    grid: Arc<Mutex<Box<dyn VtBackend>>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pty_key: String,
}

fn screen_text(grid: &Arc<Mutex<Box<dyn VtBackend>>>) -> String {
    let g = lock(grid);
    let (_cols, rows) = g.dims();
    (0..rows)
        .map(|r| g.row_text(r))
        .collect::<Vec<_>>()
        .join("\n")
}

fn wire_node(
    pty: &mut PtyManager,
    sup: &Supervisor,
    pty_key: &str,
    name: &str,
    role: Option<String>,
    cmd: PtyCommand,
) -> (NodeId, Wired) {
    let _pid = pty.spawn(pty_key, cmd, 80, 24).expect("spawn PTY real");
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

fn teardown(pty: &mut PtyManager, mut wired: Wired) {
    wired.stop.store(true, Ordering::Relaxed);
    let _ = pty.kill(wired.pty_key.as_str(), Duration::from_secs(2));
    if let Some(j) = wired.join.take() {
        let _ = j.join();
    }
}

/// Perfil fake do alvo: injeção faseada no PTY, prompt = `GATEPROMPT`, Enter 10ms depois, fim por
/// idle (mesmo do gate_onda2).
fn fake_profile() -> CliProfile {
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
    CliProfile::from_toml_str(src, "<gate-w34>").expect("profile do gate deve parsear")
}

/// Conta as ops aplicadas (AgentText/Submit) na fila serial de um nó — prova de injeção.
fn injected_ops(sup: &Supervisor, node: NodeId) -> usize {
    sup.applied_ops(node)
        .iter()
        .filter(|o| matches!(o, WriteOp::AgentText { .. } | WriteOp::Submit { .. }))
        .count()
}

// ───────────────────────────────── o gate ─────────────────────────────────

#[test]
fn w34_ask_routes_through_mailbox_and_b_receives_lina_msg() {
    let sup = Arc::new(Supervisor::new());
    let mut bus_rx = sup.subscribe(); // assina ANTES de qualquer evento.

    let tmp = TempDir::new();
    // O event log vive sob `<.lina>/events` (o `log.jsonl` é o espelho append-only).
    let lina_dir = tmp.path().join(".lina");
    let mut store = EventStore::open(lina_dir.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: "Gate W3-4".into(),
        })
        .expect("WorkspaceCreated");

    // (1) 2 PTYs REAIS. A = remetente (cat). B = alvo: bracketed-paste + GATEPROMPT + eco/resposta.
    let mut pty = PtyManager::new();
    let (node_a, wired_a) = wire_node(&mut pty, &sup, "w34-A", "@A", None, PtyCommand::new("cat"));
    let (node_b, wired_b) = wire_node(
        &mut pty,
        &sup,
        "w34-B",
        "@B",
        Some("dev".into()),
        PtyCommand::new("sh").arg("-c").arg(
            "printf 'GATEPROMPT '; printf '\\033[?2004h'; \
             while IFS= read -r _; do printf 'BREPLY_OK\\n'; done",
        ),
    );
    for (node, cli) in [(node_a, "cat"), (node_b, "sh")] {
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
            })
            .expect("NodeAdded");
        store
            .append(&DomainEvent::TerminalSpawned {
                node,
                cli: cli.into(),
            })
            .expect("TerminalSpawned");
    }

    // B sobe pronto (prompt) e em bracketed-paste (pré-condição da injeção faseada).
    assert!(
        poll_until(T, || lock(&wired_b.grid).last_nonempty_line()
            == "GATEPROMPT"),
        "B deveria mostrar 'GATEPROMPT'; tela = {:?}",
        screen_text(&wired_b.grid)
    );
    assert!(
        lock(&wired_b.grid).mode().bracketed_paste,
        "B deve estar em bracketed-paste mode"
    );

    // (2) `lina ask @B "oi"` no contexto de A == depositar a MailMessage na mailbox (o que o bin faz).
    let mailbox = Mailbox::new(&lina_dir);
    let ask = MailMessage::new("@A", "@B", "ask", "oi");
    let ask_id = ask.id.clone();
    mailbox.enqueue(&ask).expect("enqueue (lina ask)");

    // (3) O supervisor (Router) drena a mailbox e roteia. A entrega de fato é a injeção faseada
    //     no PTY do alvo (deliver_a2a com o perfil/grid de B).
    let mut router = Router::new(Arc::clone(&sup), mailbox);
    let results = {
        let sup_d = Arc::clone(&sup);
        let grid_b = Arc::clone(&wired_b.grid);
        let profile = fake_profile();
        let deliver = move |target: NodeId, from: NodeId, text: &str| {
            deliver_a2a(
                &sup_d,
                target,
                from,
                text,
                &profile,
                &grid_b,
                InjectPolicy::AllowAll,
            )
            .map_err(|e| e.to_string())
        };
        router.pump(&mut store, now_ms(), deliver)
    };
    assert_eq!(results.len(), 1, "uma mensagem drenada");
    assert_eq!(
        results[0].1,
        RouteOutcome::Delivered {
            targets: vec![node_b]
        },
        "a mensagem deveria rotear e entregar para B"
    );

    // (4) B RECEBE o bloco [LINA::MSG]: o eco do PTY mostra a sentinela, o from e o payload.
    assert!(
        poll_until(T, || {
            let s = screen_text(&wired_b.grid);
            s.contains("[LINA::MSG]") && s.contains("from: @A") && s.contains("payload: oi")
        }),
        "B deveria receber o bloco [LINA::MSG] (from:@A, payload: oi); tela = {:?}",
        screen_text(&wired_b.grid)
    );
    // E B de fato SUBMETEU (respondeu) — não é eco morto.
    assert!(
        poll_until(T, || screen_text(&wired_b.grid).contains("BREPLY_OK")),
        "B deveria ter respondido após o Enter separado"
    );

    // (3-bus) O pulso A→B trafegou pelo Bus como evento REAL (mesmo id do envelope roteado).
    let mut saw_message = false;
    while let Ok(ev) = bus_rx.try_recv() {
        if let BusEvent::Message {
            from,
            to: Recipient::Node(n),
            ..
        } = ev
        {
            if from == node_a && n == node_b {
                saw_message = true;
            }
        }
    }
    assert!(saw_message, "o pulso A→B deveria ter passado pelo Bus");

    // (3-log) Os eventos de roteamento/entrega foram PERSISTIDOS no event log (log.jsonl) — com os
    // CAMPOS certos no MESMO registro (uma linha = um EventRecord JSON), não só "aparece no arquivo".
    let jsonl = std::fs::read_to_string(lina_dir.join("events").join("log.jsonl"))
        .expect("ler events/log.jsonl");
    let routed = jsonl
        .lines()
        .find(|l| l.contains("MessageRouted"))
        .unwrap_or_else(|| panic!("nenhum MessageRouted no log; jsonl = {jsonl}"));
    assert!(
        routed.contains(&ask_id)
            && routed.contains(&node_a.to_string())
            && routed.contains("\"to\":\"@B\"")
            && routed.contains("\"intent\":\"ask\""),
        "o MessageRouted deveria ter id={ask_id}, from={node_a}, to=@B, intent=ask; veio: {routed}"
    );
    let delivered = jsonl
        .lines()
        .find(|l| l.contains("MessageDelivered"))
        .unwrap_or_else(|| panic!("nenhum MessageDelivered no log; jsonl = {jsonl}"));
    assert!(
        delivered.contains(&ask_id) && delivered.contains(&node_b.to_string()),
        "o MessageDelivered deveria ter id={ask_id} e to={node_b}; veio: {delivered}"
    );

    // (5) DEDUPE: re-depositar a MESMA mensagem (mesmo id) é ignorado — B não recebe de novo.
    let ops_before = injected_ops(&sup, node_b);
    router
        .mailbox()
        .enqueue(&ask)
        .expect("re-enqueue (mesmo id)");
    let again = {
        let sup_d = Arc::clone(&sup);
        let grid_b = Arc::clone(&wired_b.grid);
        let profile = fake_profile();
        let deliver = move |target: NodeId, from: NodeId, text: &str| {
            deliver_a2a(
                &sup_d,
                target,
                from,
                text,
                &profile,
                &grid_b,
                InjectPolicy::AllowAll,
            )
            .map_err(|e| e.to_string())
        };
        router.pump(&mut store, now_ms(), deliver)
    };
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].1, RouteOutcome::Duplicate, "mesmo id = duplicado");
    // Nenhuma nova injeção em B: a duplicada é cortada ANTES do deliver (não enfileira ops).
    assert_eq!(
        injected_ops(&sup, node_b),
        ops_before,
        "a mensagem duplicada NÃO deveria injetar de novo em B"
    );

    teardown(&mut pty, wired_a);
    teardown(&mut pty, wired_b);
    drop(pty);
    drop(sup);
}
