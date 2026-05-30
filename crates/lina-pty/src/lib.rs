//! `lina-pty` — gerenciamento de PTYs cross-platform (story **W0-1**).
//!
//! Abre/escreve/lê/fecha N pseudo-terminais reais (Win/Mac/Linux), com o writer
//! do master de **propriedade do core**. Base de `portable-pty` (vendorizado,
//! patch dos 3 flags ConPTY + bundle `conpty.dll`/`OpenConsole.exe` no Windows).
//!
//! Gate de aceite (W0-1): `cargo test -p lina-pty` abre 8 PTYs concorrentes,
//! escreve no master de cada e confere PID distinto por PTY nos 3 SOs.

#![allow(dead_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("PTY ainda não implementado (W0-1)")]
    NotImplemented,
}

// TODO(W0-1): PtyManager { spawn(node, CommandBuilder, cwd, cols, rows) -> PtyHandle,
//   resize, kill (SIGTERM->SIGKILL c/ timeout), take_writer() (1 dono), reader clonável }.
