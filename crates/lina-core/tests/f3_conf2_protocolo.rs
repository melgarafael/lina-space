//! **F3-CONF-2 · Correção do protocolo anti-engolimento (gate a/b) — caminho real, RED→GREEN.**
//!
//! O CRITÉRIO DE ACEITE executável do CONF-CORE: o protocolo que CONSOME o alarme `DeliveredNoProgress`
//! (W3) e fecha o passo 6/7 do loop do Maestro nativo (`router::enforce_dispatch_protocol`). Um despacho
//! ENTREGUE que volta a `Idle` sem progresso na janela é re-despachado **1× informado** (reusa
//! `SpawnRequested`); persistindo, abre o breaker STICKY e ESCALA ao humano (`GoalEscalated{stalled}`).
//!
//! **Escopo v1 = goal-driven** (lido em `router.rs:2413`): o re-despacho/escala só agem quando o nó
//! POSSUI um item de uma Goal — `GoalEscalated` exige um `goal_id` auditável que justifique o recurso
//! (ADR 0007: recurso só se gasta com justificativa server-side). Handoff fora de Goal mantém só a
//! DETECÇÃO (alarme da UI). Por isso o setup destes testes SEMEIA uma Goal + item dono pelo @B antes de
//! disparar o engolimento — sem a Goal, o protocolo (corretamente) não fecha o ciclo.
//!
//! Caminho REAL: a entrega vai por `route_message`; o protocolo dispara no `pump` (que chama
//! `sync_dispatch_watch` + `enforce_dispatch_protocol`). Asserções sobre o EVENT LOG (invariante #4). O
//! relógio é explícito (`pump(now_ms)`) — determinístico, sem sleep. Cada teste traz a cláusula QUEBRA SE.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    DeliveryOutcome, DomainEvent, EventStore, MailMessage, Mailbox, NodeId, NodeStatus,
    RouteOutcome, Router, RouterConfig, Supervisor,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "lina-conf2proto-{tag}-{}-{tid}-{n}",
        std::process::id()
    ))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

fn ok_deliver() -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    |_t, _f, _txt| {
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    }
}

struct Env {
    tmp: PathBuf,
    store: EventStore,
    sup: Arc<Supervisor>,
    router: Router,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

fn env(tag: &str) -> Env {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: format!("F3-CONF-2 protocolo — {tag}"),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated");
    let sup = Arc::new(Supervisor::new());
    let router = Router::with_config(
        Arc::clone(&sup),
        Mailbox::new(&lina),
        RouterConfig::default(),
    );
    Env {
        tmp,
        store,
        sup,
        router,
    }
}

fn count_kind(recs: &[lina_core::EventRecord], kind: &str) -> usize {
    recs.iter().filter(|r| r.kind == kind).count()
}

/// SETUP goal-driven: define a Goal "g", semeia um item "T1" atribuído a ela e o faz dono pelo @B —
/// a pré-condição que o protocolo v1 exige (re-despacho/escala precisam do `goal_id` auditável). É
/// SETUP (store.append + verbos reais), não a costura sob teste — a costura é o protocolo no `pump`.
fn seed_item_owned_by_b(e: &mut Env) {
    let mut deliver = ok_deliver();
    e.store
        .append(&DomainEvent::GoalDefined {
            goal_id: "g".into(),
            statement: "meta de teste".into(),
            root_cause_id: "r".into(),
            origin: "@Maestro".into(),
            budget_tokens: 0,
        })
        .expect("GoalDefined");
    // item T1 promovido e ATRIBUÍDO à Goal g (caminho real plan.add → PlanItemAdded+PlanItemAttributed).
    e.router.route_message(
        &MailMessage::new(
            "@A",
            "plan",
            "plan.add",
            r#"{"item":"T1","desc":"tarefa","goal_id":"g"}"#,
        ),
        &mut e.store,
        500,
        &mut deliver,
    );
    // @B reivindica T1 → owner=@B (server-side, pelo sender autenticado).
    e.router.route_message(
        &MailMessage::new("@B", "plan", "plan.claim", "").with_ref("plan:T1"),
        &mut e.store,
        501,
        &mut deliver,
    );
}

/// Entrega REAL de um handoff @A→@B e marca o retorno a `Idle` SEM progresso (o "engolido" do Modo B).
/// Caminho real (`route_message` + `NodeStatusChanged{Idle}`); zero `TokenUsageReported`/`MessageRouted`.
fn entrega_engolida(e: &mut Env, txt: &str, t: u64) {
    let mut deliver = ok_deliver();
    let m = MailMessage::new("@A", "@B", "handoff", txt);
    let out = e.router.route_message(&m, &mut e.store, t, &mut deliver);
    assert!(
        matches!(out, RouteOutcome::Delivered { .. }),
        "entrega real (arma o vigia W3); veio {out:?}"
    );
    let b = e.sup.node_by_name("@B").expect("@B");
    e.store
        .append(&DomainEvent::NodeStatusChanged {
            node: b,
            status: NodeStatus::Idle.as_str().to_string(),
            from: NodeStatus::Busy.as_str().to_string(),
            reason: "end_of_response".to_string(),
        })
        .expect("idle");
    e.sup.set_status(b, NodeStatus::Idle).expect("idle roster");
}

// ════════════════════════════════ GATE (a) — re-despacho 1× → stalled ════════════════════════════════

/// **(gate a) entrega-sem-progresso → re-despacho 1× informado → persiste → `GoalEscalated{stalled}`.**
/// O OUTER loop (`enforce_dispatch_protocol`), ao ver um `MessageDelivered` que volta a `Idle` sem
/// nenhum `DomainEvent` de progresso na janela, RE-DESPACHA UMA VEZ (`SpawnRequested` com prompt
/// informado: "Não recebi nenhum sinal de progresso… Reassuma"). Persistindo o engolimento (2ª janela),
/// o breaker STICKY impede a 3ª tentativa automática e o laço ESCALA com `GoalEscalated{reason:"stalled"}`
/// — PAUSA resumível, nunca kill (events.rs:641). `requested_by`/`reason` carimbados server-side (ADR 0007).
///
/// QUEBRA SE… o protocolo re-despachasse em tempestade (>1 na 1ª janela), nunca re-despachasse, ou nunca
/// escalasse. Não-vacuoso: o caminho feliz (1 re-despacho, depois 1 stalled) é exercido pelo `pump` real.
#[test]
fn gate_a_entrega_sem_progresso_redespacha_uma_vez_e_escala_stalled() {
    let mut e = env("gate-a");
    let _a = e.sup.register("@A", Some("maestro".into()), sink());
    let b = e.sup.register("@B", Some("dev".into()), sink());
    e.sup.set_status(b, NodeStatus::Idle).expect("B idle");
    seed_item_owned_by_b(&mut e);

    let mut deliver = ok_deliver();
    let janela = lina_core::attention::DELIVERED_NO_PROGRESS_WINDOW_MS;

    // 1ª janela: entrega engolida → 1 re-despacho informado (SpawnRequested).
    entrega_engolida(&mut e, "assuma a tarefa T1", 1_000);
    let _ = e
        .router
        .pump(&mut e.store, 1_000 + janela + 1, &mut deliver);

    let recs = e.store.events().expect("events");
    let redespachos = recs
        .iter()
        .filter(|r| r.kind == "SpawnRequested")
        .filter(|r| {
            r.payload
                .get("prompt")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("progresso") || s.to_lowercase().contains("reassuma"))
        })
        .count();
    assert_eq!(
        redespachos, 1,
        "gate (a): exatamente 1 re-despacho informado após a 1ª janela sem progresso"
    );
    // ainda NÃO escalou (a escala só vem se a re-tentativa também engolir).
    assert_eq!(
        count_kind(&recs, "GoalEscalated"),
        0,
        "gate (a): a 1ª janela re-despacha, NÃO escala (escala é o backstop da persistência)"
    );

    // 2ª janela: o engolimento PERSISTE (nova entrega, novo epoch) → breaker sticky + escala.
    entrega_engolida(&mut e, "(re-despacho) reassuma T1", 1_000 + janela + 2);
    let _ = e
        .router
        .pump(&mut e.store, 1_000 + 2 * janela + 4, &mut deliver);

    let recs = e.store.events().expect("events");
    let stalled = recs
        .iter()
        .filter(|r| r.kind == "GoalEscalated")
        .filter(|r| r.payload.get("reason").and_then(|v| v.as_str()) == Some("stalled"))
        .count();
    assert_eq!(
        stalled, 1,
        "gate (a): persistindo o engolimento, escala com GoalEscalated{{stalled}} (1×, pausa resumível)"
    );
    // breaker sticky: NÃO há um 3º re-despacho automático (a escada parou no gate humano).
    assert_eq!(
        count_kind(&recs, "SpawnRequested"),
        1,
        "gate (a): breaker sticky — após a escala NÃO sai um 3º re-despacho sozinho (só 1 no total)"
    );
}

// ════════════════════════════════ GATE (b) — loop_detected calibrado ════════════════════════════════

/// **(gate b) o re-despacho de um engolido NÃO é barrado como `LoopDetected`.** O anti-loop hoje barra o
/// Maestro de redirecionar um worker travado (repro §6.4 #22c: 4 msgs em ~5min sem atividade). O
/// protocolo calibra emitindo o re-despacho como um `SpawnRequested` server-side (livro-razão do OUTER
/// loop), que NÃO passa pela detecção de loop do A2A — a correção legítima do engolido SEMPRE sai. A
/// tempestade-cascata real (saltos repetidos COM atividade fechando ciclo) continua barrada pelos testes
/// internos do anti-loop (`router.rs:4703` + `dialogue_reply_delivers_not_loop_detected`) — NÃO re-provo
/// aqui (memória *não re-provar o coberto*).
///
/// QUEBRA SE… a correção do engolido fosse barrada (apareceria `RouteBlocked{loop_detected}` ou nenhum
/// re-despacho sairia). Não-vacuoso: o re-despacho SAI (1 SpawnRequested) e zero bloqueio de loop.
#[test]
fn gate_b_redespacho_de_engolido_nao_e_barrado_como_loop() {
    let mut e = env("gate-b");
    let _a = e.sup.register("@A", Some("maestro".into()), sink());
    let b = e.sup.register("@B", Some("dev".into()), sink());
    e.sup.set_status(b, NodeStatus::Idle).expect("B idle");
    seed_item_owned_by_b(&mut e);

    let mut deliver = ok_deliver();
    let janela = lina_core::attention::DELIVERED_NO_PROGRESS_WINDOW_MS;
    entrega_engolida(&mut e, "assuma T1", 1_000);
    let r = e
        .router
        .pump(&mut e.store, 1_000 + janela + 1, &mut deliver);

    // a correção SAI (não é barrada): 1 SpawnRequested de re-despacho.
    let recs = e.store.events().expect("events");
    assert_eq!(
        count_kind(&recs, "SpawnRequested"),
        1,
        "gate (b): a correção do engolido SAI como re-despacho (não barrada)"
    );
    // e NÃO foi tratada como loop, nem no desfecho do pump nem no log.
    assert!(
        !r.iter()
            .any(|(_, o)| matches!(o, RouteOutcome::LoopDetected)),
        "gate (b): o re-despacho do engolido NÃO é LoopDetected"
    );
    assert!(
        recs.iter()
            .filter(|r| r.kind == "RouteBlocked")
            .all(|r| { r.payload.get("reason").and_then(|v| v.as_str()) != Some("loop_detected") }),
        "gate (b): zero RouteBlocked{{loop_detected}} para a correção legítima"
    );
}

// ════════════════════ GATE (a) DISCRIMINADOR — progresso REAL não é interrompido ════════════════════

/// **(gate a, vertente POSITIVA) um nó que FAZ progresso real após a entrega NÃO é re-despachado nem
/// escalado.** Os gate(a/b) acima provam a vertente NEGATIVA (zero progresso → re-despacha + escala);
/// este fecha o FALSO-POSITIVO complementar — o protocolo não pode interromper trabalho legítimo. O
/// `dispatch_watch` (reusa a `AttentionQueue` W3) DESARMA com um sinal de progresso atribuível:
/// `TokenUsageReported` (todo turno real consome tokens, `attention.rs:410`) — ∨ `MessageRouted{from}`
/// (412) ∨ `NodeStatusChanged{Busy,pty_output}` (421). Setup IDÊNTICO ao gate (a), só que com progresso
/// dentro da janela; mesmo com `pump.tick` repetido muito além da janela, o protocolo NÃO age.
///
/// QUEBRA SE… o protocolo ignorasse o progresso e re-despachasse/escalasse mesmo assim → apareceria um
/// `SpawnRequested{stall-respawn:…}` ou um `GoalEscalated{stalled}` e os asserts cairiam. Não-vacuoso por
/// CONTRASTE: o gate (a) prova que ESTE MESMO setup, SEM o progresso, dispara o re-despacho + escala.
#[test]
fn gate_a_discriminador_progresso_real_nao_e_re_despachado_nem_escalado() {
    let mut e = env("gate-a-disarm");
    let _a = e.sup.register("@A", Some("maestro".into()), sink());
    let b = e.sup.register("@B", Some("dev".into()), sink());
    e.sup.set_status(b, NodeStatus::Idle).expect("B idle");
    seed_item_owned_by_b(&mut e);

    let mut deliver = ok_deliver();
    let janela = lina_core::attention::DELIVERED_NO_PROGRESS_WINDOW_MS;

    // entrega real (arma o vigia) → o nó FAZ progresso DENTRO da janela (TokenUsageReported: turno
    // real consumiu tokens) → desarma; depois fecha o turno (Idle). Caminho real.
    entrega_engolida(&mut e, "assuma a tarefa T1", 1_000); // entrega + Idle; o "engolido" só se faltar progresso
    let b_id = e.sup.node_by_name("@B").expect("@B");
    e.store
        .append(&DomainEvent::TokenUsageReported {
            node: b_id.to_string(),
            tokens: 1_234,
        })
        .expect("progresso real (tokens) dentro da janela");

    // pump.tick REPETIDO, muito além da janela: o protocolo NÃO deve agir em nenhum tick.
    for k in 1..=3u64 {
        let _ = e
            .router
            .pump(&mut e.store, 1_000 + k * janela + 1, &mut deliver);
    }

    let recs = e.store.events().expect("events");
    // ZERO re-despacho de stall (id com o prefixo `stall-respawn:` — o livro-razão do protocolo).
    let stall_respawns = recs
        .iter()
        .filter(|r| r.kind == "SpawnRequested")
        .filter(|r| {
            r.payload
                .get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.starts_with("stall-respawn"))
        })
        .count();
    assert_eq!(
        stall_respawns, 0,
        "progresso real (tokens na janela) NÃO é re-despachado — zero stall-respawn (sem falso-positivo)"
    );
    // ZERO escala por stall.
    let stalled = recs
        .iter()
        .filter(|r| r.kind == "GoalEscalated")
        .filter(|r| r.payload.get("reason").and_then(|v| v.as_str()) == Some("stalled"))
        .count();
    assert_eq!(
        stalled, 0,
        "trabalho legítimo NÃO é escalado — zero GoalEscalated{{stalled}} (o protocolo não mata turno válido)"
    );
}
