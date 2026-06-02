//! Binário **`lina`** — os verbos do agente dentro do Lina Space.
//!
//! - `lina whoami [--bootstrap]` (W3-2): imprime o **ESTADO VIVO** do terminal corrente — papel,
//!   skills, vault, **colegas reais**, plano, autonomia — lendo `.lina/bootstrap.json` (escrito
//!   pelo app no cwd do terminal). Com `--bootstrap`, emite o JSON do hook `SessionStart`.
//! - `lina ask @<alvo> "<msg>" [--await] [--intent X] [--role PAPEL]` (W3-4): monta a mensagem
//!   canônica `lina/msg@1` (`from` = este terminal) e a **deposita na mailbox** (`.lina/outbox/`),
//!   que o supervisor (no app) observa, roteia e injeta no PTY do alvo.
//! - `lina handshake` (W3-4): registra presença (ping na mailbox, **0 broadcast**) e imprime os
//!   colegas do workspace (de `.lina/agents.json` ou do roster do bootstrap).

use std::path::PathBuf;
use std::process::ExitCode;

use lina_bootstrap::{BootstrapInput, Bootstrapper};
use lina_core::{MailMessage, Mailbox};

/// Arquivo de estado, relativo ao cwd do terminal (o app o escreve antes de spawnar o shell).
const INPUT_PATH: &str = ".lina/bootstrap.json";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("whoami") => run_whoami(args.iter().any(|a| a == "--bootstrap")),
        Some("ask") => run_ask(&args[1..]),
        Some("handshake") => run_handshake(),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "uso:\n  lina whoami [--bootstrap]\n  lina ask @<alvo> \"<msg>\" [--await] [--intent ask|handoff|broadcast|...] [--role PAPEL]\n  lina handshake"
    );
}

/// Raiz da mailbox: `LINA_HOME` (o app aponta para o `.lina/` compartilhado do workspace) ou, como
/// fallback standalone, o `.lina/` do cwd.
fn mailbox_root() -> PathBuf {
    std::env::var_os("LINA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".lina"))
}

fn load_input() -> Result<BootstrapInput, String> {
    let data = std::fs::read_to_string(INPUT_PATH).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
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

/// `lina ask` — monta a `MailMessage` e a deposita na mailbox.
fn run_ask(args: &[String]) -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut intent = String::from("ask");
    let mut await_reply = false;
    let mut role: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--await" => await_reply = true,
            "--intent" => {
                i += 1;
                match args.get(i) {
                    Some(v) => intent = v.clone(),
                    None => {
                        eprintln!("lina: --intent exige um valor");
                        return ExitCode::from(2);
                    }
                }
            }
            "--role" => {
                i += 1;
                match args.get(i) {
                    Some(v) => role = Some(v.clone()),
                    None => {
                        eprintln!("lina: --role exige um valor (papel)");
                        return ExitCode::from(2);
                    }
                }
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    // Alvo + mensagem: com --role, o 1º positional é a mensagem; senão é alvo + mensagem.
    let (to, payload) = if let Some(r) = role {
        match positional.into_iter().next() {
            Some(msg) => (format!("role:{r}"), msg),
            None => {
                usage();
                return ExitCode::from(2);
            }
        }
    } else {
        let mut it = positional.into_iter();
        match (it.next(), it.next()) {
            (Some(target), Some(msg)) => (target, msg),
            _ => {
                usage();
                return ExitCode::from(2);
            }
        }
    };

    let from = match load_input() {
        Ok(i) => i.terminal_name,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'): {e}");
            return ExitCode::from(1);
        }
    };

    let mut msg = MailMessage::new(from, to, intent, payload);
    if await_reply {
        msg = msg.awaiting();
    }
    let mailbox = Mailbox::new(mailbox_root());
    match mailbox.enqueue(&msg) {
        Ok(()) => {
            println!("ok: mensagem {} enfileirada para {}", msg.id, msg.to);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
    }
}

/// `lina handshake` — registra presença (0 broadcast) e lista os colegas do workspace.
fn run_handshake() -> ExitCode {
    let input = match load_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH}: {e}");
            return ExitCode::from(1);
        }
    };
    let from = input.terminal_name.clone();
    let mailbox = Mailbox::new(mailbox_root());

    // Ping de presença na mailbox: intent=handshake, to=* — o supervisor REGISTRA a entrada
    // (`agent.joined`) mas NÃO injeta em ninguém (0 broadcast no boot, design §2/§4).
    let ping = MailMessage::new(
        &from,
        "*",
        "handshake",
        format!("{from} entrou no workspace"),
    );
    if let Err(e) = mailbox.enqueue(&ping) {
        eprintln!("lina: falha ao registrar presenca: {e}");
        return ExitCode::from(1);
    }

    // Colegas: do agents.json (escrito pelo supervisor) se houver; senão do roster do bootstrap.
    let colegas: Vec<String> = match mailbox.read_agents() {
        Ok(agents) if !agents.is_empty() => agents
            .into_iter()
            .filter(|a| a.name != from)
            .map(|a| format!("{} ({})", a.name, a.role.unwrap_or_else(|| "—".into())))
            .collect(),
        _ => input
            .roster
            .iter()
            .filter(|n| **n != from)
            .cloned()
            .collect(),
    };

    println!("=== Lina Space HANDSHAKE ===");
    println!("Voce: {from}");
    if colegas.is_empty() {
        println!("COLEGAS: (nenhum colega no workspace ainda)");
    } else {
        println!("COLEGAS: {}", colegas.join("; "));
    }
    println!("(NAO responda — handshake e informativo, nao interrogativo)");
    println!("=== FIM HANDSHAKE ===");
    ExitCode::SUCCESS
}
