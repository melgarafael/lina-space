//! `obsidian` — **tela "Seu segundo cérebro" do onboarding** (passo após "Ferramentas de
//! desenvolvimento"). Detecta o Obsidian + as pastas de anotações ("vaults"), instala se faltar,
//! deixa o usuário escolher quais a Lina pode ler, persiste a escolha em `.lina/vault.json` e gera um
//! **mapa estrutural determinístico (PageIndex, SEM IA — inv#1)** de cada vault em
//! `.lina/vault-index/<slug>.md` — que a doutrina dos assistentes manda consultar (BLOCO 3).
//!
//! ## Split (igual ao resto do shell, espelha `dev_tools.rs`)
//! - [`SecondBrainModel`] + helpers (detecção, PageIndex, persistência, [`footer_label`]) são
//!   **gpui-free e testáveis** — toda a lógica vive aqui.
//! - [`SecondBrainModel::render`] só desenha o modelo e roteia cliques pela view-pai
//!   ([`OnboardingView`]).
//!
//! ## Por que NÃO toca `lina-core`/`lina-cli-profiles`/`lina-bootstrap` (anti-colisão multi-terminal)
//! - O vault SEMPRE foi **config** (não evento): `.lina/vault.json` já é contrato documentado na
//!   doutrina (BLOCO 3) — esta feature o PREENCHE; nenhum `DomainEvent` novo.
//! - A instalação REUSA o pipeline dos assistentes ([`run_install`] PTY oculto + [`decide_plan`]); a
//!   receita cask cabe no schema atual de [`InstallRecipe`] (`program/args/env/verify_paths`). A única
//!   diferença: o Obsidian é um **app bundle**, não um binário no PATH → a verificação pós-instalação
//!   usa [`find_app_bundle`] (não `find_in_path` — armadilha verificada).
//!
//! **Invariantes (CLAUDE.md):** #1 (zero LLM — o PageIndex é pattern-match puro no markdown), #2
//! (local-first — só lê o vault na máquina, nada sai), #6 (não-técnico-first: zero jargão na UI,
//! nunca beco sem saída — "Pular esta etapa" sempre disponível; estado salvo e visível).

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use gpui::{
    div, prelude::*, px, rgb, text, AnyElement, ClickEvent, Context, FontWeight, PathPromptOptions,
    Window,
};

use lina_cli_profiles::{InstallRecipe, Installers, CURRENT_OS};
use lina_core::{find_in_path, DiscoveredCli};
use serde::{Deserialize, Serialize};

use crate::dev_tools::{decide_plan, open_in_terminal, InstallPlan};
use crate::onboarding::{install_recipe_with, run_install, InstallState, OnboardingView};

// ───────────────────────────── paleta (espelha o onboarding/canvas) ─────────────────────────────
const PANEL: u32 = 0x141a36;
const PANEL_SEL: u32 = 0x1b2347; // linha selecionada (UX §6)
const ACCENT: u32 = 0x7aa2f7;
const TEXT: u32 = 0xc8d3f5;
const MUTED: u32 = 0x5b658f;
const GREEN: u32 = 0x9ece6a;
const AMBER: u32 = 0xe0af68;

/// Nome do app bundle do Obsidian (macOS `Obsidian.app`).
const OBSIDIAN_APP: &str = "Obsidian";
/// id lógico da receita no `second-brain.toml`.
const OBSIDIAN_ID: &str = "obsidian";

// ═══════════════════════════ detecção do app + vaults (gpui-free, injetável) ═══════════════════════════

/// Diretório HOME do usuário (`HOME` no Unix, `USERPROFILE` no Windows).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// Acha o **bundle do app** `name` no SO (substitui `find_in_path` — um `.app`/`.exe` não está no
/// PATH; a armadilha do blueprint §6). macOS: `/Applications/<name>.app` + `~/Applications/…`.
#[cfg(target_os = "macos")]
#[must_use]
pub fn find_app_bundle(name: &str) -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(format!("/Applications/{name}.app"))];
    if let Some(h) = home_dir() {
        candidates.push(h.join("Applications").join(format!("{name}.app")));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// Fora do macOS (Windows/Linux das docs + fallback): checa os caminhos de instalação conhecidos.
/// Windows: `%LOCALAPPDATA%\<name>\<name>.exe` / `%ProgramFiles%`. Linux: flatpak/usr/local.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn find_app_bundle(name: &str) -> Option<PathBuf> {
    let lname = name.to_ascii_lowercase();
    let mut candidates: Vec<PathBuf> = Vec::new();
    // Windows.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join(name).join(format!("{name}.exe")));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join(name).join(format!("{name}.exe")));
    }
    // Linux (flatpak oficial + pacotes).
    candidates.push(PathBuf::from("/var/lib/flatpak/exports/bin/md.obsidian.Obsidian"));
    candidates.push(PathBuf::from(format!("/usr/bin/{lname}")));
    candidates.push(PathBuf::from(format!("/usr/local/bin/{lname}")));
    if let Some(h) = home_dir() {
        candidates.push(h.join(".local/share/flatpak/exports/bin/md.obsidian.Obsidian"));
        candidates.push(h.join(".local/bin").join(&lname));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Caminho do registro de vaults do Obsidian (`obsidian.json`). macOS: Application Support.
#[cfg(target_os = "macos")]
fn obsidian_config_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Library/Application Support/obsidian/obsidian.json"))
}

/// Fora do macOS: `%APPDATA%\obsidian\` (Windows) ou `$XDG_CONFIG_HOME`/`~/.config/obsidian/` (Linux).
#[cfg(not(target_os = "macos"))]
fn obsidian_config_path() -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata).join("obsidian").join("obsidian.json"));
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".config")))?;
    Some(config.join("obsidian").join("obsidian.json"))
}

/// Uma pasta de anotações ("vault") detectada ou adicionada manualmente. `open` = "aberta agora" no
/// Obsidian (vem pré-marcada); `added_manually` distingue as escolhidas pelo usuário no seletor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultLink {
    pub name: String,
    pub path: PathBuf,
    pub open: bool,
    pub added_manually: bool,
}

/// PURO: extrai os vaults do `obsidian.json`. O mapa é `"vaults": { id: { path, open, ts } }`.
/// Ordena por caminho (determinístico). Ignora entradas sem `path`.
#[must_use]
pub fn parse_vaults_from_json(json: &str) -> Vec<VaultLink> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(map) = value.get("vaults").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<VaultLink> = map
        .values()
        .filter_map(|entry| {
            let path = entry.get("path").and_then(|p| p.as_str())?;
            let open = entry.get("open").and_then(serde_json::Value::as_bool).unwrap_or(false);
            let path = PathBuf::from(path);
            let name = vault_name(&path);
            Some(VaultLink { name, path, open, added_manually: false })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Nome amigável do vault = último componente do caminho (fallback: o caminho inteiro).
fn vault_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// `true` se `<path>/.obsidian/` existe — o marcador de um vault de verdade.
#[must_use]
pub fn is_vault_dir(path: &Path) -> bool {
    path.join(".obsidian").is_dir()
}

/// Resultado de uma varredura: o app está presente? quais vaults existem no disco?
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObsidianScan {
    pub app_present: bool,
    pub vaults: Vec<VaultLink>,
}

/// Varredura REAL (produção): acha o app por bundle e lê o registro de vaults, filtrando os que
/// ainda existem no disco. **Roda fora da thread de UI** (ver [`SecondBrainModel::redetect`]).
#[must_use]
pub fn discover_obsidian() -> ObsidianScan {
    let app_present = find_app_bundle(OBSIDIAN_APP).is_some();
    // Valida pelo marcador `<path>/.obsidian/` (blueprint §1): vault real e ainda no disco. Descarta
    // entradas obsoletas do registro (pasta movida/apagada) sem all-or-nothing — as demais seguem.
    let vaults = obsidian_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| parse_vaults_from_json(&s))
        .unwrap_or_default()
        .into_iter()
        .filter(|v| is_vault_dir(&v.path))
        .collect();
    ObsidianScan { app_present, vaults }
}

// ═══════════════════════════ PageIndex — mapa estrutural determinístico (inv#1) ═══════════════════════════

/// Uma nota do índice: caminho relativo + headings + alvos de wikilink (saída do grafo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub rel_path: String,
    pub headings: Vec<String>,
    pub links: Vec<String>,
}

/// O índice de um vault: pastas + notas (ambos ordenados — determinístico).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultIndexData {
    pub folders: Vec<String>,
    pub notes: Vec<NoteEntry>,
}

/// PURO: extrai os headings (`^#{1,6} `) preservando o nível (`# Título`, `## Seção`). Só conta `#`
/// no início ABSOLUTO da linha (headings de markdown), não `#` indentado de código.
#[must_use]
pub fn extract_headings(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
                Some(line.trim_end().to_string())
            } else {
                None
            }
        })
        .collect()
}

/// PURO: extrai os alvos de `[[wikilink]]` (sem alias `|` nem âncora `#`), na ordem, sem repetir.
#[must_use]
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let inner = &after[..end];
        let target = inner
            .split('|')
            .next()
            .unwrap_or(inner)
            .split('#')
            .next()
            .unwrap_or(inner)
            .trim();
        if !target.is_empty() && !out.iter().any(|t| t == target) {
            out.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

/// Varre o vault (recursivo, READ-ONLY, sem rede, sem LLM): coleta pastas e notas `*.md`, ignorando
/// `.obsidian/`, `.trash/` e qualquer dir oculto. Ordena tudo (determinístico).
#[must_use]
pub fn scan_vault(root: &Path) -> VaultIndexData {
    let mut folders = Vec::new();
    let mut notes = Vec::new();
    walk_dir(root, root, &mut folders, &mut notes);
    folders.sort();
    folders.dedup();
    notes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    VaultIndexData { folders, notes }
}

fn walk_dir(root: &Path, dir: &Path, folders: &mut Vec<String>, notes: &mut Vec<NoteEntry>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Ignora o sistema do Obsidian, a lixeira e qualquer dir oculto (inclui o `.lina` se o
            // index for escrito por engano dentro — defesa extra; o index mora FORA do vault).
            if name.starts_with('.') || name == ".trash" {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                folders.push(format!("{}/", rel.to_string_lossy().replace('\\', "/")));
            }
            walk_dir(root, &path, folders, notes);
        } else if path.extension().and_then(|x| x.to_str()) == Some("md") {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            notes.push(NoteEntry {
                rel_path: rel,
                headings: extract_headings(&content),
                links: extract_wikilinks(&content),
            });
        }
    }
}

/// PURO: renderiza o índice como markdown determinístico (regenerado a cada link). Formato do
/// blueprint §3: cabeçalho "NÃO editar", pastas, notas+headings, grafo de [[wikilinks]].
#[must_use]
pub fn render_vault_index(name: &str, root: &Path, data: &VaultIndexData) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Vault Index — {name}  (gerado pelo Lina · determinístico · sem IA · NÃO editar)\n"
    ));
    out.push_str(&format!("> Origem: {}\n\n", root.display()));

    out.push_str("## Pastas\n");
    if data.folders.is_empty() {
        out.push_str("- (nenhuma subpasta)\n");
    } else {
        for f in &data.folders {
            out.push_str(&format!("- {f}\n"));
        }
    }

    out.push_str("\n## Notas (headings)\n");
    if data.notes.is_empty() {
        out.push_str("- (nenhuma nota)\n");
    } else {
        for note in &data.notes {
            out.push_str(&format!("### {}\n", note.rel_path));
            if note.headings.is_empty() {
                out.push_str("- (sem headings)\n");
            } else {
                for h in &note.headings {
                    out.push_str(&format!("- {h}\n"));
                }
            }
        }
    }

    out.push_str("\n## Grafo de [[wikilinks]]\n");
    if data.notes.is_empty() {
        out.push_str("- (nenhuma nota)\n");
    } else {
        for note in &data.notes {
            let node = note.rel_path.strip_suffix(".md").unwrap_or(&note.rel_path);
            if note.links.is_empty() {
                out.push_str(&format!("- {node} → (folha)\n"));
            } else {
                let links: Vec<String> = note.links.iter().map(|l| format!("[[{l}]]")).collect();
                out.push_str(&format!("- {node} → {}\n", links.join(", ")));
            }
        }
    }
    out
}

/// Slug ASCII do nome do vault (minúsculo, não-alfanumérico → `-`, sem `-` repetido/nas pontas).
fn slugify(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "vault".to_string()
    } else {
        s
    }
}

/// Nome do arquivo de índice: `<slug>-<hash do caminho>.md` — o hash desambigua vaults de mesmo nome
/// em pastas diferentes (determinístico: `DefaultHasher` tem semente fixa).
fn index_filename(name: &str, path: &Path) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("{}-{:08x}.md", slugify(name), (h.finish() & 0xffff_ffff) as u32)
}

/// Gera e grava o índice de `vault` em `<lina_dir>/vault-index/<slug>-<hash>.md` (FORA do vault do
/// usuário — respeita "leitura por padrão"). Escrita atômica. Devolve o caminho gravado.
pub fn write_vault_index(lina_dir: &Path, vault: &VaultLink) -> std::io::Result<PathBuf> {
    let data = scan_vault(&vault.path);
    let md = render_vault_index(&vault.name, &vault.path, &data);
    let path = lina_dir
        .join("vault-index")
        .join(index_filename(&vault.name, &vault.path));
    write_atomic(&path, &md)?;
    Ok(path)
}

// ═══════════════════════════ persistência — `.lina/vault.json` (config, não evento) ═══════════════════════════

/// Uma entrada de vault no `.lina/vault.json`. `writable` = `<path>/Lina` (a única pasta onde a Lina
/// escreve; convenção já documentada na doutrina).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    pub name: String,
    pub path: String,
    pub writable: String,
}

/// O contrato `.lina/vault.json` (projeção em arquivo, par de `agents.json`/`plan.md`). `primary` é o
/// vault que o bootstrap usa em `{{vault_writable_paths}}`/`{{vault_tino_path}}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultConfig {
    pub primary: String,
    pub vaults: Vec<VaultEntry>,
}

/// Escrita atômica (tmp + rename) — robusta a crash/concorrência (espelha `write_agents`).
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Grava `.lina/vault.json` (atômico). Best-effort no caller (loga, não panica).
pub fn write_vault_config(lina_dir: &Path, config: &VaultConfig) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(&lina_dir.join("vault.json"), &json)
}

/// Lê o vault `primary` de `.lina/vault.json` (para o `main.rs` montar o `BootstrapWriter`). `None`
/// se o arquivo não existe / é inválido / `primary` vazio — o caller cai no fallback `LINA_VAULT`.
#[must_use]
pub fn read_primary_vault(lina_dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(lina_dir.join("vault.json")).ok()?;
    let cfg: VaultConfig = serde_json::from_str(&s).ok()?;
    let p = cfg.primary.trim();
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}

// ═══════════════════════════ estados da tela + rótulo do rodapé (puro) ═══════════════════════════

/// Os 5 estados da tela (UX §3). Fonte única de verdade do banner E do rótulo do rodapé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Procurando o Obsidian (carregando).
    Searching,
    /// Obsidian não instalado.
    NotInstalled,
    /// Instalado, COM pastas (multi-seleção).
    WithVaults,
    /// Instalado, SEM nenhuma pasta.
    NoVaults,
    /// Confirmado (sucesso).
    Confirmed,
}

/// PURO (UX §5): o rótulo do slot primário do rodapé, derivado do estado + contagem. **NUNCA fica
/// vazio nem desabilitado** (inv#6) — sempre há um avanço ("Pular" ou "Continuar").
#[must_use]
pub fn footer_label(screen: Screen, selected_count: usize) -> String {
    match screen {
        Screen::Confirmed => "Continuar →".to_string(),
        Screen::WithVaults => match selected_count {
            0 => "Pular esta etapa →".to_string(),
            1 => "Continuar com 1 pasta →".to_string(),
            n => format!("Continuar com {n} pastas →"),
        },
        Screen::Searching | Screen::NotInstalled | Screen::NoVaults => {
            "Pular esta etapa →".to_string()
        }
    }
}

// ═══════════════════════════ receita de instalação do Obsidian (reusa o pipeline) ═══════════════════════════

/// `second-brain.toml` embutido (config, NÃO hardcoded — inv#3).
const SECOND_BRAIN_TOML: &str = include_str!("../../../profiles/installers/second-brain.toml");

/// Tabela parseada uma vez. TOML inválido → tabela vazia (o botão vira fallback manual); nunca derruba.
fn second_brain_installers() -> &'static Installers {
    static INSTALLERS: OnceLock<Installers> = OnceLock::new();
    INSTALLERS.get_or_init(|| {
        Installers::from_toml_str(SECOND_BRAIN_TOML, "profiles/installers/second-brain.toml")
            .unwrap_or_else(|e| {
                eprintln!("obsidian: second-brain.toml inválido ({e}); instalação automática off");
                Installers::default()
            })
    })
}

/// Receita do Obsidian p/ o SO atual, com override `LINA_INSTALL_OBSIDIAN` (`sh -c`).
#[must_use]
pub fn install_recipe_for_obsidian() -> Option<InstallRecipe> {
    install_recipe_with(
        OBSIDIAN_ID,
        std::env::var("LINA_INSTALL_OBSIDIAN").ok().as_deref(),
        second_brain_installers(),
    )
}

/// Aviso do caminho NÃO-silencioso da instalação (o [`InstallState`] só modela o PTY oculto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallNotice {
    /// Abrimos um Terminal real p/ instalar (pode pedir a senha do Mac).
    TerminalOpened,
    /// Não consegui abrir o instalador — instrução p/ baixar do site.
    UseWebsite,
}

// ═══════════════════════════ o modelo (gpui-free) ═══════════════════════════

/// Função de varredura INJETÁVEL (testes passam um [`ObsidianScan`] determinístico, sem tocar disco).
pub type ScanFn = Arc<dyn Fn() -> ObsidianScan + Send + Sync>;

/// Caminho canônico (resolve symlinks/atalhos p/ dedup); fallback no caminho cru se não resolver.
fn canonical(path: &Path) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Resultado de "Adicionar outra pasta…" (UX §7.1, versão essencial: nova / já-na-lista).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddResult {
    Added,
    AlreadyPresent,
}

/// Estado da tela "segundo cérebro", sem nenhum tipo de gpui. Espelha o `DevToolsModel`: snapshot de
/// varredura compartilhado (escrito por uma thread, lido por frame), instalação no PTY oculto reusada,
/// seleção persistível e o destino `.lina/` onde grava o vault.json + os índices.
pub struct SecondBrainModel {
    /// `<ws_root>/.lina` — onde escreve `vault.json` e `vault-index/`.
    lina_dir: PathBuf,
    scan: Arc<Mutex<ObsidianScan>>,
    discovering: Arc<AtomicBool>,
    discovery_handle: Option<thread::JoinHandle<()>>,
    discover: ScanFn,
    install: Arc<Mutex<InstallState>>,
    install_handle: Option<thread::JoinHandle<()>>,
    install_consumed: bool,
    notice: Option<InstallNotice>,
    /// Caminhos canônicos marcados (lidos pela Lina). Persistido só no confirmar (vault.json).
    selected: BTreeSet<String>,
    /// Pastas adicionadas manualmente (preservadas entre re-detecções).
    manual: Vec<VaultLink>,
    /// Pré-seleção do vault "aberto agora" já aplicada (1x, após a 1ª varredura).
    initialized: bool,
    /// `true` após o usuário confirmar (mostra o estado 5 / sucesso).
    confirmed: bool,
    /// Handle da thread que gera o índice (PageIndex) em background ao confirmar. **Test-only** join
    /// via [`SecondBrainModel::block_on_index`] (a produção nunca bloqueia a UI).
    index_handle: Option<thread::JoinHandle<()>>,
}

impl SecondBrainModel {
    /// Modelo com a varredura REAL ([`discover_obsidian`]).
    pub fn new(lina_dir: PathBuf) -> Self {
        Self::new_with(lina_dir, Arc::new(discover_obsidian))
    }

    /// Modelo com a varredura INJETADA (testes passam um snapshot determinístico).
    pub fn new_with(lina_dir: PathBuf, discover: ScanFn) -> Self {
        let mut model = Self {
            lina_dir,
            scan: Arc::new(Mutex::new(ObsidianScan::default())),
            discovering: Arc::new(AtomicBool::new(false)),
            discovery_handle: None,
            discover,
            install: Arc::new(Mutex::new(InstallState::Idle)),
            install_handle: None,
            install_consumed: true,
            notice: None,
            selected: BTreeSet::new(),
            manual: Vec::new(),
            initialized: false,
            confirmed: false,
            index_handle: None,
        };
        model.redetect();
        model
    }

    /// Snapshot da varredura corrente.
    fn scan(&self) -> ObsidianScan {
        self.scan.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// `true` se o app Obsidian foi encontrado.
    #[must_use]
    pub fn app_present(&self) -> bool {
        self.scan().app_present
    }

    /// `true` enquanto a varredura roda (a view mostra "procurando…").
    #[must_use]
    pub fn is_discovering(&self) -> bool {
        self.discovering.load(Ordering::SeqCst)
    }

    /// Estado atual da instalação (clone do compartilhado).
    #[must_use]
    pub fn install_state(&self) -> InstallState {
        self.install.lock().map(|g| g.clone()).unwrap_or(InstallState::Idle)
    }

    /// Aviso corrente do caminho interativo (terminal aberto / use o site).
    #[must_use]
    pub fn notice(&self) -> Option<&InstallNotice> {
        self.notice.as_ref()
    }

    /// Todos os vaults conhecidos: detectados + manuais, deduplicados por caminho canônico (ordem:
    /// detectados primeiro, manuais depois).
    #[must_use]
    pub fn all_vaults(&self) -> Vec<VaultLink> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for v in self.scan().vaults.into_iter().chain(self.manual.iter().cloned()) {
            if seen.insert(canonical(&v.path)) {
                out.push(v);
            }
        }
        out
    }

    /// `true` se o vault está marcado p/ a Lina ler.
    #[must_use]
    pub fn is_selected(&self, path: &Path) -> bool {
        self.selected.contains(&canonical(path))
    }

    /// Quantos vaults conhecidos estão marcados.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.all_vaults().iter().filter(|v| self.is_selected(&v.path)).count()
    }

    /// Estado da tela (UX §3) — fonte única do banner e do rodapé.
    #[must_use]
    pub fn screen(&self) -> Screen {
        if self.confirmed {
            Screen::Confirmed
        } else if self.is_discovering() {
            Screen::Searching
        } else if !self.app_present() {
            Screen::NotInstalled
        } else if self.all_vaults().is_empty() {
            Screen::NoVaults
        } else {
            Screen::WithVaults
        }
    }

    /// Marca/desmarca um vault (consentimento explícito: nada é lido sem o ✓ deliberado).
    pub fn toggle(&mut self, path: &Path) {
        let key = canonical(path);
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
    }

    /// Adiciona uma pasta escolhida no seletor nativo: dedup por caminho canônico; entra **marcada**.
    pub fn add_manual_vault(&mut self, path: PathBuf) -> AddResult {
        let key = canonical(&path);
        if self.all_vaults().iter().any(|v| canonical(&v.path) == key) {
            self.selected.insert(key); // já existe → garante marcada (UX §7.1 caso 2)
            return AddResult::AlreadyPresent;
        }
        self.manual.push(VaultLink {
            name: vault_name(&path),
            path,
            open: false,
            added_manually: true,
        });
        self.selected.insert(key);
        AddResult::Added
    }

    /// Re-varre em **BACKGROUND** (não trava a UI nem se a leitura do registro pendurar). Idempotente.
    pub fn redetect(&mut self) {
        if self.discovering.swap(true, Ordering::SeqCst) {
            return;
        }
        let discover = Arc::clone(&self.discover);
        let scan = Arc::clone(&self.scan);
        let discovering = Arc::clone(&self.discovering);
        let handle = thread::spawn(move || {
            let found = discover();
            if let Ok(mut s) = scan.lock() {
                *s = found;
            }
            discovering.store(false, Ordering::SeqCst);
        });
        self.discovery_handle = Some(handle);
    }

    /// Bloqueia até a varredura terminar. **Test-only** (determinismo).
    #[allow(dead_code)]
    pub fn block_on_discovery(&mut self) {
        if let Some(h) = self.discovery_handle.take() {
            let _ = h.join();
        }
    }

    /// Bloqueia até a geração do índice (background, disparada por [`confirm`]) terminar. **Test-only**
    /// (a produção nunca bloqueia — o índice fica pronto quando ficar; o assistente lê depois).
    #[allow(dead_code)]
    pub fn block_on_index(&mut self) {
        if let Some(h) = self.index_handle.take() {
            let _ = h.join();
        }
    }

    /// Re-detecção manual (botão "Verificar"/"Já instalei"). Limpa o aviso (o usuário agiu).
    pub fn verify_now(&mut self) {
        self.notice = None;
        self.redetect();
    }

    /// "Instalar para mim": escolhe o caminho pelo [`decide_plan`] sobre o PATH real (silencioso no
    /// PTY oculto / terminal real), com a verificação por **bundle** (não PATH).
    pub fn start_install(&mut self) {
        let path = std::env::var("PATH").unwrap_or_default();
        self.start_install_with(&|bin| find_in_path(bin, &path).is_some());
    }

    /// Núcleo de [`start_install`] com a presença de binário INJETADA (testável sem mutar o env global).
    fn start_install_with(&mut self, bin_present: &dyn Fn(&str) -> bool) {
        if matches!(
            self.install_state(),
            InstallState::Installing { .. } | InstallState::Verifying
        ) {
            return;
        }
        self.notice = None;

        let Some(recipe) = install_recipe_for_obsidian() else {
            set_install(
                &self.install,
                InstallState::Failed {
                    reason: "ainda não sei instalar o Obsidian neste sistema automaticamente — \
                             baixe do site (obsidian.md) e clique em Verificar"
                        .into(),
                },
            );
            return;
        };

        match decide_plan(OBSIDIAN_ID, CURRENT_OS, Some(&recipe), bin_present) {
            InstallPlan::Manual | InstallPlan::NeedsFirst { .. } => set_install(
                &self.install,
                InstallState::Failed {
                    reason: "ainda não sei instalar o Obsidian neste sistema automaticamente — \
                             baixe do site (obsidian.md) e clique em Verificar"
                        .into(),
                },
            ),
            InstallPlan::Interactive => match open_in_terminal(&recipe) {
                Ok(_) => self.notice = Some(InstallNotice::TerminalOpened),
                Err(_) => self.notice = Some(InstallNotice::UseWebsite),
            },
            InstallPlan::Silent => self.start_silent(recipe),
        }
    }

    /// Caminho silencioso: PTY oculto (reusa [`run_install`]); a verificação é **achar o bundle**
    /// (Obsidian é app, não binário no PATH) — devolve um `DiscoveredCli` sintético p/ o `run_install`.
    fn start_silent(&mut self, recipe: InstallRecipe) {
        self.install_consumed = false;
        set_install(&self.install, InstallState::Installing { line: "iniciando…".into() });
        let handle = run_install(recipe, Arc::clone(&self.install), move || {
            find_app_bundle(OBSIDIAN_APP).map(|p| DiscoveredCli {
                id: OBSIDIAN_ID.to_string(),
                version: None,
                path: p.display().to_string(),
            })
        });
        self.install_handle = Some(handle);
    }

    /// A cada frame: conclui a instalação (re-detecta 1x) e aplica a pré-seleção inicial do vault
    /// "aberto agora" (1x, após a 1ª varredura) — novas pastas em re-detecção entram DESMARCADAS.
    pub fn poll(&mut self) {
        if !self.install_consumed && matches!(self.install_state(), InstallState::Ok { .. }) {
            self.install_consumed = true;
            self.redetect();
        }
        if !self.initialized && !self.is_discovering() {
            self.initialized = true;
            for v in self.scan().vaults {
                if v.open {
                    self.selected.insert(canonical(&v.path));
                }
            }
        }
    }

    /// **Confirma**: grava `.lina/vault.json` (atômico) + o índice PageIndex de cada vault marcado, e
    /// entra no estado de sucesso. Sem nenhum marcado, vira "pular" (não grava config vazia). A
    /// integração com a doutrina é via os arquivos `.lina/` (o onboarding NÃO tem handles do canvas —
    /// não chama `rewrite_bootstrap`; a próxima reescrita natural do bootstrap pega o `primary`).
    pub fn confirm(&mut self) {
        let chosen: Vec<VaultLink> =
            self.all_vaults().into_iter().filter(|v| self.is_selected(&v.path)).collect();
        if chosen.is_empty() {
            self.skip();
            return;
        }
        let primary = chosen
            .iter()
            .find(|v| v.open)
            .or_else(|| chosen.first())
            .map(|v| v.path.display().to_string())
            .unwrap_or_default();
        let entries = chosen
            .iter()
            .map(|v| VaultEntry {
                name: v.name.clone(),
                path: v.path.display().to_string(),
                writable: v.path.join("Lina").display().to_string(),
            })
            .collect();
        let config = VaultConfig { primary, vaults: entries };
        // `vault.json` é pequeno → grava já (rápido na thread de UI).
        if let Err(e) = write_vault_config(&self.lina_dir, &config) {
            eprintln!("obsidian: não gravei .lina/vault.json: {e}");
        }
        // O ÍNDICE (PageIndex) faz scan RECURSIVO lendo TODO `.md` do vault — um segundo cérebro real
        // tem milhares de notas, então rodar isso na thread de UI CONGELA o app (beachball / "app não
        // responde"). Roda FORA da thread de UI (detached); o índice não é necessário na hora (o
        // assistente lê depois). Best-effort: se o app fechar antes, re-confirmar regenera.
        let lina_dir = self.lina_dir.clone();
        self.index_handle = Some(thread::spawn(move || {
            for v in &chosen {
                if let Err(e) = write_vault_index(&lina_dir, v) {
                    eprintln!("obsidian: não gerei o índice de {}: {e}", v.name);
                }
            }
        }));
        self.confirmed = true;
    }

    /// "Pular esta etapa" — não conecta nada (o avanço do passo é do `OnboardingView`).
    pub fn skip(&mut self) {
        self.confirmed = false;
    }

    /// "Trocar as pastas" (estado 5 → 3): volta à seleção com as marcações preservadas.
    pub fn edit_again(&mut self) {
        self.confirmed = false;
    }

    /// "Desconectar tudo" (estado 5 → 3 com 0 marcadas).
    pub fn disconnect_all(&mut self) {
        self.selected.clear();
        self.confirmed = false;
    }

    // ═══════════════════════════ a view (fina) ═══════════════════════════

    /// Desenha a tela (UX §3): cabeçalho + banner de estado + âncoras de privacidade/não-destruição +
    /// o corpo do estado + rodapé (Voltar · Verificar · slot primário concordante). Os cliques roteiam
    /// pela view-pai ([`OnboardingView`]).
    pub fn render(&self, _window: &mut Window, cx: &mut Context<OnboardingView>) -> AnyElement {
        let screen = self.screen();
        let count = self.selected_count();

        let mut col = div().flex().flex_col().gap_5().child(heading(
            "Seu segundo cérebro de anotações",
            if screen == Screen::Confirmed {
                "Pronto — sua memória está conectada."
            } else {
                "A Lina pode aprender com as anotações que você já tem — assim ela responde do seu \
                 jeito. Isso usa um app chamado Obsidian (um caderno digital de anotações). Esta \
                 etapa é opcional: você pode pular agora e ligar isso depois."
            },
        ));

        col = col.child(self.banner(screen, count));
        // Âncora A — privacidade (🔒): cor é REFORÇO, o significado vem do ícone+texto (inv#6).
        col = col.child(banner(
            0x18301f,
            GREEN,
            "🔒 Tudo fica no SEU computador. A Lina lê suas anotações aqui, na sua máquina. Nada é \
             enviado pra internet, nem pra nuvem — e você pode desligar quando quiser.",
        ));

        col = col.child(match screen {
            Screen::Searching => self.body_searching(),
            Screen::NotInstalled => self.body_not_installed(cx),
            Screen::WithVaults => self.body_with_vaults(cx, count),
            Screen::NoVaults => self.body_no_vaults(cx),
            Screen::Confirmed => self.body_confirmed(cx),
        });

        // Âncora B — não-destruição (🛡️) + lembrete de estado salvo (💾), acima do rodapé.
        col = col.child(banner(
            PANEL,
            ACCENT,
            "🛡️ A Lina nunca apaga nem altera suas anotações. Ela só escreve numa pastinha nova \
             chamada \"Lina\", que cria dentro da pasta que você escolher. No resto, NUNCA mexe.",
        ));
        col = col.child(
            div().text_color(rgb(MUTED)).child(text!(
                "💾 Salvo automaticamente. Pode fechar e voltar quando quiser — nada se perde."
            )),
        );

        // Rodapé (UX §5): Voltar · Verificar · slot primário (rótulo concordante com o banner).
        col = col.child(self.footer(screen, count, cx));
        col.into_any_element()
    }

    /// Banner de estado (UX §4) — cor + ícone + texto; o ícone/texto carregam o significado.
    fn banner(&self, screen: Screen, count: usize) -> AnyElement {
        match screen {
            Screen::Searching => banner(
                PANEL,
                ACCENT,
                "🔵 Procurando o Obsidian no seu computador… Isso costuma levar uns segundos.",
            ),
            Screen::NotInstalled => banner(
                0x33202c,
                AMBER,
                "🟠 Ainda não encontramos o Obsidian aqui no seu computador. Tudo bem — ele não vem \
                 instalado. · é opcional",
            ),
            Screen::WithVaults => {
                let n = self.all_vaults().len();
                let pastas = if n == 1 {
                    "1 pasta de anotações".to_string()
                } else {
                    format!("{n} pastas de anotações")
                };
                banner(
                    0x18301f,
                    GREEN,
                    &format!(
                        "🟢 Achei o Obsidian e encontrei {pastas}. Marque quais você quer que a Lina \
                         use pra te ajudar."
                    ),
                )
            }
            Screen::NoVaults => banner(
                0x33202c,
                AMBER,
                "🟠 Achei o Obsidian, mas você ainda não tem nenhuma pasta de anotações criada. Sem \
                 problema — dá pra resolver.",
            ),
            Screen::Confirmed => {
                let pastas = if count == 1 {
                    "1 pasta de anotações".to_string()
                } else {
                    format!("{count} pastas de anotações")
                };
                banner(
                    0x18301f,
                    GREEN,
                    &format!(
                        "🟢 Pronto! A Lina agora aprende com {pastas}. Ela vai usar o que está nelas \
                         pra te ajudar melhor."
                    ),
                )
            }
        }
    }

    /// Estado 1 — esqueleto de carregamento (o spinner é decoração; o texto é a verdade).
    fn body_searching(&self) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_color(rgb(MUTED)).child(text!("⟳ Procurando o app Obsidian … aguarde")))
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child(text!("⟳ Lendo a lista de pastas de anotações … aguarde")),
            )
            .into_any_element()
    }

    /// Estado 2 — não instalado: "Instalar para mim" (recomendado) + baixar do site + "já instalei".
    fn body_not_installed(&self, cx: &mut Context<OnboardingView>) -> AnyElement {
        let install = self.install_state();
        let installing = matches!(
            install,
            InstallState::Installing { .. } | InstallState::Verifying
        );
        let mut col = div().flex().flex_col().gap_3().child(
            div().text_color(rgb(TEXT)).child(text!(
                "Seu caderno de anotações vira a memória da Lina — ela aprende com o que você escreve \
                 e te ajuda melhor."
            )),
        );

        if installing {
            let line = match &install {
                InstallState::Installing { line } => line.clone(),
                _ => "verificando…".to_string(),
            };
            col = col.child(banner(PANEL, AMBER, &format!("⟳ Instalando o Obsidian: {line}")));
        } else {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .items_center()
                    .child(action_button(
                        "sb-install",
                        "⬇ Instalar para mim (recomendado)",
                        0x3d59c9,
                        cx,
                        |onb, _w, cx| {
                            onb.second_brain.start_install();
                            cx.notify();
                        },
                    ))
                    .child(ghost_button(
                        "sb-download",
                        "Baixar eu mesmo do site (obsidian.md) ↗",
                        cx,
                        |_onb, _w, cx| cx.open_url("https://obsidian.md"),
                    )),
            );
        }

        if let Some(notice) = self.notice() {
            col = col.child(self.notice_banner(notice));
        }
        if let InstallState::Failed { reason } = &install {
            col = col.child(banner(0x33202c, AMBER, &format!("⚠ {reason}")));
        }
        col.into_any_element()
    }

    /// Estado 3 — multi-seleção das pastas (sinais redundantes: ☑/☐ + ✓ + texto "Vai ser usada").
    fn body_with_vaults(&self, cx: &mut Context<OnboardingView>, count: usize) -> AnyElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_color(rgb(TEXT)).child(text!(
                "O que a Lina faz: ela LÊ suas anotações pra te entender — como uma leitura. E só \
                 ESCREVE numa pastinha nova \"Lina\" que cria dentro da sua pasta. No resto, NUNCA mexe."
            )))
            .child(div().text_color(rgb(MUTED)).child(text!(
                "Marque as pastas de anotações (no Obsidian, essas pastas são chamadas de \"vault\"):"
            )))
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child(text!(format!("Selecionadas: {count} de {}", self.all_vaults().len()))),
            );

        let mut list = div().flex().flex_col().gap_2();
        for (i, v) in self.all_vaults().into_iter().enumerate() {
            list = list.child(self.vault_row(i, &v, cx));
        }
        col = col.child(list);

        // Adicionar outra pasta (seletor nativo) — erro/feedback é INLINE, nunca pop-up que bloqueia.
        col = col.child(add_folder_button("sb-add", "＋ Adicionar outra pasta…", cx));

        // Confirmar (corpo, verde) — desabilitado-com-dica em 0 (o RODAPÉ nunca trava; UX §5).
        if count == 0 {
            col = col.child(banner(PANEL, MUTED, "Confirmar (nenhuma pasta marcada)"));
        } else {
            let label = if count == 1 {
                "✓ Confirmar 1 pasta para a Lina".to_string()
            } else {
                format!("✓ Confirmar {count} pastas para a Lina")
            };
            // Confirmar GRAVA (vault.json + índice) E AVANÇA o passo — sem isto o usuário confirmava e
            // ficava "preso" tendo que achar o "Continuar →" no rodapé (que ficava cortado). O rodapé
            // segue existindo p/ quem quer pular sem confirmar.
            col = col.child(action_button("sb-confirm", &label, 0x2c7a4b, cx, |onb, _w, cx| {
                onb.second_brain.confirm();
                onb.nav_continue();
                cx.notify();
            }));
        }
        col.into_any_element()
    }

    /// Uma linha de vault (UX §6): 3 sinais redundantes + `.id` único por linha (a11y/AccessKit).
    fn vault_row(&self, i: usize, v: &VaultLink, cx: &mut Context<OnboardingView>) -> AnyElement {
        let selected = self.is_selected(&v.path);
        let path = v.path.clone();
        let mut name_line = String::new();
        name_line.push_str(if selected { "✓ " } else { "  " });
        name_line.push_str(&v.name);
        if v.open {
            name_line.push_str("  ★ aberta agora");
        }
        if v.added_manually {
            name_line.push_str("  (adicionada por você)");
        }
        let status = if selected {
            "✓ Vai ser usada"
        } else {
            "Não vai ser usada · toque p/ incluir"
        };
        // `.id((..., i))` único por linha: o `text!` gera ElementId por LOCALIZAÇÃO no fonte; repetido
        // no laço colidiria no nó AccessKit (pânico c/ leitor de tela). O índice desambigua.
        div()
            .id(("sb-vault", i))
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .py_3()
            .rounded_md()
            .bg(rgb(if selected { PANEL_SEL } else { PANEL }))
            .cursor_pointer()
            .on_click(cx.listener(move |onb, _ev: &ClickEvent, _w, cx| {
                onb.second_brain.toggle(&path);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().child(text!(if selected { "☑" } else { "☐" })))
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(TEXT))
                            .font_weight(FontWeight::BOLD)
                            .child(text!(name_line)),
                    )
                    .child(div().text_color(rgb(if selected { GREEN } else { MUTED })).child(text!(status))),
            )
            .child(div().text_color(rgb(MUTED)).child(text!(v.path.display().to_string())))
            .into_any_element()
    }

    /// Estado 4 — sem pastas: apontar uma existente (seletor) ou criar no Obsidian.
    fn body_no_vaults(&self, cx: &mut Context<OnboardingView>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_color(rgb(GREEN)).child(text!("● Obsidian (seu caderno de anotações) … encontrado")))
            .child(div().text_color(rgb(AMBER)).child(text!("● Pastas de anotações … nenhuma ainda")))
            .child(div().text_color(rgb(TEXT)).child(text!(
                "① Apontar uma pasta que você já tem — se você já guarda anotações numa pasta do \
                 computador, é só mostrar ela pra Lina."
            )))
            .child(add_folder_button("sb-pick", "＋ Escolher uma pasta…", cx))
            .child(div().text_color(rgb(TEXT)).child(text!(
                "② Criar uma pasta no Obsidian — abra o Obsidian, clique em \"Create new vault\" \
                 (criar nova pasta de anotações) e depois volte aqui."
            )))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(ghost_button("sb-open-app", "↗ Abrir o Obsidian", cx, |_onb, _w, cx| {
                        if let Some(p) = find_app_bundle(OBSIDIAN_APP) {
                            cx.open_with_system(&p);
                        }
                    }))
                    .child(ghost_button("sb-recheck", "⟳ Já criei — procurar de novo", cx, |onb, _w, cx| {
                        onb.second_brain.verify_now();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Estado 5 — confirmado: recap das pastas + limite reafirmado + reversibilidade.
    fn body_confirmed(&self, cx: &mut Context<OnboardingView>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_2();
        for (i, v) in self.all_vaults().into_iter().filter(|v| self.is_selected(&v.path)).enumerate() {
            list = list.child(
                div()
                    .id(("sb-recap", i))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(PANEL))
                    .child(
                        div()
                            .text_color(rgb(GREEN))
                            .font_weight(FontWeight::BOLD)
                            .child(text!(format!("✓ {}", v.name))),
                    )
                    .child(div().text_color(rgb(MUTED)).child(text!(format!("{}  ·  Vai ser usada", v.path.display())))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_color(rgb(TEXT)).child(text!("O que você acabou de combinar com a Lina:")))
            .child(list)
            .child(div().text_color(rgb(MUTED)).child(text!(
                "E o limite continua valendo, sempre: a Lina só escreve na pastinha \"Lina\" que ela \
                 cria; nunca toca, move ou apaga mais nada seu. Você pode mudar isto depois."
            )))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(ghost_button("sb-edit", "Trocar as pastas", cx, |onb, _w, cx| {
                        onb.second_brain.edit_again();
                        cx.notify();
                    }))
                    .child(ghost_button("sb-disconnect", "Desconectar tudo", cx, |onb, _w, cx| {
                        onb.second_brain.disconnect_all();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Banner do aviso de instalação interativa (terminal aberto / use o site).
    fn notice_banner(&self, notice: &InstallNotice) -> AnyElement {
        match notice {
            InstallNotice::TerminalOpened => banner(
                PANEL,
                ACCENT,
                "Abri uma janela de Terminal para instalar o Obsidian. Ela pode pedir a senha do seu \
                 Mac — é normal e seguro. Quando terminar, volte e clique \"Verificar\".",
            ),
            InstallNotice::UseWebsite => banner(
                0x33202c,
                AMBER,
                "Não consegui instalar sozinho. Você pode baixar do site (obsidian.md) e clicar em \
                 \"Verificar\" — ou pular esta etapa por agora.",
            ),
        }
    }

    /// Rodapé fixo (UX §5): Voltar (esq.) · Verificar (centro) · slot primário (dir., nunca trava).
    fn footer(&self, screen: Screen, count: usize, cx: &mut Context<OnboardingView>) -> AnyElement {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .child(ghost_button("sb-back", "← Voltar", cx, |onb, _w, _cx| onb.nav_back()));

        // [Verificar]: oculto no estado 5; ativo nos demais (re-roda a detecção preservando marcações).
        if screen != Screen::Confirmed {
            row = row.child(ghost_button("sb-verify", "⟳ Verificar", cx, |onb, _w, cx| {
                onb.second_brain.verify_now();
                cx.notify();
            }));
        }
        row = row.child(div().flex_1());

        // Slot primário: rótulo derivado do estado (footer_label) — concorda com o banner.
        let label = footer_label(screen, count);
        let confirm_and_advance = screen == Screen::WithVaults && count > 0;
        row = row.child(action_button("sb-primary", &label, 0x2c7a4b, cx, move |onb, _w, _cx| {
            if confirm_and_advance {
                onb.second_brain.confirm();
            } else if screen != Screen::Confirmed {
                onb.second_brain.skip();
            }
            onb.nav_continue();
        }));
        row.into_any_element()
    }
}

/// Atualiza o estado compartilhado de instalação (best-effort sob poison).
fn set_install(state: &Arc<Mutex<InstallState>>, s: InstallState) {
    if let Ok(mut g) = state.lock() {
        *g = s;
    }
}

// ═══════════════════════════ helpers de view (estilo espelhado do dev_tools/onboarding) ═══════════════════════════

/// Caixa de título + subtítulo (mesma proporção do `heading` do onboarding).
fn heading(title: &str, subtitle: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(28.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(text!(title.to_string())),
        )
        .child(div().text_size(px(15.0)).text_color(rgb(MUTED)).child(text!(subtitle.to_string())))
        .into_any_element()
}

/// Banner de uma linha (cor de fundo + cor de texto + mensagem). Cor NUNCA sozinha — sempre texto.
fn banner(bg: u32, fg: u32, msg: &str) -> AnyElement {
    div()
        .px_4()
        .py_3()
        .rounded_md()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .child(text!(msg.to_string()))
        .into_any_element()
}

/// Botão de ação (cor própria) que roteia o clique pela view-pai.
fn action_button(
    id: &'static str,
    label: &str,
    bg: u32,
    cx: &mut Context<OnboardingView>,
    on_click: impl Fn(&mut OnboardingView, &mut Window, &mut Context<OnboardingView>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .px_5()
        .py_2()
        .rounded_md()
        .bg(rgb(bg))
        .text_color(rgb(0xeef1ff))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .on_click(cx.listener(move |onb, _ev: &ClickEvent, window, cx| on_click(onb, window, cx)))
        .child(text!(label.to_string()))
        .into_any_element()
}

/// Botão secundário (voltar / verificar / alternativa).
fn ghost_button(
    id: &'static str,
    label: &str,
    cx: &mut Context<OnboardingView>,
    on_click: impl Fn(&mut OnboardingView, &mut Window, &mut Context<OnboardingView>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(0x2a3152))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .on_click(cx.listener(move |onb, _ev: &ClickEvent, window, cx| on_click(onb, window, cx)))
        .child(text!(label.to_string()))
        .into_any_element()
}

/// Botão "Adicionar/Escolher pasta": abre o **seletor nativo** (só-pastas) FORA da thread de UI
/// (async via `cx.spawn`), valida pelo caminho canônico (dedup) e adiciona marcada. Erro = silencioso
/// (cancelar é legítimo); o estado nunca se perde.
fn add_folder_button(id: &'static str, label: &str, cx: &mut Context<OnboardingView>) -> AnyElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(0x2a3152))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .on_click(cx.listener(move |_onb, _ev: &ClickEvent, _w, cx| {
            let rx = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Usar esta pasta".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    if let Some(p) = paths.into_iter().next() {
                        let _ = this.update(cx, |onb, cx| {
                            onb.second_brain.add_manual_vault(p);
                            cx.notify();
                        });
                    }
                }
            })
            .detach();
        }))
        .child(text!(label.to_string()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Tempdir único, removido no Drop (mesmo idioma dos testes do onboarding/dev_tools).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "lina-obsidian-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&p).expect("tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&p, contents).expect("write");
    }

    /// `extract_headings`: só `#{1,6} ` no início da linha; ignora `#` indentado e `#sem-espaço`.
    #[test]
    fn headings_match_only_real_markdown_headings() {
        let md = "# Título\ntexto\n## Seção\n   # indentado\n#semsespaco\n###### Seis\n####### Sete";
        let hs = extract_headings(md);
        assert_eq!(hs, vec!["# Título", "## Seção", "###### Seis"]);
    }

    /// `extract_wikilinks`: extrai alvo (sem alias `|`/âncora `#`), na ordem, sem repetir.
    #[test]
    fn wikilinks_extract_target_dedup_ordered() {
        let md = "veja [[Nota A]] e [[Nota B|apelido]] e [[Nota A]] e [[Nota C#secao]]";
        assert_eq!(extract_wikilinks(md), vec!["Nota A", "Nota B", "Nota C"]);
        // colchete não fechado não quebra.
        assert_eq!(extract_wikilinks("[[aberto"), Vec::<String>::new());
    }

    /// `parse_vaults_from_json`: lê o mapa `vaults{id:{path,open}}`, deriva o nome, ordena por path.
    #[test]
    fn parse_vaults_reads_path_open_and_sorts() {
        let json = r#"{ "vaults": {
            "b2": { "path": "/Users/voce/Zeta", "open": false },
            "a1": { "path": "/Users/voce/Alpha", "open": true }
        } }"#;
        let v = parse_vaults_from_json(json);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "Alpha"); // ordenado por caminho
        assert!(v[0].open);
        assert_eq!(v[1].name, "Zeta");
        assert!(!v[1].open);
        // json sem "vaults" → vazio (nunca panica).
        assert!(parse_vaults_from_json("{}").is_empty());
        assert!(parse_vaults_from_json("não-é-json").is_empty());
    }

    /// `footer_label` (UX §5): nunca vazio; concorda com estado+contagem (singular/plural).
    #[test]
    fn footer_label_concords_with_state_and_count() {
        assert_eq!(footer_label(Screen::Searching, 0), "Pular esta etapa →");
        assert_eq!(footer_label(Screen::NotInstalled, 0), "Pular esta etapa →");
        assert_eq!(footer_label(Screen::NoVaults, 0), "Pular esta etapa →");
        assert_eq!(footer_label(Screen::WithVaults, 0), "Pular esta etapa →");
        assert_eq!(footer_label(Screen::WithVaults, 1), "Continuar com 1 pasta →");
        assert_eq!(footer_label(Screen::WithVaults, 3), "Continuar com 3 pastas →");
        assert_eq!(footer_label(Screen::Confirmed, 2), "Continuar →");
    }

    /// `scan_vault` + `render_vault_index`: walk determinístico, ignora `.obsidian`/`.trash`, headings
    /// e grafo de wikilinks; saída estável (re-render dá o MESMO markdown).
    #[test]
    fn scan_and_render_index_is_deterministic_and_skips_system_dirs() {
        let vault = TempDir::new("scan");
        write(vault.path(), "a.md", "# A\nlink pra [[b]]\n## Sub\n");
        write(vault.path(), "Area/b.md", "# B\n"); // folha (sem links)
        write(vault.path(), ".obsidian/app.json", "{}"); // ignorado
        write(vault.path(), ".trash/old.md", "# lixo"); // ignorado

        let data = scan_vault(vault.path());
        // folders: só Area/ (não .obsidian, não .trash).
        assert_eq!(data.folders, vec!["Area/".to_string()]);
        // notas ordenadas: Area/b.md, a.md → por rel_path "Area/b.md" < "a.md".
        let paths: Vec<&str> = data.notes.iter().map(|n| n.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["Area/b.md", "a.md"]);

        let md = render_vault_index("Meu Vault", vault.path(), &data);
        assert!(md.contains("# Vault Index — Meu Vault"));
        assert!(md.contains("NÃO editar"));
        assert!(md.contains("- Area/"));
        assert!(md.contains("### a.md"));
        assert!(md.contains("- # A"));
        assert!(md.contains("- ## Sub"));
        assert!(md.contains("a → [[b]]")); // grafo: a aponta b
        assert!(md.contains("Area/b → (folha)")); // b é folha
        // determinístico: re-scan + re-render = idêntico.
        let md2 = render_vault_index("Meu Vault", vault.path(), &scan_vault(vault.path()));
        assert_eq!(md, md2);
    }

    /// `write_vault_index` grava FORA do vault, em `<lina_dir>/vault-index/` (leitura-por-padrão).
    #[test]
    fn write_index_goes_outside_the_vault() {
        let vault = TempDir::new("idx-vault");
        write(vault.path(), "nota.md", "# Oi\n");
        let lina = TempDir::new("idx-lina");
        let v = VaultLink {
            name: "Notas".into(),
            path: vault.path().to_path_buf(),
            open: true,
            added_manually: false,
        };
        let out = write_vault_index(lina.path(), &v).expect("escreveu índice");
        assert!(out.starts_with(lina.path()), "índice mora em .lina, não no vault");
        assert!(out.exists());
        assert!(std::fs::read_to_string(&out).unwrap().contains("# Vault Index — Notas"));
        // não vazou pra dentro do vault.
        assert!(!vault.path().join("vault-index").exists());
    }

    /// `.lina/vault.json`: escrita atômica + releitura do `primary` (contrato p/ o main.rs).
    #[test]
    fn vault_config_roundtrips_and_reads_primary() {
        let lina = TempDir::new("cfg");
        assert!(read_primary_vault(lina.path()).is_none()); // ausente → None (fallback)
        let cfg = VaultConfig {
            primary: "/Users/voce/Notas".into(),
            vaults: vec![VaultEntry {
                name: "Notas".into(),
                path: "/Users/voce/Notas".into(),
                writable: "/Users/voce/Notas/Lina".into(),
            }],
        };
        write_vault_config(lina.path(), &cfg).expect("gravou vault.json");
        assert_eq!(read_primary_vault(lina.path()).as_deref(), Some("/Users/voce/Notas"));
        // releitura completa = igual.
        let back: VaultConfig =
            serde_json::from_str(&std::fs::read_to_string(lina.path().join("vault.json")).unwrap())
                .unwrap();
        assert_eq!(back, cfg);
    }

    /// O `second-brain.toml` REAL parseia e cobre os 3 SOs com `program` não-vazio.
    #[test]
    fn real_second_brain_toml_is_valid_and_complete() {
        let inst = second_brain_installers();
        let prof = inst.0.get("obsidian").expect("falta receita obsidian");
        for os in ["macos", "linux", "windows"] {
            let r = prof.for_os(os).unwrap_or_else(|| panic!("falta obsidian.{os}"));
            assert!(!r.program.trim().is_empty(), "obsidian.{os} sem program");
        }
        // macOS usa cask do brew (verificado por bundle, não PATH).
        assert!(format!("{:?}", prof.for_os("macos").unwrap()).contains("--cask"));
        // Windows usa winget (→ interativo via decide_plan).
        assert_eq!(prof.for_os("windows").unwrap().program, "winget");
    }

    /// `decide_plan` reusado p/ o Obsidian: macOS com brew presente → silencioso; sem brew → terminal
    /// real; Windows (winget) → interativo; Linux (flatpak, sem sudo) → silencioso.
    #[test]
    fn install_plan_for_obsidian_per_os() {
        let inst = second_brain_installers();
        let prof = &inst.0["obsidian"];
        let mac = prof.for_os("macos").cloned();
        assert_eq!(decide_plan("obsidian", "macos", mac.as_ref(), |_| true), InstallPlan::Silent);
        assert_eq!(
            decide_plan("obsidian", "macos", mac.as_ref(), |_| false),
            InstallPlan::Interactive
        );
        let win = prof.for_os("windows").cloned();
        assert_eq!(decide_plan("obsidian", "windows", win.as_ref(), |_| true), InstallPlan::Interactive);
        let lin = prof.for_os("linux").cloned();
        assert_eq!(decide_plan("obsidian", "linux", lin.as_ref(), |_| true), InstallPlan::Silent);
    }

    /// Snapshot INJETADO: o modelo reflete app+vaults, deriva o estado (screen) e roda fora da thread.
    #[test]
    fn model_reflects_injected_scan_and_derives_screen() {
        let lina = TempDir::new("model");
        let scan = ObsidianScan {
            app_present: true,
            vaults: vec![VaultLink {
                name: "Notas".into(),
                path: PathBuf::from("/Users/voce/Notas"),
                open: true,
                added_manually: false,
            }],
        };
        let disc: ScanFn = Arc::new(move || scan.clone());
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), disc);
        m.block_on_discovery();
        m.poll(); // aplica a pré-seleção do vault aberto
        assert!(m.app_present());
        assert_eq!(m.all_vaults().len(), 1);
        assert_eq!(m.screen(), Screen::WithVaults);
        // vault "aberto agora" entra pré-marcado (UX §3).
        assert_eq!(m.selected_count(), 1);
        assert!(m.is_selected(Path::new("/Users/voce/Notas")));
    }

    /// Estados derivados: sem app → NotInstalled; app sem vaults → NoVaults.
    #[test]
    fn screen_not_installed_and_no_vaults() {
        let lina = TempDir::new("screen");
        let no_app: ScanFn = Arc::new(ObsidianScan::default);
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), no_app);
        m.block_on_discovery();
        m.poll();
        assert_eq!(m.screen(), Screen::NotInstalled);

        let app_only: ScanFn =
            Arc::new(|| ObsidianScan { app_present: true, vaults: vec![] });
        let mut m2 = SecondBrainModel::new_with(lina.path().to_path_buf(), app_only);
        m2.block_on_discovery();
        m2.poll();
        assert_eq!(m2.screen(), Screen::NoVaults);
    }

    /// `toggle` alterna a marcação (consentimento explícito).
    #[test]
    fn toggle_selects_and_deselects() {
        let lina = TempDir::new("toggle");
        let app_only: ScanFn = Arc::new(|| ObsidianScan { app_present: true, vaults: vec![] });
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), app_only);
        m.block_on_discovery();
        m.poll();
        let p = Path::new("/tmp/qualquer-pasta");
        assert!(!m.is_selected(p));
        m.toggle(p);
        assert!(m.is_selected(p));
        m.toggle(p);
        assert!(!m.is_selected(p));
    }

    /// `confirm` grava `.lina/vault.json` (com primary) + o índice de cada vault marcado, e entra no
    /// estado de sucesso — o CRITÉRIO observável da feature (persistência + PageIndex + sucesso).
    #[test]
    fn confirm_writes_config_and_index_and_succeeds() {
        let vault = TempDir::new("conf-vault");
        write(vault.path(), "nota.md", "# Olá\n[[outra]]\n");
        let lina = TempDir::new("conf-lina");
        let vpath = vault.path().to_path_buf();
        let scan = ObsidianScan {
            app_present: true,
            vaults: vec![VaultLink {
                name: "Notas".into(),
                path: vpath.clone(),
                open: true,
                added_manually: false,
            }],
        };
        let disc: ScanFn = Arc::new(move || scan.clone());
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), disc);
        m.block_on_discovery();
        m.poll(); // pré-marca o vault aberto
        assert_eq!(m.selected_count(), 1);

        m.confirm();
        assert_eq!(m.screen(), Screen::Confirmed);
        // vault.json gravado com primary correto.
        assert_eq!(
            read_primary_vault(lina.path()).as_deref(),
            Some(vpath.display().to_string().as_str())
        );
        // writable = <path>/Lina.
        let cfg: VaultConfig = serde_json::from_str(
            &std::fs::read_to_string(lina.path().join("vault.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg.vaults[0].writable, vpath.join("Lina").display().to_string());
        // índice gerado em .lina/vault-index/ (agora em BACKGROUND ao confirmar — não trava a UI;
        // o teste espera a thread terminar p/ ser determinístico).
        m.block_on_index();
        let idx_dir = lina.path().join("vault-index");
        let files: Vec<_> = std::fs::read_dir(&idx_dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1, "um índice por vault marcado");
        let idx = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(idx.contains("# Vault Index — Notas"));
        assert!(idx.contains("nota → [[outra]]"));
    }

    /// `confirm` com 0 marcadas NÃO grava config (vira "pular" — UX §5) e não cria vault.json.
    #[test]
    fn confirm_with_zero_selected_does_not_write_config() {
        let lina = TempDir::new("conf-zero");
        let app_only: ScanFn = Arc::new(|| ObsidianScan { app_present: true, vaults: vec![] });
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), app_only);
        m.block_on_discovery();
        m.poll();
        m.confirm();
        assert!(!lina.path().join("vault.json").exists());
    }

    /// `add_manual_vault` adiciona marcada e DEDUPLICA por caminho (não duplica a mesma pasta).
    #[test]
    fn add_manual_vault_dedups_and_selects() {
        let folder = TempDir::new("manual");
        let lina = TempDir::new("manual-lina");
        let app_only: ScanFn = Arc::new(|| ObsidianScan { app_present: true, vaults: vec![] });
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), app_only);
        m.block_on_discovery();
        m.poll();
        assert_eq!(m.add_manual_vault(folder.path().to_path_buf()), AddResult::Added);
        assert_eq!(m.all_vaults().len(), 1);
        assert!(m.is_selected(folder.path()));
        // mesma pasta de novo → não duplica.
        assert_eq!(m.add_manual_vault(folder.path().to_path_buf()), AddResult::AlreadyPresent);
        assert_eq!(m.all_vaults().len(), 1);
    }

    /// `slugify`/`index_filename`: ASCII estável; vaults de mesmo nome em pastas distintas NÃO colidem.
    #[test]
    fn index_filename_disambiguates_same_name() {
        assert_eq!(slugify("Minhas Anotações!!"), "minhas-anota-es");
        let a = index_filename("Notas", Path::new("/a/Notas"));
        let b = index_filename("Notas", Path::new("/b/Notas"));
        assert_ne!(a, b, "mesmo nome, pastas diferentes → arquivos diferentes");
        assert!(a.ends_with(".md") && a.starts_with("notas-"));
    }
}
