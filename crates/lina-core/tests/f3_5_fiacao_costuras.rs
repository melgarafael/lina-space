//! **F3-5 · fiação CORE — costuras intent→handler→evento provadas por `route_message`.**
//!
//! As 7 frentes da onda entregaram lógica PURA; sem a emissão dos eventos pela rota real, a
//! lógica fica INERTE. Esta suíte prova, pelo CAMINHO REAL (`Router::route_message`, nunca
//! `store.append` à-mão — lição da casa: teste à-mão não prova a costura), que cada verbo
//! estruturado emite o evento certo, com **identidade/origem carimbada SERVER-SIDE** (o `node`/
//! autor vem do remetente autenticado, jamais do payload — regra-mãe ADR 0007). Cada teste com
//! `node` FORJADO no payload prova que a forja é IGNORADA.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    DeliveryOutcome, DomainEvent, EventRecord, EventStore, MailMessage, Mailbox, NodeId, Router,
    RouterConfig, Supervisor,
};

// ───────────────────────────── substrato headless (molde f3_3_mentality_seguranca) ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lina-f35fia-{tag}-{}-{:?}-{n}",
        std::process::id(),
        std::thread::current().id()
    ))
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
            name: format!("F3-5 fiação — {tag}"),
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

fn recs_of<'a>(events: &'a [EventRecord], kind: &str) -> Vec<&'a EventRecord> {
    events.iter().filter(|r| r.kind == kind).collect()
}

/// Roda um verbo estruturado pelo caminho REAL e devolve os eventos do log após.
fn route(e: &mut Env, from: &str, to: &str, intent: &str, payload: &str) -> Vec<EventRecord> {
    let mut deliver = ok_deliver();
    e.router.route_message(
        &MailMessage::new(from, to, intent, payload),
        &mut e.store,
        1000,
        &mut deliver,
    );
    e.store.events().expect("ler eventos")
}

// ════════════════════════════════════ SKILL (F3-5-4/5) ════════════════════════════════════

/// `skill.select` → `SkillSelected{node,skill,trigger,source}` com `node` = remetente AUTENTICADO,
/// JAMAIS o payload (que aqui FORJA `node:@Atacante`). Prova a costura E a regra server-side.
#[test]
fn skill_select_emits_event_with_server_side_node() {
    let mut e = env("skill-select");
    let _w = e.sup.register("@Worker", Some("dev".into()), sink());
    let payload = r#"{"skill":"lina-code-doctrine","trigger":"conserta esse bug","source":"catalog","node":"@Atacante"}"#;
    let events = route(&mut e, "@Worker", "skill", "skill.select", payload);

    let selected = recs_of(&events, "SkillSelected");
    assert_eq!(
        selected.len(),
        1,
        "skill.select emite exatamente 1 SkillSelected"
    );
    assert_eq!(
        selected[0].payload["node"], "@Worker",
        "node = remetente autenticado (server-side), NUNCA o @Atacante forjado no payload"
    );
    assert_eq!(selected[0].payload["skill"], "lina-code-doctrine");
    assert_eq!(selected[0].payload["trigger"], "conserta esse bug");
    assert_eq!(selected[0].payload["source"], "catalog");
}

/// `skill.propose` → `SkillFactoryProposed{skill_name,via,references}` (sugere, nunca aplica — só o
/// evento, zero efeito colateral de criação).
#[test]
fn skill_propose_emits_factory_proposed() {
    let mut e = env("skill-propose");
    let _w = e.sup.register("@Worker", Some("dev".into()), sink());
    let payload =
        r#"{"skill_name":"senior-architect","via":"deep-research","references":["https://r1"]}"#;
    let events = route(&mut e, "@Worker", "skill", "skill.propose", payload);

    let proposed = recs_of(&events, "SkillFactoryProposed");
    assert_eq!(
        proposed.len(),
        1,
        "skill.propose emite 1 SkillFactoryProposed"
    );
    assert_eq!(proposed[0].payload["skill_name"], "senior-architect");
    assert_eq!(proposed[0].payload["via"], "deep-research");
    assert_eq!(proposed[0].payload["references"][0], "https://r1");
    // nunca cria/instala: a única consequência é o evento de proposta (gate humano depois).
    assert!(recs_of(&events, "SkillSelected").is_empty());
}

// ════════════════════════════════════ CLUE (F3-5-6) ════════════════════════════════════

/// `clue.define` → `ClueSetDefined{scope,paths,label}` pelo caminho real.
#[test]
fn clue_define_emits_clue_set_defined() {
    let mut e = env("clue-define");
    let _w = e.sup.register("@Worker", Some("dev".into()), sink());
    let payload =
        r#"{"scope":"projeto-x","paths":["docs/","src/api.rs"],"label":"contexto do projeto"}"#;
    let events = route(&mut e, "@Worker", "clue", "clue.define", payload);

    let defined = recs_of(&events, "ClueSetDefined");
    assert_eq!(defined.len(), 1, "clue.define emite 1 ClueSetDefined");
    assert_eq!(defined[0].payload["scope"], "projeto-x");
    assert_eq!(defined[0].payload["paths"][0], "docs/");
    assert_eq!(defined[0].payload["paths"][1], "src/api.rs");
    assert_eq!(defined[0].payload["label"], "contexto do projeto");
}

/// `clue.clear` retrai a pista (emite `ClueSetDefined` com `paths` vazio — some no próximo briefing).
#[test]
fn clue_clear_retracts_with_empty_paths() {
    let mut e = env("clue-clear");
    let _w = e.sup.register("@Worker", Some("dev".into()), sink());
    route(
        &mut e,
        "@Worker",
        "clue",
        "clue.define",
        r#"{"scope":"px","paths":["a"]}"#,
    );
    let events = route(&mut e, "@Worker", "clue", "clue.clear", r#"{"scope":"px"}"#);

    let defined = recs_of(&events, "ClueSetDefined");
    assert_eq!(defined.len(), 2, "define + clear = 2 eventos");
    assert!(
        defined[1].payload["paths"]
            .as_array()
            .is_some_and(|a| a.is_empty()),
        "clear retrai a pista (paths vazio)"
    );
}

// ════════════════════════════════════ BUFFERS (F3-5-7) ════════════════════════════════════

/// `lina check --buffers` deriva o relatório do EVENT LOG (replay puro) — a amostragem
/// (`BufferOccupancySampled`) é emitida pela frente de buffers; o verbo só PROJETA. Prova a
/// derivação do log (não há intent→handler aqui: a amostra não vem de um verbo).
#[test]
fn check_buffers_derives_report_from_log() {
    let mut e = env("buffers");
    e.store
        .append(&DomainEvent::BufferOccupancySampled {
            buffer_id: "scrollback:term-1".to_string(),
            used: 12_000,
            capacity: 10_000,
            unit: "lines".to_string(),
            pressure_ratio: 1.2,
            sampled_at_ms: 0,
        })
        .expect("append BufferOccupancySampled");

    let registry =
        lina_core::buffer_registry::BufferRegistry::replay(&e.store).expect("replay registry");
    let report = lina_core::buffer_registry::buffer_report(&registry);
    let line = report
        .lines
        .iter()
        .find(|l| l.buffer_id == "scrollback:term-1")
        .expect("linha do scrollback no relatório");
    assert_eq!(line.used, 12_000);
    assert!(line.over_warn, "12000/10000 passa do warn → realce");
}

// ════════════════════════════════════ segurança / robustez ════════════════════════════════════

/// Payload sem o campo obrigatório (`skill` vazio) NÃO emite evento — a costura rejeita o contrato
/// inválido em vez de logar lixo (mesmo princípio de `code.changed` com `branch` vazio).
#[test]
fn skill_select_without_skill_emits_nothing() {
    let mut e = env("skill-empty");
    let _w = e.sup.register("@Worker", Some("dev".into()), sink());
    let events = route(
        &mut e,
        "@Worker",
        "skill",
        "skill.select",
        r#"{"source":"catalog"}"#,
    );
    assert!(
        recs_of(&events, "SkillSelected").is_empty(),
        "skill vazia → nenhum SkillSelected (contrato rejeitado)"
    );
}
