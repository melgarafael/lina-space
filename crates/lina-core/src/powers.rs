//! F2-4 — **Área de Poderes**: scanner determinístico de disco (skills/plugins/agents/commands/
//! hooks/MCPs instalados na máquina do leigo). Projeção de disco EFÊMERA, re-derivada a cada
//! abertura do painel (~0ms) — NÃO event-sourced como fonte primária (ADR 0052 §1). O log registra
//! só a OBSERVAÇÃO (`DomainEvent::PowerScanned`, contadores), nunca a verdade dos poderes — exatamente
//! como `cli_discovery` é projeção do `$PATH`, não do log.
//!
//! ## Doutrina (ADR 0052 · transplante do ADR 0008)
//! - **manifest-first é LEI:** ler o manifesto pequeno SEMPRE (`installed_plugins.json` 13KB), varrer
//!   a árvore pesada NUNCA (`~/.claude/plugins` 1,9GB → freeze/ENOSPC). Skills via frontmatter raso
//!   (`crate::skill_index::parse_frontmatter`). Molde de varredura: `cli_discovery::discover_clis_in`.
//! - **mostrar ≠ autorizar (§4):** nenhum campo de `Power` lido do disco (`name`/`description`/`origin`)
//!   entra em caminho de identidade/ordem/autorização. O produto é DADO de exibição; os gates de
//!   execução continuam em custódia (ADR 0004) e WorkspaceTrust (ADR 0006/0010).
//! - **heurística nunca decide estado:** o estado vem do disco (frontmatter válido? arquivo presente?
//!   na pasta do CLI focado?), determinístico — adivinhar pelo output do terminal é banido.
//!
//! ## Contrato (ADR 0052 §3) — tipos + scanner
//! Os TIPOS são o contrato selado do ADR 0052 §3, consumidos direto pela UI (`powers_panel.rs`) e
//! pelo QA. `scan_powers`/`watch_targets` implementam a varredura real (manifest-first, 5 estados);
//! os caminhos por CLI vêm do `CliProfile` (`skills_dir`/`mcp_config_path`), sem hardcode (inv#3).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::skill_index::parse_frontmatter;
use crate::CliProfile;

/// Id do perfil Claude — origem dos poderes que são **convenção do `~/.claude/`** (plugins/agents/
/// commands/hooks). O ADR 0052 §5 mantém esses 4 como convenção (não campo TOML) porque não há 2º
/// CLI que os tenha; vira campo aditivo quando um 2º CLI os introduzir (porta aberta, não pré-aberta).
const CLAUDE_CLI_ID: &str = "claude-code";

/// Família de um Poder. `String` minúsculo ("skill","plugin",…) é a chave no evento `PowerScanned`
/// (replay-safe); aqui o enum é o tipo forte do view-model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PowerKind {
    Skill,
    Plugin,
    Agent,
    Command,
    Hook,
    Mcp,
}

impl PowerKind {
    /// Rótulo minúsculo estável para a chave de contadores do evento de auditoria (ADR 0052 §2).
    pub fn key(&self) -> &'static str {
        match self {
            PowerKind::Skill => "skill",
            PowerKind::Plugin => "plugin",
            PowerKind::Agent => "agent",
            PowerKind::Command => "command",
            PowerKind::Hook => "hook",
            PowerKind::Mcp => "mcp",
        }
    }
}

/// Eixo "onde vale": em tudo (home) ou só no Espaço aberto. Ortogonal ao CLI dono (ADR 0052 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerScope {
    Global,
    Project,
}

/// Origem ROTULADA de um Poder — dois eixos ortogonais (ADR 0052 §3: uma skill é "global" E "do
/// Gemini" ao mesmo tempo; colapsar num enum perderia um eixo). É o que o leigo vê rotulado
/// ("global / deste projeto / do Gemini") e o que decide `InertHere`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerOrigin {
    pub scope: PowerScope,
    /// pasta de QUAL CLI ("claude-code","gemini","codex",…); `None` = não atrelado a um CLI.
    pub cli: Option<String>,
}

/// Os 5 estados leigos (ADR 0052 §3 · entrega-d4 §III.b). Cada um (exceto `Ready`) tem ação de
/// 1 clique obrigatória na UI; `Disabled` só se materializa se o app puder religar (a tela nunca mente).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerState {
    Ready,
    UpdateAvailable,
    NeedsRepair,
    InertHere,
    Disabled,
}

/// Um Poder visto no disco — DADO de exibição (nunca autoridade). `name`/`description` são CRUS do
/// manifesto; a tradução leiga + âncora "(skill)" é da copy do painel, não do scanner (ADR 0052 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    pub kind: PowerKind,
    pub name: String,
    pub description: String,
    pub origin: PowerOrigin,
    pub state: PowerState,
}

/// O inventário que o painel consome. `counts` alimenta o resumo do nível 1 ("75 Poderes · 33 Plugins").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerInventory {
    pub powers: Vec<Power>,
    pub counts: BTreeMap<PowerKind, u32>,
}

impl PowerInventory {
    /// Deriva os contadores de AUDITORIA (`total`, mapa `kind-minúsculo → n`) que o Maestro emite em
    /// `DomainEvent::PowerScanned` — só metadados, ZERO conteúdo de poder (ADR 0052 §2). Puro.
    pub fn audit_counts(&self) -> (u32, BTreeMap<String, u32>) {
        let mut by_kind: BTreeMap<String, u32> = BTreeMap::new();
        for p in &self.powers {
            *by_kind.entry(p.kind.key().to_string()).or_insert(0) += 1;
        }
        let total = self.powers.len() as u32;
        (total, by_kind)
    }
}

/// Raízes de scan — derivadas dos `CliProfile` (campos `skills_dir`/`mcp_config_path`, inv#3) + a
/// convenção do perfil Claude para plugins/agents/commands/hooks (ADR 0052 §5). Sem caminho hardcoded
/// por CLI. **F24CORE pode estender** (ex.: cache de mtime) — é a fronteira do scanner.
#[derive(Debug, Clone)]
pub struct PowerRoots {
    /// diretório home do usuário (base para expandir o `~` dos globs do perfil).
    pub home: PathBuf,
    /// diretório do projeto/Espaço aberto (escopo `Project` + `.mcp.json` local). `None` = sem projeto.
    pub project_dir: Option<PathBuf>,
    /// perfis de CLI carregados — fonte de `skills_dir`/`mcp_config_path` (sem hardcode, inv#3).
    pub profiles: Vec<CliProfile>,
}

/// Debounce mínimo do watcher raso (ADR 0052 §6 · achado A7): atomic saves geram rajadas; ≥750ms coalesce.
pub const POWERS_DEBOUNCE_MS: u64 = 750;

/// Varre o disco (manifest-first) e devolve o inventário de Poderes com os 5 estados computados
/// contra `focused_cli` (skill na pasta do CLI X com terminal CLI Y → `InertHere`). Re-derivado a
/// cada abertura (~0ms, efêmero). Caminhos por CLI vêm dos `CliProfile` (inv#3); plugins/agents/
/// commands/hooks são convenção do `~/.claude/` (ADR 0052 §5). Nunca varre a árvore pesada de plugins.
pub fn scan_powers(roots: &PowerRoots, focused_cli: Option<&str>) -> PowerInventory {
    let mut powers = Vec::new();

    // Por CLI (sem caminho hardcoded, inv#3): skills do `skills_dir` + MCPs do `mcp_config_path`.
    for profile in &roots.profiles {
        if let Some(glob) = profile.skills_dir.as_deref() {
            scan_skills(&expand_root(glob, &roots.home), &profile.id, &mut powers);
        }
        if let Some(glob) = profile.mcp_config_path.as_deref() {
            let path = expand_root(glob, &roots.home);
            scan_mcps(
                &path,
                &profile.id,
                roots.project_dir.as_deref(),
                &mut powers,
            );
        }
    }

    // Convenção do perfil Claude (ADR 0052 §5): plugins/agents/commands/hooks derivados de `~/.claude/`.
    let claude_home = roots.home.join(".claude");
    scan_plugins(
        &claude_home.join("plugins").join("installed_plugins.json"),
        &mut powers,
    );
    scan_md_dir(&claude_home.join("agents"), PowerKind::Agent, &mut powers);
    scan_md_dir(
        &claude_home.join("commands"),
        PowerKind::Command,
        &mut powers,
    );
    scan_hooks(&claude_home.join("settings.json"), &mut powers);

    apply_focus(&mut powers, focused_cli);
    let counts = derive_counts(&powers);
    PowerInventory { powers, counts }
}

/// Expande o prefixo `~` de um glob do perfil contra o `home` INJETADO (não o do ambiente) — é o que
/// torna o scan testável headless. Caminho absoluto/relativo sem `~` passa intacto.
fn expand_root(glob: &str, home: &Path) -> PathBuf {
    match glob.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if glob == "~" => home.to_path_buf(),
        None => PathBuf::from(glob),
    }
}

/// Skills em `<dir>/<nome>/SKILL.md` (scan raso, ~0ms). Frontmatter via `parse_frontmatter` (REUSE).
/// SKILL.md ausente OU sem `name` ⇒ `NeedsRepair` (frontmatter inválido/incompleto). `origin.cli` = o
/// CLI dono da pasta — é o que decide `InertHere` no `apply_focus`.
fn scan_skills(dir: &Path, cli: &str, out: &mut Vec<Power>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // Pasta ausente = degradação intencional (CLI sem skills instaladas), não erro engolido.
        return;
    };
    for entry in entries.filter_map(Result::ok).filter(|e| e.path().is_dir()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let (description, state) = match std::fs::read_to_string(entry.path().join("SKILL.md")) {
            Ok(md) => {
                let fm = parse_frontmatter(&md);
                if fm.name.is_empty() {
                    (String::new(), PowerState::NeedsRepair)
                } else {
                    (fm.description, PowerState::Ready)
                }
            }
            Err(_) => (String::new(), PowerState::NeedsRepair),
        };
        out.push(Power {
            kind: PowerKind::Skill,
            name,
            description,
            origin: PowerOrigin {
                scope: PowerScope::Global,
                cli: Some(cli.to_string()),
            },
            state,
        });
    }
}

/// Plugins do Claude: lê SÓ o manifesto `installed_plugins.json` (13KB). Recebe o ARQUIVO, nunca o
/// diretório — manifest-first é GARANTIDO por construção (impossível varrer a árvore de 1,9GB daqui).
/// Forma: `{"plugins": {"nome@marketplace": [{scope,...}]}}`. Chave = id cru exibido pelo painel.
fn scan_plugins(manifest: &Path, out: &mut Vec<Power>) {
    let Ok(raw) = std::fs::read_to_string(manifest) else {
        return;
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return; // manifesto corrompido: skip silencioso (molde channel.rs::from_records).
    };
    let Some(plugins) = doc.get("plugins").and_then(serde_json::Value::as_object) else {
        return;
    };
    for key in plugins.keys() {
        out.push(Power {
            kind: PowerKind::Plugin,
            name: key.clone(),
            description: String::new(), // o manifesto não traz descrição; o rótulo leigo é da copy do painel.
            origin: PowerOrigin {
                scope: PowerScope::Global,
                cli: Some(CLAUDE_CLI_ID.to_string()),
            },
            state: PowerState::Ready,
        });
    }
}

/// MCPs do `mcp_config_path` do CLI. O FORMATO é decidido pela extensão (determinístico): `.toml` =
/// Codex (`[mcp_servers.*]`, todos `Global`); qualquer outra = JSON do Claude (`mcpServers` global +
/// `projects[<project_dir>].mcpServers` — SÓ o projeto aberto, nunca os outros 162 do arquivo).
fn scan_mcps(config: &Path, cli: &str, project_dir: Option<&Path>, out: &mut Vec<Power>) {
    let Ok(raw) = std::fs::read_to_string(config) else {
        return;
    };
    if config.extension().and_then(|e| e.to_str()) == Some("toml") {
        if let Ok(doc) = toml::from_str::<toml::Value>(&raw) {
            if let Some(servers) = doc.get("mcp_servers").and_then(toml::Value::as_table) {
                for name in servers.keys() {
                    out.push(mcp_power(name, PowerScope::Global, cli));
                }
            }
        }
        return;
    }
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    if let Some(global) = doc.get("mcpServers").and_then(serde_json::Value::as_object) {
        for name in global.keys() {
            out.push(mcp_power(name, PowerScope::Global, cli));
        }
    }
    if let Some(dir) = project_dir {
        // Chave literal = path absoluto do projeto; navega direto (sem iterar os demais projetos).
        let by_project = doc
            .get("projects")
            .and_then(|p| p.get(dir.to_string_lossy().as_ref()))
            .and_then(|p| p.get("mcpServers"))
            .and_then(serde_json::Value::as_object);
        if let Some(servers) = by_project {
            for name in servers.keys() {
                out.push(mcp_power(name, PowerScope::Project, cli));
            }
        }
    }
}

fn mcp_power(name: &str, scope: PowerScope, cli: &str) -> Power {
    Power {
        kind: PowerKind::Mcp,
        name: name.to_string(),
        description: String::new(),
        origin: PowerOrigin {
            scope,
            cli: Some(cli.to_string()),
        },
        state: PowerState::Ready,
    }
}

/// Agents/commands: scan raso de `<dir>/*.md` (nome = stem do arquivo; descrição = frontmatter, REUSE).
fn scan_md_dir(dir: &Path, kind: PowerKind, out: &mut Vec<Power>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let description = std::fs::read_to_string(&path)
            .map(|md| parse_frontmatter(&md).description)
            .unwrap_or_default();
        out.push(Power {
            kind,
            name,
            description,
            origin: PowerOrigin {
                scope: PowerScope::Global,
                cli: Some(CLAUDE_CLI_ID.to_string()),
            },
            state: PowerState::Ready,
        });
    }
}

/// Hooks: `~/.claude/settings.json → hooks` é config JSON (não pasta). Um Poder por TIPO de hook
/// configurado (`PreToolUse`, `UserPromptSubmit`, …) — só os metadados, nunca o comando do hook.
fn scan_hooks(settings: &Path, out: &mut Vec<Power>) {
    let Ok(raw) = std::fs::read_to_string(settings) else {
        return;
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(hooks) = doc.get("hooks").and_then(serde_json::Value::as_object) else {
        return;
    };
    for hook_type in hooks.keys() {
        out.push(Power {
            kind: PowerKind::Hook,
            name: hook_type.clone(),
            description: String::new(),
            origin: PowerOrigin {
                scope: PowerScope::Global,
                cli: Some(CLAUDE_CLI_ID.to_string()),
            },
            state: PowerState::Ready,
        });
    }
}

/// Aplica o foco do terminal (ADR 0052 §3): um poder `Ready` cujo `origin.cli` ≠ `focused_cli` é
/// `InertHere` (skill na pasta do CLI X, terminal roda CLI Y — o caso NORMAL do multi-CLI). Sem foco
/// (`None`) nada é inerte. Só sobrescreve `Ready`: um `NeedsRepair` continua pedindo conserto (não
/// se mascara um problema real atrás de "não funciona neste motor").
fn apply_focus(powers: &mut [Power], focused_cli: Option<&str>) {
    let Some(focus) = focused_cli else {
        return;
    };
    for power in powers.iter_mut() {
        let foreign = power.origin.cli.as_deref().is_some_and(|cli| cli != focus);
        if foreign && power.state == PowerState::Ready {
            power.state = PowerState::InertHere;
        }
    }
}

fn derive_counts(powers: &[Power]) -> BTreeMap<PowerKind, u32> {
    let mut counts = BTreeMap::new();
    for power in powers {
        *counts.entry(power.kind).or_insert(0) += 1;
    }
    counts
}

/// Os ~5 diretórios-PAI dos manifestos que o watcher raso observa (atomic saves trocam o inode →
/// observar o pai, NUNCA o arquivo; recursivo na árvore pesada é PROIBIDO). PURO, testável headless.
/// A fiação do `notify` real (NÃO-recursivo, debounce [`POWERS_DEBOUNCE_MS`]) é do app (Maestro).
pub fn watch_targets(roots: &PowerRoots) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    let claude_home = roots.home.join(".claude");

    // Pai de `installed_plugins.json` = `~/.claude/plugins`. A fiação observa este dir de forma
    // NÃO-recursiva: pega a troca do manifesto SEM descer na árvore de 1,9GB (recursivo é PROIBIDO).
    targets.push(claude_home.join("plugins"));
    // `~/.claude/settings.json` (hooks), `~/.claude/agents`, `~/.claude/commands` → pai comum.
    targets.push(claude_home);

    for profile in &roots.profiles {
        // A pasta de skills é ela mesma o pai dos `<skill>/SKILL.md` (raso; subpastas não entram).
        if let Some(glob) = profile.skills_dir.as_deref() {
            targets.push(expand_root(glob, &roots.home));
        }
        // Pai do arquivo de config de MCP (atomic save troca o inode → observar o pai, não o arquivo).
        if let Some(glob) = profile.mcp_config_path.as_deref() {
            if let Some(parent) = expand_root(glob, &roots.home).parent() {
                targets.push(parent.to_path_buf());
            }
        }
    }
    // `.mcp.json` do projeto aberto → pai = o próprio diretório do projeto.
    if let Some(dir) = &roots.project_dir {
        targets.push(dir.clone());
    }

    targets.sort();
    targets.dedup();
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tempdir removido no `Drop` (mesmo padrão de `cli_discovery::tests`).
    struct TempHome(PathBuf);
    impl TempHome {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("lina-powers-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn write(&self, rel: &str, body: &str) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().expect("tem pai")).expect("mkdir");
            std::fs::write(path, body).expect("escrever fixture");
        }
        fn mkdir(&self, rel: &str) {
            std::fs::create_dir_all(self.0.join(rel)).expect("mkdir");
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `CliProfile` mínimo via TOML (o struct só deriva `Deserialize`; construir à mão exigiria todos
    /// os campos). Só `id`/`skills_dir`/`mcp_config_path` importam ao scanner.
    fn profile(id: &str, skills_dir: Option<&str>, mcp: Option<&str>) -> CliProfile {
        let mut toml = format!(
            "id = \"{id}\"\nprogram = \"x\"\ndelivery = \"pty_inject\"\nprompt_ready_regex = \">\"\n"
        );
        if let Some(s) = skills_dir {
            toml.push_str(&format!("skills_dir = \"{s}\"\n"));
        }
        if let Some(m) = mcp {
            toml.push_str(&format!("mcp_config_path = \"{m}\"\n"));
        }
        toml.push_str("[end_signal]\nkind = \"idle\"\n");
        toml::from_str(&toml).expect("perfil de teste válido")
    }

    const VALID_SKILL: &str = "---\nname: minha-skill\ndescription: faz coisas\n---\n# corpo\n";

    fn roots(
        home: &TempHome,
        project_dir: Option<PathBuf>,
        profiles: Vec<CliProfile>,
    ) -> PowerRoots {
        PowerRoots {
            home: home.0.clone(),
            project_dir,
            profiles,
        }
    }

    fn find<'a>(inv: &'a PowerInventory, kind: PowerKind, name: &str) -> Option<&'a Power> {
        inv.powers.iter().find(|p| p.kind == kind && p.name == name)
    }

    #[test]
    fn skill_valida_e_ready_e_quebrada_e_needs_repair() {
        let home = TempHome::new();
        home.write(".claude/skills/boa/SKILL.md", VALID_SKILL);
        home.write(
            ".claude/skills/sem-frontmatter/SKILL.md",
            "# só corpo, zero frontmatter",
        );
        home.mkdir(".claude/skills/sem-md"); // pasta sem SKILL.md

        let inv = scan_powers(
            &roots(
                &home,
                None,
                vec![profile("claude-code", Some("~/.claude/skills"), None)],
            ),
            Some("claude-code"),
        );

        assert_eq!(
            find(&inv, PowerKind::Skill, "boa").unwrap().state,
            PowerState::Ready
        );
        assert_eq!(
            find(&inv, PowerKind::Skill, "boa").unwrap().description,
            "faz coisas"
        );
        assert_eq!(
            find(&inv, PowerKind::Skill, "sem-frontmatter")
                .unwrap()
                .state,
            PowerState::NeedsRepair
        );
        assert_eq!(
            find(&inv, PowerKind::Skill, "sem-md").unwrap().state,
            PowerState::NeedsRepair
        );
    }

    #[test]
    fn skill_de_outro_cli_e_inert_here_com_a_origem_do_dono() {
        let home = TempHome::new();
        home.write(".claude/skills/local/SKILL.md", VALID_SKILL);
        home.write(".codex/skills/forasteira/SKILL.md", VALID_SKILL);

        let profiles = vec![
            profile("claude-code", Some("~/.claude/skills"), None),
            profile("codex", Some("~/.codex/skills"), None),
        ];
        let inv = scan_powers(&roots(&home, None, profiles), Some("claude-code"));

        // A skill do terminal focado roda; a do outro motor é inerte AQUI, rotulada com o CLI dono.
        assert_eq!(
            find(&inv, PowerKind::Skill, "local").unwrap().state,
            PowerState::Ready
        );
        let forasteira = find(&inv, PowerKind::Skill, "forasteira").unwrap();
        assert_eq!(forasteira.state, PowerState::InertHere);
        assert_eq!(forasteira.origin.cli.as_deref(), Some("codex"));
    }

    #[test]
    fn sem_foco_nada_e_inerte() {
        let home = TempHome::new();
        home.write(".codex/skills/qualquer/SKILL.md", VALID_SKILL);
        let inv = scan_powers(
            &roots(
                &home,
                None,
                vec![profile("codex", Some("~/.codex/skills"), None)],
            ),
            None, // sem terminal focado
        );
        assert_eq!(
            find(&inv, PowerKind::Skill, "qualquer").unwrap().state,
            PowerState::Ready
        );
    }

    #[test]
    fn plugins_so_do_manifesto_a_arvore_pesada_nunca_e_varrida() {
        let home = TempHome::new();
        home.write(
            ".claude/plugins/installed_plugins.json",
            r#"{"version":1,"plugins":{"a@mkt":[{"scope":"user"}],"b@mkt":[{"scope":"user"}]}}"#,
        );
        // Isca: um repo dentro da árvore de plugins com algo que PARECE skill. Se o scanner descesse
        // na árvore (proibido), este nome vazaria no inventário. `scan_plugins` recebe o ARQUIVO, não
        // o diretório → varrer a árvore é impossível por construção; este teste morde se isso mudar.
        home.write(".claude/plugins/repo-gigante/SKILL.md", VALID_SKILL);

        let inv = scan_powers(&roots(&home, None, vec![]), Some("claude-code"));

        let plugins: Vec<&str> = inv
            .powers
            .iter()
            .filter(|p| p.kind == PowerKind::Plugin)
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(plugins, vec!["a@mkt", "b@mkt"]);
        // Nada da árvore de plugins vira poder (nem como skill, nem por nome do repo).
        assert!(inv.powers.iter().all(|p| p.name != "repo-gigante"));
    }

    #[test]
    fn mcp_json_le_global_e_so_o_projeto_aberto() {
        let home = TempHome::new();
        home.write(
            ".claude.json",
            r#"{"mcpServers":{"global-mcp":{}},
                "projects":{
                  "/proj/aberto":{"mcpServers":{"mcp-aberto":{}}},
                  "/proj/outro":{"mcpServers":{"mcp-outro":{}}}
                }}"#,
        );
        let inv = scan_powers(
            &roots(
                &home,
                Some(PathBuf::from("/proj/aberto")),
                vec![profile("claude-code", None, Some("~/.claude.json"))],
            ),
            Some("claude-code"),
        );

        assert!(find(&inv, PowerKind::Mcp, "global-mcp").is_some());
        assert_eq!(
            find(&inv, PowerKind::Mcp, "mcp-aberto")
                .unwrap()
                .origin
                .scope,
            PowerScope::Project
        );
        // Os MCPs dos OUTROS 162 projetos do arquivo NUNCA aparecem (escopo = projeto aberto).
        assert!(find(&inv, PowerKind::Mcp, "mcp-outro").is_none());
    }

    #[test]
    fn mcp_toml_do_codex_le_mcp_servers() {
        let home = TempHome::new();
        home.write(
            ".codex/config.toml",
            "[mcp_servers.playwright]\ncommand = \"x\"\n[mcp_servers.outro]\ncommand = \"y\"\n",
        );
        let inv = scan_powers(
            &roots(
                &home,
                None,
                vec![profile("codex", None, Some("~/.codex/config.toml"))],
            ),
            Some("codex"),
        );
        let mcp = find(&inv, PowerKind::Mcp, "playwright").unwrap();
        assert_eq!(mcp.origin.cli.as_deref(), Some("codex"));
        assert_eq!(mcp.origin.scope, PowerScope::Global);
        assert!(find(&inv, PowerKind::Mcp, "outro").is_some());
    }

    #[test]
    fn agents_commands_hooks_da_convencao_claude() {
        let home = TempHome::new();
        home.write(
            ".claude/agents/deep-diver.md",
            "---\nname: deep-diver\ndescription: cava fundo\n---\n",
        );
        home.write(
            ".claude/commands/deploy.md",
            "# sem frontmatter, ainda conta",
        );
        home.write(
            ".claude/settings.json",
            r#"{"hooks":{"PreToolUse":[],"UserPromptSubmit":[]}}"#,
        );

        let inv = scan_powers(&roots(&home, None, vec![]), Some("claude-code"));

        assert_eq!(
            find(&inv, PowerKind::Agent, "deep-diver")
                .unwrap()
                .description,
            "cava fundo"
        );
        assert!(find(&inv, PowerKind::Command, "deploy").is_some());
        // Um Poder por TIPO de hook configurado.
        assert!(find(&inv, PowerKind::Hook, "PreToolUse").is_some());
        assert!(find(&inv, PowerKind::Hook, "UserPromptSubmit").is_some());
    }

    #[test]
    fn audit_counts_so_metadados_batem_com_o_inventario() {
        let home = TempHome::new();
        home.write(".claude/skills/s1/SKILL.md", VALID_SKILL);
        home.write(".claude/skills/s2/SKILL.md", VALID_SKILL);
        let inv = scan_powers(
            &roots(
                &home,
                None,
                vec![profile("claude-code", Some("~/.claude/skills"), None)],
            ),
            Some("claude-code"),
        );
        let (total, by_kind) = inv.audit_counts();
        assert_eq!(total, inv.powers.len() as u32);
        assert_eq!(by_kind.get("skill"), Some(&2));
    }

    #[test]
    fn watch_targets_sao_pais_rasos_nunca_a_arvore_de_plugins() {
        let home = TempHome::new();
        let profiles = vec![
            profile(
                "claude-code",
                Some("~/.claude/skills"),
                Some("~/.claude.json"),
            ),
            profile(
                "codex",
                Some("~/.codex/skills"),
                Some("~/.codex/config.toml"),
            ),
        ];
        let targets = watch_targets(&roots(&home, Some(PathBuf::from("/proj/x")), profiles));

        let plugins_dir = home.0.join(".claude/plugins");
        // O PAI raso entra (pega a troca do installed_plugins.json)...
        assert!(targets.contains(&plugins_dir));
        // ...mas NENHUM subdiretório-repo da árvore pesada entra (recursivo é proibido).
        assert!(
            targets
                .iter()
                .all(|t| t.parent() != Some(plugins_dir.as_path())),
            "nenhum alvo pode ser filho de ~/.claude/plugins"
        );
        // A pasta de skills (rasa) e o pai do config de MCP são observados.
        assert!(targets.contains(&home.0.join(".claude/skills")));
        assert!(targets.contains(&home.0.join(".codex")));
        assert!(targets.contains(&PathBuf::from("/proj/x")));
    }

    #[test]
    fn expand_root_troca_til_pelo_home_injetado() {
        let home = PathBuf::from("/home/leigo");
        assert_eq!(
            expand_root("~/.claude/skills", &home),
            home.join(".claude/skills")
        );
        assert_eq!(expand_root("~", &home), home);
        assert_eq!(
            expand_root("/abs/oluto", &home),
            PathBuf::from("/abs/oluto")
        );
    }
}
