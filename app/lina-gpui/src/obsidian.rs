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

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
use crate::ui::RadiusExt;

// ───────────────────────────── paleta (espelha o onboarding/canvas) ─────────────────────────────
/// F1-2-1: tokens VIVOS do design system — cada chamada lê o tema ATIVO (trocar dark/light ou
/// acento no T7 re-pinta esta janela no frame seguinte, sem restart). Fonte única: `theme::active`.
fn th() -> crate::theme::Theme {
    crate::theme::active()
}
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

/// PURO (injetável): candidatos do app `name` por SO. macOS: `/Applications` + `~/Applications`
/// (`.app`). Windows: `%LOCALAPPDATA%\Programs\<name>` (per-user, SEM UAC — corrige a armadilha do
/// blueprint que omitia `Programs`) + `%ProgramFiles%` (all-users). Linux: Flatpak exports + pacote
/// nativo (`/usr/bin`, `/usr/local/bin`) + Snap (`/snap/bin`) + user-local.
fn app_bundle_candidates_for(
    os: &str,
    name: &str,
    home: Option<&Path>,
    get_env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    let lname = name.to_ascii_lowercase();
    let mut out: Vec<PathBuf> = Vec::new();
    match os {
        "macos" => {
            out.push(PathBuf::from(format!("/Applications/{name}.app")));
            if let Some(h) = home {
                out.push(h.join("Applications").join(format!("{name}.app")));
            }
        }
        "windows" => {
            if let Some(local) = get_env("LOCALAPPDATA") {
                out.push(
                    PathBuf::from(local)
                        .join("Programs")
                        .join(name)
                        .join(format!("{name}.exe")),
                );
            }
            if let Some(pf) = get_env("ProgramFiles") {
                out.push(PathBuf::from(pf).join(name).join(format!("{name}.exe")));
            }
        }
        _ => {
            out.push(PathBuf::from(
                "/var/lib/flatpak/exports/bin/md.obsidian.Obsidian",
            ));
            out.push(PathBuf::from(format!("/usr/bin/{lname}")));
            out.push(PathBuf::from(format!("/usr/local/bin/{lname}")));
            out.push(PathBuf::from(format!("/snap/bin/{lname}")));
            if let Some(h) = home {
                out.push(h.join(".local/share/flatpak/exports/bin/md.obsidian.Obsidian"));
                out.push(h.join(".local/bin").join(&lname));
            }
        }
    }
    out
}

/// Acha o **bundle/binário do app** `name` no SO atual — 1º candidato existente (substitui
/// `find_in_path`: um `.app`/`.exe` não está no PATH; a armadilha do blueprint §6). macOS `.app` é
/// diretório; demais são arquivos — `exists()` cobre os dois.
#[must_use]
pub fn find_app_bundle(name: &str) -> Option<PathBuf> {
    app_bundle_candidates_for(std::env::consts::OS, name, home_dir().as_deref(), &|k| {
        std::env::var_os(k)
    })
    .into_iter()
    .find(|p| p.exists())
}

/// PURO (injetável): caminhos do registro `obsidian.json` por SO. macOS: Application Support. Windows:
/// `%APPDATA%\obsidian\`. Linux: **MERGE** de `$XDG_CONFIG_HOME`/`~/.config` (nativo) + Flatpak
/// (sandbox `~/.var/app/md.obsidian.Obsidian/config/obsidian`) + Snap (confinado
/// `~/snap/obsidian/current/.config/obsidian`) — o usuário pode ter o Obsidian por qualquer canal.
fn obsidian_config_paths_for(
    os: &str,
    home: Option<&Path>,
    get_env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    match os {
        "macos" => {
            if let Some(h) = home {
                out.push(h.join("Library/Application Support/obsidian/obsidian.json"));
            }
        }
        "windows" => {
            if let Some(appdata) = get_env("APPDATA") {
                out.push(
                    PathBuf::from(appdata)
                        .join("obsidian")
                        .join("obsidian.json"),
                );
            }
        }
        _ => {
            let xdg = get_env("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| home.map(|h| h.join(".config")));
            if let Some(cfg) = xdg {
                out.push(cfg.join("obsidian").join("obsidian.json"));
            }
            if let Some(h) = home {
                out.push(h.join(".var/app/md.obsidian.Obsidian/config/obsidian/obsidian.json"));
                out.push(h.join("snap/obsidian/current/.config/obsidian/obsidian.json"));
            }
        }
    }
    out
}

/// Caminhos de `obsidian.json` no SO atual (produção: env + home reais).
fn obsidian_config_paths() -> Vec<PathBuf> {
    obsidian_config_paths_for(std::env::consts::OS, home_dir().as_deref(), &|k| {
        std::env::var_os(k)
    })
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
            let open = entry
                .get("open")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let path = PathBuf::from(path);
            let name = vault_name(&path);
            Some(VaultLink {
                name,
                path,
                open,
                added_manually: false,
            })
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
    // MERGE dos vaults de TODOS os `obsidian.json` do SO (no Linux: nativo + Flatpak + Snap; 1 nos
    // demais), dedup por caminho canônico. Valida pelo marcador `<path>/.obsidian/` (blueprint §1):
    // descarta entradas obsoletas (pasta movida/apagada) sem all-or-nothing — as demais seguem.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut vaults: Vec<VaultLink> = Vec::new();
    for cfg in obsidian_config_paths() {
        let Ok(contents) = std::fs::read_to_string(&cfg) else {
            continue;
        };
        for v in parse_vaults_from_json(&contents) {
            if is_vault_dir(&v.path) && seen.insert(canonical(&v.path)) {
                vaults.push(v);
            }
        }
    }
    vaults.sort_by(|a, b| a.path.cmp(&b.path));
    ObsidianScan {
        app_present,
        vaults,
    }
}

// ═══════════════════════════ PageIndex — mapa estrutural determinístico (inv#1) ═══════════════════════════

/// Uma nota do índice: caminho relativo + headings + alvos de wikilink (saída do grafo) + embeds e
/// tags (inline + frontmatter, merged). Tudo extraído com os blocos de código JÁ removidos (inv#1: o
/// parser nunca captura `[[…]]`/`#tag` que aparecem em exemplos de código).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteEntry {
    pub rel_path: String,
    pub headings: Vec<String>,
    /// Alvos distintos de `[[wikilink]]` (sem `![[embed]]`), na ordem — arestas de saída do grafo.
    pub links: Vec<String>,
    /// Alvos distintos de `![[embed]]` (notas E anexos), na ordem — metadado informativo.
    pub embeds: Vec<String>,
    /// Tags distintas (sem o `#`), inline + frontmatter `tags:`, na ordem de 1ª aparição.
    pub tags: Vec<String>,
    /// Alvo de `[[wikilink]]` (limpo: sem alias/âncora) → nº de ocorrências — peso das arestas no
    /// sidecar JSON (preserva a multiplicidade que `links` perde ao deduplicar).
    pub link_counts: Vec<(String, usize)>,
    /// `true` se a nota é um **placeholder de nuvem** (iCloud/OneDrive evicted): existe no vault mas o
    /// conteúdo não foi baixado, então NÃO foi lida (headings/links/tags ficam vazios). Pular o read
    /// evita disparar um download por arquivo — a causa de travar ao indexar vaults grandes na nuvem.
    pub dataless: bool,
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

/// Alvo limpo de um wikilink interno: corta no 1º `|` (alias), `#` (heading) ou `^` (bloco); trim.
fn link_target(inner: &str) -> &str {
    let cut = inner.find(['|', '#', '^']).unwrap_or(inner.len());
    inner[..cut].trim()
}

/// PURO: ocorrências de `[[…]]`/`![[…]]` na ordem (COM repetição): `(alvo_limpo, is_embed)`. Varre por
/// índice absoluto p/ enxergar o `!` que precede um embed mesmo na fronteira de uma ocorrência anterior.
fn bracket_occurrences(content: &str) -> Vec<(String, bool)> {
    let bytes = content.as_bytes();
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut i = 0;
    while let Some(rel) = content[i..].find("[[") {
        let start = i + rel;
        let inner_start = start + 2;
        let Some(rel_end) = content[inner_start..].find("]]") else {
            break;
        };
        let inner = &content[inner_start..inner_start + rel_end];
        let is_embed = start > 0 && bytes[start - 1] == b'!';
        let target = link_target(inner);
        if !target.is_empty() {
            out.push((target.to_string(), is_embed));
        }
        i = inner_start + rel_end + 2;
    }
    out
}

/// Preserva a ordem, descarta repetições (1ª aparição vence).
fn dedup_ordered(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in items {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// PURO: alvos de `[[wikilink]]` (sem `![[embed]]`), sem alias/âncora, na ordem, sem repetir.
#[must_use]
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    dedup_ordered(
        bracket_occurrences(content)
            .into_iter()
            .filter(|(_, e)| !e)
            .map(|(t, _)| t),
    )
}

/// PURO: alvos de `![[embed]]` (notas E anexos), na ordem, sem repetir.
#[must_use]
pub fn extract_embeds(content: &str) -> Vec<String> {
    dedup_ordered(
        bracket_occurrences(content)
            .into_iter()
            .filter(|(_, e)| *e)
            .map(|(t, _)| t),
    )
}

/// Alvo de `[[wikilink]]` (não-embed) → nº de ocorrências, na ordem da 1ª aparição. Peso das arestas.
fn link_count_pairs(content: &str) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (target, is_embed) in bracket_occurrences(content) {
        if is_embed {
            continue;
        }
        if let Some(slot) = counts.iter_mut().find(|(k, _)| *k == target) {
            slot.1 += 1;
        } else {
            counts.push((target, 1));
        }
    }
    counts
}

/// Se `trimmed` abre/fecha uma cerca de código: `(char, comprimento, info_string_vazia)`. Cerca = ≥3
/// `` ` `` ou `~` no início (após trim de indentação). `bare` (sem info string) é exigido p/ FECHAR.
fn fence_marker(trimmed: &str) -> Option<(u8, usize, bool)> {
    let b = trimmed.as_bytes();
    let ch = *b.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = b.iter().take_while(|&&c| c == ch).count();
    if len < 3 {
        return None;
    }
    let bare = trimmed[len..].trim().is_empty();
    Some((ch, len, bare))
}

/// PURO (inv#1, CRÍTICO): zera as linhas DENTRO de cercas de código (` ``` ` / `~~~`), inclusive as de
/// abertura/fechamento — impede capturar `[[…]]`/`#tag` em EXEMPLOS de código. Não fecha ` ``` ` com
/// `~~~`; uma cerca de fechamento precisa ser "bare" (sem info string). Cerca não-fechada = resto do
/// arquivo tratado como código. Preserva a contagem de linhas (linhas removidas viram vazias).
#[must_use]
fn strip_code(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut fence: Option<(u8, usize)> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        match (fence, fence_marker(trimmed)) {
            (None, Some((ch, len, _))) => fence = Some((ch, len)), // abre (zera a linha)
            (Some((ch, len)), Some((mch, mlen, true))) if mch == ch && mlen >= len => fence = None, // fecha
            (Some(_), _) => {}                  // dentro do bloco: zera
            (None, None) => out.push_str(line), // fora: preserva
        }
        out.push('\n');
    }
    out
}

/// Separa o frontmatter YAML (`---` … `---`/`...` no TOPO do arquivo) do corpo. Sem frontmatter →
/// `("", content)`. Tolerante a BOM e CRLF.
fn split_frontmatter(content: &str) -> (&str, &str) {
    let s = content.strip_prefix('\u{feff}').unwrap_or(content);
    let after_open = match s
        .strip_prefix("---\n")
        .or_else(|| s.strip_prefix("---\r\n"))
    {
        Some(r) => r,
        None => return ("", content),
    };
    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            return (&after_open[..offset], &after_open[offset + line.len()..]);
        }
        offset += line.len();
    }
    ("", content) // frontmatter sem fechamento → trata tudo como corpo
}

/// Acrescenta uma tag normalizada (sem aspas, sem `#`, sem `/` nas pontas), sem repetir.
fn push_tag(out: &mut Vec<String>, raw: &str) {
    let t = raw
        .trim()
        .trim_matches(['"', '\''])
        .trim_start_matches('#')
        .trim_matches('/')
        .trim();
    if !t.is_empty() && t.chars().any(char::is_alphanumeric) && !out.iter().any(|x| x == t) {
        out.push(t.to_string());
    }
}

/// PURO: tags do frontmatter `tags:`/`tag:` — escalar (`tags: a, b`), lista flow (`[a, b]`) ou bloco
/// YAML (`- item` indentado). Merge/dedup. (Os valores entre aspas são desencapados.)
fn extract_frontmatter_tags(content: &str) -> Vec<String> {
    let (fm, _) = split_frontmatter(content);
    if fm.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut lines = fm.lines().peekable();
    while let Some(line) = lines.next() {
        let key_line = line.trim_start();
        if line.len() != key_line.len() {
            continue; // só chaves de nível raiz (sem indentação)
        }
        let Some(rest) = key_line
            .strip_prefix("tags:")
            .or_else(|| key_line.strip_prefix("tag:"))
        else {
            continue;
        };
        let rest = rest.trim();
        if rest.is_empty() {
            // bloco YAML: linhas seguintes `- item` (indentadas) até a próxima chave raiz.
            while let Some(peek) = lines.peek() {
                let pt = peek.trim();
                if let Some(item) = pt.strip_prefix("- ") {
                    push_tag(&mut out, item);
                    lines.next();
                } else if pt.is_empty() || peek.starts_with(' ') || peek.starts_with('\t') {
                    lines.next(); // em branco / continuação indentada: ignora
                } else {
                    break; // próxima chave de nível raiz
                }
            }
        } else if let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            for part in inner.split(',') {
                push_tag(&mut out, part);
            }
        } else {
            for part in rest.split([',', ' ']) {
                push_tag(&mut out, part);
            }
        }
    }
    out
}

/// Char válido em nome de tag (Unicode-aware): alfanumérico, `_`, `-`, `/` (nested).
fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '/'
}

/// Remove inline code spans (`` `…` ``) de uma linha, trocando por espaço (preserva fronteiras).
fn strip_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            out.push(' ');
        } else {
            out.push(if in_code { ' ' } else { c });
        }
    }
    out
}

/// PURO: tags inline `#tag` do corpo (já sem frontmatter/fenced). Exige `#` no início da linha ou após
/// espaço (exclui `url#frag` e `#` no meio de palavra), ignora inline code, aceita nested `#a/b`, e
/// rejeita puramente numérico (`#123`). Na ordem, sem repetir.
fn extract_inline_tags(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in body.lines() {
        let chars: Vec<char> = strip_inline_code(line).chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && is_tag_char(chars[j]) {
                    j += 1;
                }
                let name: String = chars[start..j].iter().collect();
                let trimmed = name.trim_end_matches('/');
                if !trimmed.is_empty()
                    && trimmed.chars().any(char::is_alphabetic)
                    && !out.iter().any(|x| x == trimmed)
                {
                    out.push(trimmed.to_string());
                }
                i = j.max(start);
            } else {
                i += 1;
            }
        }
    }
    out
}

/// PURO: a `NoteEntry` de um arquivo. Remove frontmatter + fenced code ANTES de extrair links/embeds/
/// tags inline (inv#1); funde as tags do frontmatter com as inline.
fn parse_note(rel_path: String, content: &str) -> NoteEntry {
    let (_, body_raw) = split_frontmatter(content);
    let body = strip_code(body_raw);
    let mut tags = extract_frontmatter_tags(content);
    for t in extract_inline_tags(&body) {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    NoteEntry {
        rel_path,
        headings: extract_headings(&body),
        links: extract_wikilinks(&body),
        embeds: extract_embeds(&body),
        tags,
        link_counts: link_count_pairs(&body),
        dataless: false,
    }
}

/// PURO: `true` se o `stat` descreve um placeholder de nuvem evicted (iCloud/OneDrive) — tem tamanho
/// LÓGICO (`len > 0`) mas ZERO blocos alocados no disco. Ler um desses dispara um download de rede;
/// pulá-los é o que evita baixar (e travar ao indexar) o vault inteiro. `len == 0` = arquivo vazio real.
#[must_use]
fn is_dataless_stat(blocks: u64, len: u64) -> bool {
    blocks == 0 && len > 0
}

/// `true` se o arquivo não tem dados locais (placeholder de nuvem). Unix: lê `st_blocks`/`st_size` por
/// `lstat` (local/barato — NÃO baixa o arquivo). Outros SOs: conservador (assume materializado); a
/// detecção on-demand do Windows/OneDrive fica como follow-up (ver achados de dogfooding).
fn file_is_dataless(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false; // não deu pra ler o stat → trata como local (não pula nada por engano)
    };
    #[cfg(unix)]
    let blocks = {
        use std::os::unix::fs::MetadataExt;
        meta.blocks()
    };
    // Sem `st_blocks` fora do Unix: finge "materializado" (não dispara o skip). A detecção on-demand
    // do Windows/OneDrive fica como follow-up (ver achados de dogfooding).
    #[cfg(not(unix))]
    let blocks: u64 = 1;
    is_dataless_stat(blocks, meta.len())
}

/// Lê+parseia UM `.md`, mas PULA o read (sem download) se `is_dataless` indicar placeholder de nuvem,
/// devolvendo uma nota marcada `dataless` (registrada no mapa, mas sem conteúdo indexado). O predicado
/// é injetado para provar o skip sem depender de um vault iCloud real.
fn parse_note_or_skip(rel: &str, path: &Path, is_dataless: impl Fn(&Path) -> bool) -> NoteEntry {
    if is_dataless(path) {
        return NoteEntry {
            rel_path: rel.to_string(),
            dataless: true,
            ..Default::default()
        };
    }
    parse_note(
        rel.to_string(),
        &std::fs::read_to_string(path).unwrap_or_default(),
    )
}

/// Varre o vault (recursivo, READ-ONLY, sem rede, sem LLM): coleta pastas e notas `*.md`, ignorando
/// `.obsidian/`, `.trash/` e qualquer dir oculto. Ordena tudo (determinístico).
#[must_use]
pub fn scan_vault(root: &Path) -> VaultIndexData {
    let mut folders = Vec::new();
    let mut md_paths: Vec<(String, PathBuf)> = Vec::new();
    // 1) Walk só de METADADOS (listar diretório fica local/rápido mesmo em iCloud) — coleta pastas e os
    //    caminhos dos `.md`, SEM ler conteúdo.
    walk_collect(root, root, &mut folders, &mut md_paths);
    folders.sort();
    folders.dedup();
    // 2) Lê+parseia os `.md` em PARALELO: o gargalo é I/O POR ARQUIVO — em vaults na nuvem (iCloud/
    //    OneDrive), cada `read_to_string` pode disparar um DOWNLOAD do arquivo evicted (~1s). Em série
    //    isso vira N×latência (um vault de 2.5k notas em iCloud levou ~50min!); com leituras concorrentes
    //    os downloads se sobrepõem e o tempo cai ~ordens de grandeza. É I/O-bound (espera de rede, não
    //    CPU) → usamos MAIS threads que cores. Determinístico: ordena por `rel_path` no fim.
    let mut notes = parse_notes_parallel(md_paths);
    notes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    VaultIndexData { folders, notes }
}

/// Walk recursivo de METADADOS: coleta pastas + caminhos dos `.md` (sem ler conteúdo). Ignora
/// `.obsidian/`, `.trash/` e qualquer dir oculto. A leitura pesada é paralelizada em [`scan_vault`].
fn walk_collect(
    root: &Path,
    dir: &Path,
    folders: &mut Vec<String>,
    md: &mut Vec<(String, PathBuf)>,
) {
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
            walk_collect(root, &path, folders, md);
        } else if path.extension().and_then(|x| x.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            md.push((rel, path));
        }
    }
}

/// Teto de threads de leitura. I/O-bound (cada thread fica PARADA esperando o download do iCloud), então
/// vale ter MUITO mais threads que cores — 64 sobrepõe bem os downloads sem custo real de CPU/RAM.
const SCAN_READ_THREADS: usize = 64;

/// Lê+parseia os `.md` em PARALELO (read+parse fora do lock; só o `push` do resultado é sincronizado).
/// Os workers puxam índices de um cursor atômico (work-stealing → balanceia mesmo com downloads de
/// duração desigual). Preserva o conteúdo de [`parse_note`]; a ORDEM é restaurada por `sort` no caller.
fn parse_notes_parallel(md: Vec<(String, PathBuf)>) -> Vec<NoteEntry> {
    if md.is_empty() {
        return Vec::new();
    }
    // `md` não é vazio (checado acima) → `len() >= 1`, então `min` já garante `workers >= 1`.
    let workers = md.len().min(SCAN_READ_THREADS);
    let cursor = AtomicUsize::new(0);
    let out: Mutex<Vec<NoteEntry>> = Mutex::new(Vec::with_capacity(md.len()));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                let Some((rel, path)) = md.get(i) else {
                    break; // cursor passou do fim → este worker terminou
                };
                // Placeholder de nuvem (iCloud/OneDrive evicted) → registra a nota SEM ler (não dispara
                // download). Só os arquivos materializados pagam o read+parse. É o que destrava vaults
                // grandes na nuvem: indexamos o que está local, sem baixar o vault inteiro de uma vez.
                let note = parse_note_or_skip(rel, path, file_is_dataless);
                if let Ok(mut g) = out.lock() {
                    g.push(note); // lock breve; o read+parse caro ficou fora dele
                }
            });
        }
    });
    out.into_inner().unwrap_or_default()
}

// ───────────────────────── grafo resolvido (métricas determinísticas, inv#1) ─────────────────────────

/// Limiar de "hub" (in-degree) — heurístico, ajustável por vault (ver caveats da pesquisa).
const HUB_MIN_INDEGREE: usize = 15;
/// Limiar de "ponto de entrada" (out-degree) — heurístico: nota que "leva a muitos lugares".
const ENTRY_POINT_MIN_OUTDEGREE: usize = 5;

/// Por que um link de saída NÃO virou aresta (sinalizado, nunca adivinhado — regra do PageIndex).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkIssue {
    /// O alvo casa >1 nota e o caminho não desambigua (Obsidian usaria shortest-path; nós sinalizamos).
    Ambiguous,
    /// O alvo não casa nenhuma nota (link pendente: nota ainda não criada — comum e legítimo).
    Unresolved,
}

/// Uma aresta resolvida do grafo (origem→destino por `rel_path`) com a multiplicidade das citações.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub count: usize,
}

/// Um link de saída que não virou aresta (ambíguo/pendente) — sinalizado p/ o assistente resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkProblem {
    pub from: String,
    pub target: String,
    pub issue: LinkIssue,
}

/// Métrica + classificação de uma nota no grafo resolvido (graus distintos, não soma de citações).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMetrics {
    pub rel_path: String,
    pub out_degree: usize,
    pub in_degree: usize,
    pub is_hub: bool,
    pub is_moc: bool,
    pub is_orphan: bool,
    pub is_entry_point: bool,
}

/// O grafo resolvido de um vault (PURO, determinístico): métricas por nota + arestas + problemas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultGraph {
    pub metrics: Vec<NoteMetrics>,
    pub edges: Vec<GraphEdge>,
    pub problems: Vec<LinkProblem>,
}

/// Chave de resolução/identidade: basename (último segmento após `/`), sem `.md`, lowercased.
fn basename_key(s: &str) -> String {
    let base = s.rsplit('/').next().unwrap_or(s);
    base.strip_suffix(".md")
        .unwrap_or(base)
        .to_ascii_lowercase()
}

/// Resolução de um alvo de wikilink contra o índice `basename → [rel_path]`.
enum Resolved {
    One(String),
    None,
    Many,
}

/// Resolve por basename lowercased; se ambíguo, tenta o caminho parcial do alvo (`Pasta/Nota`); senão
/// sinaliza ambíguo (não adivinha — regra do PageIndex). `.md` é opcional no alvo.
fn resolve_target(target: &str, by_base: &BTreeMap<String, Vec<String>>) -> Resolved {
    match by_base.get(&basename_key(target)) {
        None => Resolved::None,
        Some(paths) if paths.len() == 1 => Resolved::One(paths[0].clone()),
        Some(paths) => {
            let want = target.trim_start_matches('/').to_ascii_lowercase();
            let want_md = if want.ends_with(".md") {
                want
            } else {
                format!("{want}.md")
            };
            let suffix = format!("/{want_md}");
            let mut hit = None;
            let mut hits = 0usize;
            for p in paths {
                let pl = p.to_ascii_lowercase();
                if pl == want_md || pl.ends_with(&suffix) {
                    hits += 1;
                    hit = Some(p.clone());
                }
            }
            match (hits, hit) {
                (1, Some(p)) => Resolved::One(p),
                _ => Resolved::Many,
            }
        }
    }
}

/// `true` se `needle` aparece em `haystack` como TOKEN delimitado (fronteiras não-alfanuméricas) — não
/// embutido numa palavra maior. Evita falso-positivo de `moc` em "democracia"/"mockup".
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        let i = start + rel;
        let before_ok = haystack[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = i + needle.len();
        let after_ok = haystack[after..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = i + 1;
    }
    false
}

/// `true` se a nota é um "Map of Content": basename casa `moc` (como token, p/ não pegar "democracia")
/// ou `maps? of content` (espaços flexíveis) OU tem a tag `#moc`. Case-insensitive. Heurístico
/// (ajustável) — ver caveats da pesquisa.
fn is_moc(rel_path: &str, tags: &[String]) -> bool {
    let base = basename_key(rel_path); // já lowercased
    if contains_word(&base, "moc") {
        return true;
    }
    let collapsed: String = base.chars().filter(|c| !c.is_whitespace()).collect();
    collapsed.contains("mapofcontent")
        || collapsed.contains("mapsofcontent")
        || tags.iter().any(|t| t.eq_ignore_ascii_case("moc"))
}

/// Agrega arestas com mesmo `(from, to)` somando `count` (ordena por `from`, depois `to`).
fn merge_edges(mut edges: Vec<GraphEdge>) -> Vec<GraphEdge> {
    edges.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.to.cmp(&b.to)));
    let mut out: Vec<GraphEdge> = Vec::new();
    for e in edges {
        match out.last_mut() {
            Some(last) if last.from == e.from && last.to == e.to => last.count += e.count,
            _ => out.push(e),
        }
    }
    out
}

/// PURO (inv#1, ZERO LLM): resolve o grafo do vault. Constrói o índice por basename, resolve cada link
/// de saída (com peso) em aresta ou problema, e deriva graus + classificação (hub/MOC/órfão/entry).
/// Tudo ordenado (determinístico). Backlinks = transposta implícita (in-degree = origens distintas).
#[must_use]
pub fn analyze_graph(data: &VaultIndexData) -> VaultGraph {
    let mut by_base: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for note in &data.notes {
        by_base
            .entry(basename_key(&note.rel_path))
            .or_default()
            .push(note.rel_path.clone());
    }

    let mut raw_edges: Vec<GraphEdge> = Vec::new();
    let mut problems: Vec<LinkProblem> = Vec::new();
    for note in &data.notes {
        for (target, count) in &note.link_counts {
            match resolve_target(target, &by_base) {
                Resolved::One(to) if to == note.rel_path => {} // ignora auto-link
                Resolved::One(to) => {
                    raw_edges.push(GraphEdge {
                        from: note.rel_path.clone(),
                        to,
                        count: *count,
                    });
                }
                Resolved::None => problems.push(LinkProblem {
                    from: note.rel_path.clone(),
                    target: target.clone(),
                    issue: LinkIssue::Unresolved,
                }),
                Resolved::Many => problems.push(LinkProblem {
                    from: note.rel_path.clone(),
                    target: target.clone(),
                    issue: LinkIssue::Ambiguous,
                }),
            }
        }
    }

    let edges = merge_edges(raw_edges);
    // Após o merge cada (from,to) é único → contar arestas por nó = graus distintos.
    let mut out_deg: BTreeMap<&str, usize> = BTreeMap::new();
    let mut in_deg: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &edges {
        *out_deg.entry(e.from.as_str()).or_default() += 1;
        *in_deg.entry(e.to.as_str()).or_default() += 1;
    }

    let mut metrics: Vec<NoteMetrics> = data
        .notes
        .iter()
        .map(|note| {
            let od = out_deg.get(note.rel_path.as_str()).copied().unwrap_or(0);
            let id = in_deg.get(note.rel_path.as_str()).copied().unwrap_or(0);
            NoteMetrics {
                rel_path: note.rel_path.clone(),
                out_degree: od,
                in_degree: id,
                is_hub: id >= HUB_MIN_INDEGREE,
                is_moc: is_moc(&note.rel_path, &note.tags),
                is_orphan: od == 0 && id == 0,
                is_entry_point: od >= ENTRY_POINT_MIN_OUTDEGREE,
            }
        })
        .collect();
    metrics.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    problems.sort_by(|a, b| {
        a.from
            .cmp(&b.from)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| (a.issue as u8).cmp(&(b.issue as u8)))
    });
    problems.dedup();

    VaultGraph {
        metrics,
        edges,
        problems,
    }
}

/// `rel_path`s das notas que casam `pred`, na ordem de `metrics` (já ordenado por `rel_path`).
fn rel_paths_where(metrics: &[NoteMetrics], pred: impl Fn(&NoteMetrics) -> bool) -> Vec<String> {
    metrics
        .iter()
        .filter(|m| pred(m))
        .map(|m| m.rel_path.clone())
        .collect()
}

/// `node` do grafo = `rel_path` sem `.md` (rótulo curto, navegável).
fn node_label(rel_path: &str) -> &str {
    rel_path.strip_suffix(".md").unwrap_or(rel_path)
}

/// PURO: o markdown-árvore HÍBRIDO (blueprint §3) com o grafo JÁ resolvido. Mantém intactas as seções
/// Pastas / Notas / "Grafo de [[wikilinks]]" e ACRESCENTA navegação (entry-points, hubs, MOCs, órfãos,
/// backlinks) + grau/tags/embeds por nota + links pendentes/ambíguos. Token-eficiente e navegável.
#[must_use]
fn render_vault_index_with(
    name: &str,
    root: &Path,
    data: &VaultIndexData,
    graph: &VaultGraph,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Vault Index — {name}  (gerado pelo Lina · determinístico · sem IA · NÃO editar)\n"
    ));
    out.push_str(&format!("> Origem: {}\n", root.display()));
    let hubs = graph.metrics.iter().filter(|m| m.is_hub).count();
    let mocs = graph.metrics.iter().filter(|m| m.is_moc).count();
    let orphans = graph.metrics.iter().filter(|m| m.is_orphan).count();
    let cloud = data.notes.iter().filter(|n| n.dataless).count();
    let indexed = data.notes.len() - cloud;
    out.push_str(&format!(
        "> {} notas · {} conexões · {hubs} hubs · {mocs} MOCs · {orphans} órfãos\n",
        data.notes.len(),
        graph.edges.len()
    ));
    if cloud > 0 {
        out.push_str(&format!(
            "> ⚠ {indexed} indexadas · {cloud} ainda na nuvem (não baixadas) — abra-as no Obsidian \
             (ou baixe o vault) para incluí-las no mapa.\n"
        ));
    }
    out.push('\n');

    out.push_str("## Pastas\n");
    if data.folders.is_empty() {
        out.push_str("- (nenhuma subpasta)\n");
    } else {
        for f in &data.folders {
            out.push_str(&format!("- {f}\n"));
        }
    }

    // Pontos de entrada (por out-degree desc) e hubs (por in-degree desc) — "por onde começar".
    let mut entry: Vec<&NoteMetrics> = graph.metrics.iter().filter(|m| m.is_entry_point).collect();
    entry.sort_by(|a, b| {
        b.out_degree
            .cmp(&a.out_degree)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    out.push_str("\n## Pontos de entrada (por onde começar)\n");
    if entry.is_empty() {
        out.push_str("- (nenhum)\n");
    } else {
        for m in &entry {
            out.push_str(&format!(
                "- {} (saída: {})\n",
                node_label(&m.rel_path),
                m.out_degree
            ));
        }
    }

    let mut hub_list: Vec<&NoteMetrics> = graph.metrics.iter().filter(|m| m.is_hub).collect();
    hub_list.sort_by(|a, b| {
        b.in_degree
            .cmp(&a.in_degree)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    out.push_str("\n## Hubs (muito referenciados · entrada ≥ 15)\n");
    if hub_list.is_empty() {
        out.push_str("- (nenhum)\n");
    } else {
        for m in &hub_list {
            out.push_str(&format!(
                "- {} (entrada: {})\n",
                node_label(&m.rel_path),
                m.in_degree
            ));
        }
    }

    out.push_str("\n## MOCs (mapas de conteúdo)\n");
    let mocs_list = rel_paths_where(&graph.metrics, |m| m.is_moc);
    if mocs_list.is_empty() {
        out.push_str("- (nenhum)\n");
    } else {
        for p in &mocs_list {
            out.push_str(&format!("- {}\n", node_label(p)));
        }
    }

    out.push_str("\n## Órfãos (sem conexões)\n");
    let orphan_list = rel_paths_where(&graph.metrics, |m| m.is_orphan);
    if orphan_list.is_empty() {
        out.push_str("- (nenhum)\n");
    } else {
        for p in &orphan_list {
            out.push_str(&format!("- {}\n", node_label(p)));
        }
    }

    // Notas + headings, enriquecidas com grau/flags/tags/embeds (sem alterar as linhas de heading).
    let metrics_by_path: BTreeMap<&str, &NoteMetrics> = graph
        .metrics
        .iter()
        .map(|m| (m.rel_path.as_str(), m))
        .collect();
    out.push_str("\n## Notas (headings)\n");
    if data.notes.is_empty() {
        out.push_str("- (nenhuma nota)\n");
    } else {
        for note in &data.notes {
            out.push_str(&format!("### {}\n", note.rel_path));
            if let Some(m) = metrics_by_path.get(note.rel_path.as_str()) {
                let mut flags: Vec<&str> = Vec::new();
                if m.is_entry_point {
                    flags.push("entrada");
                }
                if m.is_hub {
                    flags.push("hub");
                }
                if m.is_moc {
                    flags.push("MOC");
                }
                if m.is_orphan {
                    flags.push("órfão");
                }
                let tail = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", flags.join(", "))
                };
                out.push_str(&format!(
                    "- _saída: {} · entrada: {}{}_\n",
                    m.out_degree, m.in_degree, tail
                ));
            }
            if !note.tags.is_empty() {
                let ts: Vec<String> = note.tags.iter().map(|t| format!("#{t}")).collect();
                out.push_str(&format!("- _tags: {}_\n", ts.join(" ")));
            }
            if !note.embeds.is_empty() {
                let es: Vec<String> = note.embeds.iter().map(|e| format!("![[{e}]]")).collect();
                out.push_str(&format!("- _embeds: {}_\n", es.join(", ")));
            }
            if note.headings.is_empty() {
                out.push_str("- (sem headings)\n");
            } else {
                for h in &note.headings {
                    out.push_str(&format!("- {h}\n"));
                }
            }
        }
    }

    // Grafo de saída — formato INALTERADO (contrato dos consumidores + testes).
    out.push_str("\n## Grafo de [[wikilinks]]\n");
    if data.notes.is_empty() {
        out.push_str("- (nenhuma nota)\n");
    } else {
        for note in &data.notes {
            let node = node_label(&note.rel_path);
            if note.links.is_empty() {
                out.push_str(&format!("- {node} → (folha)\n"));
            } else {
                let links: Vec<String> = note.links.iter().map(|l| format!("[[{l}]]")).collect();
                out.push_str(&format!("- {node} → {}\n", links.join(", ")));
            }
        }
    }

    // Backlinks (transposta do grafo) — quem aponta pra cada nota. Invertemos o grafo UMA vez (O(e))
    // em vez de varrer TODAS as arestas por nota — antes era O(notas × arestas), que explodia em vaults
    // grandes (2.5k notas × 3k arestas ≈ 8M comparações de path por render). `edges` já vem ordenado
    // por (from,to), então a ordem de origens por destino é determinística.
    out.push_str("\n## Backlinks (quem aponta pra cá)\n");
    let mut backlinks: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &graph.edges {
        backlinks
            .entry(e.to.as_str())
            .or_default()
            .push(e.from.as_str());
    }
    let mut any_back = false;
    for note in &data.notes {
        if let Some(origins) = backlinks.get(note.rel_path.as_str()) {
            any_back = true;
            let joined = origins
                .iter()
                .map(|f| format!("[[{}]]", node_label(f)))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- {} ← {}\n", node_label(&note.rel_path), joined));
        }
    }
    if !any_back {
        out.push_str("- (nenhum backlink ainda)\n");
    }

    // Links que NÃO viraram aresta (sinalizados, nunca adivinhados).
    out.push_str("\n## Links pendentes/ambíguos\n");
    if graph.problems.is_empty() {
        out.push_str("- (nenhum)\n");
    } else {
        for p in &graph.problems {
            let kind = match p.issue {
                LinkIssue::Unresolved => "pendente (nota ainda não existe)",
                LinkIssue::Ambiguous => "ambíguo (casa >1 nota — não adivinho)",
            };
            out.push_str(&format!(
                "- {} → [[{}]] — {kind}\n",
                node_label(&p.from),
                p.target
            ));
        }
    }

    out
}

// ───────────────────────── sidecar JSON (arestas exatas p/ hops programáticos) ─────────────────────────

/// Aresta no sidecar JSON (origem→destino por `rel_path`, com a multiplicidade).
#[derive(Serialize)]
struct EdgeJson {
    from: String,
    to: String,
    count: usize,
}

/// Link pendente/ambíguo no sidecar JSON.
#[derive(Serialize)]
struct ProblemJson {
    from: String,
    target: String,
    issue: &'static str,
}

/// Métrica por nota no sidecar JSON.
#[derive(Serialize)]
struct NoteJson {
    path: String,
    out_degree: usize,
    in_degree: usize,
    tags: Vec<String>,
    embeds: Vec<String>,
}

/// O documento JSON inteiro (campos em ordem fixa → saída determinística).
#[derive(Serialize)]
struct IndexJson {
    vault: String,
    root: String,
    generated_by: &'static str,
    deterministic: bool,
    note_count: usize,
    edge_count: usize,
    hubs: Vec<String>,
    mocs: Vec<String>,
    orphans: Vec<String>,
    entry_points: Vec<String>,
    edges: Vec<EdgeJson>,
    problems: Vec<ProblemJson>,
    notes: Vec<NoteJson>,
}

/// PURO: sidecar JSON com as ARESTAS EXATAS (origem→destino, contagem) + categorias + grau/tags/embeds
/// por nota — para hops programáticos depois que o assistente escolhe a região no markdown. Determinístico.
#[must_use]
fn render_vault_index_json(
    name: &str,
    root: &Path,
    data: &VaultIndexData,
    graph: &VaultGraph,
) -> String {
    let metrics_by_path: BTreeMap<&str, &NoteMetrics> = graph
        .metrics
        .iter()
        .map(|m| (m.rel_path.as_str(), m))
        .collect();
    let notes = data
        .notes
        .iter()
        .map(|n| {
            let m = metrics_by_path.get(n.rel_path.as_str());
            NoteJson {
                path: n.rel_path.clone(),
                out_degree: m.map_or(0, |x| x.out_degree),
                in_degree: m.map_or(0, |x| x.in_degree),
                tags: n.tags.clone(),
                embeds: n.embeds.clone(),
            }
        })
        .collect();
    let doc = IndexJson {
        vault: name.to_string(),
        root: root.display().to_string(),
        generated_by: "lina-pageindex",
        deterministic: true,
        note_count: data.notes.len(),
        edge_count: graph.edges.len(),
        hubs: rel_paths_where(&graph.metrics, |m| m.is_hub),
        mocs: rel_paths_where(&graph.metrics, |m| m.is_moc),
        orphans: rel_paths_where(&graph.metrics, |m| m.is_orphan),
        entry_points: rel_paths_where(&graph.metrics, |m| m.is_entry_point),
        edges: graph
            .edges
            .iter()
            .map(|e| EdgeJson {
                from: e.from.clone(),
                to: e.to.clone(),
                count: e.count,
            })
            .collect(),
        problems: graph
            .problems
            .iter()
            .map(|p| ProblemJson {
                from: p.from.clone(),
                target: p.target.clone(),
                issue: match p.issue {
                    LinkIssue::Ambiguous => "ambiguous",
                    LinkIssue::Unresolved => "unresolved",
                },
            })
            .collect(),
        notes,
    };
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// Slug ASCII do nome do vault (minúsculo, não-alfanumérico → `-`, sem `-` repetido/nas pontas).
fn slugify(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
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

/// Base do nome dos arquivos de índice: `<slug>-<hash do caminho>` (sem extensão) — o hash desambigua
/// vaults de mesmo nome em pastas diferentes (determinístico: `DefaultHasher` tem semente fixa). O
/// markdown vira `<base>.md` e o sidecar de arestas `<base>.json`.
fn index_filename_base(name: &str, path: &Path) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!(
        "{}-{:08x}",
        slugify(name),
        (h.finish() & 0xffff_ffff) as u32
    )
}

/// Gera e grava o índice HÍBRIDO de `vault` em `<lina_dir>/vault-index/` (FORA do vault do usuário —
/// respeita "leitura por padrão"): `<base>.md` (markdown-árvore navegável) + `<base>.json` (arestas
/// exatas p/ hops programáticos). Escrita atômica de cada um. Devolve o caminho do markdown.
pub fn write_vault_index(lina_dir: &Path, vault: &VaultLink) -> std::io::Result<PathBuf> {
    let data = scan_vault(&vault.path);
    let graph = analyze_graph(&data);
    let base = index_filename_base(&vault.name, &vault.path);
    let dir = lina_dir.join("vault-index");
    let md_path = dir.join(format!("{base}.md"));
    let json_path = dir.join(format!("{base}.json"));
    write_atomic(
        &md_path,
        &render_vault_index_with(&vault.name, &vault.path, &data, &graph),
    )?;
    write_atomic(
        &json_path,
        &render_vault_index_json(&vault.name, &vault.path, &data, &graph),
    )?;
    Ok(md_path)
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

/// Lê o `.lina/vault.json` COMPLETO (todos os vaults conectados). `None` se ausente/inválido.
#[must_use]
pub fn read_vault_config(lina_dir: &Path) -> Option<VaultConfig> {
    let s = std::fs::read_to_string(lina_dir.join("vault.json")).ok()?;
    serde_json::from_str(&s).ok()
}

// ───────────────────────── ADR 0056 — vault é config do USUÁRIO (global `~/.lina/`) ─────────────────────────

/// O `~/.lina` GLOBAL do usuário — a casa da config-de-usuário (par de `workspaces.json`/`license.json`).
/// O vault é config do USUÁRIO (o segundo cérebro é dele, não do projeto), então mora aqui e é herdado por
/// todo Espaço (ADR 0056). `None` se `HOME`/`USERPROFILE` não resolvem.
#[must_use]
pub fn global_lina_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|s| !s.is_empty())
        .map(|h| PathBuf::from(h).join(".lina"))
}

/// `[<ws>/.lina, ~/.lina]` — a ordem de precedência do ADR 0056 (o global só entra se `HOME` resolver e
/// não for o próprio `ws`). Override de projeto primeiro, global herdado depois.
fn layered_dirs(ws_lina_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![ws_lina_dir.to_path_buf()];
    if let Some(g) = global_lina_dir() {
        if g != ws_lina_dir {
            dirs.push(g);
        }
    }
    dirs
}

/// PURO: o `primary` do 1º `lina_dir` (em ordem de precedência) que tiver `vault.json` válido.
#[must_use]
pub fn read_primary_vault_layered(dirs: &[PathBuf]) -> Option<String> {
    dirs.iter().find_map(|d| read_primary_vault(d))
}

/// Precedência efetiva de PRODUÇÃO: `<ws>/.lina` (override) → `~/.lina` (global). Usada pelo boot do app
/// e pela regeneração da doutrina.
#[must_use]
pub fn read_primary_vault_effective(ws_lina_dir: &Path) -> Option<String> {
    read_primary_vault_layered(&layered_dirs(ws_lina_dir))
}

/// Uma entrada de `~/.lina/workspaces.json` — só o `path` interessa (campos extras ignorados).
#[derive(Deserialize)]
struct WorkspaceRegEntry {
    #[serde(default)]
    path: String,
}
#[derive(Deserialize)]
struct WorkspaceReg {
    #[serde(default)]
    workspaces: Vec<WorkspaceRegEntry>,
}

/// Os `.lina/` dos Espaços conhecidos (lidos de `<global>/workspaces.json`) — candidatos a fonte da
/// migração. Vazio se o registro não existe / é inválido. Varrer TODOS (não só o Espaço atual) é o que
/// faz um Espaço novo, aberto direto, achar o vault que o usuário linkou num Espaço anterior.
fn known_espaco_lina_dirs(global: &Path) -> Vec<PathBuf> {
    let Ok(s) = std::fs::read_to_string(global.join("workspaces.json")) else {
        return Vec::new();
    };
    let Ok(reg) = serde_json::from_str::<WorkspaceReg>(&s) else {
        return Vec::new();
    };
    reg.workspaces
        .into_iter()
        .filter(|e| !e.path.is_empty())
        .map(|e| PathBuf::from(e.path).join(".lina"))
        .collect()
}

/// Copia 1 arquivo (cria o pai). Best-effort no caller.
fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::copy(src, dst).map(|_| ())
}

/// Copia os ARQUIVOS de `src` em `dst` (raso — o `vault-index` é plano). No-op se `src` não existe.
fn copy_dir_shallow(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// **Migração one-shot p/ o vault global (ADR 0056), TESTÁVEL** (dirs explícitos). Promove o `vault.json`
/// (+ `vault-index/`) do 1º `candidate` que o tiver para `global` — SÓ se o global ainda não tem vault.
/// Idempotente: global já populado → no-op (nunca sobrescreve a escolha viva do usuário).
pub fn migrate_vault_to_global_from(candidates: &[PathBuf], global: &Path) {
    if read_primary_vault(global).is_some() {
        return; // já migrado
    }
    let Some(src) = candidates.iter().find(|d| read_primary_vault(d).is_some()) else {
        return; // nenhum Espaço tem vault linkado ainda
    };
    if let Err(e) = copy_file(&src.join("vault.json"), &global.join("vault.json")) {
        eprintln!("obsidian: migração do vault p/ global falhou (vault.json): {e}");
        return;
    }
    // o índice é best-effort: o self-heal regenera o que faltar no próximo boot.
    if let Err(e) = copy_dir_shallow(&src.join("vault-index"), &global.join("vault-index")) {
        eprintln!(
            "obsidian: migração do índice p/ global incompleta ({e}) — o self-heal regenera."
        );
    }
    eprintln!(
        "obsidian: vault promovido p/ ~/.lina (global) a partir de {}",
        src.display()
    );
}

/// PRODUÇÃO: resolve o global + os Espaços de `workspaces.json` e migra (one-shot, idempotente).
pub fn migrate_vault_to_global() {
    let Some(global) = global_lina_dir() else {
        return;
    };
    let candidates = known_espaco_lina_dirs(&global);
    migrate_vault_to_global_from(&candidates, &global);
}

/// PURO: lista os vaults conectados cujo índice (PageIndex) está FALTANDO em `<lina_dir>/vault-index/`.
/// Testável sem I/O de scan — só checa a existência do sidecar `.json` de cada vault do config.
#[must_use]
pub fn vaults_missing_index(lina_dir: &Path, cfg: &VaultConfig) -> Vec<VaultLink> {
    let idx_dir = lina_dir.join("vault-index");
    cfg.vaults
        .iter()
        .filter(|v| {
            let base = index_filename_base(&v.name, Path::new(&v.path));
            !idx_dir.join(format!("{base}.json")).exists()
        })
        .map(|v| VaultLink {
            name: v.name.clone(),
            path: PathBuf::from(&v.path),
            open: v.path == cfg.primary,
            added_manually: false,
        })
        .collect()
}

/// **Self-heal do segundo cérebro (boot).** Se `vault.json` existe (vault conectado) mas o índice de
/// algum vault está FALTANDO, regenera-o FORA da thread de UI (detached, best-effort). Cobre o caso real
/// em que a thread fire-and-forget do onboarding morreu antes de gravar (janela fechada cedo, permissão
/// de Documentos/TCC negada na 1ª execução, crash) — sem isso, o usuário não-técnico fica com o segundo
/// cérebro "conectado mas vazio" e SEM caminho de volta (o onboarding é one-shot). No-op se não há
/// `vault.json` ou se todos os índices já existem (não re-escaneia à toa). Falha de permissão é LOGADA
/// alto (não engolida em silêncio) — o sintoma deixa de ser invisível.
pub fn heal_missing_indices(lina_dir: &Path) {
    let Some(cfg) = read_vault_config(lina_dir) else {
        return;
    };
    let missing = vaults_missing_index(lina_dir, &cfg);
    if missing.is_empty() {
        return;
    }
    let lina_dir = lina_dir.to_path_buf();
    thread::spawn(move || {
        for v in &missing {
            match write_vault_index(&lina_dir, v) {
                Ok(p) => eprintln!(
                    "obsidian: self-heal regenerou o índice de '{}' → {}",
                    v.name,
                    p.display()
                ),
                Err(e) => eprintln!(
                    "obsidian: self-heal NÃO conseguiu indexar '{}' ({e}) — provável falta de \
                     permissão de acesso à pasta (Documentos/TCC) ou vault movido.",
                    v.name
                ),
            }
        }
    });
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
        self.install
            .lock()
            .map(|g| g.clone())
            .unwrap_or(InstallState::Idle)
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
        for v in self
            .scan()
            .vaults
            .into_iter()
            .chain(self.manual.iter().cloned())
        {
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
        self.all_vaults()
            .iter()
            .filter(|v| self.is_selected(&v.path))
            .count()
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
        set_install(
            &self.install,
            InstallState::Installing {
                line: "iniciando…".into(),
            },
        );
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
        let chosen: Vec<VaultLink> = self
            .all_vaults()
            .into_iter()
            .filter(|v| self.is_selected(&v.path))
            .collect();
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
        let config = VaultConfig {
            primary,
            vaults: entries,
        };
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
            th().surface.success_muted,
            th().state.success,
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
            th().surface.panel,
            th().accent.primary,
            "🛡️ A Lina nunca apaga nem altera suas anotações. Ela só escreve numa pastinha nova \
             chamada \"Lina\", que cria dentro da pasta que você escolher. No resto, NUNCA mexe.",
        ));
        col = col.child(div().text_color(rgb(th().text.muted)).child(text!(
            "💾 Salvo automaticamente. Pode fechar e voltar quando quiser — nada se perde."
        )));

        // Rodapé (UX §5): Voltar · Verificar · slot primário (rótulo concordante com o banner).
        col = col.child(self.footer(screen, count, cx));
        col.into_any_element()
    }

    /// Banner de estado (UX §4) — cor + ícone + texto; o ícone/texto carregam o significado.
    fn banner(&self, screen: Screen, count: usize) -> AnyElement {
        match screen {
            Screen::Searching => banner(
                th().surface.panel,
                th().accent.primary,
                "🔵 Procurando o Obsidian no seu computador… Isso costuma levar uns segundos.",
            ),
            Screen::NotInstalled => banner(
                th().surface.danger_muted,
                th().state.warning,
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
                    th().surface.success_muted,
                    th().state.success,
                    &format!(
                        "🟢 Achei o Obsidian e encontrei {pastas}. Marque quais você quer que a Lina \
                         use pra te ajudar."
                    ),
                )
            }
            Screen::NoVaults => banner(
                th().surface.danger_muted,
                th().state.warning,
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
                    th().surface.success_muted,
                    th().state.success,
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
            .child(
                div()
                    .text_color(rgb(th().text.muted))
                    .child(text!("⟳ Procurando o app Obsidian … aguarde")),
            )
            .child(
                div()
                    .text_color(rgb(th().text.muted))
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
        let mut col =
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().text_color(rgb(th().text.primary)).child(text!(
            "Seu caderno de anotações vira a memória da Lina — ela aprende com o que você escreve \
                 e te ajuda melhor."
        )));

        if installing {
            let line = match &install {
                InstallState::Installing { line } => line.clone(),
                _ => "verificando…".to_string(),
            };
            col = col.child(banner(
                th().surface.panel,
                th().state.warning,
                &format!("⟳ Instalando o Obsidian: {line}"),
            ));
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
                        th().accent.action,
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
            col = col.child(banner(
                th().surface.danger_muted,
                th().state.warning,
                &format!("⚠ {reason}"),
            ));
        }
        col.into_any_element()
    }

    /// Estado 3 — multi-seleção das pastas (sinais redundantes: ☑/☐ + ✓ + texto "Vai ser usada").
    fn body_with_vaults(&self, cx: &mut Context<OnboardingView>, count: usize) -> AnyElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_color(rgb(th().text.primary)).child(text!(
                "O que a Lina faz: ela LÊ suas anotações pra te entender — como uma leitura. E só \
                 ESCREVE numa pastinha nova \"Lina\" que cria dentro da sua pasta. No resto, NUNCA mexe."
            )))
            .child(div().text_color(rgb(th().text.muted)).child(text!(
                "Marque as pastas de anotações (no Obsidian, essas pastas são chamadas de \"vault\"):"
            )))
            .child(
                div()
                    .text_color(rgb(th().text.muted))
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
            col = col.child(banner(
                th().surface.panel,
                th().text.muted,
                "Confirmar (nenhuma pasta marcada)",
            ));
        } else {
            let label = if count == 1 {
                "✓ Confirmar 1 pasta para a Lina".to_string()
            } else {
                format!("✓ Confirmar {count} pastas para a Lina")
            };
            // Confirmar GRAVA (vault.json + índice) E AVANÇA o passo — sem isto o usuário confirmava e
            // ficava "preso" tendo que achar o "Continuar →" no rodapé (que ficava cortado). O rodapé
            // segue existindo p/ quem quer pular sem confirmar.
            col = col.child(action_button(
                "sb-confirm",
                &label,
                th().accent.confirm,
                cx,
                |onb, _w, cx| {
                    onb.second_brain.confirm();
                    onb.nav_continue();
                    cx.notify();
                },
            ));
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
            .rounded_content()
            .bg(rgb(if selected {
                th().surface.selected_row
            } else {
                th().surface.panel
            }))
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
                            .text_color(rgb(th().text.primary))
                            .font_weight(FontWeight::BOLD)
                            .child(text!(name_line)),
                    )
                    .child(
                        div()
                            .text_color(rgb(if selected {
                                th().state.success
                            } else {
                                th().text.muted
                            }))
                            .child(text!(status)),
                    ),
            )
            .child(
                div()
                    .text_color(rgb(th().text.muted))
                    .child(text!(v.path.display().to_string())),
            )
            .into_any_element()
    }

    /// Estado 4 — sem pastas: apontar uma existente (seletor) ou criar no Obsidian.
    fn body_no_vaults(&self, cx: &mut Context<OnboardingView>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_color(rgb(th().state.success))
                    .child(text!("● Obsidian (seu caderno de anotações) … encontrado")),
            )
            .child(
                div()
                    .text_color(rgb(th().state.warning))
                    .child(text!("● Pastas de anotações … nenhuma ainda")),
            )
            .child(div().text_color(rgb(th().text.primary)).child(text!(
                "① Apontar uma pasta que você já tem — se você já guarda anotações numa pasta do \
                 computador, é só mostrar ela pra Lina."
            )))
            .child(add_folder_button("sb-pick", "＋ Escolher uma pasta…", cx))
            .child(div().text_color(rgb(th().text.primary)).child(text!(
                "② Criar uma pasta no Obsidian — abra o Obsidian, clique em \"Create new vault\" \
                 (criar nova pasta de anotações) e depois volte aqui."
            )))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(ghost_button(
                        "sb-open-app",
                        "↗ Abrir o Obsidian",
                        cx,
                        |_onb, _w, cx| {
                            if let Some(p) = find_app_bundle(OBSIDIAN_APP) {
                                cx.open_with_system(&p);
                            }
                        },
                    ))
                    .child(ghost_button(
                        "sb-recheck",
                        "⟳ Já criei — procurar de novo",
                        cx,
                        |onb, _w, cx| {
                            onb.second_brain.verify_now();
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }

    /// Estado 5 — confirmado: recap das pastas + limite reafirmado + reversibilidade.
    fn body_confirmed(&self, cx: &mut Context<OnboardingView>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_2();
        for (i, v) in self
            .all_vaults()
            .into_iter()
            .filter(|v| self.is_selected(&v.path))
            .enumerate()
        {
            list = list.child(
                div()
                    .id(("sb-recap", i))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .rounded_content()
                    .bg(rgb(th().surface.panel))
                    .child(
                        div()
                            .text_color(rgb(th().state.success))
                            .font_weight(FontWeight::BOLD)
                            .child(text!(format!("✓ {}", v.name))),
                    )
                    .child(
                        div()
                            .text_color(rgb(th().text.muted))
                            .child(text!(format!("{}  ·  Vai ser usada", v.path.display()))),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_color(rgb(th().text.primary)).child(text!("O que você acabou de combinar com a Lina:")))
            .child(list)
            .child(div().text_color(rgb(th().text.muted)).child(text!(
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
                th().surface.panel,
                th().accent.primary,
                "Abri uma janela de Terminal para instalar o Obsidian. Ela pode pedir a senha do seu \
                 Mac — é normal e seguro. Quando terminar, volte e clique \"Verificar\".",
            ),
            InstallNotice::UseWebsite => banner(
                th().surface.danger_muted,
                th().state.warning,
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
            .child(ghost_button("sb-back", "← Voltar", cx, |onb, _w, _cx| {
                onb.nav_back()
            }));

        // [Verificar]: oculto no estado 5; ativo nos demais (re-roda a detecção preservando marcações).
        if screen != Screen::Confirmed {
            row = row.child(ghost_button(
                "sb-verify",
                "⟳ Verificar",
                cx,
                |onb, _w, cx| {
                    onb.second_brain.verify_now();
                    cx.notify();
                },
            ));
        }
        row = row.child(div().flex_1());

        // Slot primário: rótulo derivado do estado (footer_label) — concorda com o banner.
        let label = footer_label(screen, count);
        let confirm_and_advance = screen == Screen::WithVaults && count > 0;
        row = row.child(action_button(
            "sb-primary",
            &label,
            th().accent.confirm,
            cx,
            move |onb, _w, _cx| {
                if confirm_and_advance {
                    onb.second_brain.confirm();
                } else if screen != Screen::Confirmed {
                    onb.second_brain.skip();
                }
                onb.nav_continue();
            },
        ));
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
                .text_color(rgb(th().text.primary))
                .child(text!(title.to_string())),
        )
        .child(
            div()
                .text_size(px(15.0))
                .text_color(rgb(th().text.muted))
                .child(text!(subtitle.to_string())),
        )
        .into_any_element()
}

/// Banner de uma linha (cor de fundo + cor de texto + mensagem). Cor NUNCA sozinha — sempre texto.
fn banner(bg: u32, fg: u32, msg: &str) -> AnyElement {
    div()
        .px_4()
        .py_3()
        .rounded_content()
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
        .rounded_content()
        .bg(rgb(bg))
        .text_color(rgb(th().text.bright))
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
        .rounded_content()
        .bg(rgb(th().surface.raised))
        .text_color(rgb(th().text.primary))
        .cursor_pointer()
        .on_click(cx.listener(move |onb, _ev: &ClickEvent, window, cx| on_click(onb, window, cx)))
        .child(text!(label.to_string()))
        .into_any_element()
}

/// Botão "Adicionar/Escolher pasta": abre o **seletor nativo** (só-pastas) FORA da thread de UI
/// (async via `cx.spawn`), valida pelo caminho canônico (dedup) e adiciona marcada. Erro = silencioso
/// (cancelar é legítimo); o estado nunca se perde.
fn add_folder_button(
    id: &'static str,
    label: &str,
    cx: &mut Context<OnboardingView>,
) -> AnyElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_content()
        .bg(rgb(th().surface.raised))
        .text_color(rgb(th().text.primary))
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
        let md =
            "# Título\ntexto\n## Seção\n   # indentado\n#semsespaco\n###### Seis\n####### Sete";
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
        // âncora de bloco `^` também é cortada.
        assert_eq!(extract_wikilinks("[[Nota^bloco]]"), vec!["Nota"]);
    }

    /// inv#1 CRÍTICO: `[[…]]`/`#tag`/`![[…]]` DENTRO de cercas de código (``` e `~~~`) são ignorados;
    /// só os de FORA contam. (Sem isto, exemplos de código viram arestas/tags fantasmas.)
    #[test]
    fn parser_excludes_fenced_code_blocks() {
        let md = "real [[Alvo]] e #tagreal\n\
                  ```rust\n\
                  let x = [[FakeLink]]; // #faketag\n\
                  ```\n\
                  ~~~\n\
                  [[OutroFake]] #outrofake ![[fake.png]]\n\
                  ~~~\n\
                  fim [[Fim]] ![[real.png]]\n";
        let note = parse_note("n.md".into(), md);
        assert_eq!(note.links, vec!["Alvo", "Fim"]);
        assert_eq!(note.embeds, vec!["real.png"]);
        assert!(note.tags.contains(&"tagreal".to_string()));
        assert!(
            !note.tags.iter().any(|t| t == "faketag" || t == "outrofake"),
            "tags em código não contam: {:?}",
            note.tags
        );
        assert!(!note
            .links
            .iter()
            .any(|l| l == "FakeLink" || l == "OutroFake"));
        assert!(!note.embeds.iter().any(|e| e == "fake.png"));
    }

    /// `![[embed]]` é embed, `[[link]]` é link — nunca se confundem (o `!` precedente decide).
    #[test]
    fn embeds_separate_from_links() {
        let md = "veja [[NotaA]] e ![[NotaB]] e ![[img.png]] e [[NotaA]]";
        assert_eq!(extract_wikilinks(md), vec!["NotaA"]);
        assert_eq!(extract_embeds(md), vec!["NotaB", "img.png"]);
    }

    /// `link_count_pairs`: multiplicidade por alvo (peso da aresta); embed NÃO conta.
    #[test]
    fn link_counts_track_multiplicity() {
        let md = "[[B]] e [[B]] e [[C]] e ![[B]]";
        assert_eq!(
            link_count_pairs(md),
            vec![("B".to_string(), 2), ("C".to_string(), 1)]
        );
    }

    /// `extract_frontmatter_tags`: bloco YAML, lista flow e escalar; aspas desencapadas; sem fm → vazio.
    #[test]
    fn frontmatter_tags_parse_block_flow_and_scalar() {
        let block = "---\ntags:\n  - alpha\n  - beta\ntitle: x\n---\ncorpo #naoconta-aqui? não\n";
        assert_eq!(extract_frontmatter_tags(block), vec!["alpha", "beta"]);
        let flow = "---\ntags: [um, \"dois\", tres]\n---\n";
        assert_eq!(extract_frontmatter_tags(flow), vec!["um", "dois", "tres"]);
        let scalar = "---\ntags: solo, duo\n---\n";
        assert_eq!(extract_frontmatter_tags(scalar), vec!["solo", "duo"]);
        // `tags:` fora de frontmatter (não no topo) → não conta.
        assert!(extract_frontmatter_tags("# corpo\ntags: nao-conta\n").is_empty());
    }

    /// `extract_inline_tags`: fronteira (espaço/início), nested `#a/b`, ignora inline code e URL frag,
    /// rejeita puramente numérico.
    #[test]
    fn inline_tags_respect_boundaries_code_and_nesting() {
        let body =
            "tenho #projeto/lina e #ok mas https://x.com/#frag não, nem `#emcodigo`, nem #123 fim";
        let tags = extract_inline_tags(body);
        assert!(
            tags.contains(&"projeto/lina".to_string()),
            "nested: {tags:?}"
        );
        assert!(tags.contains(&"ok".to_string()));
        assert!(!tags.iter().any(|t| t == "frag"), "url frag não é tag");
        assert!(
            !tags.iter().any(|t| t == "emcodigo"),
            "inline code não é tag"
        );
        assert!(!tags.iter().any(|t| t == "123"), "numérico puro não é tag");
    }

    /// `split_frontmatter`: tolera BOM; sem frontmatter devolve o conteúdo intacto como corpo.
    #[test]
    fn split_frontmatter_handles_bom_and_absence() {
        let (fm, body) = split_frontmatter("\u{feff}---\ntags: a\n---\nCorpo\n");
        assert_eq!(fm.trim(), "tags: a");
        assert_eq!(body, "Corpo\n");
        let (fm2, body2) = split_frontmatter("Sem fm\n[[x]]\n");
        assert_eq!(fm2, "");
        assert_eq!(body2, "Sem fm\n[[x]]\n");
    }

    /// `parse_note`: funde tags do frontmatter com as inline (frontmatter primeiro), e o frontmatter NÃO
    /// vira heading nem link.
    #[test]
    fn parse_note_merges_fm_and_inline_tags() {
        let md = "---\ntags: [fromfm]\n---\nCorpo com #inline e [[Link]]\n";
        let note = parse_note("p.md".into(), md);
        assert_eq!(note.tags, vec!["fromfm", "inline"]);
        assert_eq!(note.links, vec!["Link"]);
        assert!(
            note.headings.is_empty(),
            "o `---` do frontmatter não é heading"
        );
    }

    /// Helper de teste: uma `NoteEntry` com `rel_path` + links (deriva `link_counts` p/ o peso).
    fn note(rel: &str, links: &[&str]) -> NoteEntry {
        let mut lc: Vec<(String, usize)> = Vec::new();
        for l in links {
            if let Some(slot) = lc.iter_mut().find(|(k, _)| k == l) {
                slot.1 += 1;
            } else {
                lc.push((l.to_string(), 1));
            }
        }
        NoteEntry {
            rel_path: rel.to_string(),
            links: dedup_ordered(links.iter().map(|s| s.to_string())),
            link_counts: lc,
            ..Default::default()
        }
    }

    /// `analyze_graph`: resolve arestas com peso, calcula in/out-degree DISTINTOS e detecta órfãos.
    #[test]
    fn graph_resolves_links_counts_degrees_and_orphans() {
        let data = VaultIndexData {
            folders: vec![],
            notes: vec![
                note("A.md", &["B", "B", "C"]), // A→B (x2), A→C
                note("B.md", &["C"]),
                note("C.md", &[]),
                note("Solo.md", &[]),
            ],
        };
        let g = analyze_graph(&data);
        assert_eq!(
            g.edges,
            vec![
                GraphEdge {
                    from: "A.md".into(),
                    to: "B.md".into(),
                    count: 2
                },
                GraphEdge {
                    from: "A.md".into(),
                    to: "C.md".into(),
                    count: 1
                },
                GraphEdge {
                    from: "B.md".into(),
                    to: "C.md".into(),
                    count: 1
                },
            ]
        );
        let m = |p: &str| g.metrics.iter().find(|x| x.rel_path == p).unwrap().clone();
        assert_eq!((m("A.md").out_degree, m("A.md").in_degree), (2, 0));
        assert_eq!((m("C.md").out_degree, m("C.md").in_degree), (0, 2));
        assert!(m("Solo.md").is_orphan);
        assert!(!m("A.md").is_orphan, "tem saída");
        assert!(!m("C.md").is_orphan, "tem entrada");
    }

    /// `analyze_graph`: alvo inexistente → pendente; basename duplicado sem caminho → ambíguo; com
    /// caminho parcial → desambigua (vira aresta). Nunca adivinha.
    #[test]
    fn graph_flags_ambiguous_and_unresolved() {
        let data = VaultIndexData {
            folders: vec![],
            notes: vec![
                note("Start.md", &["Dup", "Area/Dup", "Pendente"]),
                note("X/Dup.md", &[]),
                note("Area/Dup.md", &[]),
            ],
        };
        let g = analyze_graph(&data);
        assert!(
            g.edges
                .iter()
                .any(|e| e.from == "Start.md" && e.to == "Area/Dup.md"),
            "caminho parcial desambigua: {:?}",
            g.edges
        );
        assert!(g
            .problems
            .iter()
            .any(|p| p.target == "Dup" && p.issue == LinkIssue::Ambiguous));
        assert!(g
            .problems
            .iter()
            .any(|p| p.target == "Pendente" && p.issue == LinkIssue::Unresolved));
    }

    /// `analyze_graph`: hub (in-degree ≥ 15), MOC (nome OU tag), órfão e ponto de entrada (out ≥ 5).
    #[test]
    fn graph_detects_hub_moc_orphan_and_entry_point() {
        let mut notes = vec![note("Central.md", &[])];
        for i in 0..15 {
            notes.push(note(&format!("ref{i}.md"), &["Central"]));
        }
        notes.push(NoteEntry {
            rel_path: "Projetos MOC.md".into(),
            ..Default::default()
        });
        notes.push(NoteEntry {
            rel_path: "Indice.md".into(),
            tags: vec!["moc".into()],
            ..Default::default()
        });
        notes.push(note("Porta.md", &["ref0", "ref1", "ref2", "ref3", "ref4"]));
        let data = VaultIndexData {
            folders: vec![],
            notes,
        };
        let g = analyze_graph(&data);
        let m = |p: &str| g.metrics.iter().find(|x| x.rel_path == p).unwrap().clone();
        assert_eq!(m("Central.md").in_degree, 15);
        assert!(m("Central.md").is_hub);
        assert!(m("Projetos MOC.md").is_moc, "MOC por nome");
        assert!(m("Indice.md").is_moc, "MOC por tag #moc");
        assert!(
            m("Porta.md").is_entry_point,
            "out_degree {}",
            m("Porta.md").out_degree
        );
        assert!(
            m("Projetos MOC.md").is_orphan,
            "MOC pode ser órfão (categorias independentes)"
        );
    }

    /// `is_moc`: casa `moc` como TOKEN (não substring) — evita "democracia"/"mockup"; + tag e "map of
    /// content".
    #[test]
    fn moc_detection_uses_word_boundary() {
        assert!(is_moc("Saúde MOC.md", &[]));
        assert!(is_moc("MOC.md", &[]));
        assert!(is_moc("projetos-moc.md", &[]));
        assert!(is_moc("Maps of Content.md", &[]));
        assert!(is_moc("Qualquer.md", &["moc".to_string()]), "tag #moc");
        assert!(!is_moc("democracia.md", &[]), "substring no meio não conta");
        assert!(
            !is_moc("mockup.md", &[]),
            "substring no início de palavra não conta"
        );
    }

    /// `render_vault_index_with`: as seções de navegação existem, o ponto de entrada e os backlinks são
    /// listados, e os links pendentes aparecem na seção correta.
    #[test]
    fn render_index_has_navigation_sections_and_backlinks() {
        let data = VaultIndexData {
            folders: vec![],
            notes: vec![
                note("Hub.md", &["a", "b", "c", "d", "e"]), // out 5 → ponto de entrada
                note("a.md", &[]),
                note("b.md", &[]),
                note("c.md", &[]),
                note("d.md", &[]),
                note("e.md", &[]),
                note("Pendura.md", &["NaoExiste"]),
            ],
        };
        let g = analyze_graph(&data);
        let md = render_vault_index_with("V", Path::new("/tmp/v"), &data, &g);
        for sec in [
            "## Pontos de entrada",
            "## Hubs",
            "## MOCs",
            "## Órfãos",
            "## Backlinks",
            "## Links pendentes/ambíguos",
        ] {
            assert!(md.contains(sec), "falta seção {sec}");
        }
        assert!(
            md.contains("Hub (saída: 5)"),
            "ponto de entrada listado:\n{md}"
        );
        assert!(md.contains("a ← [[Hub]]"), "backlink listado:\n{md}");
        assert!(
            md.contains("NaoExiste") && md.contains("pendente"),
            "link pendente:\n{md}"
        );
        // o grafo de saída mantém o formato legado.
        assert!(md.contains("Hub → [[a]], [[b]], [[c]], [[d]], [[e]]"));
    }

    /// `render_vault_index_json`: sidecar válido com arestas exatas (peso) e categorias.
    #[test]
    fn json_sidecar_serializes_edges_and_categories() {
        let data = VaultIndexData {
            folders: vec![],
            notes: vec![note("A.md", &["B", "B"]), note("B.md", &[])],
        };
        let g = analyze_graph(&data);
        let js = render_vault_index_json("V", Path::new("/tmp/v"), &data, &g);
        let v: serde_json::Value = serde_json::from_str(&js).expect("json válido");
        assert_eq!(v["vault"], "V");
        assert_eq!(v["note_count"], 2);
        assert_eq!(v["edge_count"], 1);
        assert_eq!(v["edges"][0]["from"], "A.md");
        assert_eq!(v["edges"][0]["to"], "B.md");
        assert_eq!(v["edges"][0]["count"], 2);
        // determinístico: re-render = idêntico.
        assert_eq!(
            js,
            render_vault_index_json("V", Path::new("/tmp/v"), &data, &g)
        );
    }

    /// `obsidian_config_paths_for`: 1 caminho no macOS/Windows; no Linux MERGE de nativo+Flatpak+Snap;
    /// `$XDG_CONFIG_HOME` respeitado quando setado. (Lógica pura → testável p/ os 3 SOs num Mac.)
    #[test]
    fn config_paths_resolve_per_os() {
        let home = PathBuf::from("/home/u");
        let env_win = |k: &str| -> Option<OsString> {
            (k == "APPDATA").then(|| OsString::from("C:\\Users\\u\\AppData\\Roaming"))
        };
        let none = |_: &str| -> Option<OsString> { None };

        assert_eq!(
            obsidian_config_paths_for("macos", Some(&home), &none),
            vec![home.join("Library/Application Support/obsidian/obsidian.json")]
        );
        assert_eq!(
            obsidian_config_paths_for("windows", Some(&home), &env_win),
            vec![PathBuf::from("C:\\Users\\u\\AppData\\Roaming")
                .join("obsidian")
                .join("obsidian.json")]
        );
        // Linux: nativo (.config) + Flatpak sandbox + Snap confinado — todos juntos.
        assert_eq!(
            obsidian_config_paths_for("linux", Some(&home), &none),
            vec![
                home.join(".config/obsidian/obsidian.json"),
                home.join(".var/app/md.obsidian.Obsidian/config/obsidian/obsidian.json"),
                home.join("snap/obsidian/current/.config/obsidian/obsidian.json"),
            ]
        );
        // XDG_CONFIG_HOME sobrepõe o ~/.config.
        let env_xdg = |k: &str| -> Option<OsString> {
            (k == "XDG_CONFIG_HOME").then(|| OsString::from("/cfg"))
        };
        let lin = obsidian_config_paths_for("linux", Some(&home), &env_xdg);
        assert_eq!(lin[0], PathBuf::from("/cfg/obsidian/obsidian.json"));
    }

    /// `app_bundle_candidates_for`: macOS `.app`; Windows per-user em `Programs\` (corrige a armadilha)
    /// + all-users em `Program Files`; Linux Flatpak/usr/Snap/user-local.
    #[test]
    fn app_bundle_candidates_per_os() {
        let home = PathBuf::from("/home/u");
        let env_win = |k: &str| -> Option<OsString> {
            match k {
                "LOCALAPPDATA" => Some(OsString::from("C:\\Users\\u\\AppData\\Local")),
                "ProgramFiles" => Some(OsString::from("C:\\Program Files")),
                _ => None,
            }
        };
        let none = |_: &str| -> Option<OsString> { None };

        let mac = app_bundle_candidates_for("macos", "Obsidian", Some(&home), &none);
        assert_eq!(mac[0], PathBuf::from("/Applications/Obsidian.app"));
        assert!(mac.contains(&home.join("Applications").join("Obsidian.app")));

        let win = app_bundle_candidates_for("windows", "Obsidian", Some(&home), &env_win);
        assert!(
            win.contains(
                &PathBuf::from("C:\\Users\\u\\AppData\\Local")
                    .join("Programs")
                    .join("Obsidian")
                    .join("Obsidian.exe")
            ),
            "per-user em Programs\\: {win:?}"
        );
        assert!(win.contains(
            &PathBuf::from("C:\\Program Files")
                .join("Obsidian")
                .join("Obsidian.exe")
        ));

        let lin = app_bundle_candidates_for("linux", "Obsidian", Some(&home), &none);
        assert!(lin.contains(&PathBuf::from(
            "/var/lib/flatpak/exports/bin/md.obsidian.Obsidian"
        )));
        assert!(lin.contains(&PathBuf::from("/usr/bin/obsidian")));
        assert!(lin.contains(&PathBuf::from("/snap/bin/obsidian")));
        assert!(lin.contains(&home.join(".local/share/flatpak/exports/bin/md.obsidian.Obsidian")));
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
        assert_eq!(
            footer_label(Screen::WithVaults, 1),
            "Continuar com 1 pasta →"
        );
        assert_eq!(
            footer_label(Screen::WithVaults, 3),
            "Continuar com 3 pastas →"
        );
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

        let md = render_vault_index_with("Meu Vault", vault.path(), &data, &analyze_graph(&data));
        assert!(md.contains("# Vault Index — Meu Vault"));
        assert!(md.contains("NÃO editar"));
        assert!(md.contains("- Area/"));
        assert!(md.contains("### a.md"));
        assert!(md.contains("- # A"));
        assert!(md.contains("- ## Sub"));
        assert!(md.contains("a → [[b]]")); // grafo: a aponta b
        assert!(md.contains("Area/b → (folha)")); // b é folha
                                                  // determinístico: re-scan + re-render = idêntico.
        let data2 = scan_vault(vault.path());
        let md2 =
            render_vault_index_with("Meu Vault", vault.path(), &data2, &analyze_graph(&data2));
        assert_eq!(md, md2);
    }

    /// `is_dataless_stat`: um placeholder de nuvem (iCloud/OneDrive evicted) tem tamanho LÓGICO mas
    /// ZERO blocos no disco → ler dispararia um download de rede. Materializado = blocos > 0. Arquivo
    /// vazio de verdade (0 bytes) NÃO é placeholder. É a heurística que evita baixar o vault inteiro.
    #[test]
    fn dataless_stat_flags_only_cloud_placeholders() {
        assert!(
            is_dataless_stat(0, 1234),
            "tamanho>0 com zero blocos = placeholder evicted (conteúdo só na nuvem)"
        );
        assert!(
            !is_dataless_stat(8, 1234),
            "tem blocos alocados = materializado no disco"
        );
        assert!(
            !is_dataless_stat(0, 0),
            "arquivo realmente vazio (0 bytes) não é placeholder de nuvem"
        );
    }

    /// `parse_note_or_skip`: quando o predicado diz "está na nuvem", NÃO lê o arquivo (não dispara
    /// download) — devolve uma nota marcada `dataless`, com conteúdo vazio MESMO que o arquivo tenha
    /// headings/links no disco. Prova: o arquivo tem conteúdo, mas a nota volta vazia (não foi lida).
    #[test]
    fn parse_note_or_skip_does_not_read_cloud_files() {
        let vault = TempDir::new("dataless");
        write(vault.path(), "n.md", "# Tem Conteudo\nlink [[x]]\n");
        let path = vault.path().join("n.md");
        let note = parse_note_or_skip("n.md", &path, |_| true); // simula "na nuvem (evicted)"
        assert!(note.dataless, "marcada como na nuvem");
        assert!(
            note.headings.is_empty() && note.links.is_empty(),
            "NÃO leu o arquivo: zero download. headings={:?} links={:?}",
            note.headings,
            note.links
        );
    }

    /// `parse_note_or_skip`: arquivo local (materializado) → lê e parseia normalmente, `dataless=false`.
    /// Não-regressão: o caminho comum (vault local) continua extraindo headings/links como antes.
    #[test]
    fn parse_note_or_skip_reads_local_files() {
        let vault = TempDir::new("local");
        write(vault.path(), "n.md", "# Titulo\nlink [[x]]\n");
        let path = vault.path().join("n.md");
        let note = parse_note_or_skip("n.md", &path, |_| false); // local/materializado
        assert!(!note.dataless);
        assert_eq!(note.headings, vec!["# Titulo"]);
        assert_eq!(note.links, vec!["x"]);
    }

    /// `render_vault_index_with`: notas na nuvem (não baixadas) são contadas com HONESTIDADE no
    /// cabeçalho — o mapa não finge que indexou o que não leu (espelha a "parada honesta" do
    /// `vault search`). Com zero notas na nuvem, o cabeçalho NÃO ganha a linha extra (determinismo).
    #[test]
    fn render_reports_cloud_notes_honestly() {
        let cloud = NoteEntry {
            rel_path: "nuvem.md".into(),
            dataless: true,
            ..Default::default()
        };
        let data = VaultIndexData {
            folders: vec![],
            notes: vec![note("local.md", &["x"]), cloud],
        };
        let md = render_vault_index_with(
            "V",
            std::path::Path::new("/v"),
            &data,
            &analyze_graph(&data),
        );
        let low = md.to_lowercase();
        assert!(
            low.contains("nuvem") || low.contains("icloud") || low.contains("não baixad"),
            "o índice precisa avisar que há nota(s) não baixada(s). saída:\n{md}"
        );
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
        assert!(
            out.starts_with(lina.path()),
            "índice mora em .lina, não no vault"
        );
        assert!(out.exists());
        assert!(std::fs::read_to_string(&out)
            .unwrap()
            .contains("# Vault Index — Notas"));
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
        assert_eq!(
            read_primary_vault(lina.path()).as_deref(),
            Some("/Users/voce/Notas")
        );
        // releitura completa = igual.
        let back: VaultConfig =
            serde_json::from_str(&std::fs::read_to_string(lina.path().join("vault.json")).unwrap())
                .unwrap();
        assert_eq!(back, cfg);
    }

    /// Helper: um `VaultConfig` mínimo com o `primary` dado.
    fn cfg_with(primary: &str) -> VaultConfig {
        VaultConfig {
            primary: primary.into(),
            vaults: vec![VaultEntry {
                name: "V".into(),
                path: primary.into(),
                writable: format!("{primary}/Lina"),
            }],
        }
    }

    /// ADR 0056: precedência `<ws>/.lina` (override de projeto) → `~/.lina` (global herdado).
    #[test]
    fn vault_layered_prefers_workspace_then_global() {
        let ws = TempDir::new("layered-ws");
        let global = TempDir::new("layered-global");
        let dirs = vec![ws.path().to_path_buf(), global.path().to_path_buf()];
        // nenhum tem → None (cai no fallback do caller)
        assert!(read_primary_vault_layered(&dirs).is_none());
        // só o global tem → herda do global
        write_vault_config(global.path(), &cfg_with("/G")).unwrap();
        assert_eq!(read_primary_vault_layered(&dirs).as_deref(), Some("/G"));
        // ws tem → override de projeto vence o global
        write_vault_config(ws.path(), &cfg_with("/W")).unwrap();
        assert_eq!(read_primary_vault_layered(&dirs).as_deref(), Some("/W"));
    }

    /// ADR 0056: a migração promove o 1º Espaço com vault e é idempotente (não sobrescreve o global vivo).
    #[test]
    fn migrate_promotes_first_candidate_and_is_idempotent() {
        let espaco = TempDir::new("mig-espaco");
        let global = TempDir::new("mig-global");
        write_vault_config(espaco.path(), &cfg_with("/X")).unwrap();
        // global vazio → migra de espaco
        migrate_vault_to_global_from(&[espaco.path().to_path_buf()], global.path());
        assert_eq!(read_primary_vault(global.path()).as_deref(), Some("/X"));
        // idempotente: muda a fonte e re-migra → o global NÃO muda (já populado)
        write_vault_config(espaco.path(), &cfg_with("/Y")).unwrap();
        migrate_vault_to_global_from(&[espaco.path().to_path_buf()], global.path());
        assert_eq!(read_primary_vault(global.path()).as_deref(), Some("/X"));
    }

    /// Migração sem nenhum candidato com vault → no-op (não cria `vault.json` vazio no global).
    #[test]
    fn migrate_noop_when_no_candidate_has_vault() {
        let empty = TempDir::new("mig-empty");
        let global = TempDir::new("mig-global2");
        migrate_vault_to_global_from(&[empty.path().to_path_buf()], global.path());
        assert!(read_primary_vault(global.path()).is_none());
    }

    /// O `second-brain.toml` REAL parseia e cobre os 3 SOs com `program` não-vazio.
    #[test]
    fn real_second_brain_toml_is_valid_and_complete() {
        let inst = second_brain_installers();
        let prof = inst.0.get("obsidian").expect("falta receita obsidian");
        for os in ["macos", "linux", "windows"] {
            let r = prof
                .for_os(os)
                .unwrap_or_else(|| panic!("falta obsidian.{os}"));
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
        assert_eq!(
            decide_plan("obsidian", "macos", mac.as_ref(), |_| true),
            InstallPlan::Silent
        );
        assert_eq!(
            decide_plan("obsidian", "macos", mac.as_ref(), |_| false),
            InstallPlan::Interactive
        );
        let win = prof.for_os("windows").cloned();
        assert_eq!(
            decide_plan("obsidian", "windows", win.as_ref(), |_| true),
            InstallPlan::Interactive
        );
        let lin = prof.for_os("linux").cloned();
        assert_eq!(
            decide_plan("obsidian", "linux", lin.as_ref(), |_| true),
            InstallPlan::Silent
        );
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

        let app_only: ScanFn = Arc::new(|| ObsidianScan {
            app_present: true,
            vaults: vec![],
        });
        let mut m2 = SecondBrainModel::new_with(lina.path().to_path_buf(), app_only);
        m2.block_on_discovery();
        m2.poll();
        assert_eq!(m2.screen(), Screen::NoVaults);
    }

    /// `toggle` alterna a marcação (consentimento explícito).
    #[test]
    fn toggle_selects_and_deselects() {
        let lina = TempDir::new("toggle");
        let app_only: ScanFn = Arc::new(|| ObsidianScan {
            app_present: true,
            vaults: vec![],
        });
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
        let cfg: VaultConfig =
            serde_json::from_str(&std::fs::read_to_string(lina.path().join("vault.json")).unwrap())
                .unwrap();
        assert_eq!(
            cfg.vaults[0].writable,
            vpath.join("Lina").display().to_string()
        );
        // índice gerado em .lina/vault-index/ (agora em BACKGROUND ao confirmar — não trava a UI;
        // o teste espera a thread terminar p/ ser determinístico). HÍBRIDO: markdown + sidecar JSON.
        m.block_on_index();
        let idx_dir = lina.path().join("vault-index");
        let files: Vec<_> = std::fs::read_dir(&idx_dir).unwrap().flatten().collect();
        let ext = |f: &std::fs::DirEntry| {
            f.path()
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
        };
        assert_eq!(files.len(), 2, "markdown + json por vault marcado");
        assert_eq!(
            files
                .iter()
                .filter(|f| ext(f).as_deref() == Some("md"))
                .count(),
            1
        );
        assert_eq!(
            files
                .iter()
                .filter(|f| ext(f).as_deref() == Some("json"))
                .count(),
            1
        );
        let md_file = files
            .iter()
            .find(|f| ext(f).as_deref() == Some("md"))
            .unwrap();
        let idx = std::fs::read_to_string(md_file.path()).unwrap();
        assert!(idx.contains("# Vault Index — Notas"));
        assert!(idx.contains("nota → [[outra]]"));
        // o sidecar JSON é válido e carrega as arestas exatas.
        let json_file = files
            .iter()
            .find(|f| ext(f).as_deref() == Some("json"))
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(json_file.path()).unwrap()).unwrap();
        assert_eq!(v["vault"], "Notas");
        assert!(v["edges"].is_array());
    }

    /// `confirm` com 0 marcadas NÃO grava config (vira "pular" — UX §5) e não cria vault.json.
    #[test]
    fn confirm_with_zero_selected_does_not_write_config() {
        let lina = TempDir::new("conf-zero");
        let app_only: ScanFn = Arc::new(|| ObsidianScan {
            app_present: true,
            vaults: vec![],
        });
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
        let app_only: ScanFn = Arc::new(|| ObsidianScan {
            app_present: true,
            vaults: vec![],
        });
        let mut m = SecondBrainModel::new_with(lina.path().to_path_buf(), app_only);
        m.block_on_discovery();
        m.poll();
        assert_eq!(
            m.add_manual_vault(folder.path().to_path_buf()),
            AddResult::Added
        );
        assert_eq!(m.all_vaults().len(), 1);
        assert!(m.is_selected(folder.path()));
        // mesma pasta de novo → não duplica.
        assert_eq!(
            m.add_manual_vault(folder.path().to_path_buf()),
            AddResult::AlreadyPresent
        );
        assert_eq!(m.all_vaults().len(), 1);
    }

    /// `slugify`/`index_filename_base`: ASCII estável; vaults de mesmo nome em pastas distintas NÃO
    /// colidem (a base é compartilhada por `<base>.md` e `<base>.json`).
    #[test]
    fn index_filename_disambiguates_same_name() {
        assert_eq!(slugify("Minhas Anotações!!"), "minhas-anota-es");
        let a = index_filename_base("Notas", Path::new("/a/Notas"));
        let b = index_filename_base("Notas", Path::new("/b/Notas"));
        assert_ne!(a, b, "mesmo nome, pastas diferentes → arquivos diferentes");
        assert!(a.starts_with("notas-") && !a.ends_with(".md"));
    }

    /// **Self-heal (boot):** `vaults_missing_index` detecta o índice FALTANDO e, após gerar, NÃO o
    /// reporta mais — o gatilho do regen é exatamente "vault conectado mas índice ausente". Controle
    /// positivo (índice presente → lista vazia) prova não-vacuosidade. Sem corrida de env (dirs temp).
    #[test]
    fn self_heal_detects_missing_index_then_clears_after_generation() {
        let base = std::env::temp_dir().join(format!("lina-heal-{}", std::process::id()));
        let lina = base.join(".lina");
        let vault = base.join("Meu Vault");
        std::fs::create_dir_all(vault.join("sub")).expect("cria vault");
        std::fs::write(vault.join("a.md"), "# A\n[[b]]\n").expect("a.md");
        std::fs::write(vault.join("sub").join("b.md"), "# B\n").expect("b.md");
        let cfg = VaultConfig {
            primary: vault.display().to_string(),
            vaults: vec![VaultEntry {
                name: "Meu Vault".to_string(),
                path: vault.display().to_string(),
                writable: vault.join("Lina").display().to_string(),
            }],
        };
        write_vault_config(&lina, &cfg).expect("vault.json");

        // ANTES: vault.json existe, índice NÃO → o self-heal DEVE marcar este vault.
        let missing = vaults_missing_index(&lina, &cfg);
        assert_eq!(missing.len(), 1, "índice ausente → 1 vault a curar");
        assert_eq!(missing[0].path, vault, "aponta pro vault certo");

        // Gera o índice (o que o heal faria na thread).
        write_vault_index(&lina, &missing[0]).expect("gera índice");

        // DEPOIS: índice presente → lista VAZIA (não re-escaneia à toa).
        assert!(
            vaults_missing_index(&lina, &cfg).is_empty(),
            "índice presente → nada a curar (controle positivo)"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
