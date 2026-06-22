//! **F3-4 · Veredito de segurança AO VIVO (QA/Red-team) — broadcast só-vivos + forja de integração.**
//!
//! Roda na `develop` JÁ INTEGRADA (Trilhas A+B mergeadas). Cobre os dois ângulos do veredito final
//! (épico §F3-4(e)) que NÃO estavam cobertos pelas suítes internas das trilhas — provados pelo
//! CAMINHO REAL (`route_message` com o `intent` de produção, nunca evento montado à mão, senão não
//! prova a costura intent→handler):
//!
//!   (e4) **Broadcast por pertencimento respeita SÓ VIVOS.** O sinal de mudança avisa quem reservou
//!        um arquivo tocado — mas um dono MORTO não é alvo roteável (ADR 0007: estado de vida decide
//!        entrega, nunca um campo). As suítes de I provam o pertencimento entre VIVOS
//!        (`code_changed_notifies_only_intersecting_claim_owner`); aqui fecho o caso do dono MORTO.
//!
//!   (forja) **Forjar `BranchIntegrated` nunca destrói código.** `BranchIntegrated` é a prova de
//!        fechamento (spec 36 §3), mas seus campos (`branch`/`into`/`commit`) são DADO, sem
//!        identidade forjável — o evento não carrega `by`/`author`. Um agente que forja um
//!        `branch.integrated` falso, no PIOR caso, mexe só na PROJEÇÃO de pendência — e isso é
//!        REVERSÍVEL: um commit posterior reabre a branch. Nenhum caminho `BranchIntegrated` → git
//!        destrutivo existe (a poda é gated; `reset --hard`/`branch -D` barrados — `f3_4_salvaguardas_guard`).
//!
//! Régua (memória *provar enforcement por mutação* + *teste à-mão não prova a costura*): caminho real,
//! leitura do EVENT LOG / projeção pública (`code::CodeIntegration`) e cláusula **QUEBRA SE…** por
//! teste (não-vacuosidade). NÃO re-prova o coberto pelas trilhas (author server-side, pertencimento
//! entre vivos, branch.integrated end-to-end já verdes — auditados rodando no relatório).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    CodeIntegration, DeliveryOutcome, DomainEvent, EventStore, MailMessage, Mailbox, NodeId,
    RouteOutcome, Router, Supervisor,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "lina-f34segint-{tag}-{}-{tid}-{n}",
        std::process::id()
    ))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// `deliver` que sempre entrega — o objeto destes testes é o ROTEAMENTO (alvos) e a projeção, não a
/// injeção no PTY.
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

fn env_with(tag: &str) -> Env {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: format!("F3-4 seg-integração — {tag}"),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated");
    let sup = Arc::new(Supervisor::new());
    let router = Router::new(Arc::clone(&sup), Mailbox::new(&lina));
    Env {
        tmp,
        store,
        sup,
        router,
    }
}

/// Reserva `paths` para `owner` num item do plano (molde de `router::tests::seed_claim`): o claim com
/// `paths` é o substrato que o broadcast por pertencimento cruza.
fn seed_claim(store: &mut EventStore, item: &str, paths: &[&str], owner: &str) {
    store
        .append(&DomainEvent::PlanItemAdded {
            item: item.into(),
            desc: "reserva".into(),
        })
        .expect("PlanItemAdded");
    store
        .append(&DomainEvent::PlanItemAttributed {
            item: item.into(),
            goal_id: None,
            parents: vec![],
            acceptance: vec![],
            budget_tokens: 0,
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
        })
        .expect("PlanItemAttributed");
    store
        .append(&DomainEvent::PlanClaimed {
            id: format!("claim-{item}"),
            item: item.into(),
            by: owner.into(),
        })
        .expect("PlanClaimed");
}

fn targets_of(out: RouteOutcome) -> Vec<NodeId> {
    match out {
        RouteOutcome::Delivered { targets } => targets,
        other => panic!("esperava Delivered, veio {other:?}"),
    }
}

/// **(e4) Broadcast por pertencimento ignora o dono MORTO — só vivos são alvo.** Dois nós reservam o
/// MESMO arquivo; um (`@Morto`) morre. Um commit de `@A` que toca esse arquivo deve avisar o dono
/// VIVO (`@Vivo`) a PARAR e NÃO o morto. Prova que a entrega segue o estado de vida (ADR 0007), não a
/// mera existência de um claim — um terminal morto não recebe injeção fantasma.
///
/// Não-vacuoso: o `@Vivo` reservando o mesmo path É notificado (alguém recebe), então o teste não
/// passa por "ninguém é avisado". QUEBRA SE… o broadcast resolvesse alvos pelos claims sem filtrar
/// vivos → `@Morto` entraria em `targets` e o `assert!(!contains(morto))` falharia.
#[test]
fn broadcast_por_pertencimento_ignora_dono_morto() {
    let mut e = env_with("e4-morto");
    let _a = e.sup.register("@A", None, sink());
    let vivo = e.sup.register("@Vivo", None, sink());
    let morto = e.sup.register("@Morto", None, sink());

    // ambos reservam src/leads.rs; depois @Morto morre.
    seed_claim(&mut e.store, "T1", &["src/leads.rs"], "@Vivo");
    seed_claim(&mut e.store, "T2", &["src/leads.rs"], "@Morto");
    e.sup.mark_dead(morto).expect("marca @Morto morto");

    // @A commita tocando o arquivo reservado por AMBOS.
    let payload = r#"{"branch":"lina/a","commit":"c1","paths":["src/leads.rs"]}"#;
    let msg = MailMessage::new("@A", "@workspace", "code.changed", payload);
    let mut deliver = ok_deliver();
    let targets = targets_of(
        e.router
            .route_message(&msg, &mut e.store, 2000, &mut deliver),
    );

    assert!(
        targets.contains(&vivo),
        "o dono VIVO que reservou o arquivo é notificado a PARAR (não-vacuoso)"
    );
    assert!(
        !targets.contains(&morto),
        "o dono MORTO NÃO recebe o broadcast — entrega respeita só vivos (ADR 0007)"
    );
}

/// **(forja) Forjar `BranchIntegrated` nunca destrói código — impacto só na projeção, REVERSÍVEL.**
/// Um agente (`@EVIL`) abre `lina/vitima` com um commit e depois FORJA um `branch.integrated` sem ter
/// mergeado nada. No pior caso isso só mexe na PROJEÇÃO de pendência (`branch`/`into`/`commit` são
/// DADO). A garantia inegociável: não fecha trabalho de verdade nem apaga código — um commit
/// POSTERIOR REABRE a branch. (O fechamento real exige merge provado; a poda de worktree é gated e
/// `reset --hard`/`branch -D` ficam barrados — `f3_4_salvaguardas_guard`.)
///
/// Não-vacuoso: o passo 1 prova que a branch FICA pendente (a projeção responde de verdade); o passo
/// 3 prova a REABERTURA. QUEBRA SE… um `BranchIntegrated` forjado fechasse a branch de forma
/// PERMANENTE (commit posterior não reabrisse) → o trabalho do dono sumiria do radar em silêncio
/// (exatamente o pecado que a onda mata).
#[test]
fn branch_integrated_forjado_nunca_destroi_codigo() {
    let mut e = env_with("bi-forja");
    let _evil = e.sup.register("@EVIL", None, sink());
    let mut deliver = ok_deliver();

    // 1) um commit deixa lina/vitima PENDENTE (não-vacuoso: a projeção responde).
    let cc1 = r#"{"branch":"lina/vitima","commit":"c1","paths":["src/x.rs"]}"#;
    e.router.route_message(
        &MailMessage::new("@EVIL", "@workspace", "code.changed", cc1),
        &mut e.store,
        1000,
        &mut deliver,
    );
    assert!(
        CodeIntegration::replay(&e.store)
            .expect("proj")
            .is_branch_pending("lina/vitima"),
        "pré: lina/vitima pendente após o commit"
    );

    // 2) @EVIL FORJA branch.integrated (não mergeou de verdade).
    let bi = r#"{"branch":"lina/vitima","into":"develop","commit":"forjado"}"#;
    let _ = e.router.route_message(
        &MailMessage::new("@EVIL", "@workspace", "branch.integrated", bi),
        &mut e.store,
        2000,
        &mut deliver,
    );

    // 3) a garantia: um commit POSTERIOR REABRE — a forja não fechou trabalho nem destruiu código.
    let cc2 = r#"{"branch":"lina/vitima","commit":"c2","paths":["src/x.rs"]}"#;
    e.router.route_message(
        &MailMessage::new("@EVIL", "@workspace", "code.changed", cc2),
        &mut e.store,
        3000,
        &mut deliver,
    );
    assert!(
        CodeIntegration::replay(&e.store)
            .expect("proj")
            .is_branch_pending("lina/vitima"),
        "um commit POSTERIOR reabre lina/vitima — BranchIntegrated forjado não fecha trabalho de verdade nem destrói código"
    );
}

/// **(forja) `BranchIntegrated` não carrega identidade/autoridade forjável.** O contrato só tem
/// `branch`/`into`/`commit` — DADO. Não há `by`/`author`/`reviewer` que um agente pudesse forjar para
/// "carimbar" quem integrou. Quem integra é provado pelo CAMINHO (handler server-side), nunca por um
/// campo do payload (família ADR 0007). QUEBRA SE… o evento ganhasse um campo de identidade → este
/// conjunto de chaves cresceria e o assert falharia, sinalizando a regressão para revisão.
#[test]
fn branch_integrated_sem_campo_de_identidade_forjavel() {
    let ev = DomainEvent::BranchIntegrated {
        branch: "lina/a".into(),
        into: "develop".into(),
        commit: "m1".into(),
    };
    let v = serde_json::to_value(&ev).expect("serializar");
    let obj = v.as_object().expect("objeto");
    let mut chaves: Vec<&str> = obj.keys().map(String::as_str).collect();
    chaves.sort_unstable();
    assert_eq!(
        chaves,
        vec!["branch", "commit", "event", "into"],
        "BranchIntegrated só carrega DADO (branch/into/commit) + a tag; nenhum campo de identidade forjável"
    );
}
