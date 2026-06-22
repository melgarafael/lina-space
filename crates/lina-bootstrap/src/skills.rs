//! **F1-3 (ACHADO-1 do gate) — a safra COMPLETA de skills da Lina, embutida.**
//!
//! O instalador antigo levava SÓ `lina-agent-bus`: as 12 skills (11 da F1-3 + lina-translator da F3-2: orquestração,
//! cold-review, dispatch, spawn, retro, verification + as 4 doutrinas transversais) NÃO
//! chegavam ao terminal — o cenário do gate só funcionou porque o Maestro copiou na mão.
//! Princípio-base (Lina universal): TODAS as capacidades acessíveis em tudo; o ENFORCEMENT
//! continua por-agente (guard W3-6) — disponibilidade ≠ autorização.
//!
//! Fonte da verdade: `assets/lina-skills/` — embutida via `include_str!` (o kit funciona no
//! app DISTRIBUÍDO, sem depender de `LINA_ASSETS`/dir de assets no disco). O teste de
//! paridade (`catalog_matches_assets_dir`) trava catálogo×disco: skill nova em
//! `assets/lina-skills/` sem entrada aqui = teste vermelho.

use std::path::{Path, PathBuf};

use lina_core::skill_factory::{
    classify_skill_load, validate_format, SkillFormatError, FACTORY_METHOD,
};
use lina_core::skill_index::{parse_frontmatter, SkillIndexEntry};
use lina_core::{ActionClass, MailMessage};

/// Uma skill embutida: nome da pasta + arquivos `(caminho relativo, conteúdo)`.
/// `files` é relativo à pasta da skill (ex.: `SKILL.md`, `references/rubrica.md`).
pub struct EmbeddedSkill {
    /// Nome da pasta da skill (`<skills_root>/<name>/`).
    pub name: &'static str,
    /// Arquivos da skill: `(caminho relativo, conteúdo embutido)`.
    pub files: &'static [(&'static str, &'static str)],
}

/// Embute uma skill de `assets/lina-skills/<name>/` com os arquivos listados.
macro_rules! embed {
    ($name:literal, [$($rel:literal),+ $(,)?]) => {
        EmbeddedSkill {
            name: $name,
            files: &[$(
                (
                    $rel,
                    include_str!(concat!("../../../assets/lina-skills/", $name, "/", $rel)),
                ),
            )+],
        }
    };
}

/// **A 1ª safra completa** — espelho de `assets/lina-skills/` (ordem alfabética; o teste de
/// paridade denuncia drift). `lina-cold-review` e `lina-orchestration` carregam `references/`
/// (a rubrica do cold-review é transversal ao épico — precisa chegar junto).
pub const LINA_SKILLS: &[EmbeddedSkill] = &[
    embed!("lina-agent-bus", ["SKILL.md"]),
    embed!("lina-architecture-doctrine", ["SKILL.md"]),
    embed!("lina-code-doctrine", ["SKILL.md"]),
    embed!("lina-cold-review", ["SKILL.md", "references/rubrica.md"]),
    embed!("lina-copy-doctrine", ["SKILL.md"]),
    embed!("lina-design-doctrine", ["SKILL.md"]),
    embed!("lina-dispatch", ["SKILL.md"]),
    embed!(
        "lina-orchestration",
        ["SKILL.md", "references/monitoramento.md"]
    ),
    embed!("lina-retro", ["SKILL.md"]),
    embed!("lina-spawn-terminal", ["SKILL.md"]),
    // F3-2-1: a doutrina da porta de entrada (Tradutor). Ordem alfabética.
    embed!("lina-translator", ["SKILL.md"]),
    embed!("lina-verification", ["SKILL.md"]),
];

/// Instala TODAS as skills embutidas sob `skills_root` (ex.: `<cwd>/.claude/skills` no kit
/// por-nó; `~/.claude/skills` no global por-CLI). Aditivo + idempotente: cada skill mora em
/// pasta própria (nunca colide com skills do usuário); arquivo idêntico não é reescrito
/// (sem churn de mtime). Devolve os dirs das skills (para o report/log honesto do app).
///
/// # Errors
/// Propaga o primeiro erro de I/O (criar dirs / escrever arquivo).
pub fn install_skills_into(skills_root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::with_capacity(LINA_SKILLS.len());
    for skill in LINA_SKILLS {
        let dir = skills_root.join(skill.name);
        for (rel, content) in skill.files {
            let dest = dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if std::fs::read_to_string(&dest).unwrap_or_default() == *content {
                continue; // idêntico → não reescreve
            }
            write_atomic(&dest, content)?;
        }
        dirs.push(dir);
    }
    Ok(dirs)
}

/// Escrita atômica (tmp + rename) — robusta a crash/concorrência. Vive aqui (o módulo mais
/// fundo da cadeia de instalação); `global_install` reusa.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

// ───────────── F3-5-4/5: índice de skills + verbos `lina skill` (anti-ciclo: AQUI) ─────────────
// O catálogo (LINA_SKILLS) e a leitura do disco vivem no bootstrap; o core só recebe o índice
// pronto e roda o seletor PURO (`bootstrap → core`, sem ciclo). Os verbos enfileiram um contrato;
// o supervisor emite SkillSelected/SkillFactoryProposed carimbando o `node` SERVER-SIDE.

/// Alvo sentinela dos verbos de skill — o supervisor intercepta por INTENT, não pelo alvo.
const SKILL_TARGET: &str = "skill";

/// Constrói o índice de skills do seletor (F3-5-4) a partir do catálogo EMBUTIDO
/// ([`LINA_SKILLS`]) + as skills do usuário em `<disk_skills_root>/<nome>/SKILL.md`. Lê o
/// frontmatter neutro via parser do CORE; skills legadas (só `name`+`description`) entram com
/// `triggers`/`requires` vazios — sempre capazes, sem gatilho automático.
#[must_use]
pub fn build_skill_index(disk_skills_root: Option<&Path>) -> Vec<SkillIndexEntry> {
    let mut index: Vec<SkillIndexEntry> = LINA_SKILLS
        .iter()
        .filter_map(|skill| {
            skill
                .files
                .iter()
                .find(|(rel, _)| *rel == "SKILL.md")
                .map(|(_, body)| index_entry(skill.name, body))
        })
        .collect();
    if let Some(root) = disk_skills_root {
        index.extend(read_disk_skills(root));
    }
    index
}

/// Uma entrada do índice: `name` autoritativo (da pasta) + triggers/requires do frontmatter.
fn index_entry(name: &str, skill_md: &str) -> SkillIndexEntry {
    let fm = parse_frontmatter(skill_md);
    SkillIndexEntry {
        name: name.to_string(),
        triggers: fm.triggers,
        requires: fm.requires,
    }
}

/// As skills do usuário em `<root>/<nome>/SKILL.md`. Ausência de `root`/`SKILL.md` é DEGRADAÇÃO
/// intencional (o seletor cai no catálogo embutido), não erro engolido: um Espaço sem skills no
/// disco simplesmente não acrescenta entradas.
fn read_disk_skills(root: &Path) -> Vec<SkillIndexEntry> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let md = std::fs::read_to_string(e.path().join("SKILL.md")).ok()?;
            Some(index_entry(&name, &md))
        })
        .collect()
}

/// Monta o envelope `skill.select` (F3-5-4): o supervisor emite `SkillSelected{node,...}`
/// carimbando o `node` SERVER-SIDE (identidade autenticada) — o payload NUNCA carrega `node`
/// (dado forjável ≠ autoridade, igual `code.changed` omite `author_node`). `source` = de onde a
/// skill veio (catálogo/hub/fábrica).
#[must_use]
pub fn build_skill_select_envelope(
    from: &str,
    skill: &str,
    trigger: Option<&str>,
    source: &str,
) -> MailMessage {
    let payload = serde_json::json!({
        "skill": skill,
        "trigger": trigger,
        "source": source,
    })
    .to_string();
    MailMessage::new(from, SKILL_TARGET, "skill.select", payload)
}

/// Monta o envelope `skill.propose` (F3-5-5): o supervisor emite `SkillFactoryProposed`. SUGERE,
/// nunca aplica — habilitar a skill é gesto humano. `via` é sempre o método da fábrica.
#[must_use]
pub fn build_skill_propose_envelope(
    from: &str,
    skill_name: &str,
    references: &[String],
) -> MailMessage {
    let payload = serde_json::json!({
        "skill_name": skill_name,
        "via": FACTORY_METHOD,
        "references": references,
    })
    .to_string();
    MailMessage::new(from, SKILL_TARGET, "skill.propose", payload)
}

/// Veredito de inspecionar uma skill (`lina skill check`): o FORMATO (validação do core) + a
/// CLASSE de risco de carga (guard de inline-shell). Read-only — não cria nem habilita nada.
#[derive(Debug, PartialEq, Eq)]
pub struct SkillCheck {
    pub format: Result<(), SkillFormatError>,
    pub load_class: ActionClass,
}

/// Inspeciona uma SKILL.md (PURA): valida o formato e classifica o risco de carga. O caller
/// (`lina skill check`) imprime e mapeia para `ExitCode`; nunca carrega/instala.
#[must_use]
pub fn skill_check(skill_md: &str) -> SkillCheck {
    SkillCheck {
        format: validate_format(skill_md),
        load_class: classify_skill_load(skill_md),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lina-skills")
    }

    /// Varre `dir` recursivamente: `(caminho relativo a base, conteúdo)`. Ignora dotfiles
    /// (`.DS_Store` etc. não são parte de skill).
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir)
            .expect("read_dir")
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let p = entry.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .expect("sob a base")
                    .to_string_lossy()
                    .replace('\\', "/");
                let body = std::fs::read_to_string(&p)
                    .unwrap_or_else(|e| panic!("ler {}: {e}", p.display()));
                out.push((rel, body));
            }
        }
    }

    /// **Trava catálogo×disco (anti-drift):** o conjunto `(skill, arquivo)` embutido é
    /// EXATAMENTE o de `assets/lina-skills/`, com conteúdo idêntico. Skill/arquivo novo em
    /// assets sem entrada no catálogo (ou vice-versa) = vermelho — fecha a CLASSE do
    /// ACHADO-1 (instalador parcial), não só a safra de hoje.
    #[test]
    fn catalog_matches_assets_dir() {
        let mut disk: Vec<(String, String)> = Vec::new();
        walk(&assets_dir(), &assets_dir(), &mut disk);
        disk.sort();

        let mut embedded: Vec<(String, String)> = LINA_SKILLS
            .iter()
            .flat_map(|s| {
                s.files
                    .iter()
                    .map(|(rel, body)| (format!("{}/{rel}", s.name), (*body).to_string()))
            })
            .collect();
        embedded.sort();

        let disk_paths: Vec<&String> = disk.iter().map(|(p, _)| p).collect();
        let embedded_paths: Vec<&String> = embedded.iter().map(|(p, _)| p).collect();
        assert_eq!(
            disk_paths, embedded_paths,
            "catálogo embutido ≠ assets/lina-skills/ — adicione a skill/arquivo novo ao \
             LINA_SKILLS (ou remova do disco)"
        );
        for ((path, on_disk), (_, in_bin)) in disk.iter().zip(embedded.iter()) {
            assert_eq!(
                on_disk, in_bin,
                "{path}: conteúdo embutido divergiu do disco"
            );
        }
    }

    /// O instalador cria as 12 skills (+`references/`) sob o root dado.
    #[test]
    fn installs_all_skills_with_references() {
        let root = std::env::temp_dir().join(format!("lina-skills-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = install_skills_into(&root).expect("instala");
        assert_eq!(
            dirs.len(),
            12,
            "as 12 skills (11 da F1-3 + lina-translator da F3-2)"
        );
        for skill in LINA_SKILLS {
            assert!(
                root.join(skill.name).join("SKILL.md").is_file(),
                "{}: SKILL.md instalado",
                skill.name
            );
        }
        // A rubrica transversal do épico e o guia de monitoramento chegam JUNTO.
        for rel in [
            "lina-cold-review/references/rubrica.md",
            "lina-orchestration/references/monitoramento.md",
        ] {
            assert!(root.join(rel).is_file(), "{rel} instalado");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Idempotente + aditivo: a 2ª chamada não duplica/não quebra (árvore byte-idêntica) e
    /// uma skill ESTRANGEIRA do usuário no mesmo root permanece intocada.
    #[test]
    fn second_run_is_noop_and_preserves_user_skills() {
        let root = std::env::temp_dir().join(format!("lina-skills-idem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Skill do usuário pré-existente no mesmo namespace.
        let user = root.join("minha-skill");
        std::fs::create_dir_all(&user).expect("dir do usuário");
        std::fs::write(user.join("SKILL.md"), "conteúdo do usuário").expect("skill do usuário");

        install_skills_into(&root).expect("1ª");
        let mut snap1: Vec<(String, String)> = Vec::new();
        walk(&root, &root, &mut snap1);
        snap1.sort();

        install_skills_into(&root).expect("2ª");
        let mut snap2: Vec<(String, String)> = Vec::new();
        walk(&root, &root, &mut snap2);
        snap2.sort();

        assert_eq!(snap1, snap2, "2ª rodada não muda NADA (idempotente)");
        assert_eq!(
            std::fs::read_to_string(user.join("SKILL.md")).expect("ler"),
            "conteúdo do usuário",
            "skill do usuário intocada (aditivo)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Portabilidade 3-CLI (critério F1-3-3): o Codex REJEITA skill com `description`
    /// acima de 1024 caracteres ("invalid description: exceeds maximum length of 1024
    /// characters" — transcript real do codex-cli 0.138, 2026-06-10; 6/11 skills da
    /// safra não carregavam). Medimos em BYTES (mais estrito que chars) por folga.
    #[test]
    fn descriptions_fit_codex_limit_of_1024_chars() {
        for skill in LINA_SKILLS {
            let skill_md = skill
                .files
                .iter()
                .find(|(rel, _)| *rel == "SKILL.md")
                .expect("toda skill embute SKILL.md")
                .1;
            let mut in_desc = false;
            let mut desc = String::new();
            for line in skill_md.lines() {
                if let Some(resto) = line.strip_prefix("description:") {
                    in_desc = true;
                    let inline = resto.trim();
                    if inline != ">-" && inline != ">" && inline != "|" && !inline.is_empty() {
                        desc.push_str(inline);
                    }
                    continue;
                }
                if in_desc {
                    if let Some(cont) = line.strip_prefix("  ") {
                        if !desc.is_empty() {
                            desc.push(' ');
                        }
                        desc.push_str(cont.trim_end());
                    } else {
                        break; // próxima chave do frontmatter ou fim do bloco
                    }
                }
            }
            assert!(
                !desc.is_empty(),
                "{}: description ausente no frontmatter do SKILL.md",
                skill.name
            );
            let n = desc.len();
            assert!(
                n <= 1024,
                "{}: description com {n} bytes (>1024 — o Codex recusa a skill inteira; \
                 encurte mantendo os gatilhos de ativação)",
                skill.name
            );
        }
    }

    // ════════════ F3-5-4/5: índice de skills + verbos `lina skill` ════════════

    /// O índice inclui TODO o catálogo embutido; skills legadas (doutrinas) entram sem requisito
    /// de tool — sempre capazes.
    #[test]
    fn index_includes_embedded_catalog() {
        let index = build_skill_index(None);
        assert_eq!(index.len(), LINA_SKILLS.len());
        let bus = index
            .iter()
            .find(|e| e.name == "lina-agent-bus")
            .expect("lina-agent-bus no índice");
        assert!(bus.requires.is_empty(), "doutrina legada = sempre capaz");
    }

    /// CRITÉRIO DE ACEITE: o caller (bootstrap) popula o índice do disco lendo o frontmatter
    /// neutro (trigger/requires) — anti-ciclo: o core só recebe o índice pronto.
    #[test]
    fn index_reads_disk_skills_with_trigger_and_requires() {
        let root = std::env::temp_dir().join(format!("lina-skill-idx-disk-{}", std::process::id()));
        let dir = root.join("deploy-helper");
        std::fs::create_dir_all(&dir).expect("cria dir da skill");
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: deploy-helper\ntrigger: faz o deploy\nrequires: Bash, Vercel\n---\ncorpo\n",
        )
        .expect("escreve SKILL.md");
        let index = build_skill_index(Some(&root));
        let found = index
            .iter()
            .find(|e| e.name == "deploy-helper")
            .expect("skill do disco no índice");
        assert_eq!(found.triggers, vec!["faz o deploy"]);
        assert_eq!(found.requires, vec!["Bash", "Vercel"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Root de disco inexistente degrada para só o catálogo embutido (não derruba o índice).
    #[test]
    fn index_tolerates_missing_disk_root() {
        let index = build_skill_index(Some(Path::new("/caminho/inexistente/lina-xyz-999")));
        assert_eq!(index.len(), LINA_SKILLS.len());
    }

    /// CRITÉRIO DE ACEITE: o envelope de seleção carrega skill/trigger/source e OMITE o `node`
    /// (carimbado server-side, igual `code.changed` omite o autor).
    #[test]
    fn select_envelope_omits_node_and_carries_selection() {
        let msg = build_skill_select_envelope(
            "Terminal J",
            "lina-code-doctrine",
            Some("conserta esse bug"),
            "catalog",
        );
        assert_eq!(msg.intent, "skill.select");
        assert_eq!(
            msg.to, "skill",
            "alvo sentinela — supervisor intercepta por intent"
        );
        let p: serde_json::Value = serde_json::from_str(&msg.payload).expect("payload json");
        assert_eq!(p["skill"], "lina-code-doctrine");
        assert_eq!(p["trigger"], "conserta esse bug");
        assert_eq!(p["source"], "catalog");
        assert!(
            p.get("node").is_none(),
            "node é carimbado SERVER-SIDE, nunca vem do payload"
        );
    }

    /// CRITÉRIO DE ACEITE: a proposta da fábrica é via deep-research e carrega as referências.
    #[test]
    fn propose_envelope_carries_deep_research_proposal() {
        let refs = vec!["https://doc.rust-lang.org/book".to_string()];
        let msg = build_skill_propose_envelope("Terminal J", "senior-architect", &refs);
        assert_eq!(msg.intent, "skill.propose");
        assert_eq!(msg.to, "skill");
        let p: serde_json::Value = serde_json::from_str(&msg.payload).expect("payload json");
        assert_eq!(p["skill_name"], "senior-architect");
        assert_eq!(p["via"], "deep-research");
        assert_eq!(
            p["references"],
            serde_json::json!(["https://doc.rust-lang.org/book"])
        );
    }

    /// CRITÉRIO DE ACEITE: `lina skill check` aponta inline-shell como gate (gated-hard) mesmo
    /// com formato válido.
    #[test]
    fn skill_check_flags_inline_shell_as_gated_hard() {
        let check = skill_check("---\nname: x\n---\nrodar !`rm -rf /`\n");
        assert_eq!(check.format, Ok(()));
        assert_eq!(check.load_class, ActionClass::GatedHard);
    }

    #[test]
    fn skill_check_passes_plain_skill() {
        let check = skill_check("---\nname: doutrina\ndescription: texto\n---\nsó texto\n");
        assert_eq!(check.format, Ok(()));
        assert_eq!(check.load_class, ActionClass::Routine);
    }
}
