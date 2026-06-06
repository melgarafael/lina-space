//! GATE F1-0-7 — idempotência durável + dead-letter queue + circuit breaker (ADR 0020).
//!
//! Critérios da story (tasks/epico-f1/ondas-0-1.md §F1-0-7):
//! (1) "kill -9" no meio de N entregas + replay → **0 efeitos duplicados** (exactly-once
//!     por idempotency key — o ledger de entrega derivado do log);
//! (2) alvo morto/entrega falhando → DLQ após N tentativas, com timestamps no log
//!     provando **backoff exponencial + jitter**, e motivo legível;
//! (3) N falhas consecutivas → `CircuitOpened` + `Blocked`; novas msgs NEM tentam
//!     (retêm) até reset externo;
//! (4) ADR 0020 em docs/adr/ (verificado por existência/conteúdo no teste g);
//! (5) suíte do router verde (este arquivo soma a ela) + append concorrente (lição do
//!     projeto: "quem mais escreve nesse caminho?").

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use lina_core::{
    now_ms, DeliveryOutcome, DomainEvent, EventStore, MailMessage, Mailbox, NodeId, NodeStatus,
    RouteOutcome, Router, RouterConfig, Supervisor,
};

// ───────────────────────── helpers ─────────────────────────

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-gatef107-{tag}-{}-{}",
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

fn events_of(store: &EventStore, kind: &str) -> Vec<lina_core::EventRecord> {
    store
        .events()
        .expect("events")
        .into_iter()
        .filter(|r| r.kind == kind)
        .collect()
}

/// Arena: sup + router + store isolados, @A (origem) e @B (alvo) registrados.
fn arena(tag: &str, cfg: RouterConfig) -> (Router, Arc<Supervisor>, EventStore, TempDir, NodeId) {
    let tmp = TempDir::new(tag);
    let sup = Arc::new(Supervisor::new());
    sup.register("@A", None, sink());
    let b = sup.register("@B", None, sink());
    sup.set_status(b, NodeStatus::Idle).expect("B livre");
    let store = EventStore::open(tmp.path().join(".lina/events")).expect("abrir store");
    let mb = Mailbox::new(tmp.path().join(".lina"));
    let router = Router::with_config(Arc::clone(&sup), mb, cfg);
    (router, sup, store, tmp, b)
}

/// Deliver chaveável: falha enquanto `fail=true`; registra os textos entregues.
type Delivered = Rc<std::cell::RefCell<Vec<(NodeId, String)>>>;
fn switchable_deliver(
    fail: Rc<Cell<bool>>,
    delivered: Delivered,
) -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    move |target, _from, text| {
        if fail.get() {
            Err("pty fechado (simulado)".to_string())
        } else {
            delivered.borrow_mut().push((target, text.to_string()));
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        }
    }
}

// ───────────────────── (1) exactly-once sob crash no meio de N ─────────────────────

/// "kill -9" no meio de N entregas: simulamos o pior crash A6 (entrega feita, ack NÃO —
/// o arquivo volta ao `.inflight`) + restart (router NOVO, memória zerada). O ledger do
/// log garante: cada id entregue EXATAMENTE 1× (0 efeitos duplicados), e o que não foi
/// entregue continua o curso normal.
#[test]
fn a_exactly_once_apos_crash_no_meio_de_n_entregas() {
    let (mut router, sup, mut store, tmp, b) = arena("exonce", RouterConfig::default());
    let fail = Rc::new(Cell::new(false));
    let delivered: Delivered = Rc::default();
    let mut deliver = switchable_deliver(Rc::clone(&fail), Rc::clone(&delivered));
    let outbox = Mailbox::new(tmp.path().join(".lina"));

    // N=3 mensagens A→B no canal real (outbox por-nó).
    let msgs: Vec<MailMessage> = (0..3)
        .map(|i| MailMessage::new("@A", "@B", "ask", format!("tarefa {i}")))
        .collect();
    for m in &msgs {
        outbox.enqueue_as("@A", m).expect("enqueue");
    }

    // Tick 1: m0 entrega (B fica Busy pela serialização F1-0-4); m1/m2 retidas.
    let r1 = router.pump(&mut store, 1_000, &mut deliver);
    assert_eq!(
        r1.iter()
            .filter(|(_, o)| matches!(o, RouteOutcome::Delivered { .. }))
            .count(),
        1
    );

    // CRASH simulado no pior ponto (A6): m0 foi entregue+logada mas o ack "se perdeu" —
    // o arquivo dela REAPARECE no `.inflight` (re-escrevemos o JSON dela lá).
    let inflight = tmp.path().join(".lina/outbox/.inflight");
    std::fs::create_dir_all(&inflight).expect("inflight dir");
    std::fs::write(
        inflight.join(format!("{}.json", msgs[0].id)),
        serde_json::to_vec(&msgs[0]).expect("json"),
    )
    .expect("re-plantar órfão");
    drop(router); // restart: estado em memória zerado

    let mut router2 = Router::with_config(
        Arc::clone(&sup),
        Mailbox::new(tmp.path().join(".lina")),
        RouterConfig::default(),
    );
    // B terminou o turno de m0 (lifecycle real faria) → m1 pode entregar.
    sup.set_status(b, NodeStatus::Idle).expect("idle");
    let r2 = router2.pump(&mut store, 2_000, &mut deliver);
    // O órfão de m0 vira no-op com resultado conhecido (Duplicate) — NUNCA re-injeta.
    assert!(
        r2.iter()
            .any(|(id, o)| *id == msgs[0].id && matches!(o, RouteOutcome::Duplicate)),
        "órfão re-apresentado é deduplicado pelo ledger durável; r2={r2:?}"
    );

    // Drena o resto, terminando o turno entre entregas (serialização F1-0-4).
    for t in [3_000u64, 4_000, 5_000] {
        sup.set_status(b, NodeStatus::Idle).expect("idle");
        let _ = router2.pump(&mut store, t, &mut deliver);
    }

    // EXACTLY-ONCE por idempotency key: 3 injeções no PTY e 3 MessageDelivered, 1 por id.
    assert_eq!(
        delivered.borrow().len(),
        3,
        "3 injeções no total (0 duplicadas)"
    );
    let recs = events_of(&store, "MessageDelivered");
    assert_eq!(recs.len(), 3);
    let mut per_id: HashMap<&str, usize> = HashMap::new();
    for r in &recs {
        *per_id
            .entry(r.payload.get("id").and_then(|v| v.as_str()).unwrap_or(""))
            .or_insert(0) += 1;
    }
    for m in &msgs {
        assert_eq!(
            per_id.get(m.id.as_str()),
            Some(&1),
            "id {} exatamente 1×",
            m.id
        );
    }
}

// ───────────────── (2a) falha de entrega → backoff+jitter → DLQ ─────────────────

/// Entrega falhando SEMPRE: as tentativas são agendadas com backoff EXPONENCIAL+jitter
/// (provado pelos `next_retry_ms` dos `MessageDeliveryFailed` no log) e, esgotadas,
/// a mensagem vai à DLQ durável com motivo legível. Nada é dropado em silêncio.
#[test]
fn b_falha_persistente_faz_backoff_exponencial_e_dlq() {
    let cfg = RouterConfig {
        delivery_backoff_base_ms: 1_000,
        delivery_max_attempts: 5,
        breaker_threshold: 99, // breaker fora do caminho neste teste (testado em (3))
        ..RouterConfig::default()
    };
    let (mut router, _sup, mut store, tmp, b) = arena("backoff", cfg);
    let fail = Rc::new(Cell::new(true));
    let delivered: Delivered = Rc::default();
    let mut deliver = switchable_deliver(Rc::clone(&fail), Rc::clone(&delivered));
    let outbox = Mailbox::new(tmp.path().join(".lina"));

    let msg = MailMessage::new("@A", "@B", "ask", "nunca chega");
    outbox.enqueue_as("@A", &msg).expect("enqueue");

    // Tick inicial: rota ok, ENTREGA falha → attempt 1 agendado.
    let mut now = 100_000u64;
    let r = router.pump(&mut store, now, &mut deliver);
    assert!(
        r.iter()
            .any(|(_, o)| matches!(o, RouteOutcome::RetryBackoff { attempt: 1, to } if *to == b)),
        "1ª falha agenda retry; r={r:?}"
    );
    // Avança o relógio até cada agendamento e re-pumpa, até a DLQ.
    for _ in 0..6 {
        let fails = events_of(&store, "MessageDeliveryFailed");
        let Some(last) = fails.last() else { break };
        let next = last
            .payload
            .get("next_retry_ms")
            .and_then(serde_json::Value::as_u64)
            .expect("next_retry_ms");
        now = next + 1;
        let _ = router.pump(&mut store, now, &mut deliver);
        if !events_of(&store, "MessageDeadLettered").is_empty() {
            break;
        }
    }

    // Backoff EXPONENCIAL + jitter provados pelos eventos: para cada attempt k,
    // delay_k = next_k − (agendado em) ∈ [base·2^(k−1), base·2^(k−1)·1.25 + 1].
    let fails = events_of(&store, "MessageDeliveryFailed");
    assert_eq!(fails.len(), 4, "attempts 1..4 logados (o 5º vira DLQ)");
    for (i, rec) in fails.iter().enumerate() {
        let attempt = rec
            .payload
            .get("attempt")
            .and_then(serde_json::Value::as_u64)
            .expect("attempt");
        assert_eq!(attempt, i as u64 + 1, "attempts em ordem 1..4");
        let exp = 1_000u64 << i;
        let next = rec
            .payload
            .get("next_retry_ms")
            .and_then(serde_json::Value::as_u64)
            .expect("next");
        // O agendamento foi feito num `now` conhecido só indiretamente; o DELAY mínimo
        // garantido é exp e o máximo exp+exp/4 (jitter determinístico) — derivamos o
        // delay do próprio protocolo do teste: now da rodada = next_{k-1}+1 (k>1).
        let scheduled_at = if i == 0 {
            100_000
        } else {
            fails[i - 1]
                .payload
                .get("next_retry_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap()
                + 1
        };
        let delay = next - scheduled_at;
        assert!(
            delay >= exp && delay <= exp + exp / 4 + 1,
            "attempt {attempt}: delay {delay} ms fora da banda exponencial+jitter \
             [{exp}, {}]",
            exp + exp / 4 + 1
        );
    }

    // DLQ: evento com motivo legível + arquivo durável + .inflight confirmado (vazio).
    let dead = events_of(&store, "MessageDeadLettered");
    assert_eq!(dead.len(), 1);
    let reason = dead[0]
        .payload
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("reason");
    assert!(
        reason.contains("esgotadas") && reason.contains("pty fechado"),
        "motivo legível com o último erro; veio: {reason}"
    );
    let dlq = Mailbox::new(tmp.path().join(".lina"));
    let letters = dlq.dead_letters().expect("ler DLQ");
    assert_eq!(letters.len(), 1, "projeção visível da DLQ");
    assert_eq!(letters[0].id, msg.id);
    assert_eq!(
        letters[0].payload, "nunca chega",
        "o CORPO original preservado"
    );
    assert!(delivered.borrow().is_empty(), "nada foi injetado");
    let inflight = tmp.path().join(".lina/outbox/.inflight");
    assert_eq!(
        std::fs::read_dir(&inflight).map(|d| d.count()).unwrap_or(0),
        0,
        "inflight confirmado após a DLQ (a casa da msg agora é a dead-letter/)"
    );
}

// ───────────────── (2b) destino inexistente persistente → DLQ ─────────────────

/// Alvo que NUNCA resolve (morto/inexistente): esgotado o retry transiente, a mensagem
/// NÃO é mais dropada em silêncio — vai à DLQ com motivo (visível e recuperável).
#[test]
fn c_destino_inexistente_persistente_vai_para_dlq() {
    let (mut router, _sup, mut store, tmp, _b) = arena("ghost", RouterConfig::default());
    let fail = Rc::new(Cell::new(false));
    let delivered: Delivered = Rc::default();
    let mut deliver = switchable_deliver(fail, Rc::clone(&delivered));
    let outbox = Mailbox::new(tmp.path().join(".lina"));

    let msg = MailMessage::new("@A", "@Fantasma", "ask", "ninguém mora aí");
    outbox.enqueue_as("@A", &msg).expect("enqueue");

    // Pumpa além do teto transiente (50): a última re-tentativa converte em DLQ.
    let mut dead = Vec::new();
    for t in 0..60u64 {
        let _ = router.pump(&mut store, 1_000 + t, &mut deliver);
        dead = events_of(&store, "MessageDeadLettered");
        if !dead.is_empty() {
            break;
        }
    }
    assert_eq!(dead.len(), 1, "destino ausente persistente termina na DLQ");
    let reason = dead[0]
        .payload
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("reason");
    assert!(
        reason.contains("indisponivel") && reason.contains("re-tentativas"),
        "motivo legível; veio: {reason}"
    );
    let letters = Mailbox::new(tmp.path().join(".lina"))
        .dead_letters()
        .expect("DLQ");
    assert_eq!(letters.len(), 1);
    assert!(delivered.borrow().is_empty());
}

// ───────────────── (3) circuit breaker por nó ─────────────────

/// 5 falhas CONSECUTIVAS → `CircuitOpened` + nó `Blocked{circuit_breaker}`; novas
/// mensagens NEM TENTAM entregar (retidas com `circuit_open`). Reset EXTERNO (lifecycle/
/// humano tira de Blocked) fecha o breaker e a entrega volta.
#[test]
fn d_circuit_breaker_abre_bloqueia_e_reseta() {
    let cfg = RouterConfig {
        delivery_backoff_base_ms: 100,
        delivery_max_attempts: 99, // DLQ fora do caminho (testada em (2))
        breaker_threshold: 5,
        ..RouterConfig::default()
    };
    let (mut router, sup, mut store, tmp, b) = arena("breaker", cfg);
    let fail = Rc::new(Cell::new(true));
    let delivered: Delivered = Rc::default();
    let mut deliver = switchable_deliver(Rc::clone(&fail), Rc::clone(&delivered));
    let outbox = Mailbox::new(tmp.path().join(".lina"));

    let msg = MailMessage::new("@A", "@B", "ask", "vai falhar");
    outbox.enqueue_as("@A", &msg).expect("enqueue");

    // Falha 5× (1 da rota + retries devidos): breaker abre.
    let mut now = 1_000u64;
    let _ = router.pump(&mut store, now, &mut deliver);
    for _ in 0..4 {
        let fails = events_of(&store, "MessageDeliveryFailed");
        let next = fails
            .last()
            .and_then(|r| r.payload.get("next_retry_ms"))
            .and_then(serde_json::Value::as_u64)
            .expect("next");
        now = next + 1;
        let _ = router.pump(&mut store, now, &mut deliver);
    }
    let opened = events_of(&store, "CircuitOpened");
    assert_eq!(opened.len(), 1, "CircuitOpened no log (1×)");
    assert_eq!(
        opened[0]
            .payload
            .get("failures")
            .and_then(serde_json::Value::as_u64),
        Some(5)
    );
    assert_eq!(
        sup.get(b).map(|i| i.status),
        Some(NodeStatus::Blocked),
        "nó Blocked pelo breaker (consultável via roster/agents.json/`lina list`)"
    );
    assert!(
        events_of(&store, "NodeStatusChanged").iter().any(|r| {
            r.payload.get("reason").and_then(|v| v.as_str()) == Some("circuit_breaker")
        }),
        "motivo `circuit_breaker` no log (o que o futuro `lina check`/F1-0-6 exibe)"
    );

    // Nova mensagem ao nó com breaker aberto: NEM TENTA (retida com circuit_open).
    let probe = MailMessage::new("@A", "@B", "ask", "nem tente");
    outbox.enqueue_as("@A", &probe).expect("enqueue probe");
    let before = events_of(&store, "MessageDeliveryFailed").len();
    let r = router.pump(&mut store, now + 10, &mut deliver);
    assert!(
        r.iter()
            .any(|(id, o)| *id == probe.id && matches!(o, RouteOutcome::Retained { .. })),
        "msg nova é RETIDA sem tentativa; r={r:?}"
    );
    assert_eq!(
        events_of(&store, "MessageDeliveryFailed").len(),
        before,
        "ZERO novas tentativas com o breaker aberto (anti retry-storm)"
    );
    assert!(
        events_of(&store, "MessageRetained").iter().any(|r| r
            .payload
            .get("reason")
            .and_then(|v| v.as_str())
            == Some("circuit_open")),
        "retenção com motivo circuit_open no log"
    );

    // RESET externo: humano/lifecycle tira o nó de Blocked → breaker fecha → entrega volta.
    fail.set(false);
    sup.set_status(b, NodeStatus::Idle).expect("reset externo");
    let _ = router.pump(&mut store, now + 20, &mut deliver);
    let _ = router.pump(&mut store, now + 30, &mut deliver);
    assert!(
        delivered
            .borrow()
            .iter()
            .any(|(_, text)| text.contains("nem tente")),
        "após o reset, a fila retida volta a entregar"
    );
}

// ───────────────── (durabilidade) backoff sobrevive a restart ─────────────────

/// O agendamento de retry é DURÁVEL: um restart no meio do backoff retoma do attempt e
/// do horário corretos (re-derivados dos `MessageDeliveryFailed` do log) — não volta ao
/// attempt 1, não re-injeta, não dropa.
#[test]
fn e_backoff_e_durevel_atravessa_restart() {
    let cfg = RouterConfig {
        delivery_backoff_base_ms: 1_000,
        delivery_max_attempts: 5,
        breaker_threshold: 99,
        ..RouterConfig::default()
    };
    let (mut router, sup, mut store, tmp, b) = arena("durable", cfg);
    let fail = Rc::new(Cell::new(true));
    let delivered: Delivered = Rc::default();
    let mut deliver = switchable_deliver(Rc::clone(&fail), Rc::clone(&delivered));
    let outbox = Mailbox::new(tmp.path().join(".lina"));

    let msg = MailMessage::new("@A", "@B", "ask", "persisto");
    outbox.enqueue_as("@A", &msg).expect("enqueue");

    // 2 falhas (attempt 1 na rota; attempt 2 no retry devido).
    let _ = router.pump(&mut store, 10_000, &mut deliver);
    let next1 = events_of(&store, "MessageDeliveryFailed")[0]
        .payload
        .get("next_retry_ms")
        .and_then(serde_json::Value::as_u64)
        .expect("next1");
    let _ = router.pump(&mut store, next1 + 1, &mut deliver);
    assert_eq!(events_of(&store, "MessageDeliveryFailed").len(), 2);

    // RESTART no meio do backoff do attempt 2.
    drop(router);
    let cfg2 = RouterConfig {
        delivery_backoff_base_ms: 1_000,
        delivery_max_attempts: 5,
        breaker_threshold: 99,
        ..RouterConfig::default()
    };
    let mut router2 = Router::with_config(
        Arc::clone(&sup),
        Mailbox::new(tmp.path().join(".lina")),
        cfg2,
    );
    let next2 = events_of(&store, "MessageDeliveryFailed")[1]
        .payload
        .get("next_retry_ms")
        .and_then(serde_json::Value::as_u64)
        .expect("next2");

    // ANTES do horário: re-apresentada → re-derivada do log no attempt 2, SEM tentar.
    let r = router2.pump(&mut store, next2 - 1, &mut deliver);
    assert!(
        r.iter().any(|(id, o)| *id == msg.id
            && matches!(o, RouteOutcome::RetryBackoff { attempt: 2, to } if *to == b)),
        "restart retoma do attempt 2 (durável via log); r={r:?}"
    );
    assert_eq!(
        events_of(&store, "MessageDeliveryFailed").len(),
        2,
        "nenhuma tentativa antes do horário agendado"
    );

    // NO horário, com a entrega sã: entrega 1× e fecha.
    fail.set(false);
    let _ = router2.pump(&mut store, next2 + 1, &mut deliver);
    assert_eq!(delivered.borrow().len(), 1, "entregue exatamente 1×");
    assert_eq!(events_of(&store, "MessageDelivered").len(), 1);
}

// ───────────────── (5) quem mais escreve aqui? — append concorrente ─────────────────

/// Lição do projeto (W5): dois escritores no MESMO event store (2 handles = 2 conexões,
/// como 2 processos) apendando concorrentemente → zero "database is locked", zero perda.
#[test]
fn f_append_concorrente_de_dois_handles_nao_perde_nem_trava() {
    let tmp = TempDir::new("concurrent");
    let dir = tmp.path().join(".lina/events");
    let mut a = EventStore::open(&dir).expect("handle A");
    let b = std::thread::spawn({
        let dir = dir.clone();
        move || {
            let mut b = EventStore::open(&dir).expect("handle B");
            for i in 0..50u32 {
                b.append(&DomainEvent::TokenUsageReported {
                    node: format!("@B{i}"),
                    tokens: u64::from(i),
                })
                .expect("append B");
            }
        }
    });
    for i in 0..50u32 {
        a.append(&DomainEvent::TokenUsageReported {
            node: format!("@A{i}"),
            tokens: u64::from(i),
        })
        .expect("append A");
    }
    b.join().expect("thread B");

    let reopened = EventStore::open(&dir).expect("reabrir");
    let total = reopened
        .events()
        .expect("events")
        .iter()
        .filter(|r| r.kind == "TokenUsageReported")
        .count();
    assert_eq!(
        total, 100,
        "100 appends concorrentes presentes (0 perdidos)"
    );
    let _ = reopened.project().expect("replay íntegro");
}

// ───────────────── (4) ADR 0020 registrado ─────────────────

/// O ADR formal exigido pelo critério 4 existe, na série do arquiteto, e registra a
/// decisão "recovery por event log + idempotência; sem durable execution full".
#[test]
fn g_adr_0020_registrado() {
    let adr = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/adr/0020-recovery-por-event-log-sem-durable-execution.md");
    let body = std::fs::read_to_string(&adr)
        .unwrap_or_else(|e| panic!("ADR 0020 ausente em {}: {e}", adr.display()));
    for needle in [
        "event log",
        "idempot",
        "Temporal",
        "dead-letter",
        "circuit breaker",
        "Aceito",
    ] {
        assert!(
            body.to_lowercase().contains(&needle.to_lowercase()),
            "ADR 0020 deve registrar {needle:?}"
        );
    }
}
