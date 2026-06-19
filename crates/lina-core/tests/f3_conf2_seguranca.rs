//! **F3-CONF-2 · Segurança do protocolo anti-engolimento — suíte adversarial (QA/Red-team).**
//!
//! O protocolo anti-engolimento (CONF-CORE) toca o caminho MAIS sensível da orquestração:
//! re-despacho, escala de recurso e reset do circuit breaker. Se ele virar um PORTAL — um agente
//! forjando `GoalEscalated`, auto-resetando o próprio breaker, ou afrouxando o limiar do breaker por
//! conta própria — trocamos um bug de confiabilidade (família #22/#23) por um buraco de SEGURANÇA.
//! Esta suíte é a rede que garante a invariante-mãe da fase (onda §"Invariante que ATRAVESSA"):
//!
//!   **reason / escala / effort / veredito é DADO transportado, JAMAIS autoridade.**
//!   `by`/`reason`/`origin` são carimbados SERVER-SIDE (família ADR 0007); ação que gasta/libera
//!   recurso (reset de breaker) exige GESTO HUMANO — um agente não se auto-autoriza.
//!
//! Régua (memória *provar enforcement por mutação* + dispatch §2): caminho REAL (`route_message`/
//! `pump`, nunca o evento montado à mão — senão o teste não prova a costura) + leitura do EVENT LOG
//! (nunca o `RouteOutcome` em memória) + cláusula **QUEBRA SE…** por teste (não-vacuosidade). A
//! prova por mutação (desligar a guarda → RED → religar) é o ritual de verificação registrado no
//! relatório; o entregável é o teste verde com produção intacta.
//!
//! NÃO re-prova o já coberto (memória *não re-provar o coberto*): `f3_1_goal_adversarial.rs` +
//! `f3_2_seguranca_loop.rs` cobrem `by`/`reviewer`/`origin`/`acceptance` no ciclo da Goal; os testes
//! internos `f3_2_5_*` (router.rs) cobrem a escada de effort + breaker sticky. Esta suíte cobre o GAP
//! NOVO: o reset do breaker e a forge-resistência do `GoalEscalated{stalled}`.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    DeliveryOutcome, DomainEvent, EventRecord, EventStore, MailMessage, Mailbox, NodeId,
    NodeStatus, RouteOutcome, Router, RouterConfig, Supervisor,
};

// ───────────────────────────── substrato headless ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

/// tmpdir único por (processo, THREAD, contador) — `thread::id` mata o flaky sistêmico que envenena o
/// gate paralelo (dispatch §3 / memória *lina-gpui: testes flaky*; molde de `router.rs` tmp por-thread).
fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "lina-conf2sec-{tag}-{}-{tid}-{n}",
        std::process::id()
    ))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

fn events_of(store: &EventStore, kind: &str) -> Vec<EventRecord> {
    store
        .events()
        .expect("events")
        .into_iter()
        .filter(|r| r.kind == kind)
        .collect()
}

fn count_kind(store: &EventStore, kind: &str) -> usize {
    events_of(store, kind).len()
}

fn field<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload.get(key).and_then(|v| v.as_str())
}

/// `deliver` que SEMPRE entrega (alvo pronto) — o objeto destes testes é o EVENTO no log.
fn ok_deliver() -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    |_t, _f, _txt| {
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    }
}

/// `deliver` chaveável: falha enquanto `fail=true` (alimenta o breaker); senão entrega.
fn switchable(
    fail: Rc<Cell<bool>>,
) -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    move |_t, _f, _txt| {
        if fail.get() {
            Err("pty fechado (simulado)".into())
        } else {
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        }
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

fn env_with(tag: &str, cfg: RouterConfig) -> Env {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: format!("F3-CONF-2 segurança — {tag}"),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated");
    let sup = Arc::new(Supervisor::new());
    let router = Router::with_config(Arc::clone(&sup), Mailbox::new(&lina), cfg);
    Env {
        tmp,
        store,
        sup,
        router,
    }
}

/// Abre o circuit breaker de `target` por N falhas consecutivas de entrega (molde
/// `gate_f1_0_7::d_circuit_breaker_abre_bloqueia_e_reseta`). Devolve quando `CircuitOpened` foi logado.
fn open_breaker(e: &mut Env, target: NodeId, fail: &Rc<Cell<bool>>) {
    let mut deliver = switchable(Rc::clone(fail));
    let outbox = Mailbox::new(e.tmp.join(".lina"));
    let m = MailMessage::new("@A", "@B", "ask", "vai falhar");
    outbox.enqueue_as("@A", &m).expect("enqueue");
    let mut now = 1_000u64;
    let _ = e.router.pump(&mut e.store, now, &mut deliver);
    // re-pumpa avançando o relógio até cada retry agendado, até o breaker abrir (limite folgado:
    // breaker_threshold=3 nesta arena; 12 ticks cobrem a 1ª falha + retries com sobra).
    for _ in 0..12 {
        if count_kind(&e.store, "CircuitOpened") >= 1 {
            break;
        }
        let next = events_of(&e.store, "MessageDeliveryFailed")
            .last()
            .and_then(|r| r.payload.get("next_retry_ms").and_then(|v| v.as_u64()));
        now = match next {
            Some(n) => n + 1,
            None => now + 1_000,
        };
        let _ = e.router.pump(&mut e.store, now, &mut deliver);
    }
    assert_eq!(
        count_kind(&e.store, "CircuitOpened"),
        1,
        "pré-condição: breaker de {target} ABERTO (1 CircuitOpened)"
    );
    assert_eq!(
        e.sup.get(target).map(|i| i.status),
        Some(NodeStatus::Blocked),
        "pré-condição: {target} Blocked pelo breaker"
    );
}

/// O alvo continua com o breaker ABERTO? Prova comportamental: uma NOVA msg ao alvo é RETIDA com
/// `circuit_open` (nem tenta entregar), e o nó segue `Blocked`. (Não há evento `CircuitClosed`; a
/// verdade é o comportamento do roteador, não um carimbo.)
fn breaker_ainda_aberto(e: &mut Env, target: NodeId, probe_txt: &str, now: u64) -> bool {
    let mut deliver = ok_deliver(); // entrega sã: se NÃO retiver, é porque o breaker fechou
    let outbox = Mailbox::new(e.tmp.join(".lina"));
    let probe = MailMessage::new("@A", "@B", "ask", probe_txt);
    outbox.enqueue_as("@A", &probe).expect("enqueue probe");
    let r = e.router.pump(&mut e.store, now, &mut deliver);
    let retida = r.iter().any(|(id, o)| {
        *id == probe.id && matches!(o, RouteOutcome::Retained { to } if *to == target)
    });
    let blocked = e.sup.get(target).map(|i| i.status) == Some(NodeStatus::Blocked);
    retida && blocked
}

// ════════════════════════ S1 — GoalEscalated é server-side: agente não FORJA ════════════════════════

/// **(gate e) `GoalEscalated` só nasce SERVER-SIDE — não há rota para um agente forjá-lo.** O protocolo
/// anti-engolimento vai emitir `GoalEscalated{reason:"stalled"}` quando o re-despacho não fecha o ciclo.
/// Um agente NÃO pode plantar esse evento: (a) `goal.escalate` não é um verbo de Goal (`is_goal_intent`
/// = só define/interpret/confirm, `router.rs:2902`) → vira mensagem comum, ZERO evento de Goal; (b)
/// injetar `reason:"stalled"` num verbo legítimo (`goal.confirm`) é DADO ignorado — emite `GoalConfirmed`
/// (`by` server-side), JAMAIS um `GoalEscalated`. A escalada é decisão do OUTER loop, nunca do payload.
///
/// QUEBRA SE… `handle_goal` ganhasse um braço que lê `payload["reason"]` para emitir `GoalEscalated`
/// (ou tratasse "goal.escalate" como verbo) → apareceria `GoalEscalated{stalled}` e o `is_empty` cairia.
/// Não-vacuoso: o verbo legítimo FUNCIONA (`GoalConfirmed` é emitido — a confirmação foi registrada).
#[test]
fn goal_escalated_e_server_side_agente_nao_forja_a_escalada() {
    let mut e = env_with("forge-escalate", RouterConfig::default());
    let _w = e.sup.register("@Worker", Some("dev".into()), sink());
    let mut deliver = ok_deliver();

    // meta definida pelo caminho real (goal_id cunhado server-side).
    let goal_id = match e.router.route_message(
        &MailMessage::new(
            "@Worker",
            "goal",
            "goal.define",
            r#"{"statement":"criar landing"}"#,
        ),
        &mut e.store,
        1_000,
        &mut deliver,
    ) {
        RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
        other => panic!("define falhou: {other:?}"),
    };

    // ATAQUE (a): verbo de escalada FORJADO — não existe como verbo de Goal.
    e.router.route_message(
        &MailMessage::new(
            "@Worker",
            "goal",
            "goal.escalate",
            format!(r#"{{"goal_id":"{goal_id}","reason":"stalled"}}"#),
        ),
        &mut e.store,
        1_001,
        &mut deliver,
    );
    // ATAQUE (b): `reason:"stalled"` injetado num verbo LEGÍTIMO (confirm) — DADO, não autoridade.
    e.router.route_message(
        &MailMessage::new(
            "@Worker",
            "goal",
            "goal.confirm",
            format!(r#"{{"goal_id":"{goal_id}","reason":"stalled"}}"#),
        ),
        &mut e.store,
        1_002,
        &mut deliver,
    );

    // não-vacuosidade: o verbo legítimo funcionou (a confirmação foi REGISTRADA).
    assert_eq!(
        count_kind(&e.store, "GoalConfirmed"),
        1,
        "o verbo legítimo (confirm) emite GoalConfirmed — caminho real exercido (não-vacuoso)"
    );
    // a propriedade: ZERO GoalEscalated nasceu de um agente.
    assert_eq!(
        count_kind(&e.store, "GoalEscalated"),
        0,
        "agente NÃO forja GoalEscalated: nem verbo de escalada, nem reason de payload viram o evento"
    );
}

// ════════════════ S2 — reset do breaker exige reset externo: agente não se auto-libera ════════════════

/// **(gate e — o ponto adversarial central) um AGENTE não reseta o próprio circuit breaker.** O breaker
/// abre por falhas de entrega e fecha SÓ quando o nó sai de `Blocked` por uma via EXTERNA (lifecycle/
/// humano — `router.rs:1357-1366`). O CONF-CORE formaliza isso como "reset sob gesto humano". Aqui
/// provamos a metade que JÁ vale e é inegociável: NENHUMA ação roteável por um agente fecha o breaker —
/// (a) re-enviar mensagem ao nó (RETIDA, nem tenta), (b) `params.set` afrouxando o limiar (gated, S3),
/// (c) verbos-fantasma de "reset"/"unblock" (não existem → mensagem comum, sem efeito de status). Só o
/// reset EXTERNO (`sup.set_status` — o que o gesto humano/lifecycle faz, NÃO roteável por A2A) libera.
///
/// QUEBRA SE… alguma ação de agente fechasse o breaker → o probe pós-ataques ENTREGARIA (não Retido) e
/// `breaker_ainda_aberto` devolveria `false`. Não-vacuoso: o CONTRASTE (reset externo) libera de fato.
#[test]
fn agente_nao_reseta_o_proprio_breaker_so_reset_externo_fecha() {
    let cfg = RouterConfig {
        delivery_backoff_base_ms: 100,
        delivery_max_attempts: 99, // DLQ fora do caminho
        breaker_threshold: 3,
        ..RouterConfig::default()
    };
    let mut e = env_with("breaker-self-reset", cfg);
    let _a = e.sup.register("@A", None, sink());
    let b = e.sup.register("@B", Some("dev".into()), sink());
    e.sup.set_status(b, NodeStatus::Idle).expect("B idle");
    let fail = Rc::new(Cell::new(true));
    open_breaker(&mut e, b, &fail);

    // — ATAQUES roteáveis por um AGENTE (sender = @B, o próprio nó bloqueado) —
    fail.set(false); // a entrega já "sararia"; só o breaker é que deve segurar
    let mut deliver = ok_deliver();

    // (a) re-enviar uma mensagem ao nó NÃO fecha o breaker (é justamente o que retém).
    //     (coberto pelo probe final; aqui o foco são os verbos estruturados)
    // (b) `params.set` para afrouxar o limiar do breaker — vem em S3; aqui só os verbos de "reset".
    // (c) verbos-FANTASMA: não existem na tabela de verbos → tratados como mensagem comum, sem status.
    for fake in ["node.unblock", "breaker.reset", "status.set"] {
        e.router.route_message(
            &MailMessage::new(
                "@B",
                "@workspace",
                fake,
                r#"{"node":"@B","status":"idle","value":"unblock"}"#,
            ),
            &mut e.store,
            5_000,
            &mut deliver,
        );
    }
    // (b') o agente tenta afrouxar o limiar do breaker por params.set (deve ser barrado — ver S3).
    e.router.route_message(
        &MailMessage::new(
            "@B",
            "@workspace",
            "params.set",
            r#"{"key":"breaker_threshold","scope":"workspace","value":"99"}"#,
        ),
        &mut e.store,
        5_001,
        &mut deliver,
    );

    // PROPRIEDADE: após TODAS as tentativas do agente, o breaker continua ABERTO.
    assert!(
        breaker_ainda_aberto(&mut e, b, "ainda preso", 6_000),
        "nenhuma ação roteável por agente reseta o breaker (msg RETIDA, nó Blocked)"
    );
    assert_eq!(
        count_kind(&e.store, "SystemParamsChanged"),
        0,
        "o params.set de afrouxamento NÃO aplicou (limiar do breaker intacto)"
    );

    // CONTRASTE (não-vacuosidade): o reset EXTERNO (gesto humano/lifecycle — NÃO roteável por A2A)
    // tira o nó de Blocked → o breaker fecha → a fila retida volta a entregar.
    e.sup
        .set_status(b, NodeStatus::Idle)
        .expect("reset externo");
    let entregue_antes = count_kind(&e.store, "MessageDelivered");
    let outbox = Mailbox::new(e.tmp.join(".lina"));
    let probe = MailMessage::new("@A", "@B", "ask", "agora vai");
    outbox.enqueue_as("@A", &probe).expect("enqueue");
    let _ = e.router.pump(&mut e.store, 7_000, &mut deliver);
    let _ = e.router.pump(&mut e.store, 7_010, &mut deliver);
    assert!(
        count_kind(&e.store, "MessageDelivered") > entregue_antes,
        "após o reset EXTERNO, a entrega volta — prova que existe um reset, mas NÃO via agente"
    );
}

// ════════════════ S3 — afrouxar o breaker (params) sobe ao gate humano (GatedHard) ════════════════

/// **(gate e) um agente não enfraquece o circuit breaker por conta própria.** REDUZIR `breaker_threshold`
/// abaixo do default (= fazer o breaker tolerar mais falhas antes de abrir, ou nunca abrir) é
/// ENFRAQUECER um laço balanceador → `ActionClass::GatedHard` (`params.rs:934`): sobe ao gate humano e
/// NÃO aplica. AUMENTAR (fortalecer) aplica livre — fortalecer a defesa nunca precisa de gate. É a regra
/// F3-0-7 §C exercida pelo CAMINHO REAL do router para o parâmetro do BREAKER (o F3-0-7 router-test usa
/// `delegation_budget`; aqui o ângulo é a defesa do próprio anti-engolimento).
///
/// QUEBRA SE… `param_change_action_class("breaker_threshold", <abaixo do default>)` virasse `Routine` →
/// o `route_message` aplicaria (ParamsApplied + SystemParamsChanged) e o 1º bloco cairia. Não-vacuoso:
/// AUMENTAR aplica (o contraste verde prova que o gate não barra tudo).
#[test]
fn afrouxar_breaker_threshold_via_params_sobe_ao_gate_humano() {
    let mut e = env_with("breaker-weaken", RouterConfig::default());
    let _a = e.sup.register("@A", None, sink());
    let mut deliver = ok_deliver();

    // ENFRAQUECER (reduzir o limiar do breaker, 5 → 2) → GatedHard, NÃO aplica.
    let out = e.router.route_message(
        &MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"breaker_threshold","scope":"workspace","value":"2"}"#,
        ),
        &mut e.store,
        1,
        &mut deliver,
    );
    assert!(
        matches!(out, RouteOutcome::ParamsGated { .. }),
        "afrouxar o breaker → gate humano; veio {out:?}"
    );
    assert_eq!(
        count_kind(&e.store, "SystemParamsChanged"),
        0,
        "gated NÃO aplica — o limiar do breaker fica intacto (agente não desliga a defesa)"
    );
    let gated = events_of(&e.store, "ActionGated");
    assert_eq!(gated.len(), 1, "1 ActionGated logado (auditável)");
    assert_eq!(
        field(&gated[0], "decision"),
        Some("ask"),
        "gated-hard = Ask"
    );

    // CONTRASTE (não-vacuosidade): FORTALECER (aumentar o limiar) aplica livre.
    let out2 = e.router.route_message(
        &MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"breaker_threshold","scope":"workspace","value":"10"}"#,
        ),
        &mut e.store,
        2,
        &mut deliver,
    );
    assert!(
        matches!(out2, RouteOutcome::ParamsApplied { .. }),
        "aumentar o limiar (fortalecer a defesa) aplica livre; veio {out2:?}"
    );
}
