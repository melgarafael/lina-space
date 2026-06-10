//! **F1-3 (ACHADO-1 do gate) — a safra COMPLETA de skills da Lina, embutida.**
//!
//! O instalador antigo levava SÓ `lina-agent-bus`: as 11 skills da F1-3 (orquestração,
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

    /// O instalador cria as 11 skills (+`references/`) sob o root dado.
    #[test]
    fn installs_all_skills_with_references() {
        let root = std::env::temp_dir().join(format!("lina-skills-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = install_skills_into(&root).expect("instala");
        assert_eq!(dirs.len(), 11, "as 11 skills da 1ª safra");
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
}
