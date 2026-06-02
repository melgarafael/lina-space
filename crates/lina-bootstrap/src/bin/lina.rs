//! Binário **`lina`** — verbo de bootstrap turno-0 (W3-2).
//!
//! `lina whoami --bootstrap` imprime o **ESTADO VIVO** do terminal corrente — papel, skills,
//! vault, **colegas reais**, plano, autonomia, primeiro passo — lendo `.lina/bootstrap.json`
//! (escrito pelo app no cwd do terminal). Com `--bootstrap` emite o JSON do hook `SessionStart`
//! (`hookSpecificOutput.additionalContext`); sem a flag, imprime o bloco legível.

use std::process::ExitCode;

use lina_bootstrap::{BootstrapInput, Bootstrapper};

/// Arquivo de estado, relativo ao cwd do terminal (o app o escreve antes de spawnar o shell).
const INPUT_PATH: &str = ".lina/bootstrap.json";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("whoami") => run_whoami(args.iter().any(|a| a == "--bootstrap")),
        _ => {
            eprintln!("uso: lina whoami [--bootstrap]");
            ExitCode::from(2)
        }
    }
}

/// `hook = true` → JSON do `SessionStart`; `false` → bloco legível.
fn run_whoami(hook: bool) -> ExitCode {
    let input = match load_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH}: {e}");
            return ExitCode::from(1);
        }
    };
    let bs = match Bootstrapper::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("lina: registry de papeis invalido: {e}");
            return ExitCode::from(1);
        }
    };
    if hook {
        println!("{}", bs.whoami_hook_json(&input));
    } else {
        println!("{}", bs.whoami(&input));
    }
    ExitCode::SUCCESS
}

fn load_input() -> Result<BootstrapInput, String> {
    let data = std::fs::read_to_string(INPUT_PATH).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}
