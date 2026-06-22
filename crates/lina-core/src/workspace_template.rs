//! F3-5-10 · frente TEMPLATES (dono: Terminal G) — **gabaritos de Espaço**.
//!
//! Templates de Workspace (doc-fonte 59): o usuário cria um Espaço de "SaaS" ou "Marketing" com
//! roster + configurações já montados, sem montar tudo na mão. Um [`WorkspaceTemplate`] é DADO
//! puro (papéis + `SystemParams` + pistas + foco-que-liga-doutrinas + backlog inicial); instanciá-lo
//! SEMEIA o Espaço encadeando appends de eventos JÁ EXISTENTES — **nenhuma variante nova** (inv #4):
//!
//! | o que o template traz | evento semeado | projeção que reconstrói |
//! |---|---|---|
//! | roster de papéis      | [`DomainEvent::SpawnRequested`]      | nós/spawns no log |
//! | params por Espaço     | [`DomainEvent::SystemParamsChanged`]| [`crate::ParamsLedger`] |
//! | pistas (doc-fonte 65) | [`DomainEvent::ClueSetDefined`]     | [`crate::clue::ClueSet`] |
//! | backlog inicial       | [`DomainEvent::PlanItemAdded`] + [`DomainEvent::PlanItemAttributed`] | [`crate::Plan`] |
//! | doutrinas-como-hooks  | `focus_preset` no `WorkspaceCreated` (via `create`) | [`crate::briefing::focus_builds_software`] |
//!
//! ## Por que a doutrina entra pelo FOCO, e não por um evento de "doutrina"
//! Doutrinas-como-hooks (F3-5-9) **não têm evento próprio**: são ativadas pelo `focus_preset` do
//! Espaço (`briefing.rs::focus_builds_software` casa `dev_app`) e não existe evento pós-`create`
//! que mude o foco. Logo a única forma fiel de "semear a doutrina certa" é o template **carregar o
//! `focus_preset`**, gravado por [`Workspace::create`] (que NÃO tocamos — apenas o invocamos com o
//! foco do template). [`create_workspace_from_template`] materializa o "encadeie DEPOIS do create"
//! do despacho: `create(foco do template)` → [`instanciar`].
//!
//! ## Invariantes honrados
//! - **Template é DADO, JAMAIS autoridade (regra-mãe da onda).** `requested_by` dos spawns semeados
//!   é a ORIGEM SINTÉTICA [`TEMPLATE_ORIGIN`] (carimbo server-side de auditoria, padrão
//!   `STRUCTURAL_JUDGE`/`HUMAN_GESTURE` do router), nunca um nó forjado: o `SpawnRequested` é só um
//!   PEDIDO no log — o terminal físico só nasce pelo funil `admit_node`/gate (ADR 0022). Semear não
//!   cria nó nem escala privilégio. `by` dos params é `"preset:<slug>"` (camada de origem, não
//!   credencial). ZERO LLM: a expansão do template é determinística.
//! - **Determinismo / replay idêntico (inv #4):** [`seed_events`] é PURO (sem relógio, sem
//!   `Uuid::now_v7`) — os ids são derivados do `slug`, então duas instanciações produzem os MESMOS
//!   payloads e as projeções reconstroem byte-a-byte. (O `ts` que `EventStore::append` carimba é
//!   wall-clock, mas nenhuma projeção do template o consome.)
//! - **Compat total:** sem template → [`Workspace::create`] segue inalterado (nenhum dos eventos
//!   acima é emitido).

use std::path::Path;

use crate::events::{AcceptanceCriterion, CheckKind, DomainEvent, Effort, EventStore, StoreError};
use crate::workspace::{Workspace, WorkspaceError};
use crate::NodeId;

/// Origem SINTÉTICA dos `SpawnRequested` semeados por um template (os bytes soletram `TMPL`).
/// Espelha [`STRUCTURAL_JUDGE`](crate::router) (`nil`) / `HUMAN_GESTURE` do router: um `NodeId`
/// fixo e determinístico que **não é um nó do roster** e nunca colide com um nó real (que nasce
/// `Uuid::now_v7`). É CARIMBO DE ORIGEM para auditoria ("este pedido veio de um template"), **jamais
/// autoridade** (regra-mãe ADR 0007): o pedido semeado só vira terminal pelo funil `admit_node`/gate.
pub const TEMPLATE_ORIGIN: NodeId = NodeId::from_u128(0x544D_504C_0000_0000_0000_0000_0000_0000);

/// Um papel pré-definido do roster de um template → um [`DomainEvent::SpawnRequested`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRole {
    /// `@Nome` pedido para o terminal (ex.: `"Arquiteto"`).
    pub name: String,
    /// Papel pedido — casa com os patterns do `role-discovery` (ex.: `"arquiteto"`, `"backend"`).
    pub role: String,
    /// Effort por DIFICULDADE do papel (Arquiteto/Backend = `High`; execução rotineira = `Medium`).
    pub effort: Effort,
    /// Modelo PEDIDO (alias do CLI Profile, inv #3); `None` = o que o CLI já usa.
    pub model: Option<String>,
    /// 1º prompt opcional do spawn (a missão inicial do papel naquele gabarito).
    pub prompt: String,
}

/// Um parâmetro de sistema pré-definido → um [`DomainEvent::SystemParamsChanged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateParam {
    /// Chave canônica (ex.: `"token_budget_day"`, `"fanout_gate"`).
    pub key: String,
    /// Camada que o template aplica (normalmente `"workspace"` — config do Espaço inteiro).
    pub scope: String,
    /// Valor aplicado (string auditável, espelha `SystemParamsChanged.new`).
    pub value: String,
    /// Alvo quando `scope` é `"terminal"`/`"preset"`; `None` para `workspace`/`global`.
    pub target: Option<String>,
}

/// Uma pista pré-definida (doc-fonte 65) → um [`DomainEvent::ClueSetDefined`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateClue {
    /// Escopo da pista (projeto/foco que passa a enxergar os `paths`).
    pub scope: String,
    /// Pastas/arquivos que entram no contexto. DADO, nunca autoridade.
    pub paths: Vec<String>,
    /// Rótulo humano opcional.
    pub label: Option<String>,
}

/// Um item de backlog inicial → [`DomainEvent::PlanItemAdded`] + [`DomainEvent::PlanItemAttributed`]
/// (o `Added` cria o item no [`crate::Plan`]; o `Attributed` anexa o DoD — o `apply_item_attributed`
/// é no-op sem o item existir, por isso ambos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePlanItem {
    /// Id estável do item no plano (ex.: `"saas-auth"`).
    pub id: String,
    /// Descrição legível ao leigo.
    pub desc: String,
    /// Critérios de aceite OBSERVÁVEIS (o setpoint do laço de qualidade).
    pub acceptance: Vec<AcceptanceCriterion>,
    /// Dependências (ids de outros itens).
    pub parents: Vec<String>,
    /// Arquivos/globs que o item reserva.
    pub paths: Vec<String>,
    /// Orçamento de tokens; `0` = herda o do Espaço.
    pub budget_tokens: u64,
}

/// Um gabarito de Espaço: o pacote {foco, roster, params, pistas, backlog} de um template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTemplate {
    /// Id estável do gabarito (semeia ids determinísticos; replay idêntico).
    pub slug: String,
    /// Nome legível do Espaço criado (vira `WorkspaceCreated.name`).
    pub name: String,
    /// Foco que LIGA as doutrinas-como-hooks (`"dev_app"` ativa; `"research_content"` não).
    pub focus_preset: String,
    /// Papéis pré-montados.
    pub roster: Vec<TemplateRole>,
    /// Configurações de Espaço pré-definidas.
    pub params: Vec<TemplateParam>,
    /// Pistas pré-definidas.
    pub clues: Vec<TemplateClue>,
    /// Backlog inicial (pode ser vazio).
    pub plan: Vec<TemplatePlanItem>,
}

impl WorkspaceTemplate {
    /// `root_cause_id` comum dos spawns deste template — âncora de auditoria determinística.
    fn root_cause(&self) -> String {
        format!("template:{}", self.slug)
    }
}

/// **Coração determinístico (PURO).** A sequência EXATA de eventos que instanciar este template
/// semeia, em ordem estável (roster → params → pistas → backlog). Sem relógio, sem `Uuid::now_v7`:
/// os ids derivam do `slug`, logo duas chamadas devolvem `Vec`s idênticos (replay reconstrói
/// byte-a-byte). Testável sem I/O — o `EventStore` só entra em [`instanciar`].
///
/// NÃO inclui o `WorkspaceCreated` (foco/doutrina): esse é do `create`, encadeado ANTES por
/// [`create_workspace_from_template`] (o template não toca a criação — só semeia depois).
#[must_use]
pub fn seed_events(template: &WorkspaceTemplate) -> Vec<DomainEvent> {
    let root_cause_id = template.root_cause();
    let mut events = Vec::new();

    // ── roster → SpawnRequested (PEDIDOS; o terminal só nasce pelo admit_node/gate) ──
    for (idx, r) in template.roster.iter().enumerate() {
        events.push(DomainEvent::SpawnRequested {
            id: format!("template:{}:spawn:{idx}", template.slug),
            requested_by: TEMPLATE_ORIGIN,
            name: r.name.clone(),
            role: r.role.clone(),
            root_cause_id: root_cause_id.clone(),
            hops: 0,
            prompt: r.prompt.clone(),
            model: r.model.clone(),
            effort: Some(r.effort),
            goal_id: None,
        });
    }

    // ── params do Espaço → SystemParamsChanged (by = preset:<slug>, origem de auditoria) ──
    for p in &template.params {
        events.push(DomainEvent::SystemParamsChanged {
            key: p.key.clone(),
            scope: p.scope.clone(),
            new: p.value.clone(),
            target: p.target.clone(),
            old: String::new(),
            by: Some(format!("preset:{}", template.slug)),
        });
    }

    // ── pistas → ClueSetDefined ──
    for c in &template.clues {
        events.push(DomainEvent::ClueSetDefined {
            scope: c.scope.clone(),
            paths: c.paths.clone(),
            label: c.label.clone(),
        });
    }

    // ── backlog → PlanItemAdded (cria) + PlanItemAttributed (anexa o DoD) ──
    for it in &template.plan {
        events.push(DomainEvent::PlanItemAdded {
            item: it.id.clone(),
            desc: it.desc.clone(),
        });
        events.push(DomainEvent::PlanItemAttributed {
            item: it.id.clone(),
            goal_id: None,
            parents: it.parents.clone(),
            acceptance: it.acceptance.clone(),
            budget_tokens: it.budget_tokens,
            paths: it.paths.clone(),
        });
    }

    events
}

/// Semeia o template no `store` JÁ ABERTO (encadeia [`seed_events`] via `append`). Devolve quantos
/// eventos foram semeados. Idempotência é responsabilidade do caller (instanciar 2× no MESMO store
/// duplica os pedidos) — o uso canônico é UMA vez, sobre um Espaço recém-criado.
///
/// # Errors
/// Falha ao persistir um evento no event store.
pub fn instanciar(
    template: &WorkspaceTemplate,
    store: &mut EventStore,
) -> Result<usize, StoreError> {
    let events = seed_events(template);
    for ev in &events {
        store.append(ev)?;
    }
    Ok(events.len())
}

/// Cria um Espaço A PARTIR de um template: `create(name, foco do template)` → [`instanciar`]. É o
/// "instanciar um template" COMPLETO do gate (f) — o `create` grava o `focus_preset` (que liga as
/// doutrinas-como-hooks), e o seed encadeia roster + params + pistas + backlog DEPOIS. Não toca
/// `Workspace::create` (apenas o invoca com o foco do template).
///
/// # Errors
/// Falha ao criar o Espaço (já existe / I/O) ou ao semear os eventos.
pub fn create_workspace_from_template(
    root: impl AsRef<Path>,
    template: &WorkspaceTemplate,
    default_cwd: Option<&Path>,
) -> Result<Workspace, WorkspaceError> {
    let mut ws = Workspace::create(root, &template.name, &template.focus_preset, default_cwd)?;
    instanciar(template, ws.store_mut())?;
    Ok(ws)
}

// ───────────────────────────── catálogo de gabaritos embutidos ─────────────────────────────

/// Açúcar para um critério de aceite por revisão humana (o default conservador da onda).
fn human_review(desc: &str) -> AcceptanceCriterion {
    AcceptanceCriterion {
        desc: desc.to_string(),
        check_kind: CheckKind::HumanReview,
        check_arg: None,
    }
}

/// Açúcar para um papel do roster.
fn role(name: &str, role: &str, effort: Effort, prompt: &str) -> TemplateRole {
    TemplateRole {
        name: name.to_string(),
        role: role.to_string(),
        effort,
        model: None,
        prompt: prompt.to_string(),
    }
}

/// Gabarito **"Construir um SaaS"**: foco `dev_app` (liga as doutrinas de arquitetura/segurança/
/// código), time Arquiteto + Backend + Frontend + QA com efforts por dificuldade (os papéis que
/// decidem estrutura/contrato rodam em `High`; a execução rotineira em `Medium`), config de Espaço
/// de trabalho longo e um backlog inicial.
#[must_use]
pub fn template_saas() -> WorkspaceTemplate {
    WorkspaceTemplate {
        slug: "saas".to_string(),
        name: "Construir um SaaS".to_string(),
        focus_preset: "dev_app".to_string(),
        roster: vec![
            role(
                "Arquiteto",
                "arquiteto",
                Effort::High,
                "Defina a arquitetura e os contratos antes de a equipe codar.",
            ),
            role(
                "Backend",
                "backend",
                Effort::High,
                "Monte a API, o banco e a lógica de servidor conforme os contratos.",
            ),
            role(
                "Frontend",
                "frontend",
                Effort::Medium,
                "Implemente as telas a partir dos contratos definidos.",
            ),
            role(
                "QA",
                "qa",
                Effort::Medium,
                "Valide cada entrega contra os critérios de aceite.",
            ),
        ],
        params: vec![
            TemplateParam {
                key: "token_budget_day".to_string(),
                scope: "workspace".to_string(),
                value: "200000".to_string(),
                target: None,
            },
            TemplateParam {
                key: "fanout_gate".to_string(),
                scope: "workspace".to_string(),
                value: "4".to_string(),
                target: None,
            },
        ],
        clues: vec![TemplateClue {
            scope: "saas".to_string(),
            paths: vec!["src/".to_string(), "docs/arquitetura.md".to_string()],
            label: Some("código e documentação do produto".to_string()),
        }],
        plan: vec![
            TemplatePlanItem {
                id: "saas-auth".to_string(),
                desc: "Montar autenticação (cadastro + login)".to_string(),
                acceptance: vec![human_review("o usuário cria conta e entra sem erro")],
                parents: Vec::new(),
                paths: Vec::new(),
                budget_tokens: 0,
            },
            TemplatePlanItem {
                id: "saas-api".to_string(),
                desc: "Expor a API principal do produto".to_string(),
                acceptance: vec![human_review("a API responde às rotas do MVP")],
                parents: vec!["saas-auth".to_string()],
                paths: Vec::new(),
                budget_tokens: 0,
            },
        ],
    }
}

/// Gabarito **"Marketing"**: foco `research_content` (sem doutrinas de software — não é dev de
/// app), time Writer + Designer, orçamento menor e pistas dos materiais de marca.
#[must_use]
pub fn template_marketing() -> WorkspaceTemplate {
    WorkspaceTemplate {
        slug: "marketing".to_string(),
        name: "Marketing".to_string(),
        focus_preset: "research_content".to_string(),
        roster: vec![
            role(
                "Writer",
                "writer",
                Effort::Medium,
                "Escreva a copy e os textos de campanha no tom da marca.",
            ),
            role(
                "Designer",
                "design",
                Effort::Medium,
                "Crie as peças visuais alinhadas à identidade da marca.",
            ),
        ],
        params: vec![TemplateParam {
            key: "token_budget_day".to_string(),
            scope: "workspace".to_string(),
            value: "80000".to_string(),
            target: None,
        }],
        clues: vec![TemplateClue {
            scope: "marketing".to_string(),
            paths: vec!["campanhas/".to_string(), "marca/".to_string()],
            label: Some("materiais de marca e campanhas".to_string()),
        }],
        plan: vec![TemplatePlanItem {
            id: "mkt-landing".to_string(),
            desc: "Criar a página de vendas".to_string(),
            acceptance: vec![human_review("a página abre e comunica a oferta")],
            parents: Vec::new(),
            paths: Vec::new(),
            budget_tokens: 0,
        }],
    }
}

/// Todos os gabaritos embutidos, em ordem estável (a galeria F3-5-10 os lista).
#[must_use]
pub fn builtin_templates() -> Vec<WorkspaceTemplate> {
    vec![template_saas(), template_marketing()]
}

/// Busca um gabarito embutido pelo `slug` (o que o verbo `lina template <slug>` resolve).
#[must_use]
pub fn template_by_slug(slug: &str) -> Option<WorkspaceTemplate> {
    builtin_templates().into_iter().find(|t| t.slug == slug)
}

/// Relatório legível dos gabaritos disponíveis (handler do verbo `lina template` / a galeria).
/// Honesto: lista cada gabarito com o que ele monta, sem jargão.
#[must_use]
pub fn list_templates_report() -> String {
    let mut out = String::from("gabaritos de Espaço (criam um Espaço já montado):\n");
    for t in builtin_templates() {
        out.push_str(&format!(
            "  [{}] {} — {} {}, {} config, {} {}, {} {}\n",
            t.slug,
            t.name,
            t.roster.len(),
            plural(t.roster.len(), "pessoa no time", "pessoas no time"),
            t.params.len(),
            t.clues.len(),
            plural(t.clues.len(), "pista", "pistas"),
            t.plan.len(),
            plural(t.plan.len(), "tarefa inicial", "tarefas iniciais"),
        ));
    }
    out.trim_end().to_string()
}

/// Plural pt-br simples (1 → singular; resto → plural). Os rótulos já chegam como `&'static str`.
fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::focus_builds_software;
    use crate::clue::ClueSet;
    use crate::params::ParamsLedger;
    use uuid::Uuid;

    /// Event store temporário (convenção do crate — espelha `params.rs`/`events.rs` tests).
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-template-{tag}-{}", Uuid::now_v7()));
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

    fn count_kind(events: &[crate::events::EventRecord], kind: &str) -> usize {
        events.iter().filter(|e| e.kind == kind).count()
    }

    /// Critério (f): instanciar SEMEIA o log com roster + params + pistas + backlog na ORDEM certa.
    #[test]
    fn instanciar_semeia_o_log_dos_quatro_tipos() {
        let tmp = TempDir::new("seed");
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        let t = template_saas();

        let n = instanciar(&t, &mut store).expect("instanciar");
        let events = store.events().expect("ler log");

        // roster (4) + params (2) + pistas (1) + backlog (2 itens × 2 eventos = 4) = 11
        assert_eq!(n, 11, "contagem de eventos semeados");
        assert_eq!(
            count_kind(&events, "SpawnRequested"),
            4,
            "4 papéis no roster"
        );
        assert_eq!(count_kind(&events, "SystemParamsChanged"), 2, "2 params");
        assert_eq!(count_kind(&events, "ClueSetDefined"), 1, "1 pista");
        assert_eq!(count_kind(&events, "PlanItemAdded"), 2, "2 itens criados");
        assert_eq!(
            count_kind(&events, "PlanItemAttributed"),
            2,
            "2 itens com DoD"
        );
    }

    /// Critério (f): replay reconstrói o Espaço semeado — params/pistas/plano pelas projeções reais.
    #[test]
    fn replay_reconstroi_params_pistas_e_plano() {
        let tmp = TempDir::new("replay");
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        let t = template_saas();
        instanciar(&t, &mut store).expect("instanciar");

        // params → ParamsLedger (camada workspace)
        let ledger = ParamsLedger::replay(&store).expect("replay params");
        assert_eq!(
            ledger.workspace.token_budget_day,
            Some(200_000),
            "token_budget_day do template entra na camada workspace"
        );

        // pistas → ClueSet
        let clues = ClueSet::replay(&store).expect("replay clues");
        assert_eq!(
            clues.paths_for("saas"),
            &["src/".to_string(), "docs/arquitetura.md".to_string()],
            "a pista do template é reconstruída por replay"
        );

        // backlog → Plan (item criado E com o DoD anexado)
        let plan = store.project().expect("project").plan;
        let item = plan
            .itens
            .iter()
            .find(|i| i.id == "saas-auth")
            .expect("item saas-auth existe no plano");
        assert!(
            !item.acceptance.is_empty(),
            "PlanItemAttributed anexou o critério de aceite (não ficou no-op)"
        );
        assert!(
            plan.itens.iter().any(|i| i.id == "saas-api"),
            "o segundo item do backlog também foi semeado"
        );
    }

    /// Critério (f): reabrir o store → projeções IDÊNTICAS (replay determinístico, inv #4).
    #[test]
    fn reabrir_o_store_reproduz_as_projecoes() {
        let tmp = TempDir::new("reopen");
        let t = template_marketing();
        {
            let mut store = EventStore::open(tmp.path()).expect("abrir store");
            instanciar(&t, &mut store).expect("instanciar");
        }
        // Nova abertura, do zero, do mesmo diretório.
        let reaberto = EventStore::open(tmp.path()).expect("reabrir store");
        let clues = ClueSet::replay(&reaberto).expect("replay clues");
        assert_eq!(
            clues.paths_for("marketing"),
            &["campanhas/".to_string(), "marca/".to_string()],
            "as pistas sobrevivem a uma reabertura (são do log, não de memória)"
        );
        let ledger = ParamsLedger::replay(&reaberto).expect("replay params");
        assert_eq!(ledger.workspace.token_budget_day, Some(80_000));
    }

    /// Determinismo do coração puro (inv #4): instanciar 2× produz os MESMOS eventos (ids derivam do
    /// slug — zero `Uuid::now_v7`/relógio). Sem isso, replay não reconstruiria "idêntico".
    #[test]
    fn seed_events_e_deterministico() {
        let t = template_saas();
        assert_eq!(
            seed_events(&t),
            seed_events(&t),
            "duas expansões do mesmo template são byte-a-byte iguais"
        );
        // E os ids carregam a origem do template (auditável, não-aleatório).
        match &seed_events(&t)[0] {
            DomainEvent::SpawnRequested {
                id,
                requested_by,
                root_cause_id,
                effort,
                ..
            } => {
                assert_eq!(id, "template:saas:spawn:0");
                assert_eq!(root_cause_id, "template:saas");
                assert_eq!(
                    *requested_by, TEMPLATE_ORIGIN,
                    "origem sintética, não nó real"
                );
                assert_eq!(*effort, Some(Effort::High), "Arquiteto roda em High");
            }
            other => {
                panic!("o 1º evento deveria ser o SpawnRequested do Arquiteto, veio {other:?}")
            }
        }
    }

    /// SEGURANÇA (regra-mãe): a origem do template NUNCA é um nó real do roster — é a sentinela
    /// determinística, distinta de qualquer `Uuid::now_v7`. Forjá-la não confere identidade
    /// (o nó só nasce pelo admit_node/gate; aqui só há um PEDIDO no log).
    #[test]
    fn origem_de_template_e_sentinela_nao_no_real() {
        assert_ne!(
            TEMPLATE_ORIGIN,
            NodeId::nil(),
            "distinta do STRUCTURAL_JUDGE"
        );
        // Um nó real nasce de Uuid::now_v7 (versão 7); a sentinela do template não tem versão v7.
        assert_ne!(
            TEMPLATE_ORIGIN.get_version_num(),
            7,
            "a origem do template nunca coincide com um NodeId real (v7)"
        );
        for ev in seed_events(&template_marketing()) {
            if let DomainEvent::SpawnRequested { requested_by, .. } = ev {
                assert_eq!(
                    requested_by, TEMPLATE_ORIGIN,
                    "todo spawn carimba a sentinela"
                );
            }
        }
    }

    /// Critério (f): instanciar pela GALERIA (foco do template → doutrinas-como-hooks) — o caminho
    /// completo. O SaaS é foco `dev_app` (doutrinas ON); o Marketing não.
    #[test]
    fn create_from_template_grava_o_foco_que_liga_as_doutrinas() {
        let tmp_saas = TempDir::new("create-saas");
        let saas = template_saas();
        let mut ws =
            create_workspace_from_template(tmp_saas.path(), &saas, None).expect("criar saas");
        let proj = ws.store_mut().project().expect("project");
        assert_eq!(
            proj.focus_preset.as_deref(),
            Some("dev_app"),
            "o foco do template é gravado no WorkspaceCreated"
        );
        assert!(
            focus_builds_software(proj.focus_preset.as_deref().unwrap_or("")),
            "foco dev_app → as doutrinas-como-hooks entram no briefing"
        );
        // e o seed entrou DEPOIS do create (roster presente no mesmo log).
        let events = ws.store_mut().events().expect("ler log");
        assert_eq!(count_kind(&events, "SpawnRequested"), 4);
        assert_eq!(count_kind(&events, "WorkspaceCreated"), 1);

        let tmp_mkt = TempDir::new("create-mkt");
        let mkt = template_marketing();
        let mut ws2 =
            create_workspace_from_template(tmp_mkt.path(), &mkt, None).expect("criar mkt");
        let proj2 = ws2.store_mut().project().expect("project");
        assert!(
            !focus_builds_software(proj2.focus_preset.as_deref().unwrap_or("")),
            "marketing não é dev de software → sem doutrinas-como-hooks"
        );
    }

    /// Critério (f): SEM template → `Workspace::create` segue INALTERADO (nenhum evento de seed).
    #[test]
    fn sem_template_a_criacao_e_inalterada() {
        let tmp = TempDir::new("plain");
        let mut ws =
            Workspace::create(tmp.path(), "Espaço em branco", "blank", None).expect("criar");
        let events = ws.store_mut().events().expect("ler log");
        assert_eq!(count_kind(&events, "SpawnRequested"), 0);
        assert_eq!(count_kind(&events, "SystemParamsChanged"), 0);
        assert_eq!(count_kind(&events, "ClueSetDefined"), 0);
        assert_eq!(count_kind(&events, "PlanItemAdded"), 0);
        assert_eq!(count_kind(&events, "PlanItemAttributed"), 0);
    }

    /// O catálogo embutido resolve por slug e o relatório é honesto (lista o que monta).
    #[test]
    fn catalogo_resolve_por_slug_e_relata() {
        assert_eq!(builtin_templates().len(), 2);
        assert_eq!(
            template_by_slug("saas").map(|t| t.name),
            Some("Construir um SaaS".to_string())
        );
        assert_eq!(
            template_by_slug("marketing").map(|t| t.slug),
            Some("marketing".to_string())
        );
        assert!(template_by_slug("inexistente").is_none());

        let report = list_templates_report();
        assert!(report.contains("[saas]"));
        assert!(report.contains("Construir um SaaS"));
        assert!(report.contains("[marketing]"));
    }
}
