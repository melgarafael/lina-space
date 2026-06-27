//! `lina-pty` — gerenciamento de PTYs cross-platform (story **W0-1**).
//!
//! Abre/escreve/lê/fecha N pseudo-terminais reais (Win/Mac/Linux), com o writer
//! do master de **propriedade do core** — 1 único dono por terminal. Esse
//! single-owner é a base da fila serial de injeção A2A faseada (`CLAUDE.md` §A2A;
//! ver `[[47 - R2 LLM Engineer]]` §1): humano e agente nunca interleavam bytes no
//! mesmo master.
//!
//! ## Gate de aceite (W0-1)
//! `cargo test -p lina-pty` abre 8 PTYs concorrentes, escreve no master de cada
//! (`echo $$`) e confere **PID distinto por PTY**.
//!
//! ## Pragmatismo de plataforma (Onda 0)
//! Esta versão usa `portable-pty` do crates.io — Mac/Linux funcionam direto. O
//! **vendoring** read-only + **patch dos 3 flags ConPTY** + bundle de
//! `conpty.dll`/`OpenConsole.exe` no Windows são da story **W5-3** (não bloqueiam
//! o Mac aqui). O gate cross-SO completo vem com o CI da **W5-6**.

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use portable_pty::{
    native_pty_system, Child, CommandBuilder, MasterPty, PtyPair, PtySize, PtySystem,
};
use thiserror::Error;

/// Erros do subsistema de PTY. Sem `unwrap()` em produção: todo caminho falível
/// vira uma destas variantes (regra do `CLAUDE.md`).
#[derive(Debug, Error)]
pub enum PtyError {
    /// Nenhum PTY registrado sob esse `NodeId`.
    #[error("PTY '{0}' não encontrado")]
    NotFound(String),
    /// `take_writer` foi chamado mais de uma vez para o mesmo terminal.
    #[error("writer do master de '{0}' já foi tomado — o core é o único dono")]
    WriterAlreadyTaken(String),
    /// Falha ao abrir o par master/slave.
    #[error("falha ao abrir o PTY '{0}': {1}")]
    Open(String, String),
    /// Falha ao spawnar o processo no slave.
    #[error("falha ao spawnar comando no PTY '{0}': {1}")]
    Spawn(String, String),
    /// Falha ao redimensionar.
    #[error("falha ao redimensionar o PTY '{0}': {1}")]
    Resize(String, String),
    /// Falha ao clonar o reader do master.
    #[error("falha ao clonar reader do PTY '{0}': {1}")]
    Reader(String, String),
    /// Falha ao tomar o writer do master.
    #[error("falha ao tomar writer do PTY '{0}': {1}")]
    Writer(String, String),
    /// Falha ao encerrar o processo.
    #[error("falha ao encerrar o PTY '{0}': {1}")]
    Kill(String, String),
}

/// Identidade de um nó-terminal no workspace. Mantida desacoplada do `uuid` do
/// core: aqui só precisamos de uma chave estável e legível.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId(String);

impl NodeId {
    /// Cria um `NodeId` a partir de qualquer coisa convertível em `String`.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// A forma textual do id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NodeId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for NodeId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Descrição de um comando a rodar dentro do PTY, montada só a partir de
/// parâmetros (binário, args, env, cwd) — **sem conhecimento de CLI específico**
/// (isto é responsabilidade dos CLI Profiles, não deste crate).
#[derive(Debug, Clone)]
pub struct PtyCommand {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<PathBuf>,
}

impl PtyCommand {
    /// Comando a partir do nome/caminho do binário.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        }
    }

    /// Acrescenta um argumento (builder).
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Acrescenta vários argumentos (builder).
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Define uma variável de ambiente (builder).
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.push((key.into(), val.into()));
        self
    }

    /// Os pares `(chave, valor)` de ENV já carimbados (read-only). Para inspeção/teste do que o
    /// spawn injeta no processo filho (ex.: `LINA_NODE_ID`/`LINA_AUTONOMY`/`LINA_EFFORT` — ADR 0026
    /// / F3-0-4), sem expor o `Vec` mutável.
    #[must_use]
    pub fn envs(&self) -> &[(String, String)] {
        &self.env
    }

    /// Define o diretório de trabalho (builder).
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    fn to_builder(&self) -> CommandBuilder {
        let resolved = resolve_program(self.program.as_str());
        let mut builder = CommandBuilder::new(resolved);
        for arg in &self.args {
            builder.arg(arg.as_str());
        }
        for (key, val) in &self.env {
            builder.env(key.as_str(), val.as_str());
        }
        if let Some(dir) = &self.cwd {
            builder.cwd(dir.as_os_str());
        }
        builder
    }
}

/// No Windows, um `program` nu como `"claude"` é passado direto pro `CreateProcessW` pelo
/// `portable-pty` — e o npm-shim do Claude Code instala TRÊS arquivos em `%AppData%\npm\`:
/// `claude` (shell script Unix, sem extensão), `claude.cmd` (shim Windows que o kernel
/// executa) e `claude.ps1`. A stdlib do Rust no Windows não tenta `.cmd`/`.bat` ao resolver
/// pelo PATH — só `.exe`. Resultado: o kernel recebe o shell script Unix e devolve
/// `os error 193` ("não é um aplicativo Win32 válido"), quebrando a criação de qualquer
/// agente Claude no Lina Windows. Resolvemos AQUI, no momento do `to_builder`, varrendo
/// o PATH na ordem `.cmd → .exe → .bat` e devolvendo o path absoluto resolvido. Se o
/// nome já vem com separador (caller resolveu) ou com extensão executável, passa direto.
#[cfg(windows)]
fn resolve_program(name: &str) -> String {
    if name.contains('\\') || name.contains('/') {
        return name.to_string();
    }
    let lower = name.to_lowercase();
    for ext in &[".exe", ".cmd", ".bat", ".com"] {
        if lower.ends_with(ext) {
            return name.to_string();
        }
    }
    let path_env = std::env::var("PATH").unwrap_or_default();
    for dir in path_env.split(';') {
        if dir.is_empty() {
            continue;
        }
        for ext in &[".cmd", ".exe", ".bat"] {
            let cand = std::path::Path::new(dir).join(format!("{name}{ext}"));
            if cand.is_file() {
                return cand.to_string_lossy().into_owned();
            }
        }
    }
    name.to_string()
}

#[cfg(not(windows))]
fn resolve_program(name: &str) -> String {
    name.to_string()
}

/// Um PTY vivo: o master (de posse do core), o processo filho e o PID.
pub struct PtyHandle {
    node_id: NodeId,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pid: Option<u32>,
    writer_taken: bool,
}

impl PtyHandle {
    /// O id do nó dono deste PTY.
    #[must_use]
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// PID do processo filho (quando o SO reporta).
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Se o writer do master já foi entregue (single-owner).
    #[must_use]
    pub fn writer_taken(&self) -> bool {
        self.writer_taken
    }
}

impl fmt::Debug for PtyHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtyHandle")
            .field("node_id", &self.node_id)
            .field("pid", &self.pid)
            .field("writer_taken", &self.writer_taken)
            .finish()
    }
}

/// Gerencia N PTYs por `NodeId`. Dono dos masters; entrega reader clonável e o
/// writer (uma única vez) por terminal.
pub struct PtyManager {
    pty_system: Box<dyn PtySystem + Send>,
    handles: HashMap<NodeId, PtyHandle>,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyManager {
    /// Cria um gerenciador sobre o PTY nativo do SO.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pty_system: native_pty_system(),
            handles: HashMap::new(),
        }
    }

    /// Abre um PTY, spawna `cmd` no slave e registra o handle sob `node_id`.
    /// Retorna o PID do processo (0 se o SO não reportar).
    pub fn spawn(
        &mut self,
        node_id: impl Into<NodeId>,
        cmd: PtyCommand,
        cols: u16,
        rows: u16,
    ) -> Result<u32, PtyError> {
        let node_id = node_id.into();
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair: PtyPair = self
            .pty_system
            .openpty(size)
            .map_err(|e| PtyError::Open(node_id.to_string(), e.to_string()))?;
        let PtyPair { master, slave } = pair;
        let child = slave
            .spawn_command(cmd.to_builder())
            .map_err(|e| PtyError::Spawn(node_id.to_string(), e.to_string()))?;
        // Solta o slave: assim o master recebe EOF quando o filho sai. Sem isso o
        // reader bloquearia para sempre (o slave aberto mantém a sessão viva).
        drop(slave);

        let pid = child.process_id();
        tracing::debug!(node = %node_id, ?pid, cols, rows, "PTY aberto");

        self.handles.insert(
            node_id.clone(),
            PtyHandle {
                node_id,
                master,
                child,
                pid,
                writer_taken: false,
            },
        );
        Ok(pid.unwrap_or(0))
    }

    /// Redimensiona o PTY de `node_id`.
    pub fn resize(
        &mut self,
        node_id: impl Into<NodeId>,
        cols: u16,
        rows: u16,
    ) -> Result<(), PtyError> {
        let node_id = node_id.into();
        let handle = self
            .handles
            .get(&node_id)
            .ok_or_else(|| PtyError::NotFound(node_id.to_string()))?;
        handle
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Resize(node_id.to_string(), e.to_string()))
    }

    /// Entrega o **único** writer do master de `node_id`. A segunda chamada falha
    /// com [`PtyError::WriterAlreadyTaken`] — o core é o dono soberano do writer.
    pub fn take_writer(
        &mut self,
        node_id: impl Into<NodeId>,
    ) -> Result<Box<dyn Write + Send>, PtyError> {
        let node_id = node_id.into();
        let handle = self
            .handles
            .get_mut(&node_id)
            .ok_or_else(|| PtyError::NotFound(node_id.to_string()))?;
        if handle.writer_taken {
            return Err(PtyError::WriterAlreadyTaken(node_id.to_string()));
        }
        let writer = handle
            .master
            .take_writer()
            .map_err(|e| PtyError::Writer(node_id.to_string(), e.to_string()))?;
        handle.writer_taken = true;
        Ok(writer)
    }

    /// Devolve um reader **clonável** do master de `node_id` (vários leitores ok).
    pub fn clone_reader(
        &self,
        node_id: impl Into<NodeId>,
    ) -> Result<Box<dyn Read + Send>, PtyError> {
        let node_id = node_id.into();
        let handle = self
            .handles
            .get(&node_id)
            .ok_or_else(|| PtyError::NotFound(node_id.to_string()))?;
        handle
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Reader(node_id.to_string(), e.to_string()))
    }

    /// Encerra o processo de `node_id`: **SIGTERM** primeiro (unix) e, se não sair
    /// dentro de `timeout`, **SIGKILL**. No Windows não há SIGTERM (ver W5-3): cai
    /// direto no `kill()` (TerminateProcess).
    pub fn kill(&mut self, node_id: impl Into<NodeId>, timeout: Duration) -> Result<(), PtyError> {
        let node_id = node_id.into();
        let handle = self
            .handles
            .get_mut(&node_id)
            .ok_or_else(|| PtyError::NotFound(node_id.to_string()))?;

        #[cfg(unix)]
        if let Some(pid) = handle.pid {
            // SAFETY: FFI direto; `pid` é de um filho que nós spawnamos.
            let _ = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            let start = Instant::now();
            while start.elapsed() < timeout {
                match handle.child.try_wait() {
                    Ok(Some(_)) => {
                        tracing::debug!(node = %node_id, "encerrou via SIGTERM");
                        return Ok(());
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
        }
        #[cfg(not(unix))]
        let _ = timeout;

        handle
            .child
            .kill()
            .map_err(|e| PtyError::Kill(node_id.to_string(), e.to_string()))?;
        let _ = handle.child.wait();
        tracing::debug!(node = %node_id, "encerrado (forçado)");
        Ok(())
    }

    /// PID do processo de `node_id` (se houver).
    pub fn pid(&self, node_id: impl Into<NodeId>) -> Option<u32> {
        self.handles.get(&node_id.into()).and_then(PtyHandle::pid)
    }

    /// Se há um PTY registrado para `node_id`.
    pub fn contains(&self, node_id: impl Into<NodeId>) -> bool {
        self.handles.contains_key(&node_id.into())
    }

    /// Remove (e devolve) o handle, fechando o master ao soltá-lo.
    pub fn remove(&mut self, node_id: impl Into<NodeId>) -> Option<PtyHandle> {
        self.handles.remove(&node_id.into())
    }

    /// Quantidade de PTYs vivos no gerenciador.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// Se não há nenhum PTY registrado.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }
}
