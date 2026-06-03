//! **W4-5 · Galeria de Focos (T3) — presets que montam o Espaço com time + papéis, 100% local.**
//!
//! Módulo **gpui-free, puro e testável** (mesmo padrão de [`crate::wiring`]/[`crate::canvas`]): aqui
//! mora a LÓGICA durável de W4-5; o render gpui da galeria (grid de cards) a CONSOME no shell.
//!
//! Dois passos, na ordem da realidade do core:
//! 1. [`preset_team`] — **puro** — deriva o time `{nome, papel}` pelo **role-discovery** (W3-1). NÃO
//!    cunha `NodeId`: o id é alocado pelo `Supervisor` no spawn-vivo (fonte ÚNICA de id), e a galeria
//!    NÃO o duplica (senão a projeção e o roster `agents.json` divergiriam).
//! 2. [`apply_preset`] — **event-sourcing** (inv #4) — recebe os agentes JÁ com o `NodeId` que o
//!    shell alocou ([`PlacedAgent`]) e loga `WorkspaceCreated{focus_preset}` + por agente
//!    `NodeAdded`(Terminal) + `NodeRoleAssigned`. **NADA sai da máquina** (inv #2): só I/O do event
//!    log local — zero rede, zero PTY aqui (o PTY é cabeado pelo shell via `wire_terminal`, W3-2).
//!
//! Fluxo do shell (a cabear no render T3 — validação NA TELA):
//! ```ignore
//! let team = gallery::preset_team(preset, &registry);                  // {nome, papel} (puro)
//! let placed: Vec<_> = team.iter().map(|a| {
//!     let (node, _grid) = wire_terminal(&mut pty, &sup, …, &a.name, …); // Supervisor aloca o NodeId
//!     gallery::PlacedAgent { node, name: a.name.clone(), role: a.role.clone() }
//! }).collect();
//! gallery::apply_preset(preset, ws_name, &placed, &mut store)?;          // loga o MESMO id
//! ```
//!
//! ⚠️ **[A CONFIRMAR] taxonomia:** uso a do épico W4-5 (`dev_app|research_content|blank`); a
//! Arquitetura §9.2 lista `dev_app|landing|automation|design` — DIVERGEM (decidir o canônico do MVP).
//! A **composição de cada time** também não é fixada no épico (só os nomes dos presets) → defaults
//! sensatos, igualmente **[A CONFIRMAR]**.

// Módulo-biblioteca aguardando o WIRING do render gpui da galeria T3 no shell: a API pública é
// CONSUMIDA pelo render (a cabear) e, por ora, exercida pelos testes — mesmo padrão de `canvas.rs`/
// `onboarding.rs` (`allow(dead_code)` para o que ainda só os testes usam). Remover ao wirar o render.
#![allow(dead_code)]

use lina_core::{DomainEvent, EventStore, StoreError};
use lina_host::NodeId;
use lina_role_discovery::RoleRegistry;

/// Presets de Foco do MVP (épico W4-5). `Blank` é o caminho de fuga (Espaço vazio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPreset {
    /// "App" — time de desenvolvimento de produto.
    DevApp,
    /// "Pesquisa & Conteúdo" — curadoria + redação.
    ResearchContent,
    /// "Em Branco" — Espaço vazio (caminho de fuga).
    Blank,
}

impl FocusPreset {
    /// Todos os presets, na ordem de exibição da galeria (App → Pesquisa → Em Branco).
    #[must_use]
    pub fn all() -> [FocusPreset; 3] {
        [
            FocusPreset::DevApp,
            FocusPreset::ResearchContent,
            FocusPreset::Blank,
        ]
    }

    /// Id canônico gravado em `WorkspaceCreated.focus_preset` (projeção SQLite). Estável (contrato).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            FocusPreset::DevApp => "dev_app",
            FocusPreset::ResearchContent => "research_content",
            FocusPreset::Blank => "blank",
        }
    }

    /// Rótulo do card (PT-BR, zero jargão — inv #6, não-técnico-first).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FocusPreset::DevApp => "App",
            FocusPreset::ResearchContent => "Pesquisa & Conteúdo",
            FocusPreset::Blank => "Em Branco",
        }
    }

    /// Descrição curta do card (o que o leigo ganha ao escolher).
    #[must_use]
    pub fn blurb(self) -> &'static str {
        match self {
            FocusPreset::DevApp => {
                "Um time de produto pronto: arquiteto, back-end, front-end e qualidade."
            }
            FocusPreset::ResearchContent => {
                "Curadoria e redação para pesquisa e conteúdo, já com papéis."
            }
            FocusPreset::Blank => "Comece do zero — você adiciona agentes quando quiser.",
        }
    }

    /// Nomes-semente do time; o **papel** é DERIVADO por nome via role-discovery (W3-1). `Blank` = vazio.
    /// **[A CONFIRMAR]** — composição não fixada no épico.
    #[must_use]
    fn team_seed(self) -> &'static [&'static str] {
        match self {
            FocusPreset::DevApp => &["@Arquiteto", "@Dev Backend", "@Dev Frontend", "@QA"],
            FocusPreset::ResearchContent => &["@Curador", "@Redator"],
            FocusPreset::Blank => &[],
        }
    }
}

/// Um membro do time a montar: nome-semente + papel derivado (role-discovery). **Sem `NodeId`** — o
/// id é do `Supervisor`, alocado no spawn-vivo do shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetAgent {
    pub name: String,
    pub role: String,
}

/// Um agente JÁ posicionado: o [`PresetAgent`] + o `NodeId` que o `Supervisor` alocou. É o que
/// [`apply_preset`] loga (o MESMO id que o roster vivo/`agents.json` usa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedAgent {
    pub node: NodeId,
    pub name: String,
    pub role: String,
}

/// Deriva o time de um preset — nome-semente → papel via role-discovery (W3-1). **Puro** (sem I/O,
/// sem rede, sem cunhar id).
#[must_use]
pub fn preset_team(preset: FocusPreset, registry: &RoleRegistry) -> Vec<PresetAgent> {
    preset
        .team_seed()
        .iter()
        .map(|name| PresetAgent {
            name: (*name).to_string(),
            role: registry.infer_role(name).role,
        })
        .collect()
}

/// **W4-5 — aplica um preset (camada durável, local-first).** Loga no `store`, em ordem:
/// `WorkspaceCreated{focus_preset}` e, por agente JÁ posicionado, `NodeAdded`(kind `Terminal`) +
/// `NodeRoleAssigned`. **Não abre rede** (inv #2) e **não spawna PTY** (runtime do shell). `team` vem
/// do shell, que alocou o `NodeId` de cada agente via `Supervisor` (id ÚNICO, igual ao roster vivo).
///
/// `Blank` (team vazio) loga só `WorkspaceCreated{focus_preset:"blank"}` — Espaço vazio (fuga).
///
/// # Errors
/// [`StoreError`] na 1ª falha de persistência do event log (aborta — sem meio-termo silencioso).
pub fn apply_preset(
    preset: FocusPreset,
    workspace_name: &str,
    team: &[PlacedAgent],
    store: &mut EventStore,
) -> Result<(), StoreError> {
    store.append(&DomainEvent::WorkspaceCreated {
        name: workspace_name.to_string(),
        focus_preset: preset.id().to_string(),
    })?;

    // Coordenadas-semente em linha; o canvas (W4-2) reposiciona — aqui só não nascem sobrepostos.
    for (i, agent) in team.iter().enumerate() {
        let x = 30.0 + (i as f64) * 360.0;
        store.append(&DomainEvent::NodeAdded {
            node: agent.node,
            kind: "Terminal".to_string(),
            x,
            y: 96.0,
        })?;
        store.append(&DomainEvent::NodeRoleAssigned {
            node: agent.node,
            role: agent.role.clone(),
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid; // `uuid` é dev-dependency — fabrica `NodeId` no teste (o que o Supervisor faria).

    fn registry() -> RoleRegistry {
        RoleRegistry::with_defaults().expect("default-roles.yaml embutido compila")
    }

    fn temp_store(tag: &str) -> (EventStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lina-gallery-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = EventStore::open(&dir).expect("abrir EventStore temp");
        (store, dir)
    }

    /// Simula o shell: deriva o time e "aloca" um `NodeId` por agente (o que `Supervisor::register`
    /// faria no spawn-vivo), devolvendo os [`PlacedAgent`] que `apply_preset` loga.
    fn place(preset: FocusPreset, reg: &RoleRegistry) -> Vec<PlacedAgent> {
        preset_team(preset, reg)
            .into_iter()
            .map(|a| PlacedAgent {
                node: Uuid::now_v7(),
                name: a.name,
                role: a.role,
            })
            .collect()
    }

    /// Aceite W4-5: preset "App" cria um Espaço com `focus_preset='dev_app'` na projeção + N nós, cada
    /// um com o papel DERIVADO pelo role-discovery (W3-1) — não um papel hardcoded na galeria.
    #[test]
    fn apply_dev_app_logs_focus_preset_and_team_with_roles() {
        let reg = registry();
        let (mut store, dir) = temp_store("devapp");
        let placed = place(FocusPreset::DevApp, &reg);
        assert_eq!(placed.len(), 4, "time de 4 agentes");

        apply_preset(FocusPreset::DevApp, "App de Teste", &placed, &mut store).expect("apply");

        let state = store.project().expect("projetar");
        assert_eq!(
            state.focus_preset.as_deref(),
            Some("dev_app"),
            "focus_preset na projecao SQLite"
        );
        assert_eq!(state.nodes.len(), 4, "4 nos na projecao");

        // Cada nó projetado tem o papel inferido pelo role-discovery p/ o seu nome (prova a W3-1).
        for agent in &placed {
            let expected = reg.infer_role(&agent.name).role;
            assert_eq!(agent.role, expected, "papel via role-discovery");
            let node = state.nodes.get(&agent.node).expect("no na projecao");
            assert_eq!(
                node.role.as_deref(),
                Some(expected.as_str()),
                "papel projetado"
            );
            assert_eq!(node.kind, "Terminal");
        }

        // O time de "App" cobre os papéis de produto esperados (arquitetura + back + front + QA).
        let roles: std::collections::BTreeSet<&str> =
            placed.iter().map(|a| a.role.as_str()).collect();
        for must in ["ARQUITETO", "BACKEND", "FRONTEND", "QA"] {
            assert!(
                roles.contains(must),
                "preset App deve ter o papel {must}; veio {roles:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Aceite W4-5: "Em Branco" cria Espaço VAZIO (caminho de fuga) — só o `WorkspaceCreated`, 0 nós.
    #[test]
    fn apply_blank_is_empty_escape_hatch() {
        let reg = registry();
        let (mut store, dir) = temp_store("blank");
        let placed = place(FocusPreset::Blank, &reg);
        assert!(placed.is_empty(), "Em Branco nao spawna ninguem");

        apply_preset(FocusPreset::Blank, "Vazio", &placed, &mut store).expect("apply blank");

        let state = store.project().expect("projetar");
        assert_eq!(state.focus_preset.as_deref(), Some("blank"));
        assert!(state.nodes.is_empty(), "Espaco vazio");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Local-first (inv #2): o ÚNICO efeito de `apply_preset` é o event log — exatamente
    /// `1 WorkspaceCreated + N*(NodeAdded + NodeRoleAssigned)`, nada mais (sem rede, sem PTY, sem
    /// evento surpresa). Prova estrutural de que a criação não dispara nada além do log local.
    #[test]
    fn apply_preset_side_effects_are_only_local_events() {
        let reg = registry();
        let (mut store, dir) = temp_store("localfirst");
        let placed = place(FocusPreset::DevApp, &reg);

        apply_preset(FocusPreset::DevApp, "App", &placed, &mut store).expect("apply");

        let kinds: Vec<String> = store
            .events()
            .expect("events")
            .into_iter()
            .map(|r| r.kind)
            .collect();
        // 1 WorkspaceCreated + 4*(NodeAdded + NodeRoleAssigned) = 9 eventos, todos locais.
        assert_eq!(kinds.len(), 9, "exatamente 9 eventos; veio {kinds:?}");
        assert_eq!(kinds[0], "WorkspaceCreated");
        assert_eq!(
            kinds.iter().filter(|k| *k == "NodeAdded").count(),
            4,
            "4 NodeAdded"
        );
        assert_eq!(
            kinds.iter().filter(|k| *k == "NodeRoleAssigned").count(),
            4,
            "4 NodeRoleAssigned"
        );
        // Nenhum evento de rede/entrega/discovery foi disparado na criação.
        for forbidden in ["MessageRouted", "MessageDelivered", "DiscoveryIndexed"] {
            assert!(
                !kinds.iter().any(|k| k == forbidden),
                "criacao nao deve disparar {forbidden}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `preset_team` deriva papéis pelo role-discovery (W3-1), sem cunhar id (isso é do Supervisor).
    #[test]
    fn preset_team_uses_role_discovery_without_minting_ids() {
        let reg = registry();
        let team = preset_team(FocusPreset::ResearchContent, &reg);
        assert_eq!(team.len(), 2);
        for agent in &team {
            assert_eq!(agent.role, reg.infer_role(&agent.name).role);
        }
    }
}
