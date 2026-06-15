//! **W4-2 · P4 — Inspetor de Nó (painel lateral do nó SELECIONADO).**
//!
//! Módulo **gpui-free, puro e testável** (mesmo padrão de `wiring.rs`/`canvas.rs`/`gallery.rs`): aqui
//! mora a LÓGICA do inspetor; o render gpui do painel a CONSOME no shell.
//!
//! **SÓ-LEITURA** — não muta o nó. Monta os dados de exibição do nó selecionado a partir de:
//! - a **projeção** do nó ([`lina_core::ProjectedNode`]): `name`/`role` (`NodeRoleAssigned`, W3-1)/
//!   `cli` (`CliProfileSet`)/`status` — todos do event log (inv #4);
//! - a **activity** idle/busy derivada do `damage` do `lina-vt` ([`activity_from_damage`]).
//!
//! O nó selecionado é o `self.focused` do root view (já existe — o card com borda no canvas, é o que
//! recebe input); o render passa `Some((&ProjectedNode, Activity))` ao [`inspect`]. **NÃO precisa de
//! hook novo de seleção em bridge** (ver `.entrega-p4.md` para a nota de wiring/otimização).

// Módulo-biblioteca aguardando o WIRING do render gpui do painel P4 no shell: a API pública é
// CONSUMIDA pelo render (a cabear) e, por ora, exercida pelos testes — mesmo padrão de `canvas.rs`/
// `onboarding.rs`/`gallery.rs` (`allow(dead_code)` até o render consumir). Remover ao wirar o painel.
#![allow(dead_code)]

use lina_core::ProjectedNode;

/// Atividade do nó derivada do `damage` do grid (lina-vt): houve saída recente? Conceito SEPARADO do
/// `status` de lifecycle (Iniciando/Ativo/Encerrado) — um nó "Ativo" pode estar Ocioso ou Trabalhando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Sem saída na janela observada — o agente está esperando/parado.
    Idle,
    /// Saída recente — o agente está produzindo (pintando o grid).
    Busy,
}

impl Activity {
    /// Rótulo PT-BR (não-técnico, inv #6).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Activity::Busy => "Trabalhando",
            Activity::Idle => "Ocioso",
        }
    }
}

/// Activity a partir do `damage` observado (linhas alteradas desde o último `reset_damage`). `Busy` se
/// houve QUALQUER dano na janela. A leitura de `damaged_rows()` é **não-destrutiva** (coexiste com o
/// reset incremental do canvas); o render deve passar o dano acumulado na sua janela de refresh do
/// inspetor (não um único frame) para o indicador não tremular.
#[must_use]
pub fn activity_from_damage(damaged_rows: usize) -> Activity {
    if damaged_rows > 0 {
        Activity::Busy
    } else {
        Activity::Idle
    }
}

/// Rótulo PT-BR do `status` de lifecycle do nó (projeção: `Running`/`Starting`/`Dead`/…). Desconhecido
/// passa cru (o role-discovery/core pode evoluir sem quebrar o painel).
#[must_use]
pub fn status_label(raw: &str) -> String {
    match raw {
        "Running" => "Ativo".to_string(),
        "Starting" => "Iniciando".to_string(),
        "Dead" => "Encerrado".to_string(),
        other => other.to_string(),
    }
}

/// Dados de exibição do inspetor — já prontos p/ o render, com fallbacks legíveis p/ campos ausentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorData {
    /// `false` → nada selecionado (painel mostra o estado vazio).
    pub selected: bool,
    pub name: String,
    pub role: String,
    pub activity: Activity,
    pub cli_profile: String,
    pub status: String,
}

const NO_SELECTION: &str = "Nenhum nó selecionado";
const NO_NAME: &str = "(sem nome)";
const NO_ROLE: &str = "— ainda sem papel —";
const NO_PROFILE: &str = "— perfil de CLI não definido —";
const NO_STATUS: &str = "—";

/// **Monta o inspetor do nó selecionado.** `None` → painel vazio ("Nenhum nó selecionado").
/// `Some((node, activity))` → os campos do `ProjectedNode` + a activity, com fallbacks p/ os `None`
/// (um nó recém-criado pode ainda não ter papel/perfil — o painel não some nem mostra "None").
#[must_use]
pub fn inspect(selected: Option<(&ProjectedNode, Activity)>) -> InspectorData {
    match selected {
        None => InspectorData {
            selected: false,
            name: NO_SELECTION.to_string(),
            role: NO_ROLE.to_string(),
            activity: Activity::Idle,
            cli_profile: NO_PROFILE.to_string(),
            status: NO_STATUS.to_string(),
        },
        Some((node, activity)) => InspectorData {
            selected: true,
            name: node.name.clone().unwrap_or_else(|| NO_NAME.to_string()),
            role: node.role.clone().unwrap_or_else(|| NO_ROLE.to_string()),
            activity,
            cli_profile: node.cli.clone().unwrap_or_else(|| NO_PROFILE.to_string()),
            status: node
                .status
                .as_deref()
                .map_or_else(|| NO_STATUS.to_string(), status_label),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constrói um `ProjectedNode` de terminal (helper de teste — o que a projeção do log produziria).
    fn node(
        name: Option<&str>,
        role: Option<&str>,
        cli: Option<&str>,
        status: Option<&str>,
    ) -> ProjectedNode {
        ProjectedNode {
            kind: "Terminal".to_string(),
            name: name.map(str::to_string),
            role: role.map(str::to_string),
            status: status.map(str::to_string),
            x: 0.0,
            y: 0.0,
            cli: cli.map(str::to_string),
            // Costura rodada 360 (ADR 0022): campo novo da projeção; fixture sem cwd.
            cwd: None,
            // Costura F1-0-3 (coordenada via Maestro): campo novo da projeção; fixture sem WARN.
            stalled: false,
            // Costura F2-3-2 (coordenada via Maestro): dims da projeção (resize); fixture sem resize.
            cols: None,
            rows: None,
            w: None,
            h: None,
        }
    }

    /// Aceite P4: nó selecionado COMPLETO → o painel mostra nome/papel/perfil/status corretos +
    /// activity. (O caminho feliz do critério "selecionar um nó → inspetor mostra ... corretos".)
    #[test]
    fn inspect_full_node_shows_all_fields() {
        let n = node(
            Some("@Arquiteto"),
            Some("ARQUITETO"),
            Some("claude-code"),
            Some("Running"),
        );
        let data = inspect(Some((&n, Activity::Busy)));
        assert!(data.selected);
        assert_eq!(data.name, "@Arquiteto");
        assert_eq!(data.role, "ARQUITETO");
        assert_eq!(data.cli_profile, "claude-code");
        assert_eq!(data.activity, Activity::Busy);
        assert_eq!(data.activity.label(), "Trabalhando");
        assert_eq!(data.status, "Ativo", "Running → PT-BR");
    }

    /// Nada selecionado → painel vazio (não some, não mostra lixo).
    #[test]
    fn inspect_none_is_empty_panel() {
        let data = inspect(None);
        assert!(!data.selected);
        assert_eq!(data.name, "Nenhum nó selecionado");
    }

    /// Nó recém-criado SEM papel/perfil → fallbacks legíveis (nunca "None"/vazio).
    #[test]
    fn inspect_missing_role_and_profile_uses_fallbacks() {
        let n = node(Some("@Novo"), None, None, Some("Starting"));
        let data = inspect(Some((&n, Activity::Idle)));
        assert_eq!(data.role, "— ainda sem papel —");
        assert_eq!(data.cli_profile, "— perfil de CLI não definido —");
        assert_eq!(data.status, "Iniciando");
        assert_eq!(data.activity.label(), "Ocioso");
    }

    /// Aceite P4: TROCAR a seleção atualiza o painel (dados de A ≠ dados de B).
    #[test]
    fn selection_change_updates_inspector() {
        let a = node(Some("@A"), Some("BACKEND"), Some("claude"), Some("Running"));
        let b = node(Some("@B"), Some("QA"), Some("codex"), Some("Dead"));
        let da = inspect(Some((&a, Activity::Busy)));
        let db = inspect(Some((&b, Activity::Idle)));
        assert_ne!(da, db, "trocar de nó muda o inspetor");
        assert_eq!(db.role, "QA");
        assert_eq!(db.status, "Encerrado");
        assert_eq!(db.activity, Activity::Idle);
    }

    /// `activity_from_damage`: qualquer dano observado → Busy; nenhum → Idle.
    #[test]
    fn activity_reflects_damage() {
        assert_eq!(activity_from_damage(0), Activity::Idle);
        assert_eq!(activity_from_damage(1), Activity::Busy);
        assert_eq!(activity_from_damage(24), Activity::Busy);
    }

    /// `status_label`: conhecidos → PT-BR; desconhecido passa cru (resiliente à evolução do core).
    #[test]
    fn status_label_maps_known_and_passes_unknown() {
        assert_eq!(status_label("Running"), "Ativo");
        assert_eq!(status_label("Starting"), "Iniciando");
        assert_eq!(status_label("Dead"), "Encerrado");
        assert_eq!(status_label("Quarantined"), "Quarantined");
    }
}
