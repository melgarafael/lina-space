//! **Repro residual da família #22/#23 — "delivered-REAL-sem-trabalho" (modo B).**
//!
//! O `repro_w4_delivered_sem_trabalho.rs` cobre o modo A (entrega FALSA: `deliver_a2a` injeta num
//! turno fechando — fix W1, prontidão confirmada). Este arquivo cobre o modo RESIDUAL e mais
//! resistente: a entrega é REAL (`MessageDelivered` legítimo), mas o alvo volta a `Idle` **sem produzir
//! nenhum sinal de progresso atribuível** (zero `TokenUsageReported`/`MessageRouted`/`pty_output`). É o
//! "engolido" puro: handoff confirmado ≠ trabalho feito (achados #22/#22b/#23/#23b/#22c — 5 ocorrências).
//!
//! ## Por que NÃO há fix de core para a CAUSA (e o que existe)
//! A causa do modo B é o INNER loop do CLI de terceiro (zero LLM no core — invariante #1): o texto foi
//! entregue ao PTY, o CLI simplesmente não agiu. O core não pode forçar o CLI a trabalhar. O que o core
//! PODE — e faz — é DETECTAR estruturalmente: o alarme W3 [`AttentionKind::DeliveredNoProgress`]
//! (`attention.rs:744`) arma na entrega (`MessageDelivered`, `attention.rs:407`), abre a janela no
//! retorno a `Idle` (`mark_dispatch_idle`, `attention.rs:429`) e dispara após
//! [`DELIVERED_NO_PROGRESS_WINDOW_MS`] sem progresso atribuível.
//!
//! ## O que estes testes provam (e o que falta)
//! Bloco 1 — CAMINHO REAL: `route_message` com entrega legítima loga `MessageDelivered{to}` (a entrega
//! que arma o vigia é a de verdade, não sintética). Bloco 2 — DETECÇÃO: com relógio controlado, o
//! "engolido" vira alarme e a entrega que VIROU trabalho não (zero falso-positivo). O FIX da CAUSA
//! (protocolo "confirme o 1º DomainEvent, não o ack" + re-despacho informado) é rodada dedicada —
//! ver `tasks/epico-f3/repro-22-23.md`.
//!
//! Determinístico (relógio explícito via `observe`, sem thread/sleep → sem flake). Invariante #4.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::attention::DELIVERED_NO_PROGRESS_WINDOW_MS;
use lina_core::{
    AttentionKind, AttentionQueue, DeliveryOutcome, DomainEvent, EventRecord, EventStore,
    MailMessage, Mailbox, NodeId, NodeStatus, RouteOutcome, Router, RouterConfig, Supervisor,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-repro2223-{tag}-{}-{n}", std::process::id()))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

fn count_kind(recs: &[EventRecord], kind: &str) -> usize {
    recs.iter().filter(|r| r.kind == kind).count()
}

fn alarms_for(q: &AttentionQueue, now_ms: u64, node: &str) -> usize {
    q.items(now_ms)
        .into_iter()
        .filter(|i| i.kind == AttentionKind::DeliveredNoProgress && i.node_id == node)
        .count()
}

/// Um `NodeStatusChanged` de retorno a `Idle` (o turno "fechou" — `end_of_response`).
fn went_idle(node: NodeId) -> DomainEvent {
    DomainEvent::NodeStatusChanged {
        node,
        status: NodeStatus::Idle.as_str().to_string(),
        from: NodeStatus::Busy.as_str().to_string(),
        reason: "end_of_response".to_string(),
    }
}

// ════════════════════════ Bloco 1 — a entrega REAL loga MessageDelivered (caminho real) ════════════════════════

/// **CAMINHO REAL (gate (a)): um handoff entregue de verdade loga `MessageDelivered{to:@B}`.** É o
/// evento que ARMA o vigia W3 — a base do repro. Entrega real (alvo pronto), não a falsa do `repro_w4`.
///
/// QUEBRA SE… a entrega não logasse `MessageDelivered` (sem ele o "engolido" jamais seria detectável).
#[test]
fn handoff_real_loga_message_delivered_que_arma_o_vigia() {
    let tmp = unique("real");
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: "repro #22/#23".into(),
            focus_preset: String::new(),
        })
        .unwrap();

    let sup = Arc::new(Supervisor::new());
    let _a = sup.register("@A", None, sink());
    let b = sup.register("@B", None, sink());
    sup.set_status(b, NodeStatus::Idle).expect("B Idle");

    let mut router = Router::with_config(
        Arc::clone(&sup),
        Mailbox::new(&lina),
        RouterConfig::default(),
    );
    let mut deliver = |_t: NodeId, _f: NodeId, _txt: &str| {
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    };
    let m = MailMessage::new("@A", "@B", "handoff", "assuma a tarefa T1");
    let out = router.route_message(&m, &mut store, 1_000, &mut deliver);
    assert!(
        matches!(out, RouteOutcome::Delivered { .. }),
        "entrega REAL (não retida): {out:?}"
    );

    let recs = store.events().unwrap();
    assert_eq!(
        count_kind(&recs, "MessageDelivered"),
        1,
        "a entrega real loga exatamente 1 MessageDelivered (arma o vigia W3)"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

// ════════════════════════ Bloco 2 — o "engolido" é detectável (relógio controlado) ════════════════════════

/// **SINTOMA: entrega REAL + retorno a `Idle` + zero progresso → "engolido" detectável.** Com o mesmo
/// `MessageDelivered` que a entrega real produz (Bloco 1), o alvo volta a `Idle` e NUNCA emite sinal de
/// trabalho. A projeção W3 expõe o alarme `DeliveredNoProgress` após a janela — o Maestro deixa de ser
/// cego ao despacho engolido (achados #22/#23). Relógio controlado via `observe` (determinístico).
///
/// QUEBRA SE… o vigia não armasse na entrega ou a janela não fechasse. Não-vacuoso: ANTES da janela,
/// zero alarme (assert explícito).
#[test]
fn handoff_entregue_que_volta_a_idle_sem_trabalho_e_detectado() {
    let b = NodeId::from_u128(0xB);
    let b_str = b.to_string();
    let t0 = 1_000_000u64;

    let mut q = AttentionQueue::new();
    q.observe(
        &DomainEvent::MessageDelivered {
            id: "m1".into(),
            to: b,
        },
        t0,
    ); // arma
    let idle_ts = t0 + 1_000;
    q.observe(&went_idle(b), idle_ts); // abre o ponto-cego (turno fechou sem trabalho)

    assert_eq!(
        alarms_for(&q, idle_ts + DELIVERED_NO_PROGRESS_WINDOW_MS - 1, &b_str),
        0,
        "antes da janela ({DELIVERED_NO_PROGRESS_WINDOW_MS}ms) NÃO alarma (não-vacuosidade)"
    );
    assert_eq!(
        alarms_for(&q, idle_ts + DELIVERED_NO_PROGRESS_WINDOW_MS, &b_str),
        1,
        "após a janela: 1 alarme DeliveredNoProgress p/ @B (entregue, zero trabalho = #22/#23)"
    );
}

/// **DISCRIMINADOR: a 2ª tentativa que FUNCIONA não alarma.** Quando a entrega vira trabalho real (um
/// `TokenUsageReported` — todo turno real consome tokens), o vigia DESARMA (`attention.rs:411`) e nenhum
/// alarme dispara. É o que separa "engolido" (#22) de "trabalhou" — a propriedade que torna o alarme
/// confiável (zero falso-positivo no caminho feliz).
///
/// QUEBRA SE… o detector ignorasse o progresso (alarme dispararia mesmo com trabalho).
#[test]
fn entrega_que_vira_trabalho_nao_alarma() {
    let b = NodeId::from_u128(0xB);
    let b_str = b.to_string();
    let t0 = 1_000_000u64;

    let mut q = AttentionQueue::new();
    q.observe(
        &DomainEvent::MessageDelivered {
            id: "m1".into(),
            to: b,
        },
        t0,
    ); // arma
    q.observe(
        &DomainEvent::TokenUsageReported {
            node: b.to_string(),
            tokens: 1_234,
        },
        t0 + 500,
    ); // progresso atribuível → DESARMA
    let idle_ts = t0 + 1_000;
    q.observe(&went_idle(b), idle_ts);

    assert_eq!(
        alarms_for(
            &q,
            idle_ts + DELIVERED_NO_PROGRESS_WINDOW_MS + 10_000,
            &b_str
        ),
        0,
        "entrega que VIROU trabalho (tokens consumidos) NÃO alarma — sem falso-positivo"
    );
}
