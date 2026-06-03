//! W4-1 — **CliDiscovery**: o substrato do check-up de onboarding (T1). Varre o `PATH`
//! por CLIs de IA conhecidos (Claude Code, Codex, Gemini, …), resolve a versão e devolve
//! `{id, version, path}`. É a fonte do evento [`crate::DomainEvent::DiscoveryIndexed`].
//!
//! **ZERO LLM** (invariante #1) e **local-first** (invariante #2 — só lê o `PATH`/roda
//! `--version` localmente; nada sai da máquina). Puro Rust + I/O de processo; sem `unwrap`
//! em caminho de produção. A varredura do `PATH` é fatorada numa função pura
//! ([`find_in_path`]) injetável por teste; a resolução de versão é um passo separado.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// CLIs de IA que o check-up (T1) procura no `PATH` (ids canônicos). Estender = uma linha.
pub const KNOWN_CLIS: &[&str] = &["claude", "codex", "gemini", "opencode", "copilot"];

/// Um CLI de IA encontrado no `PATH`. É o item do payload de `DiscoveryIndexed`.
/// `version: None` quando o binário existe mas `--version` falhou (não engole o achado).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredCli {
    pub id: String,
    pub version: Option<String>,
    pub path: String,
}

/// Separador de entradas do `PATH` por SO (`;` no Windows, `:` no resto).
#[cfg(windows)]
const PATH_SEP: char = ';';
#[cfg(not(windows))]
const PATH_SEP: char = ':';

/// Sufixos de executável a testar por id. No Windows um CLI pode ser `.exe`/`.cmd`/`.bat`
/// (CLIs Node viram `.cmd`); no unix, só o nome cru.
#[cfg(windows)]
const EXE_EXTS: &[&str] = &["", ".exe", ".cmd", ".bat"];
#[cfg(not(windows))]
const EXE_EXTS: &[&str] = &[""];

/// Acha o 1º executável `id` num `PATH` dado. **Puro** (depende só de `path_env` + do FS),
/// para ser testável com um `PATH` sintético sem mexer no ambiente do processo.
#[must_use]
pub fn find_in_path(id: &str, path_env: &str) -> Option<PathBuf> {
    for dir in path_env.split(PATH_SEP) {
        if dir.is_empty() {
            continue;
        }
        for ext in EXE_EXTS {
            let cand = Path::new(dir).join(format!("{id}{ext}"));
            if is_executable_file(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// `true` se `p` é um arquivo com bit de execução (unix) / um arquivo (windows — o bit de
/// exec não existe lá; a extensão em `EXE_EXTS` é o sinal).
#[cfg(unix)]
fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(p) {
        Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}
#[cfg(not(unix))]
fn is_executable_file(p: &Path) -> bool {
    p.is_file()
}

/// Roda `<bin> --version` e devolve a 1ª linha não-vazia trimada. `None` em qualquer falha
/// (binário some, exit≠0, stdout vazio) — sem `unwrap`; o achado fica com `version: None`.
#[must_use]
pub fn query_version(bin: &Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// Descobre os [`KNOWN_CLIS`] num `PATH` dado (injetável por teste). Roda `--version` em
/// cada achado.
#[must_use]
pub fn discover_clis_in(path_env: &str) -> Vec<DiscoveredCli> {
    KNOWN_CLIS
        .iter()
        .filter_map(|id| {
            let path = find_in_path(id, path_env)?;
            let version = query_version(&path);
            Some(DiscoveredCli {
                id: (*id).to_string(),
                version,
                path: path.display().to_string(),
            })
        })
        .collect()
}

/// Descobre os CLIs no `PATH` do processo (default de produção). Lê `PATH` do ambiente.
#[must_use]
pub fn discover_clis() -> Vec<DiscoveredCli> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    discover_clis_in(&path_env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    /// Tempdir único, removido no `Drop`.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-disc-{tag}-{}", Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
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

    /// Escreve um "CLI" falso executável que responde a `--version`. Só unix (bit de exec).
    #[cfg(unix)]
    fn write_fake_cli(dir: &Path, id: &str, version_line: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(id);
        let script = format!("#!/bin/sh\necho '{version_line}'\n");
        std::fs::write(&p, script).expect("escrever cli falso");
        let mut perm = std::fs::metadata(&p).expect("metadata").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&p, perm).expect("chmod");
        p
    }

    /// `find_in_path` acha um executável presente e ignora um diretório sem ele.
    #[cfg(unix)]
    #[test]
    fn find_in_path_locates_executable() {
        let dir = TempDir::new("find");
        let bin = write_fake_cli(dir.path(), "claude", "claude 1.2.3");
        let empty = TempDir::new("empty");
        // PATH = empty:dir → acha em dir.
        let path_env = format!("{}:{}", empty.path().display(), dir.path().display());
        assert_eq!(find_in_path("claude", &path_env), Some(bin));
        // CLI ausente → None.
        assert_eq!(find_in_path("codex", &path_env), None);
    }

    /// Arquivo NÃO-executável não conta como achado (precisa do bit de exec).
    #[cfg(unix)]
    #[test]
    fn find_in_path_ignores_non_executable() {
        let dir = TempDir::new("nonexec");
        std::fs::write(dir.path().join("gemini"), "x").expect("arquivo sem +x");
        assert_eq!(
            find_in_path("gemini", &dir.path().display().to_string()),
            None
        );
    }

    /// `discover_clis_in` resolve id + versão + path do CLI achado e ignora os ausentes.
    #[cfg(unix)]
    #[test]
    fn discover_resolves_version_of_present_cli() {
        let dir = TempDir::new("discover");
        write_fake_cli(dir.path(), "claude", "claude 9.9.9");
        let found = discover_clis_in(&dir.path().display().to_string());
        assert_eq!(found.len(), 1, "só `claude` está presente");
        assert_eq!(found[0].id, "claude");
        assert_eq!(found[0].version.as_deref(), Some("claude 9.9.9"));
        assert!(found[0].path.ends_with("claude"));
    }

    /// PATH vazio → nenhum CLI (não entra em pânico).
    #[test]
    fn discover_empty_path_finds_nothing() {
        assert!(discover_clis_in("").is_empty());
    }
}
