//! GATE F1-0-5 — contrato de handoff com schema + intents canônicos (`lina/msg@2`).
//!
//! Critérios da story (tasks/epico-f1/ondas-0-1.md §F1-0-5):
//! (1) serialização do envelope `@2` COMPLETO — espelha o critério que o épico 32
//!     (§"Contrato do envelope A2A") exigiu para o `@1` em W3-7 (gate_onda3 (g));
//! (2) router REJEITA handoff sem schema/timeout com erro estruturado (campo a campo);
//! (3) upcasting provado: log com envelopes `@1` replaya sem erro e sem mudança de
//!     fingerprint (estilo gate_onda0/2; molde de PTY real do gate_w34);
//! (4) constraint-transfer: handoff carregando constraint de autonomia chega ao destino
//!     com a constraint LEGÍVEL no payload injetado (PTY real → tela, molde gate_w34);
//! (5) red-team: NENHUM campo do contrato vindo do outbox decide identidade/ordem/
//!     autorização — autenticação em duas camadas é a única autoridade (ADR 0006/0007).
//!
//! Validação é DE FORMA e NO ROUTER (nunca no LLM): presença/shape dos campos do
//! contrato; o conteúdo semântico (o schema em si) é honrado pelo agente destino.

use std::collections::BTreeMap;
// F1-6-8: imports `#[cfg(unix)]` = exclusivos do teste de PTY real (gateado abaixo); no Windows viram unused.
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::thread::{self, JoinHandle};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use lina_core::{deliver_a2a, AlacrittyBackend, CliProfile, PtyCommand, PtyManager, VtBackend};
use lina_core::{
    now_ms, AutonomyLevel, DeliveryOutcome, DomainEvent, EventStore, HandoffContract, MailMessage,
    Mailbox, NodeId, RouteOutcome, Router, RouterConfig, Supervisor, CANONICAL_INTENTS_V2,
    MAIL_SCHEMA_V1, MAIL_SCHEMA_V2,
};

// F1-6-8: T e poll_until são usados só pelo teste de PTY real (gateado) — cfg(unix) evita dead_code no Windows.
#[cfg(unix)]
const T: Duration = Duration::from_secs(5);

// ───────────────────────── helpers (molde do gate_w34) ─────────────────────────

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(unix)]
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
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-gatef105-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
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

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// `reason`s (snake_case) de todos os `RouteBlocked` no log — a recusa é assertada
/// NO LOG (ADR 0003), não só no enum de retorno.
fn blocked_reasons(store: &EventStore) -> Vec<String> {
    store
        .events()
        .expect("events")
        .into_iter()
        .filter(|r| r.kind == "RouteBlocked")
        .filter_map(|r| {
            r.payload
                .get("reason")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect()
}

/// Contrato VÁLIDO completo (base dos testes; os de rejeição derivam dele zerando 1 campo).
fn full_contract() -> HandoffContract {
    let mut constraints = BTreeMap::new();
    constraints.insert("autonomy".to_string(), "assistido".to_string());
    constraints.insert("file_scope".to_string(), "docs/".to_string());
    constraints.insert("budget_restante".to_string(), "2 delegações".to_string());
    HandoffContract {
        input_schema: "{tarefa: string, contexto: path}".into(),
        output_schema: "{resultado: string, arquivos_tocados: [path]}".into(),
        error_codes: vec!["E_TIMEOUT".into(), "E_SCOPE".into()],
        timeout_sec: 90,
        retry_policy: "manual".into(),
        constraints_metadata: constraints,
    }
}

/// Router + supervisor + store isolados, com @A/@B registrados (origens autenticáveis).
fn arena(tag: &str) -> (Router, Arc<Supervisor>, EventStore, TempDir, NodeId) {
    let tmp = TempDir::new(tag);
    let sup = Arc::new(Supervisor::new());
    sup.register("@A", None, sink());
    let b = sup.register("@B", Some("dev".into()), sink());
    let mb = Mailbox::new(tmp.path().join(".lina"));
    let store = EventStore::open(tmp.path().join(".lina/events")).expect("abrir store");
    let router = Router::new(Arc::clone(&sup), mb);
    (router, sup, store, tmp, b)
}

/// `deliver` de teste que só registra o texto injetado (sem PTY).
fn recording_deliver(
    sink_vec: Arc<Mutex<Vec<(NodeId, NodeId, String)>>>,
) -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    move |target, from, text| {
        lock(&sink_vec).push((target, from, text.to_string()));
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    }
}

// ───────────────────────── (1) serialização do @2 completo ─────────────────────────

/// Espelho do gate_onda3 (g) para o `@2`: roundtrip JSON preservando TODOS os campos —
/// os herdados do contrato canônico do épico 32 (`id·root_cause_id·from·to·intent·hops·
/// await·ts` + `ref`/`reply_to`) E o bloco de contrato novo
/// (`input_schema/output_schema/error_codes/timeout_sec/retry_policy/constraints_metadata`).
#[test]
fn a_envelope_v2_serializa_completo() {
    let mut msg = MailMessage::new_v2("@A", "@B", "handoff", "implemente o parser")
        .awaiting()
        .with_ref("plan:T4")
        .replying_to("msg_pergunta")
        .with_contract(full_contract());
    msg.root_cause_id = Some("msg_turno0".into());
    msg.hops = 2;
    msg.ts_ms = 1_700_000_000_000;

    let json = serde_json::to_string(&msg).expect("serialize");
    let back: MailMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, back, "roundtrip preserva todos os campos do @2");
    assert_eq!(back.schema, MAIL_SCHEMA_V2, "schema lina/msg@2");
    assert!(back.await_reply);
    assert_eq!(back.hops, 2);
    assert_eq!(back.root_cause_id.as_deref(), Some("msg_turno0"));
    assert_eq!(back.reply_to.as_deref(), Some("msg_pergunta"));
    assert_eq!(back.plan_ref(), Some("T4"));
    assert_eq!(back.ts_ms, 1_700_000_000_000);

    let c = back.contract.as_ref().expect("contrato presente");
    assert_eq!(c.input_schema, "{tarefa: string, contexto: path}");
    assert_eq!(
        c.output_schema,
        "{resultado: string, arquivos_tocados: [path]}"
    );
    assert_eq!(c.error_codes, vec!["E_TIMEOUT", "E_SCOPE"]);
    assert_eq!(c.timeout_sec, 90);
    assert_eq!(c.retry_policy, "manual");
    assert_eq!(
        c.constraints_metadata.get("autonomy").map(String::as_str),
        Some("assistido")
    );

    // O enum canônico fechado (decisão registrada na story/no código): `status`
    // CANONIZADO (DIRECIONAMENTO §P2-bônus); `plan` realizado nos 2 verbos reais do
    // W3-5; `review` (resíduo do doc @1) FORA — nenhum mecanismo o consome.
    for intent in [
        "ask",
        "handoff",
        "reply",
        "status",
        "broadcast",
        "handshake",
        "plan.claim",
        "plan.check",
        "permission",
    ] {
        assert!(
            CANONICAL_INTENTS_V2.contains(&intent),
            "{intent} deve ser canônico"
        );
    }
    assert!(
        !CANONICAL_INTENTS_V2.contains(&"review"),
        "review sai do contrato @2"
    );
}

// ───────────────────────── (2) rejeição estruturada campo a campo ─────────────────────────

#[test]
fn b_router_rejeita_handoff_v2_campo_a_campo() {
    let (mut router, _sup, mut store, _tmp, _b) = arena("rejeita");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mut deliver = recording_deliver(Arc::clone(&captured));

    // Cada caso: (mutação do contrato, fragmento OBRIGATÓRIO no erro legível).
    type Mutation = Box<dyn Fn(&mut MailMessage)>;
    let cases: Vec<(Mutation, &str)> = vec![
        (Box::new(|m| m.contract = None), "contrato"),
        (
            Box::new(|m| m.contract.as_mut().expect("c").input_schema.clear()),
            "input_schema",
        ),
        (
            Box::new(|m| m.contract.as_mut().expect("c").output_schema.clear()),
            "output_schema",
        ),
        (
            Box::new(|m| m.contract.as_mut().expect("c").timeout_sec = 0),
            "timeout_sec",
        ),
        (
            Box::new(|m| m.contract.as_mut().expect("c").retry_policy.clear()),
            "retry_policy",
        ),
    ];
    for (mutate, field) in cases {
        let mut msg =
            MailMessage::new_v2("@A", "@B", "handoff", "tarefa").with_contract(full_contract());
        mutate(&mut msg);
        match router.route_message(&msg, &mut store, now_ms(), &mut deliver) {
            RouteOutcome::ContractRejected(why) => {
                assert!(
                    why.contains(field),
                    "erro legível deve nomear '{field}' para o agente corrigir; veio: {why}"
                );
            }
            other => panic!("esperava ContractRejected({field}); veio {other:?}"),
        }
    }
    // Nada foi entregue; cada recusa está NO LOG com motivo tipado (ADR 0003).
    assert!(lock(&captured).is_empty(), "nenhuma injeção deve ocorrer");
    let reasons = blocked_reasons(&store);
    assert_eq!(
        reasons.iter().filter(|r| *r == "invalid_contract").count(),
        5,
        "5 recusas de contrato no log; reasons = {reasons:?}"
    );

    // Intent fora do enum canônico (@2) → rejeição estruturada própria.
    let msg = MailMessage::new_v2("@A", "@B", "review", "tarefa");
    match router.route_message(&msg, &mut store, now_ms(), &mut deliver) {
        RouteOutcome::ContractRejected(why) => {
            assert!(
                why.contains("intent") && why.contains("review"),
                "veio: {why}"
            );
            // O erro ENSINA o enum (agente corrige sozinho — 13.11: validação no router).
            assert!(
                why.contains("ask") && why.contains("handoff"),
                "veio: {why}"
            );
        }
        other => panic!("esperava ContractRejected(intent); veio {other:?}"),
    }
    // Intent vazio (@2) idem — o @2 torna o intent OBRIGATÓRIO.
    let msg = MailMessage::new_v2("@A", "@B", "", "tarefa");
    assert!(matches!(
        router.route_message(&msg, &mut store, now_ms(), &mut deliver),
        RouteOutcome::ContractRejected(_)
    ));
    // `reply` @2 exige `reply_to` (sem ele não fecha await — contrato coerente).
    let msg = MailMessage::new_v2("@A", "@B", "reply", "resposta");
    match router.route_message(&msg, &mut store, now_ms(), &mut deliver) {
        RouteOutcome::ContractRejected(why) => assert!(why.contains("reply_to"), "veio: {why}"),
        other => panic!("esperava ContractRejected(reply_to); veio {other:?}"),
    }
    // Schema desconhecido → recusado (forward-guard; nunca rotear o que não entendemos).
    let mut msg = MailMessage::new("@A", "@B", "ask", "oi");
    msg.schema = "lina/msg@3".into();
    assert!(matches!(
        router.route_message(&msg, &mut store, now_ms(), &mut deliver),
        RouteOutcome::ContractRejected(_)
    ));
    let reasons = blocked_reasons(&store);
    assert!(
        reasons.iter().any(|r| r == "invalid_intent"),
        "intent fora do enum logado como invalid_intent; reasons = {reasons:?}"
    );
}

// ───────────────────────── (3) compat @1 + upcasting/fingerprint ─────────────────────────

/// `@1` segue INTOCADO: handoff sem contrato roteia (compat com o contorno
/// `lina ask --intent handoff` visto em campo), e um JSON @1 LITERAL (congelado, só com
/// os campos da era W3-7) desserializa com `contract = None` e roteia.
#[test]
fn c_compat_v1_handoff_sem_contract_roteia() {
    let (mut router, _sup, mut store, _tmp, b) = arena("compat-v1");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mut deliver = recording_deliver(Arc::clone(&captured));

    let v1 = MailMessage::new("@A", "@B", "handoff", "tarefa sem contrato");
    assert_eq!(v1.schema, MAIL_SCHEMA_V1);
    assert!(matches!(
        router.route_message(&v1, &mut store, now_ms(), &mut deliver),
        RouteOutcome::Delivered { .. }
    ));
    // F1-0-4: desde a entrega ciente de estado, entregar marca o alvo Busy — o turno de
    // @B "termina" antes da próxima entrega (como o lifecycle real faria no fim-de-resposta).
    _sup.set_status(b, lina_core::NodeStatus::Idle)
        .expect("B terminou o turno");

    // JSON @1 congelado (forma da era W3-7 — sem `contract`, com `ref`).
    let frozen = r#"{
        "schema": "lina/msg@1",
        "id": "msg_0001f105aaaa",
        "from": "@A",
        "to": "@B",
        "intent": "ask",
        "payload": "oi do passado",
        "await_reply": false,
        "ref": "plan:T1",
        "hops": 0,
        "ts_ms": 1700000000000
    }"#;
    let old: MailMessage = serde_json::from_str(frozen).expect("JSON @1 literal parseia");
    assert!(old.contract.is_none(), "@1 não tem contrato");
    assert_eq!(old.plan_ref(), Some("T1"));
    assert!(matches!(
        router.route_message(&old, &mut store, now_ms(), &mut deliver),
        RouteOutcome::Delivered { .. }
    ));
    let injected = lock(&captured);
    assert_eq!(injected.len(), 2);
    assert!(injected.iter().all(|(t, _, _)| *t == b));
    // O bloco @1 injetado NÃO ganha seção de contrato (formato congelado do W3-7).
    assert!(
        !injected[0].2.contains("contract."),
        "@1 sem seção de contrato"
    );
}

/// Upcasting provado no ESTILO gate_onda0/2: um log gerado por tráfego `@1` replaya
/// sem erro e o fingerprint do estado projetado NÃO muda entre o store vivo e a
/// reabertura do MESMO log (inclui os `RouteBlocked`/`MessageRouted` da era @1).
#[test]
fn d_upcasting_log_v1_replaya_sem_erro_e_fingerprint_estavel() {
    let tmp = TempDir::new("upcast");
    let sup = Arc::new(Supervisor::new());
    sup.register("@A", None, sink());
    sup.register("@B", None, sink());
    let mb = Mailbox::new(tmp.path().join(".lina"));
    let mut router = Router::new(Arc::clone(&sup), mb);
    let store_dir = tmp.path().join(".lina/events");
    let fp_live = {
        let mut store = EventStore::open(&store_dir).expect("abrir store");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Upcast".into(),
                focus_preset: String::new(),
            })
            .expect("ws");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let mut deliver = recording_deliver(captured);
        // Tráfego @1 real: ask entregue + handoff entregue + um bloqueio (alvo fantasma).
        // F1-0-4: entre entregas ao MESMO alvo, o turno termina (Idle) — senão a 2ª é Retained.
        let b = sup.node_by_name("@B").expect("@B registrado");
        for msg in [
            MailMessage::new("@A", "@B", "ask", "oi"),
            MailMessage::new("@A", "@B", "handoff", "tarefa @1"),
            MailMessage::new("@A", "@Ghost", "ask", "ninguém"),
        ] {
            sup.set_status(b, lina_core::NodeStatus::Idle)
                .expect("B livre p/ o próximo turno");
            let _ = router.route_message(&msg, &mut store, now_ms(), &mut deliver);
        }
        store.project().expect("project vivo").fingerprint()
    };
    // Reabre o MESMO diretório: replay do log (com upcasting) — sem erro, mesmo fingerprint.
    let reopened = EventStore::open(&store_dir).expect("reabrir store");
    let fp_replay = reopened
        .project()
        .expect("replay sem erro (upcasting)")
        .fingerprint();
    assert_eq!(
        fp_live, fp_replay,
        "fingerprint estável entre vivo e replay"
    );
}

// ───────────────────────── (4) constraint-transfer em PTY REAL ─────────────────────────

/// Golden transcript de terminal REAL (molde gate_w34): um handoff `@2` com constraint
/// de autonomia chega ao PTY do destino com a constraint LEGÍVEL no bloco injetado.
#[cfg(unix)]
// F1-6-8: PTY real com `cat`/`sh -c` — runtime-Unix; pendente-windows na tabela ci-3so-triagem.md.
#[test]
fn e_constraint_transfer_legivel_em_pty_real() {
    let sup = Arc::new(Supervisor::new());
    let tmp = TempDir::new("pty");
    let lina_dir = tmp.path().join(".lina");
    let mut store = EventStore::open(lina_dir.join("events")).expect("abrir store");

    let mut pty = PtyManager::new();
    let (_node_a, wired_a) = wire_node(&mut pty, &sup, "f105-A", "@A", PtyCommand::new("cat"));
    let (node_b, wired_b) = wire_node(
        &mut pty,
        &sup,
        "f105-B",
        "@B",
        PtyCommand::new("sh").arg("-c").arg(
            "printf 'GATEPROMPT '; printf '\\033[?2004h'; \
             while IFS= read -r _; do printf 'BREPLY_OK\\n'; done",
        ),
    );
    assert!(
        poll_until(T, || lock(&wired_b.grid).last_nonempty_line()
            == "GATEPROMPT"),
        "B deveria mostrar GATEPROMPT; tela = {:?}",
        screen_text(&wired_b.grid)
    );

    let mailbox = Mailbox::new(&lina_dir);
    let msg = MailMessage::new_v2("@A", "@B", "handoff", "rode os testes do parser")
        .with_contract(full_contract());
    mailbox.enqueue_as("@A", &msg).expect("enqueue por-nó");

    let mut router = Router::new(Arc::clone(&sup), mailbox);
    let results = {
        let sup_d = Arc::clone(&sup);
        let grid_b = Arc::clone(&wired_b.grid);
        let profile = fake_profile();
        let mut deliver = move |target: NodeId, from: NodeId, text: &str| {
            deliver_a2a(
                &sup_d,
                target,
                from,
                text,
                &profile,
                &grid_b,
                lina_core::InjectPolicy::AllowAll,
            )
            .map_err(|e| e.to_string())
        };
        router.pump(&mut store, now_ms(), &mut deliver)
    };
    assert!(
        results.iter().any(
            |(_, o)| matches!(o, RouteOutcome::Delivered { targets } if targets.contains(&node_b))
        ),
        "handoff @2 com contrato VÁLIDO deve rotear; results = {results:?}"
    );

    // O destino VÊ as constraints e o contrato, legíveis, no bloco injetado.
    assert!(
        poll_until(T, || {
            let s = screen_text(&wired_b.grid);
            s.contains("constraint.autonomy: assistido")
                && s.contains("constraint.file_scope: docs/")
                && s.contains("contract.timeout_sec: 90")
                && s.contains("contract.retry_policy: manual")
                && s.contains("payload: rode os testes do parser")
        }),
        "constraints/contrato deveriam estar LEGÍVEIS na tela de B; tela = {:?}",
        screen_text(&wired_b.grid)
    );

    teardown(&mut pty, wired_a);
    teardown(&mut pty, wired_b);
}

// ───────────────────────── (5) red-team: contrato não é autoridade ─────────────────────────

#[test]
fn f_redteam_contrato_nao_decide_identidade_nem_autorizacao() {
    // (a) IDENTIDADE: origem desconhecida/anonimizada (drain FLAT) com contrato suculento
    //     → UnknownSender ANTES de qualquer validação de contrato. A autenticação em duas
    //     camadas (carimbo no drain + node_by_name) segue a ÚNICA autoridade (ADR 0006).
    let (mut router, _sup, mut store, _tmp, _b) = arena("redteam-id");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mut deliver = recording_deliver(Arc::clone(&captured));
    let mut forged =
        MailMessage::new_v2("", "@B", "handoff", "payload").with_contract(full_contract());
    forged.from.clear(); // como o drain FLAT anonimiza
    assert!(matches!(
        router.route_message(&forged, &mut store, now_ms(), &mut deliver),
        RouteOutcome::UnknownSender(_)
    ));
    let reasons = blocked_reasons(&store);
    assert_eq!(
        reasons,
        vec!["unknown_sender".to_string()],
        "identidade decide ANTES do contrato (nenhum invalid_contract aqui)"
    );

    // (b) AUTORIZAÇÃO: autonomia `manual` bloqueia a delegação MESMO com o contrato
    //     alegando `autonomy: autonomo` — constraints são DADOS transportados, jamais
    //     entrada das decisões do router.
    let tmp2 = TempDir::new("redteam-auth");
    let sup2 = Arc::new(Supervisor::new());
    sup2.register("@A", None, sink());
    sup2.register("@B", None, sink());
    let mb2 = Mailbox::new(tmp2.path().join(".lina"));
    let mut store2 = EventStore::open(tmp2.path().join(".lina/events")).expect("store");
    let mut router2 = Router::with_config(
        Arc::clone(&sup2),
        mb2,
        RouterConfig {
            autonomy: AutonomyLevel::Manual,
            ..RouterConfig::default()
        },
    );
    let mut c = full_contract();
    c.constraints_metadata
        .insert("autonomy".into(), "autonomo".into());
    let msg = MailMessage::new_v2("@A", "@B", "handoff", "escala sozinho?").with_contract(c);
    assert!(
        matches!(
            router2.route_message(&msg, &mut store2, now_ms(), &mut deliver),
            RouteOutcome::BlockedByAutonomy
        ),
        "constraint forjada no contrato NÃO compra autonomia"
    );
    assert!(
        lock(&captured).is_empty(),
        "nada foi injetado em nenhum caso"
    );
}

// ───────────────────────── infra de PTY real (cópia fiel do gate_w34) ─────────────────────────
// F1-6-8: infra usada SÓ pelo teste `e_constraint_transfer_legivel_em_pty_real` (gateado) — cfg(unix) evita dead_code no Windows.

#[cfg(unix)]
struct Wired {
    grid: Arc<Mutex<Box<dyn VtBackend>>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pty_key: String,
}

#[cfg(unix)]
fn screen_text(grid: &Arc<Mutex<Box<dyn VtBackend>>>) -> String {
    let g = lock(grid);
    let (_cols, rows) = g.dims();
    (0..rows)
        .map(|r| g.row_text(r))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(unix)]
fn wire_node(
    pty: &mut PtyManager,
    sup: &Supervisor,
    pty_key: &str,
    name: &str,
    cmd: PtyCommand,
) -> (NodeId, Wired) {
    // 48 rows: o bloco @2 (~16 linhas com contrato/constraints) + os ecos/replies de B
    // precisam caber no viewport — em 24 rows o TOPO do bloco rola para fora e o assert
    // de legibilidade falharia por scroll, não por ausência (visto na 1ª rodada do gate).
    let _pid = pty.spawn(pty_key, cmd, 80, 48).expect("spawn PTY real");
    let writer = pty.take_writer(pty_key).expect("take_writer");
    let mut reader = pty.clone_reader(pty_key).expect("clone_reader");

    let grid: Arc<Mutex<Box<dyn VtBackend>>> =
        Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(80, 48))));
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
    let node = sup.register(name, None, writer);
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

#[cfg(unix)]
fn teardown(pty: &mut PtyManager, mut wired: Wired) {
    wired.stop.store(true, Ordering::Relaxed);
    let _ = pty.kill(wired.pty_key.as_str(), Duration::from_secs(2));
    if let Some(j) = wired.join.take() {
        let _ = j.join();
    }
}

#[cfg(unix)]
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
    CliProfile::from_toml_str(src, "<gate-f105>").expect("profile do gate parseia")
}
