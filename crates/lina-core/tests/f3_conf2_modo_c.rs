//! **F3-CONF-2 · Vigia do Modo C + métrica de adoção (QA, read-only).**
//!
//! ## Modo C — "roteado/retido sem entregar" (a vigia que o repro §6.2 recomendou)
//! A família #22/#23 tem 3 modos (`tasks/epico-f3/repro-22-23.md` §2). O Modo A (entrega falsa) e o
//! Modo B (entrega real sem trabalho) têm repro determinístico (`repro_w4_*` / `repro_22_23_residual`).
//! O Modo C — a msg fica RETIDA e a re-tentativa não corre (pump não tickou para o nó) — NÃO ganhou
//! repro determinístico naquela rodada porque simular "pump morto" toca o AGENDAMENTO do pump (fronteira
//! do CORE). A recomendação (§6.2) foi uma **vigia por contagem no log**: `MessageRetained` sem
//! `MessageDelivered`/`MessageDeadLettered` subsequente. Este arquivo entrega essa vigia como uma
//! PROJEÇÃO PURA (read-only) e a exercita pelo caminho real (`route_message` com alvo `Busy` → retenção).
//!
//! ## Métrica de adoção (risco §VI-3 do épico: "adoção 0%")
//! O protocolo anti-engolimento emitirá `GoalEscalated{reason:"stalled"}` quando o re-despacho não fecha
//! o ciclo. Medir QUANTOS `stalled` reais acontecem é a prova de que o mecanismo é USADO (não construído
//! e nunca acionado). A métrica nasce DESDE O DIA 1: hoje = 0 (CONF-CORE não pousou); quando o protocolo
//! disparar, vira > 0. Aqui provamos que o contador discrimina o `reason` (não é always-0 espúrio).
//!
//! Tudo read-only sobre o log (nenhuma alteração de produção). Invariante #4: a verdade é o EVENT LOG.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    DeliveryOutcome, DomainEvent, EventRecord, EventStore, MailMessage, Mailbox, NodeId,
    NodeStatus, RouteOutcome, Router, RouterConfig, Supervisor,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "lina-conf2modc-{tag}-{}-{tid}-{n}",
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
            name: format!("F3-CONF-2 modo C — {tag}"),
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

// ─────────────────────────── as projeções PURAS (a vigia e a métrica) ───────────────────────────

/// **Vigia do Modo C (read-only):** ids RETIDOS (`MessageRetained`) que NUNCA foram resolvidos por um
/// `MessageDelivered` nem `MessageDeadLettered` subsequente. É o §6.2 do repro: retenção presa = o pump
/// não re-tickou para o nó. Pura sobre o log (sem estado, replay-idempotente).
fn modo_c_retencoes_presas(recs: &[EventRecord]) -> Vec<String> {
    let id_of = |r: &EventRecord| {
        r.payload
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
    };
    let mut resolvidos = std::collections::HashSet::new();
    for r in recs {
        if r.kind == "MessageDelivered" || r.kind == "MessageDeadLettered" {
            if let Some(id) = id_of(r) {
                resolvidos.insert(id);
            }
        }
    }
    let mut presas = Vec::new();
    for r in recs {
        if r.kind == "MessageRetained" {
            if let Some(id) = id_of(r) {
                if !resolvidos.contains(&id) && !presas.contains(&id) {
                    presas.push(id);
                }
            }
        }
    }
    presas
}

/// **Métrica de adoção (read-only):** quantos `GoalEscalated{reason:"stalled"}` REAIS o protocolo
/// anti-engolimento produziu. Discrimina o `reason` (um `turn_budget_exhausted`/`judge_unreliable` não
/// conta como adoção do protocolo NOVO).
fn adocao_stalled(recs: &[EventRecord]) -> usize {
    recs.iter()
        .filter(|r| r.kind == "GoalEscalated")
        .filter(|r| r.payload.get("reason").and_then(|v| v.as_str()) == Some("stalled"))
        .count()
}

// ════════════════════════════════ Modo C — a vigia detecta retenção presa ════════════════════════════════

/// **A vigia FLAGA uma retenção que nunca foi entregue/DLQ — e ignora a que foi resolvida.** Pelo caminho
/// real: alvo `Busy` → a msg RETÉM (`MessageRetained{target_busy}`, `router.rs:954`). Enquanto o pump não
/// re-tickar para o nó (Modo C), o id fica RETIDO sem `MessageDelivered`/DLQ → a vigia o conta. Uma
/// SEGUNDA msg, cujo alvo destrava (Idle) e ENTREGA, NÃO é flagada (tem `MessageDelivered` com o mesmo id).
///
/// QUEBRA SE… a vigia contasse retenções já resolvidas (falso-positivo) → a msg entregue apareceria na
/// lista e o `contains` cairia. Não-vacuoso: a retenção presa É contada (1), a resolvida NÃO (a lista a exclui).
#[test]
fn vigia_modo_c_conta_retencao_presa_e_ignora_a_resolvida() {
    let mut e = env("vigia-presa");
    let _a = e.sup.register("@A", None, sink());
    let b = e.sup.register("@B", Some("dev".into()), sink());
    let mut deliver = ok_deliver();

    // (1) RETENÇÃO PRESA: @B Busy → msg retém e (Modo C) nunca é re-tentada/entregue.
    e.sup.set_status(b, NodeStatus::Busy).expect("B busy");
    let presa = MailMessage::new("@A", "@B", "ask", "fica retida");
    let out = e
        .router
        .route_message(&presa, &mut e.store, 1_000, &mut deliver);
    assert!(
        matches!(out, RouteOutcome::Retained { .. }),
        "alvo Busy → retenção (caminho real); veio {out:?}"
    );

    // (2) RETENÇÃO RESOLVIDA: outra msg que, com @B livre, ENTREGA (mesmo id em Retained e Delivered).
    let resolvida = MailMessage::new("@A", "@B", "ask", "essa entrega");
    e.router
        .route_message(&resolvida, &mut e.store, 1_001, &mut deliver); // retém (ainda Busy)
    e.sup.set_status(b, NodeStatus::Idle).expect("B idle");
    // re-rota da MESMA msg agora entrega (alvo livre).
    let out2 = e
        .router
        .route_message(&resolvida, &mut e.store, 1_002, &mut deliver);
    assert!(
        matches!(out2, RouteOutcome::Delivered { .. }),
        "alvo livre → entrega (resolve a retenção); veio {out2:?}"
    );

    // A VIGIA: só a retenção PRESA é flagada.
    let recs = e.store.events().expect("events");
    let presas = modo_c_retencoes_presas(&recs);
    assert!(
        presas.contains(&presa.id),
        "a retenção presa (sem Delivered/DLQ) é flagada pela vigia do Modo C"
    );
    assert!(
        !presas.contains(&resolvida.id),
        "a retenção RESOLVIDA (entregue depois) NÃO é flagada — zero falso-positivo"
    );
    assert_eq!(
        presas.len(),
        1,
        "exatamente 1 suspeita de Modo C nesta janela"
    );
}

// ════════════════════════════════ Adoção — a métrica do protocolo (day-1) ════════════════════════════════

/// **A adoção do protocolo é mensurável DESDE O DIA 1.** Hoje, com o CONF-CORE ainda não pousado,
/// `GoalEscalated{stalled}` reais = 0 (a linha de base honesta — risco §VI-3 "adoção 0%"). E o contador
/// DISCRIMINA o `reason`: alimentado com um `stalled` e um `turn_budget_exhausted`, conta só o `stalled`
/// (=1) — então, quando o protocolo disparar de verdade, a adoção sobe e é visível, sem confundir com a
/// escalada de turn-budget que já existia.
///
/// Nota de método: o 2º bloco alimenta o store com eventos SINTÉTICOS — é um teste de UNIDADE da MÉTRICA
/// (o contador discrimina o reason?), NÃO uma prova da costura do protocolo. A costura (entrega-sem-
/// progresso → `stalled` pelo caminho real) é provada em `f3_conf2_protocolo.rs` via `route_message`.
///
/// QUEBRA SE… a métrica contasse qualquer `GoalEscalated` (sem filtrar o reason) → o `turn_budget_exhausted`
/// somaria e o `==1` cairia. Não-vacuoso: o bloco sintético prova que o contador efetivamente conta.
#[test]
fn adocao_de_stalled_e_zero_hoje_e_o_contador_discrimina_o_reason() {
    let mut e = env("adocao");

    // linha de base: nada de protocolo ainda → ZERO stalled reais.
    let base = e.store.events().expect("events");
    assert_eq!(
        adocao_stalled(&base),
        0,
        "day-1: GoalEscalated{{stalled}} = 0 (CONF-CORE não pousou) — linha de base honesta"
    );

    // unidade da métrica: um stalled + um turn_budget_exhausted → o contador isola o stalled.
    e.store
        .append(&DomainEvent::GoalEscalated {
            goal_id: "g-sintetico".into(),
            reason: "stalled".into(),
        })
        .expect("append stalled");
    e.store
        .append(&DomainEvent::GoalEscalated {
            goal_id: "g-sintetico".into(),
            reason: "turn_budget_exhausted".into(),
        })
        .expect("append turn_budget");
    let recs = e.store.events().expect("events");
    assert_eq!(
        adocao_stalled(&recs),
        1,
        "a métrica conta SÓ os stalled (discrimina o reason) — adoção mensurável quando o protocolo agir"
    );
}
