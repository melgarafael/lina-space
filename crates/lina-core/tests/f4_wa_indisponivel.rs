//! F4-WA-5 — terminal alvo INDISPONÍVEL no caminho do webhook ativo (entrega ciente de estado).
//!
//! Fonte: spec 55 §Terminal indisponível · ADR 0035 §4 · ADR 0020 (DLQ). O webhook REUSA a
//! maquinaria A2A (F1-0-4 retenção/drain, F1-0-7 retry/DLQ) — não inventa fila própria. Estes testes
//! provam a costura REAL (o `Router` emite os eventos; a `AttentionQueue` os consome do log), não um
//! mock:
//!   (1) alvo BUSY no instante do despacho → `MessageRetained` → drena no Idle (entregue, 0 perda);
//!   (2) alvo MORTO/fechado → `MessageDeadLettered` (durável) + 1 item na fila de atenção (nada some).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use lina_core::{
    now_ms, AttentionKind, AttentionQueue, DeliveryOutcome, EventStore, Mailbox, NodeId,
    NodeStatus, PromptKind, RouteOutcome, Router, RouterConfig, Supervisor,
};

// ───────────────────────────── helpers ─────────────────────────────

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-f4wa5-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&p);
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

fn count_events(store: &EventStore, kind: &str) -> usize {
    store
        .events()
        .expect("events")
        .into_iter()
        .filter(|r| r.kind == kind)
        .count()
}

/// Arena: sup + router + store isolados, `@Dev` (alvo do webhook) registrado e Idle.
fn arena(tag: &str, cfg: RouterConfig) -> (Router, Arc<Supervisor>, EventStore, TempDir, NodeId) {
    let tmp = TempDir::new(tag);
    let sup = Arc::new(Supervisor::new());
    let dev = sup.register("@Dev", None, sink());
    sup.set_status(dev, NodeStatus::Idle).expect("Dev livre");
    let store = EventStore::open(tmp.path().join(".lina/events")).expect("abrir store");
    let mb = Mailbox::new(tmp.path().join(".lina"));
    let router = Router::with_config(Arc::clone(&sup), mb, cfg);
    (router, sup, store, tmp, dev)
}

// ───────── (1) alvo BUSY → retém → drena no Idle (F1-0-4, reusado pelo webhook) ─────────

/// O webhook chega com o terminal OCUPADO (prompt não-pronto): a entrega de sistema RETÉM
/// (`MessageRetained`, nada injetado), e quando o alvo fica livre o pump DRENA (entrega real). Zero
/// perda e zero fila própria — é a mesma retenção F1-0-4 que o A2A usa.
#[test]
fn webhook_busy_target_retains_then_drains_on_idle() {
    let (mut router, _sup, mut store, _tmp, dev) = arena("busy", RouterConfig::default());

    let busy = Rc::new(Cell::new(true));
    let delivered = Rc::new(RefCell::new(Vec::<NodeId>::new()));
    let busy_d = Rc::clone(&busy);
    let delivered_d = Rc::clone(&delivered);
    let mut deliver = move |t: NodeId, _f: NodeId, _x: &str| {
        if busy_d.get() {
            // prompt não-pronto: NADA escrito (mesmo destino que a retenção por estado)
            Ok(DeliveryOutcome::Injected {
                ready: false,
                bracketed: false,
            })
        } else {
            delivered_d.borrow_mut().push(t);
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        }
    };

    let block = "[LINA::WEBHOOK]\n--- DADOS ---\nx\n--- INSTRUÇÃO ---\nfaça X\n[/LINA::WEBHOOK]";
    let out = router.system_deliver(
        dev,
        "@Dev",
        "hk_b",
        "POST",
        "wh_busy",
        block,
        "shaB",
        4,
        &mut store,
        1_000,
        &mut deliver,
    );
    assert_eq!(
        out,
        RouteOutcome::Retained { to: dev },
        "alvo ocupado RETÉM (não descarta, não força)"
    );
    assert!(
        count_events(&store, "MessageRetained") >= 1,
        "retenção logada"
    );
    assert_eq!(
        count_events(&store, "MessageDelivered"),
        0,
        "nada entregue ainda"
    );
    assert!(
        delivered.borrow().is_empty(),
        "nenhuma injeção enquanto ocupado"
    );

    // Alvo fica LIVRE → o pump drena a retenção no Idle (backoff de retenção é 2 s; avanço com folga).
    busy.set(false);
    for tick in 1..=6u64 {
        let _ = router.pump(&mut store, 1_000 + tick * 3_000, &mut deliver);
        if count_events(&store, "MessageDelivered") >= 1 {
            break;
        }
    }
    assert_eq!(
        count_events(&store, "MessageDeadLettered"),
        0,
        "drenou no Idle: jamais foi à DLQ"
    );
    assert_eq!(
        count_events(&store, "MessageDelivered"),
        1,
        "drena no Idle → entregue exatamente 1× (0 perda)"
    );
    assert_eq!(
        delivered.borrow().as_slice(),
        &[dev],
        "injetado no alvo certo"
    );

    // E a fila de atenção NÃO acusa DLQ (não houve perda).
    let q = AttentionQueue::replay(&store.events().expect("events"));
    let dl = q
        .items(50_000)
        .into_iter()
        .filter(|i| i.kind == AttentionKind::DeadLetter)
        .count();
    assert_eq!(dl, 0, "Busy→drena não gera notificação de DLQ");
}

// ───────── (2) alvo MORTO → DLQ durável + NOTIFICA o usuário (ADR 0035 §4 + ADR 0020) ─────────

/// O webhook chega para um terminal MORTO/fechado: a injeção sempre falha. Reusando o motor de retry
/// (F1-0-7), a entrega esgota as tentativas e aterrissa na DLQ durável (`MessageDeadLettered`) — e,
/// crucialmente (o que o WA5 adiciona), a `AttentionQueue` reconstruída do log NOTIFICA o usuário:
/// exatamente 1 item, não-aprovável (retry é manual/`--resume`). Nada some em silêncio (0 perda).
///
/// **F4-WA-5 (resolvido):** bug ALTA caçado por J — `mailbox::valid_msg_id` só aceitava `msg_` e
/// recusava o `delivery_id` `wh_<uuid>` do webhook, fazendo o DLQ a alvo morto sumir em silêncio
/// (violava 0-perda, gate (e)). O fix (`valid_msg_id` aceitar `wh_`, MESMO charset) foi ligado pelo
/// integrador (Maestro 01) com prova RED→GREEN + suíte de segurança verde. Roda no gate do core.
#[test]
fn webhook_dead_target_dead_letters_and_notifies_attention() {
    // Breaker fora do caminho (99); DLQ por esgotamento em 3 tentativas; backoff curto.
    let cfg = RouterConfig {
        delivery_max_attempts: 3,
        delivery_backoff_base_ms: 1,
        breaker_threshold: 99,
        ..RouterConfig::default()
    };
    let (mut router, _sup, mut store, _tmp, dev) = arena("dead", cfg);

    // Alvo MORTO: o PTY está fechado → toda injeção erra.
    let mut dead_deliver =
        |_t: NodeId, _f: NodeId, _x: &str| Err("pty fechado (alvo morto)".to_string());

    let out = router.dispatch_webhook(
        dev,
        "@Dev",
        "hk_d",
        "POST",
        "application/json",
        "{\"ok\":1}",
        "faça X com os dados do POST",
        "sha256d",
        8,
        5_000, // received_ts
        &mut store,
        10_000, // now_ms
        &mut dead_deliver,
    );
    assert!(
        matches!(out, RouteOutcome::RetryBackoff { .. }),
        "1ª falha agenda retry (não perde de cara): {out:?}"
    );

    // Esgota as tentativas pelo pump (relógio bem à frente do backoff) → DLQ.
    let mut now = 10_000u64;
    for _ in 0..30 {
        if count_events(&store, "MessageDeadLettered") >= 1 {
            break;
        }
        now += 1_000_000;
        let _ = router.pump(&mut store, now, &mut dead_deliver);
    }
    assert_eq!(
        count_events(&store, "MessageDeadLettered"),
        1,
        "alvo morto → exatamente 1 MessageDeadLettered (durável, nada some)"
    );

    // A COSTURA: a fila de atenção, reconstruída SÓ do log, notifica o usuário.
    let q = AttentionQueue::replay(&store.events().expect("events"));
    let items = q.items(now);
    let dl: Vec<_> = items
        .iter()
        .filter(|i| i.kind == AttentionKind::DeadLetter)
        .collect();
    assert_eq!(dl.len(), 1, "morto → 1 item na fila de atenção (0 perda)");
    assert_ne!(
        dl[0].prompt_kind,
        PromptKind::Yn,
        "DLQ não é aprovável na fila (retry é manual/--resume, jamais y/n)"
    );
    // Zero jargão na superfície: o detail leigo não vaza termo técnico nem o id de entrega.
    let detail = dl[0].detail.clone().unwrap_or_default();
    assert!(
        !detail.to_lowercase().contains("dlq")
            && !detail.to_lowercase().contains("dead")
            && !detail.contains("wh_"),
        "detail é copy leiga (sem jargão/id): {detail:?}"
    );
}
