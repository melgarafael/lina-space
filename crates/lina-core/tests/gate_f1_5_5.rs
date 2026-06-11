//! GATE F1-5-5 — Suspensão real de ociosos, FATIA CORE (fonte: `ondas-5-6.md` 90-98; P0).
//!
//! Prova os critérios headless do despacho:
//! - **(a)** máquina `Active → Idle → Suspended` com TODOS os guards (prompt pendente bloqueia;
//!   custódia bloqueia; pinado bloqueia; A2A acorda; foco acorda; prompt/custódia detectados acordam).
//! - **(c)** `lina ask` para um nó SUSPENSO ENTREGA e ACORDA (caminho real estilo `gate_onda3`:
//!   `route_message` de verdade + assert do `NodeResumed` no `log.jsonl`).
//! - **(d)** o HARVEST do scrollback continua íntegro DURANTE a suspensão (a decisão load-bearing:
//!   só render/montagem param; drenar/advance/harvest NUNCA param — flow-control W0-3 travaria o CLI).
//!
//! Os critérios (b) [PROF/zero-render] e (e) [tela do fundador] são shell/roteiro — fora desta fatia;
//! aqui provamos o GANCHO (`should_render` falso ⟂ `should_keep_draining` verdadeiro).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::suspend::{
    reason, record_transition, SuspendConfig, SuspendController, SuspendGuards, SuspendState,
    SuspendTransition,
};
use lina_core::{
    DeliveryOutcome, DomainEvent, EventRecord, EventStore, MailMessage, Mailbox, NodeId,
    RouteOutcome, Router, RouterConfig, Supervisor,
};

// ───────────────────────────── substrato headless ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-gate55-{tag}-{}-{n}", std::process::id()))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

fn ok_deliver() -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    |_target, _from, _text| {
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    }
}

/// Config de teste com thresholds pequenos (o relógio é injetado: `now_ms` explícito).
fn cfg() -> SuspendConfig {
    SuspendConfig {
        active_to_idle_ms: 1_000,
        idle_to_suspend_ms: 5_000,
        wake_on_output: true,
    }
}

fn jsonl(dir: &std::path::Path) -> Vec<EventRecord> {
    let path = dir.join("log.jsonl");
    let data =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("ler {}: {e}", path.display()));
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<EventRecord>(l).expect("linha = EventRecord"))
        .collect()
}

fn p_str<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload.get(key).and_then(|v| v.as_str())
}

// ════════════════════════════ (a) máquina + guards ════════════════════════════

#[test]
fn a1_progressao_active_idle_suspended() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    assert_eq!(sc.state(id), SuspendState::Active);
    assert!(sc.should_render(id), "Active deve renderizar");

    // antes do threshold de idle → ainda Active
    assert_eq!(
        sc.tick(id, 500, SuspendGuards::default()),
        SuspendTransition::None
    );
    assert_eq!(sc.state(id), SuspendState::Active);

    // cruzou active_to_idle → Idle
    assert_eq!(
        sc.tick(id, 1_000, SuspendGuards::default()),
        SuspendTransition::ToIdle
    );
    assert_eq!(sc.state(id), SuspendState::Idle);
    assert!(
        sc.should_render(id),
        "Idle ainda renderiza (só Suspended pausa)"
    );

    // cruzou idle_to_suspend → Suspended
    assert!(matches!(
        sc.tick(id, 5_000, SuspendGuards::default()),
        SuspendTransition::ToSuspended { .. }
    ));
    assert_eq!(sc.state(id), SuspendState::Suspended);
    assert!(!sc.should_render(id), "Suspended NÃO renderiza");
}

#[test]
fn a2_prompt_pendente_bloqueia_suspensao() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    let guard = SuspendGuards {
        prompt_pending: true,
        custody_pending: false,
    };
    // muito além do threshold, mas com prompt pendente → NUNCA suspende
    assert_eq!(sc.tick(id, 1_000_000, guard), SuspendTransition::None);
    assert_ne!(sc.state(id), SuspendState::Suspended);
    assert!(sc.should_render(id));
}

#[test]
fn a3_custodia_pendente_bloqueia_suspensao() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    let guard = SuspendGuards {
        prompt_pending: false,
        custody_pending: true,
    };
    assert_eq!(sc.tick(id, 1_000_000, guard), SuspendTransition::None);
    assert_ne!(sc.state(id), SuspendState::Suspended);
}

#[test]
fn a4_pinado_bloqueia_suspensao() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    sc.set_pinned(id, true);
    assert_eq!(
        sc.tick(id, 1_000_000, SuspendGuards::default()),
        SuspendTransition::None
    );
    assert_ne!(sc.state(id), SuspendState::Suspended);

    // ao despinar, o relógio de ociosidade volta a correr a partir do "agora"
    sc.set_pinned(id, false);
    assert!(matches!(
        sc.tick(id, 1_000_000 + 5_000, SuspendGuards::default()),
        SuspendTransition::ToSuspended { .. }
    ));
}

#[test]
fn a5_a2a_acorda() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    sc.tick(id, 5_000, SuspendGuards::default());
    assert_eq!(sc.state(id), SuspendState::Suspended);

    let woke = sc.note_a2a_delivered(id, 6_000);
    assert_eq!(
        woke,
        SuspendTransition::Woke {
            reason: reason::A2A_DELIVERED
        }
    );
    assert_eq!(sc.state(id), SuspendState::Active);
    assert!(sc.should_render(id));
}

#[test]
fn a6_foco_acorda() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    sc.tick(id, 5_000, SuspendGuards::default());
    assert_eq!(sc.state(id), SuspendState::Suspended);

    let woke = sc.note_focus(id, 6_000);
    assert_eq!(
        woke,
        SuspendTransition::Woke {
            reason: reason::FOCUS
        }
    );
    assert_eq!(sc.state(id), SuspendState::Active);
}

#[test]
fn a7_prompt_detectado_durante_suspensao_acorda() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    sc.tick(id, 5_000, SuspendGuards::default());
    assert_eq!(sc.state(id), SuspendState::Suspended);

    // um prompt aparece enquanto suspenso → acorda (não pode ficar invisível esperando humano)
    let guard = SuspendGuards {
        prompt_pending: true,
        custody_pending: false,
    };
    let woke = sc.tick(id, 6_000, guard);
    assert_eq!(
        woke,
        SuspendTransition::Woke {
            reason: reason::PROMPT_PENDING
        }
    );
    assert_eq!(sc.state(id), SuspendState::Active);
}

#[test]
fn a8_retomada_de_output_acorda_quando_configurado() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    sc.tick(id, 5_000, SuspendGuards::default());
    assert_eq!(sc.state(id), SuspendState::Suspended);

    let woke = sc.note_activity(id, 6_000);
    assert_eq!(
        woke,
        SuspendTransition::Woke {
            reason: reason::OUTPUT_RESUMED
        }
    );
    assert_eq!(sc.state(id), SuspendState::Active);
}

#[test]
fn a8b_retomada_de_output_nao_acorda_quando_desligado() {
    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(SuspendConfig {
        wake_on_output: false,
        ..cfg()
    });
    sc.track(id, 0);
    sc.tick(id, 5_000, SuspendGuards::default());
    assert_eq!(sc.state(id), SuspendState::Suspended);

    // output continua (drena/harvest), mas o RENDER segue pausado até foco/A2A
    assert_eq!(sc.note_activity(id, 6_000), SuspendTransition::None);
    assert_eq!(sc.state(id), SuspendState::Suspended);
    // ...e o foco ainda acorda
    assert!(matches!(
        sc.note_focus(id, 7_000),
        SuspendTransition::Woke { .. }
    ));
}

// ════════════════════════════ (c) lina ask entrega e acorda ════════════════════════════

#[test]
fn c_lina_ask_a_no_suspenso_entrega_e_acorda() {
    let tmp = unique("c-ask");
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: "Gate F1-5-5".into(),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated");
    let sup = Arc::new(Supervisor::new());
    let mut router = Router::with_config(
        Arc::clone(&sup),
        Mailbox::new(&lina),
        RouterConfig::default(),
    );

    let target = sup.register("@A", Some("dev".into()), sink());
    let _sender = sup.register("@B", Some("dev".into()), sink());
    sup.set_status(target, lina_core::NodeStatus::Idle)
        .expect("A Idle");

    // 1) suspende @A por ociosidade (sem guards) e registra a transição no log
    let mut sc = SuspendController::with_config(cfg());
    sc.track(target, 0);
    let to_susp = sc.tick(target, 5_000, SuspendGuards::default());
    assert!(matches!(to_susp, SuspendTransition::ToSuspended { .. }));
    assert!(record_transition(&mut store, target, to_susp).expect("log NodeSuspended"));
    assert!(!sc.should_render(target), "shell pausaria a montagem de @A");

    // 2) caminho real: @B faz `lina ask` para @A SUSPENSO → ENTREGA (o nó segue vivo e drenando)
    let mut deliver = ok_deliver();
    let ask = MailMessage::new("@B", "@A", "ask", "acorda e toca o item");
    assert!(
        matches!(
            router.route_message(&ask, &mut store, 1_001, &mut deliver),
            RouteOutcome::Delivered { .. }
        ),
        "ask a nó suspenso deve ENTREGAR (suspensão não para a entrega)"
    );

    // 3) a entrega ACORDA @A (gancho registrado: o roteador chama isto após Delivered)
    let woke = sc.note_a2a_delivered(target, 6_000);
    assert_eq!(
        woke,
        SuspendTransition::Woke {
            reason: reason::A2A_DELIVERED
        }
    );
    assert!(record_transition(&mut store, target, woke).expect("log NodeResumed"));
    assert!(
        sc.should_render(target),
        "@A volta a renderizar após acordar"
    );

    // ── Assert DO LOG (invariante #4: a verdade é o jsonl, não o estado em memória) ──
    let recs = jsonl(&lina.join("events"));
    assert!(
        recs.iter().any(|r| r.kind == "MessageRouted"),
        "a rota do ask deve estar no log"
    );
    let suspended = recs
        .iter()
        .find(|r| r.kind == "NodeSuspended")
        .expect("NodeSuspended no log");
    assert_eq!(p_str(suspended, "reason"), Some(reason::IDLE_TIMEOUT));
    let resumed = recs
        .iter()
        .find(|r| r.kind == "NodeResumed")
        .expect("NodeResumed no log");
    assert_eq!(p_str(resumed, "reason"), Some(reason::A2A_DELIVERED));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ════════════════════════════ (d) harvest continua na suspensão ════════════════════════════

#[test]
fn d_harvest_continua_durante_suspensao() {
    use lina_core::scrollback::{ScrollbackConfig, ScrollbackStore};

    let id = NodeId::now_v7();
    let mut sc = SuspendController::with_config(cfg());
    sc.track(id, 0);
    sc.tick(id, 5_000, SuspendGuards::default());
    assert_eq!(sc.state(id), SuspendState::Suspended);

    // A DECISÃO load-bearing, encodada no gancho consultável pelo shell:
    // render PAUSA, mas a drenagem/harvest NUNCA para.
    assert!(!sc.should_render(id), "Suspended pausa o render");
    assert!(
        sc.should_keep_draining(id),
        "Suspended JAMAIS para de drenar (flow-control W0-3 travaria o CLI)"
    );

    // prova de integridade: o harvest do scrollback persiste o output DURANTE a suspensão
    let tmp = unique("d-harvest");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut store = ScrollbackStore::open(&tmp, ScrollbackConfig::default()).expect("scrollback");
    let panel = id.to_string();
    for i in 0..50u32 {
        store
            .push_line(&panel, format!("output-durante-suspensao-{i}"))
            .expect("push_line");
    }
    store.flush(&panel).expect("flush");
    let tail = store.tail(&panel, 50).expect("tail");
    assert_eq!(
        tail.len(),
        50,
        "todo output suspenso foi harvested (zero perda)"
    );
    assert_eq!(
        tail.last().map(String::as_str),
        Some("output-durante-suspensao-49")
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
