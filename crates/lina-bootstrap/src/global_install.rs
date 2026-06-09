//! **Lina universal — disponibilidade GLOBAL das capacidades em TODO CLI/terminal.**
//!
//! Por que existe: a ficha por-agente (`CLAUDE.md`/`AGENTS.md`/`GEMINI.md` no cwd gerenciado)
//! só torna AGENTES Lina-aware. Um terminal PURO (shell no `$HOME`) ou um `claude`/`codex`/`gemini`
//! rodado em qualquer pasta ficava CEGO pro Lina. O usuário é leigo — não pode ter de escolher
//! "novo agente" vs "novo terminal" pra ter as capacidades. **A base do Lina:** as skills + a
//! doutrina + os comandos `lina` SEMPRE acessíveis, em tudo.
//!
//! Como: instala a **doutrina global auto-gated** (`assets/lina-doctrine/GLOBAL.md`) no config
//! GLOBAL de cada CLI (memória lida em TODA sessão, qualquer cwd) + a skill `lina-agent-bus` na
//! pasta de skills global. **ADITIVO + IDEMPOTENTE:** o conteúdo do usuário é preservado; a doutrina
//! mora num bloco marcado `LINA:START..LINA:END` (re-rodar só atualiza o bloco). **AUTO-GATED:** a
//! doutrina manda checar `lina whoami` antes de valer — zero efeito no uso do CLI fora do Lina.
//!
//! Não instala ENFORCEMENT (guard/custódia hooks) globalmente — isso continua por-agente-gerenciado
//! (onde o Lina é dono do cwd); enforcement no uso global do CLI do usuário seria overreach.

use std::path::{Path, PathBuf};

/// Doutrina global (auto-gated) — fonte da verdade embutida.
const GLOBAL_DOCTRINE: &str = include_str!("../../../assets/lina-doctrine/GLOBAL.md");
/// A skill SEMPRE (comunicação A2A) — embutida; copiada pra pasta de skills global de cada CLI.
const AGENT_BUS_SKILL: &str = include_str!("../../../assets/lina-skills/lina-agent-bus/SKILL.md");

const BLOCK_START: &str = "<!-- LINA:START -->";
const BLOCK_END: &str = "<!-- LINA:END -->";

/// O que foi tocado numa execução (para log honesto do app).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GlobalInstallReport {
    /// Arquivos de memória/doutrina criados ou atualizados.
    pub doctrine_files: Vec<PathBuf>,
    /// Pastas de skill instaladas/atualizadas.
    pub skill_dirs: Vec<PathBuf>,
}

/// Um CLI alvo: onde mora a memória global e a pasta de skills (relativo ao `home`).
struct CliTarget {
    /// Subpasta de config do CLI (ex.: `.claude`).
    config_dir: &'static str,
    /// Nome do arquivo de memória global lido em toda sessão (ex.: `CLAUDE.md`).
    memory_file: &'static str,
}

/// Os CLIs suportados (inv#3 multi-CLI). Codex usa `AGENTS.md`; Gemini, `GEMINI.md`.
const TARGETS: [CliTarget; 3] = [
    CliTarget {
        config_dir: ".claude",
        memory_file: "CLAUDE.md",
    },
    CliTarget {
        config_dir: ".codex",
        memory_file: "AGENTS.md",
    },
    CliTarget {
        config_dir: ".gemini",
        memory_file: "GEMINI.md",
    },
];

/// Garante que o Lina está disponível GLOBALMENTE para todo CLI suportado sob `home`.
/// Aditivo + idempotente. Best-effort POR CLI: falha de I/O num CLI não impede os outros
/// (o `Err` agregado é devolvido só para log; o app NÃO deve abortar o boot por isto).
///
/// # Errors
/// Devolve o PRIMEIRO erro de I/O encontrado (após tentar todos os alvos), para o app logar.
pub fn ensure_lina_globally_available(home: &Path) -> std::io::Result<GlobalInstallReport> {
    let mut report = GlobalInstallReport::default();
    let mut first_err: Option<std::io::Error> = None;

    for t in &TARGETS {
        let config = home.join(t.config_dir);
        // Doutrina na memória global (bloco marcado, aditivo).
        match upsert_marked_block(&config.join(t.memory_file), GLOBAL_DOCTRINE) {
            Ok(()) => report.doctrine_files.push(config.join(t.memory_file)),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
        // Skill `lina-agent-bus` na pasta de skills do CLI.
        let skill_dir = config.join("skills").join("lina-agent-bus");
        match install_skill(&skill_dir) {
            Ok(()) => report.skill_dirs.push(skill_dir),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }

    match first_err {
        Some(e) => Err(e),
        None => Ok(report),
    }
}

/// Insere/atualiza o bloco marcado `LINA:START..LINA:END` em `path`, preservando todo o resto.
/// - arquivo não existe → cria só com o bloco;
/// - existe sem o bloco → ANEXA o bloco no fim (conteúdo do usuário intacto);
/// - existe com o bloco → SUBSTITUI só o miolo do bloco (idempotente; re-roda sem duplicar).
fn upsert_marked_block(path: &Path, doctrine: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let block = format!("{BLOCK_START}\n{}\n{BLOCK_END}", doctrine.trim_end());
    let existing = std::fs::read_to_string(path).unwrap_or_default();

    let next = match (existing.find(BLOCK_START), existing.find(BLOCK_END)) {
        (Some(s), Some(e)) if e > s => {
            // Substitui o bloco existente (do START até o fim do END).
            let end = e + BLOCK_END.len();
            format!("{}{}{}", &existing[..s], block, &existing[end..])
        }
        _ if existing.trim().is_empty() => format!("{block}\n"),
        _ => {
            // Anexa preservando o conteúdo do usuário, com uma linha em branco de separação.
            let sep = if existing.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{existing}{sep}{block}\n")
        }
    };

    if next == existing {
        return Ok(()); // já idêntico → não reescreve (evita churn de mtime)
    }
    write_atomic(path, &next)
}

/// Instala a skill `lina-agent-bus` em `skill_dir/SKILL.md` (cria a pasta; sobrescreve só o SKILL.md
/// se mudou). Skills são aditivas por natureza (pasta própria) — nunca colide com skills do usuário.
fn install_skill(skill_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(skill_dir)?;
    let skill_md = skill_dir.join("SKILL.md");
    if std::fs::read_to_string(&skill_md).unwrap_or_default() == AGENT_BUS_SKILL {
        return Ok(()); // idêntico → não reescreve
    }
    write_atomic(&skill_md, AGENT_BUS_SKILL)
}

/// Escrita atômica (tmp + rename) — robusta a crash/concorrência.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lina-global-{}-{tag}", std::process::id()))
    }

    #[test]
    fn installs_doctrine_and_skill_for_all_three_clis() {
        let home = temp_home("all");
        let _ = std::fs::remove_dir_all(&home);
        let report = ensure_lina_globally_available(&home).expect("install");

        for (cfg, mem) in [
            (".claude", "CLAUDE.md"),
            (".codex", "AGENTS.md"),
            (".gemini", "GEMINI.md"),
        ] {
            let mem_path = home.join(cfg).join(mem);
            let body = std::fs::read_to_string(&mem_path).expect("memória");
            assert!(
                body.contains(BLOCK_START) && body.contains(BLOCK_END),
                "{cfg}: bloco marcado"
            );
            assert!(
                body.contains("lina whoami"),
                "{cfg}: doutrina auto-gated presente"
            );
            let skill = home
                .join(cfg)
                .join("skills")
                .join("lina-agent-bus")
                .join("SKILL.md");
            assert!(skill.exists(), "{cfg}: skill instalada");
            assert!(report.doctrine_files.contains(&mem_path));
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn preserves_user_content_and_is_idempotent() {
        let home = temp_home("preserve");
        let _ = std::fs::remove_dir_all(&home);
        let claude_md = home.join(".claude").join("CLAUDE.md");
        std::fs::create_dir_all(claude_md.parent().unwrap()).unwrap();
        let user = "# Minhas instruções pessoais\nSempre responda em pt-br.\n";
        std::fs::write(&claude_md, user).unwrap();

        // 1ª instalação: ANEXA o bloco, preserva o conteúdo do usuário.
        ensure_lina_globally_available(&home).expect("1ª");
        let after1 = std::fs::read_to_string(&claude_md).unwrap();
        assert!(
            after1.contains(user.trim()),
            "conteúdo do usuário preservado"
        );
        assert!(after1.contains(BLOCK_START), "bloco anexado");
        assert_eq!(after1.matches(BLOCK_START).count(), 1, "um único bloco");

        // 2ª instalação (idempotente): NÃO duplica o bloco, NÃO perde o usuário.
        ensure_lina_globally_available(&home).expect("2ª");
        let after2 = std::fs::read_to_string(&claude_md).unwrap();
        assert_eq!(
            after2.matches(BLOCK_START).count(),
            1,
            "ainda um único bloco (idempotente)"
        );
        assert!(after2.contains(user.trim()), "usuário ainda preservado");
        assert_eq!(
            after1, after2,
            "2ª rodada não muda nada (controle de não-vacuosidade)"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn updates_block_in_place_when_doctrine_changes() {
        let home = temp_home("update");
        let _ = std::fs::remove_dir_all(&home);
        let claude_md = home.join(".claude").join("CLAUDE.md");
        std::fs::create_dir_all(claude_md.parent().unwrap()).unwrap();
        // Simula um bloco LINA antigo + conteúdo do usuário em volta.
        let stale = format!(
            "topo do usuário\n{BLOCK_START}\nDOUTRINA ANTIGA\n{BLOCK_END}\nrodapé do usuário\n"
        );
        std::fs::write(&claude_md, &stale).unwrap();

        ensure_lina_globally_available(&home).expect("update");
        let after = std::fs::read_to_string(&claude_md).unwrap();
        assert!(
            after.contains("topo do usuário") && after.contains("rodapé do usuário"),
            "moldura do usuário intacta"
        );
        assert!(
            !after.contains("DOUTRINA ANTIGA"),
            "miolo antigo substituído"
        );
        assert!(after.contains("lina whoami"), "doutrina nova no lugar");
        assert_eq!(after.matches(BLOCK_START).count(), 1, "um único bloco");
        let _ = std::fs::remove_dir_all(&home);
    }
}
