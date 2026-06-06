//! GATE F1-0-9 (parte MECÂNICA) — **`lina plan claim` END-TO-END headless + telemetria de adoção**.
//!
//! Prova os critérios determinísticos (b) e (c) da story reformulada:
//! - **(b)** o ciclo COMPLETO do claim no caminho real do verbo (o `lina plan claim` da CLI é um
//!   escritor fino de `MailMessage{intent:"plan.claim", ref}` — aqui a mesma mensagem atravessa
//!   `Router::route_message`): claim → `PlanClaimed` no `log.jsonl` → projeção do plano reflete
//!   `@owner`/`#status` → **reconstrução por replay** (store reaberto do disco) → 2º claim de
//!   OUTRO owner **rejeitado** sem evento; 2º claim do MESMO owner **idempotente** (owner/estado
//!   inalterados). Cenário da story: *Maestro semeia item sem owner; um worker o reivindica.*
//! - **(c)** a telemetria de adoção (`plan_adoption`, bin offline) RODA sobre o log real deste
//!   teste e devolve a contagem/taxa exata — exercitada aqui como MÉTRICA ACOMPANHADA (o assert
//!   é sobre a *contagem correta*, jamais sobre "taxa mínima": adoção não é gate — story §3).
//!
//! Invariantes (CLAUDE.md): #4 — **cada assert lê o `log.jsonl`/projeção, nunca só o
//! `RouteOutcome` em memória**; #1 — zero LLM (tudo contador/estado determinístico). Substrato
//! espelha o idioma do `gate_onda3` (validação independente: este gate só LÊ o core).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    DeliveryOutcome, DomainEvent, EventRecord, EventStore, ItemState, MailMessage, Mailbox, NodeId,
    RouteOutcome, Router, RouterConfig, Supervisor,
};

// ───────────────────────────── substrato headless (idioma do gate_onda3) ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-gate-f109-{tag}-{}-{n}", std::process::id()))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// `deliver` trivial: a entrega ao PTY não é o objeto deste gate (intents de plano nem chegam a
/// PTY — desviam no router); o ask do cenário (c) só precisa "entregar" p/ virar `MessageRouted`.
fn ok_deliver() -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    |_target, _from, _text| {
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

impl Env {
    fn store_dir(&self) -> PathBuf {
        self.tmp.join(".lina").join("events")
    }
}

fn env(tag: &str) -> Env {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: format!("Gate F1-0-9 — {tag}"),
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

/// Lê o espelho canônico `log.jsonl` (fonte da verdade dos asserts — invariante #4).
fn jsonl(dir: &std::path::Path) -> Vec<EventRecord> {
    let path = dir.join("log.jsonl");
    let data =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("ler {}: {e}", path.display()));
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<EventRecord>(l).expect("linha do log.jsonl = EventRecord"))
        .collect()
}

fn p_str<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload.get(key).and_then(|v| v.as_str())
}

// ════════════════════════════════════ (b) o ciclo e2e do claim ════════════════════════════════════

/// **Claim end-to-end**: Maestro semeia `T1` sem owner → `@Dev` reivindica pelo caminho real do
/// verbo → `PlanClaimed` no `log.jsonl` + projeção `@owner:@Dev`/`status:doing` + `plan.md` em
/// disco refletem ATOMICAMENTE → **replay de um store reaberto do disco reconstrói o mesmo
/// estado** → 2º claim de `@Maestro` é REJEITADO (sem evento novo, projeção intacta) → 2º claim
/// do PRÓPRIO `@Dev` é IDEMPOTENTE (aplica sem mudar owner/estado).
#[test]
fn plan_claim_e2e_log_projecao_replay_e_segundo_claim() {
    let mut e = env("claim-e2e");
    let _maestro = e.sup.register("@Maestro", Some("maestro".into()), sink());
    let _dev = e.sup.register("@Dev", Some("dev".into()), sink());
    let mut deliver = ok_deliver();

    // Maestro distribui: item nasce `status:todo` / `@owner:?` (cenário da story).
    e.router
        .seed_plan_item(&mut e.store, "T1", "desenhar o schema")
        .expect("seed T1");
    {
        let plan = e.store.project().expect("project").plan;
        let item = plan
            .itens
            .iter()
            .find(|i| i.id == "T1")
            .expect("T1 semeado");
        assert_eq!(item.owner, None, "nasce SEM owner (Maestro só distribui)");
        assert_eq!(item.status, ItemState::Todo);
    }

    // (1) O claim do worker — a MESMA mensagem que `lina plan claim T1` deposita.
    let claim = MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref("plan:T1");
    let out = e
        .router
        .route_message(&claim, &mut e.store, 1_000, &mut deliver);
    assert!(
        matches!(&out, RouteOutcome::PlanApplied { intent, item } if intent == "plan.claim" && item == "T1"),
        "claim aplica: {out:?}"
    );

    // (2) EVENTO no log.jsonl (fonte da verdade — não o RouteOutcome).
    let recs = jsonl(&e.store_dir());
    let claims: Vec<&EventRecord> = recs.iter().filter(|r| r.kind == "PlanClaimed").collect();
    assert_eq!(claims.len(), 1, "exatamente 1 PlanClaimed");
    assert_eq!(p_str(claims[0], "item"), Some("T1"));
    assert_eq!(p_str(claims[0], "by"), Some("@Dev"));

    // (3) PROJEÇÃO reflete a mudança atômica @owner/#status.
    let plan = e.store.project().expect("project").plan;
    let item = plan.itens.iter().find(|i| i.id == "T1").expect("T1");
    assert_eq!(item.owner.as_deref(), Some("@Dev"), "@owner mudou");
    assert_eq!(
        item.status,
        ItemState::Doing,
        "#status mudou junto (atômico)"
    );

    // (3b) `plan.md` em disco (escritor único) espelha a projeção — o humano vê o claim.
    let plan_md = std::fs::read_to_string(e.tmp.join(".lina").join("plan.md")).expect("plan.md");
    assert!(
        plan_md.contains("@Dev") && plan_md.contains("T1"),
        "plan.md reflete o claim: {plan_md}"
    );

    // (4) RECONSTRUÇÃO por replay: um store NOVO, reaberto do disco, projeta o mesmo estado.
    {
        let reopened = EventStore::open(e.store_dir()).expect("reabrir do disco");
        let plan = reopened.project().expect("project pós-replay").plan;
        let item = plan
            .itens
            .iter()
            .find(|i| i.id == "T1")
            .expect("T1 no replay");
        assert_eq!(
            item.owner.as_deref(),
            Some("@Dev"),
            "replay reconstrói o owner"
        );
        assert_eq!(item.status, ItemState::Doing, "replay reconstrói o status");
    }

    // (5) 2º claim de OUTRO owner → REJEITADO; nada muda no log nem na projeção.
    let steal = MailMessage::new("@Maestro", "plan", "plan.claim", "").with_ref("plan:T1");
    let out = e
        .router
        .route_message(&steal, &mut e.store, 2_000, &mut deliver);
    assert!(
        matches!(&out, RouteOutcome::PlanRejected(reason) if reason.contains("T1")),
        "claim de outro owner é rejeitado com motivo legível: {out:?}"
    );
    let recs = jsonl(&e.store_dir());
    assert_eq!(
        recs.iter().filter(|r| r.kind == "PlanClaimed").count(),
        1,
        "rejeição NÃO apenda evento (o log é o livro-razão dos fatos, não das tentativas)"
    );
    let plan = e.store.project().expect("project").plan;
    let item = plan.itens.iter().find(|i| i.id == "T1").expect("T1");
    assert_eq!(item.owner.as_deref(), Some("@Dev"), "owner intacto");

    // (6) 2º claim do MESMO owner → IDEMPOTENTE: aplica (re-afirmação), owner/estado inalterados.
    let again = MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref("plan:T1");
    let out = e
        .router
        .route_message(&again, &mut e.store, 3_000, &mut deliver);
    assert!(
        matches!(&out, RouteOutcome::PlanApplied { .. }),
        "re-claim do próprio owner é idempotente (não erro): {out:?}"
    );
    let plan = e.store.project().expect("project").plan;
    let item = plan.itens.iter().find(|i| i.id == "T1").expect("T1");
    assert_eq!(
        item.owner.as_deref(),
        Some("@Dev"),
        "idempotência: owner igual"
    );
    assert_eq!(item.status, ItemState::Doing, "idempotência: estado igual");

    // Item inexistente: rejeição legível, sem evento (defesa do verbo, mesma classe do (5)).
    let ghost = MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref("plan:T99");
    let out = e
        .router
        .route_message(&ghost, &mut e.store, 4_000, &mut deliver);
    assert!(matches!(&out, RouteOutcome::PlanRejected(_)), "{out:?}");
}

// ════════════════════════════════════ (c) telemetria sobre o log REAL ════════════════════════════════════

/// **Telemetria de adoção sobre um log real**: monta um Espaço com 2 claims + 1 cadeia ad-hoc
/// (ask), roda o bin `plan_adoption` (offline, `--json`) sobre o `log.jsonl` produzido e assere a
/// CONTAGEM exata — taxa 2/(2+1) = 66.7%. O assert é sobre a *exatidão da medição*; a taxa em si
/// é MÉTRICA ACOMPANHADA do gate F1-0 (nunca critério binário — story F1-0-9 §3).
#[test]
fn telemetria_de_adocao_roda_sobre_o_log_real_do_teste() {
    let mut e = env("telemetria");
    let _maestro = e.sup.register("@Maestro", Some("maestro".into()), sink());
    let _dev = e.sup.register("@Dev", Some("dev".into()), sink());
    let mut deliver = ok_deliver();

    // 2 itens distribuídos e reivindicados via PLANO…
    for (item, desc) in [("T1", "schema"), ("T2", "endpoints")] {
        e.router
            .seed_plan_item(&mut e.store, item, desc)
            .expect("seed");
        let claim =
            MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref(format!("plan:{item}"));
        assert!(matches!(
            e.router
                .route_message(&claim, &mut e.store, 1_000, &mut deliver),
            RouteOutcome::PlanApplied { .. }
        ));
    }
    // …e 1 delegação ad-hoc (ask) — a cadeia que o denominador conta.
    let ask = MailMessage::new("@Maestro", "@Dev", "ask", "revisa o T1 quando der");
    assert!(matches!(
        e.router
            .route_message(&ask, &mut e.store, 2_000, &mut deliver),
        RouteOutcome::Delivered { .. }
    ));

    // O bin OFFLINE roda sobre o log.jsonl recém-escrito (caminho real de leitura do relatório).
    let out = Command::new(env!("CARGO_BIN_EXE_plan_adoption"))
        .arg(e.store_dir())
        .arg("--json")
        .output()
        .expect("rodar plan_adoption");
    assert!(
        out.status.success(),
        "plan_adoption saiu com erro: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("saída --json parseável");
    assert_eq!(report["plan_claims"], 2, "2 claims medidos: {report}");
    assert_eq!(report["ask_chains"], 1, "1 cadeia ad-hoc: {report}");
    assert_eq!(report["ask_msgs"], 1);
    assert_eq!(report["rate_pct"], 66.7, "taxa exata 2/(2+1): {report}");

    // O render humano circula com o rótulo anti-gate (parte do contrato da story).
    let human = Command::new(env!("CARGO_BIN_EXE_plan_adoption"))
        .arg(e.store_dir())
        .output()
        .expect("rodar plan_adoption (humano)");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("MÉTRICA ACOMPANHADA"), "{text}");
}
