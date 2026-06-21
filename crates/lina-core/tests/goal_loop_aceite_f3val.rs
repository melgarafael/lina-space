//! **F3-VAL · Aceite e2e dos FREIOS do loop — o termostato de qualidade, prova REPETÍVEL.**
//!
//! F3-1/F3-2 fecharam o código e o ciclo feliz já foi provado ao vivo pelo Maestro; o que faltava era a
//! prova **repetível, no caminho do código real**, dos FREIOS — o que separa "executar pedido" de
//! "perseguir meta" (Meadows [8], feedback negativo). Esta suíte é a **camada de aceite INDEPENDENTE**
//! (QA/red-team) sobre o OUTER loop que o CORE-Goal entregou: encena o cenário COMPLETO de ponta a ponta
//! pela **API pública** (`route_message` → `handle_goal`/`handle_plan` → `review_and_advance`), e assere a
//! sequência EXATA de eventos no log reconstruído. NUNCA monta o `ReviewVerdict` à mão — o veredito nasce
//! do JUIZ ESTRUTURAL real (`run_acceptance`, `sh -c`, exit code, ZERO LLM — invariante #1). Molde:
//! `f3_1_goal_adversarial.rs` (a suíte abençoada que dirige o loop só pela superfície pública).
//!
//! Invariante #4: todo assert lê o EVENT LOG, jamais o `RouteOutcome` em memória. Cada teste traz a
//! cláusula **QUEBRA SE…** (não-vacuosidade — prova que a asserção morde de verdade).
//!
//! Gates provados (spec 52 §Aceite b/c/d/e):
//! - **(c)** turn-budget: critério que SEMPRE falha → EXATAMENTE 3 `ReviewVerdict{Fail}` →
//!   `GoalEscalated{turn_budget_exhausted}` → zero iteração/despacho depois.
//! - **(b)** caminho feliz: critério exit 0 → ao Pass de todos os itens → EXATAMENTE 1 `GoalAchieved`.
//! - **(d)** juiz≠executor: todo `ReviewVerdict` real tem `reviewer != target`; e NENHUM verbo do
//!   barramento cunha um veredito (a forja não é sequer expressável — o juiz é server-side).
//! - **(e)** escala de effort: re-spawn no `Fail` carrega `effort` ESTRITAMENTE maior por `Ord` no log;
//!   breaker sticky impede o re-spawn IDÊNTICO (mesma tarefa+effort), mordendo na 2ª falha do topo.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    AcceptanceCriterion, CheckKind, DeliveryOutcome, DomainEvent, Effort, EventRecord, EventStore,
    MailMessage, Mailbox, NodeId, RouteOutcome, Router, RouterConfig, Supervisor,
};

// ───────────────────────────── substrato headless (molde f3_1_goal_adversarial) ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

/// tmpdir único por processo+thread+seq — `thread::id` mata o flaky sistêmico do gate paralelo do
/// Maestro (lição `f3_conf2_protocolo`: dois testes no mesmo dir envenenam o event store).
fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    std::env::temp_dir().join(format!("lina-f3val-{tag}-{}-{tid}-{n}", std::process::id()))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// `deliver` trivial — o objeto destes testes é o EVENTO no log, não a entrega ao PTY.
fn ok_deliver() -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    |_t, _f, _txt| {
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    }
}

/// Um Espaço headless: event store + supervisor + roteador. Remove o tmp no `Drop` (best-effort).
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

/// Constrói o Espaço com uma `RouterConfig` dada (a config é parte do caminho real — injetável); já
/// registra `@User` (maestro) e `@Dev` (o executor revisado). Devolve o `Env` pronto para o loop.
fn env_with(tag: &str, config: RouterConfig) -> Env {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: format!("F3-VAL freios — {tag}"),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated");
    let sup = Arc::new(Supervisor::new());
    sup.register("@User", Some("maestro".into()), sink());
    sup.register("@Dev", Some("dev".into()), sink());
    let router = Router::with_config(Arc::clone(&sup), Mailbox::new(&lina), config);
    Env {
        tmp,
        store,
        sup,
        router,
    }
}

fn env(tag: &str) -> Env {
    env_with(tag, RouterConfig::default())
}

// ───────────────────────────── encenação do CAMINHO REAL ─────────────────────────────

impl Env {
    /// `goal.define` + `goal.confirm` pela superfície pública — o `goal_id` é CUNHADO server-side
    /// (UUIDv7), nunca do payload. Devolve o id real para atribuir o item.
    fn define_and_confirm(&mut self, statement: &str) -> String {
        let mut deliver = ok_deliver();
        let goal_id = match self.router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.define",
                format!(r#"{{"statement":"{statement}"}}"#),
            ),
            &mut self.store,
            1_000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            other => panic!("esperava GoalApplied no define, veio {other:?}"),
        };
        self.router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.confirm",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut self.store,
            1_001,
            &mut deliver,
        );
        goal_id
    }

    /// SETUP do item: semeia, ATRIBUI à Goal com um DoD verificável por máquina (`CheckKind::Command`)
    /// e decompõe. É SETUP (não a costura sob teste); a costura — o veredito — nasce no `plan.check`.
    fn attribute_item_with_command(&mut self, goal_id: &str, item: &str, cmd: &str) {
        self.router
            .seed_plan_item(&mut self.store, item, "tarefa")
            .expect("seed item");
        self.store
            .append(&DomainEvent::PlanItemAttributed {
                item: item.to_string(),
                goal_id: Some(goal_id.to_string()),
                parents: vec![],
                acceptance: vec![AcceptanceCriterion {
                    desc: "critério de máquina".into(),
                    check_kind: CheckKind::Command,
                    check_arg: Some(cmd.to_string()),
                }],
                budget_tokens: 0,
            })
            .expect("PlanItemAttributed");
        self.store
            .append(&DomainEvent::GoalDecomposed {
                goal_id: goal_id.to_string(),
                plan_items: vec![item.to_string()],
            })
            .expect("GoalDecomposed");
    }

    /// `@Dev` reivindica o item (owner=@Dev server-side, pelo sender autenticado).
    fn claim(&mut self, item: &str) {
        let mut deliver = ok_deliver();
        self.router.route_message(
            &MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref(format!("plan:{item}")),
            &mut self.store,
            1_002,
            &mut deliver,
        );
    }

    /// `@Dev` declara o item pronto — o GATILHO REAL do OUTER loop (juiz estrutural → veredito → avanço).
    fn check(&mut self, item: &str, t: u64) {
        let mut deliver = ok_deliver();
        self.router.route_message(
            &MailMessage::new("@Dev", "plan", "plan.check", "").with_ref(format!("plan:{item}")),
            &mut self.store,
            t,
            &mut deliver,
        );
    }
}

// ───────────────────────────── leitores do event log (a fonte da verdade) ─────────────────────────────

fn recs_of<'a>(events: &'a [EventRecord], kind: &str) -> Vec<&'a EventRecord> {
    events.iter().filter(|r| r.kind == kind).collect()
}

fn field<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload.get(key).and_then(serde_json::Value::as_str)
}

/// Reidrata o `effort` do log de volta a [`Effort`] — para comparar por `Ord` NATIVO (não por string).
/// `serde(rename_all="lowercase")`: `"medium"` → `Effort::Medium`. Falha = effort ausente/inválido.
fn effort_of(rec: &EventRecord) -> Effort {
    serde_json::from_value(rec.payload.get("effort").cloned().expect("campo effort"))
        .expect("effort do log desserializa em Effort")
}

// ════════════════════════════════ (c) turn-budget freia em N=3 ════════════════════════════════

/// **(gate c) o termostato freia em EXATAMENTE 3 voltas.** Um DoD `Command` que SEMPRE falha (`false`,
/// exit 1) é submetido 3× (cada `plan.check` = uma re-submissão do worker): o juiz real emite 3
/// `ReviewVerdict{Fail}` e, na 3ª (iteração == `goal_max_iterations`), troca o re-despacho por
/// `GoalEscalated{turn_budget_exhausted}` — PAUSA resumível, nunca mais um turno (Meadows [9]). Depois
/// disso, NADA: nem veredito, nem despacho, nem mesmo sob `pump` repetido (o freio é estável).
///
/// QUEBRA SE… o teto não fosse 3 (veríamos ≠3 Fails), a escala não disparasse (0 `GoalEscalated`), ou o
/// loop seguisse auto-iterando após o budget (evento de avanço com `seq > esc_seq`). Não-vacuoso: os 3
/// Fails e a escala SÃO emitidos pelo caminho real.
#[test]
fn freio_c_turn_budget_freia_em_tres_fails_e_escala_resumivel() {
    let mut e = env("turn-budget");
    let goal_id = e.define_and_confirm("entregar T1");
    e.attribute_item_with_command(&goal_id, "T1", "false"); // exit 1 → SEMPRE Fail
    e.claim("T1");

    // 3 re-submissões — o worker "tenta de novo" a cada volta.
    for t in [2_000u64, 2_001, 2_002] {
        e.check("T1", t);
    }

    let events = e.store.events().expect("events");
    let fails = events
        .iter()
        .filter(|r| r.kind == "ReviewVerdict" && field(r, "verdict") == Some("Fail"))
        .count();
    assert_eq!(
        fails, 3,
        "(c) turn-budget = 3: exatamente 3 ReviewVerdict{{Fail}}"
    );

    let escalated = recs_of(&events, "GoalEscalated");
    assert_eq!(escalated.len(), 1, "(c) exatamente 1 GoalEscalated");
    assert_eq!(
        field(escalated[0], "reason"),
        Some("turn_budget_exhausted"),
        "(c) a razão é o esgotamento do orçamento de voltas (pausa, não kill)"
    );

    // zero iteração/despacho DEPOIS da escala — o backstop do ponto [9] segura o laço.
    let esc_seq = escalated[0].seq;
    assert!(
        !events.iter().any(|r| r.seq > esc_seq
            && matches!(
                r.kind.as_str(),
                "ReviewVerdict" | "GoalAchieved" | "SpawnRequested"
            )),
        "(c) zero iteração/despacho após GoalEscalated"
    );

    // o freio é ESTÁVEL: rodar o pump (sem entrega armada → anti-engolimento ocioso) não ressuscita o
    // laço. Re-conta os mesmos números — o sistema não auto-avança após o budget.
    let mut deliver = ok_deliver();
    for t in [3_000u64, 4_000, 5_000] {
        let _ = e.router.pump(&mut e.store, t, &mut deliver);
    }
    let after = e.store.events().expect("events pós-pump");
    assert_eq!(
        after
            .iter()
            .filter(|r| r.kind == "ReviewVerdict" && field(r, "verdict") == Some("Fail"))
            .count(),
        3,
        "(c) o pump não fabrica novos vereditos — o freio segura"
    );
    assert_eq!(
        recs_of(&after, "GoalEscalated").len(),
        1,
        "(c) o pump não re-escala — a escala é idempotente após o budget"
    );
}

// ════════════════════════════════ (b) caminho feliz fecha em 1 GoalAchieved ════════════════════════════════

/// **(gate b) ao Pass de TODOS os itens, a Goal fecha com EXATAMENTE 1 `GoalAchieved` — e nada depois.**
/// Dois itens com DoD `Command` exit 0 (`true`): o 1º Pass NÃO fecha (o 2º está pendente); o 2º Pass
/// fecha. Prova o termostato no setpoint atingido (o complemento do freio: o laço SABE parar quando a
/// meta É cumprida). Inclui o gate (d) parte 1: todo veredito do caminho real tem `reviewer != target`.
///
/// QUEBRA SE… a Goal fechasse no 1º Pass (fecharia com itens pendentes), fechasse 2× (`achieved.len()!=1`),
/// ou re-despachasse após o sucesso (`SpawnRequested` com `seq > achieved_seq`). Não-vacuoso: o
/// `GoalAchieved` É emitido pelo caminho real.
#[test]
fn freio_b_caminho_feliz_fecha_em_um_goal_achieved_sem_nada_depois() {
    let mut e = env("caminho-feliz");
    let goal_id = e.define_and_confirm("entregar T1 e T2");
    e.attribute_item_with_command(&goal_id, "T1", "true"); // exit 0 → Pass
    e.attribute_item_with_command(&goal_id, "T2", "true"); // exit 0 → Pass
    e.claim("T1");
    e.claim("T2");

    e.check("T1", 2_000); // Pass, mas T2 pendente → NÃO fecha.
    let mid = e.store.events().expect("events");
    assert_eq!(
        recs_of(&mid, "GoalAchieved").len(),
        0,
        "(b) com um item ainda pendente, a Goal NÃO fecha"
    );

    e.check("T2", 2_001); // Pass, todos os itens → fecha.
    let events = e.store.events().expect("events");

    let achieved = recs_of(&events, "GoalAchieved");
    assert_eq!(
        achieved.len(),
        1,
        "(b) exatamente 1 GoalAchieved ao 2º Pass"
    );
    let achieved_seq = achieved[0].seq;
    assert!(
        !events
            .iter()
            .any(|r| r.seq > achieved_seq && r.kind == "SpawnRequested"),
        "(b) nenhum re-despacho após o sucesso (o laço para)"
    );

    // gate (d) parte 1: todo veredito do caminho real é Pass e tem juiz ≠ executor.
    let verdicts = recs_of(&events, "ReviewVerdict");
    assert_eq!(verdicts.len(), 2, "(b) um veredito por item");
    for v in &verdicts {
        assert_eq!(field(v, "verdict"), Some("Pass"), "(b) ambos Pass");
        assert_ne!(
            field(v, "reviewer"),
            field(v, "target"),
            "(d) juiz != executor em todo veredito do caminho real"
        );
    }
}

// ════════════════════════════════ (d) juiz nunca é o executor + forja inexpressável ════════════════════════════════

/// **(gate d) o executor NUNCA se auto-avalia — e a forja não é sequer expressável pelo barramento.**
/// Duas evidências sobre o caminho público:
/// (1) CONSEQUÊNCIA: dirigindo o loop real, todo `ReviewVerdict` tem `reviewer != target` E
///     `reviewer != @Dev` (o owner revisado) — o juiz é o estrutural (`NodeId::nil()`), distinto de
///     qualquer nó real (que nasce `Uuid::now_v7()`).
/// (2) FORJA: um agente que MANDA uma mensagem A2A `intent:"review"` (o único verbo de "revisão" do
///     barramento) alegando um veredito NÃO produz NENHUM `ReviewVerdict` no log — o veredito é cunhado
///     SÓ server-side pelo juiz; `reviewer`/`verdict` num payload de agente são DADO, jamais autoridade
///     (regra-mãe ADR 0007 / spec 52 §Segurança).
///
/// QUEBRA SE… o loop usasse `reviewer = owner` (cairia o `assert_ne` de (1)) ou um verbo do barramento
/// cunhasse vereditos (apareceria um `ReviewVerdict` em (2)). Não-vacuoso: (1) emite 1 veredito real; (2)
/// a mensagem `review` É entregue (existe no caminho) sem virar veredito.
#[test]
fn freio_d_juiz_estrutural_nunca_e_o_executor_e_forja_inexpressavel() {
    let mut e = env("juiz-vs-executor");
    let dev = e.sup.node_by_name("@Dev").expect("@Dev registrado");
    let goal_id = e.define_and_confirm("entregar T1");
    e.attribute_item_with_command(&goal_id, "T1", "true");
    e.claim("T1");
    e.check("T1", 2_000); // dispara o juiz real.

    // (1) consequência: o único veredito real tem juiz ≠ executor (e ≠ @Dev, o revisado).
    let events = e.store.events().expect("events");
    let verdicts = recs_of(&events, "ReviewVerdict");
    assert_eq!(verdicts.len(), 1, "(d) o loop real emitiu 1 veredito");
    let reviewer = field(verdicts[0], "reviewer").expect("reviewer");
    let target = field(verdicts[0], "target").expect("target");
    let dev_str = dev.to_string();
    assert_ne!(
        reviewer, target,
        "(d) juiz != executor — auto-avaliação impossível"
    );
    assert_eq!(target, dev_str, "(d) o target é o owner revisado (@Dev)");
    assert_ne!(reviewer, dev_str, "(d) o reviewer NUNCA é o owner revisado");

    // (2) forja: @Dev manda uma mensagem A2A `review` alegando um Pass de si mesmo. É só ENTREGA — não
    // cunha veredito. O nº de ReviewVerdict NÃO muda (segue 1, o do juiz estrutural).
    let mut deliver = ok_deliver();
    let out = e.router.route_message(
        &MailMessage::new(
            "@Dev",
            "@User",
            "review",
            r#"{"verdict":"Pass","reviewer":"@Dev","target":"@Dev"}"#,
        ),
        &mut e.store,
        2_001,
        &mut deliver,
    );
    assert!(
        matches!(out, RouteOutcome::Delivered { .. } | RouteOutcome::Retained { .. }),
        "(d) a mensagem review é tratada como ENTREGA A2A comum, não como cunhagem de veredito: {out:?}"
    );
    let after = e.store.events().expect("events pós-forja");
    assert_eq!(
        recs_of(&after, "ReviewVerdict").len(),
        1,
        "(d) a forja NÃO cunhou veredito — o barramento não tem verbo que emita ReviewVerdict"
    );
}

// ════════════════════════════════ (e) escala de effort + breaker sticky ════════════════════════════════

/// **(gate e, parte 1) cada `Fail` abaixo do budget escala o effort ESTRITAMENTE por `Ord`.** Escada
/// default `[Medium, High]`: a 1ª falha re-despacha no degrau 0 (`Medium`), a 2ª no degrau 1 (`High`,
/// novo-dev). Reidratando os efforts do log, `Medium < High` por `Ord` NATIVO — a comparação que o
/// doc-fonte (spec 52 §3/§4, linha 12) pede para o "novo desenvolvedor com effort maior".
///
/// QUEBRA SE… o re-despacho não escalasse (os dois efforts iguais → `e0 < e1` falharia) ou regredisse.
/// Não-vacuoso: 2 `SpawnRequested` SÃO emitidos, com efforts distintos e crescentes.
#[test]
fn freio_e_re_spawn_escala_effort_estritamente_maior_por_ord() {
    let mut e = env("escala-effort"); // RouterConfig::default → escada [Medium, High], budget 3
    let goal_id = e.define_and_confirm("entregar T1");
    e.attribute_item_with_command(&goal_id, "T1", "false"); // SEMPRE Fail → realimenta a escala
    e.claim("T1");

    e.check("T1", 2_000); // Fail iter 1 → re-spawn degrau 0 (Medium)
    e.check("T1", 2_001); // Fail iter 2 → re-spawn degrau 1 (High, novo-dev)

    let events = e.store.events().expect("events");
    let respawns = recs_of(&events, "SpawnRequested");
    assert_eq!(
        respawns.len(),
        2,
        "(e) dois re-despachos informados (um por Fail abaixo do budget)"
    );

    let e0 = effort_of(respawns[0]);
    let e1 = effort_of(respawns[1]);
    assert_eq!(e0, Effort::Medium, "(e) 1ª volta no degrau base da escada");
    assert_eq!(e1, Effort::High, "(e) 2ª volta SOBE um degrau");
    assert!(
        e0 < e1,
        "(e) effort ESTRITAMENTE maior por Ord nativo no log: {e0:?} < {e1:?}"
    );
}

/// **(gate e, parte 2) o breaker sticky impede o re-spawn IDÊNTICO — morde já na 2ª falha do topo.**
/// Com uma escada de 1 degrau (`[High]`) e budget alto (5, para o turn-budget não mascarar): a 1ª falha
/// re-despacha (`High`); a 2ª falha repetiria o MESMO effort no MESMO item — o que não adianta (auto-
/// escalada estéril). O breaker (`count_respawns_with_effort >= 1`) SUPRIME e defere ao gate humano.
/// Resultado: exatamente 1 re-despacho, mesmo com 2 falhas.
///
/// QUEBRA SE… o sticky não mordesse (sairiam 2 `SpawnRequested(High)` — auto-escalada infinita). Não-
/// vacuoso por CONTRASTE com a parte 1: lá, escada de 2 degraus → 2 re-spawns; aqui, 1 degrau saturado → 1.
#[test]
fn freio_e_breaker_sticky_suprime_respawn_identico_no_topo() {
    let config = RouterConfig {
        effort_ladder: &[Effort::High], // 1 degrau: a 2ª volta repetiria o topo
        goal_max_iterations: 5,         // budget alto: isola o breaker do turn-budget
        ..RouterConfig::default()
    };
    let mut e = env_with("breaker-sticky", config);
    let goal_id = e.define_and_confirm("entregar T1");
    e.attribute_item_with_command(&goal_id, "T1", "false"); // SEMPRE Fail
    e.claim("T1");

    e.check("T1", 2_000); // Fail iter 1 → re-spawn (High)
    e.check("T1", 2_001); // Fail iter 2 → re-spawn IDÊNTICO (High) → SUPRIMIDO pelo sticky

    let events = e.store.events().expect("events");
    let respawns = recs_of(&events, "SpawnRequested");
    assert_eq!(
        respawns.len(),
        1,
        "(e) breaker sticky: 2 falhas no topo → exatamente 1 re-despacho (o 2º idêntico é suprimido)"
    );
    assert_eq!(
        effort_of(respawns[0]),
        Effort::High,
        "(e) o único re-despacho carrega o effort do topo da escada"
    );

    // o sticky não engole o termostato: os 2 Fails FORAM julgados (o freio é a escala, não o juízo).
    let fails = events
        .iter()
        .filter(|r| r.kind == "ReviewVerdict" && field(r, "verdict") == Some("Fail"))
        .count();
    assert_eq!(
        fails, 2,
        "(e) ambas as voltas foram julgadas Fail — o sticky só freia o re-despacho"
    );
}
