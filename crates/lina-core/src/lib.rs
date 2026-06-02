//! `lina-core` — o cérebro framework-agnóstico do Lina Space.
//!
//! Reúne o substrato da Onda 0 (Trilho A): pty-host isolado (W0-3),
//! Workspace Bus / Supervisor (W0-4), Event Store (W0-5), recuperação
//! pós-crash (W0-6), Secret Vault (W0-7), CLI Profiles (W0-8), entrega A2A
//! faseada (W0-9) e contrato de fim-de-resposta (W0-10).
//!
//! **Invariante:** este crate NÃO importa tipos de framework de UI. Fala com a
//! superfície apenas via `lina_host::UiHost`. (Âncora de continuidade — `CLAUDE.md`.)

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lina_pty::PtyError;
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Re-exports da camada de PTY/VT para abrir terminais e ler o grid (gate E2E inclusive).
pub use lina_cli_profiles::CliProfile;
/// Re-exports do contrato de UI para os consumidores do core (e o host headless de teste).
pub use lina_host::{BusTarget, HeadlessUiHost, HostEvent, NodeId, NodeKind, UiHost};
pub use lina_pty::{PtyCommand, PtyManager};
pub use lina_vt::{AlacrittyBackend, VtBackend, VtCell, VtCursor, VtRgb, VtScreen};

/// Event Store (W0-5) + recuperação pós-crash visível (W0-6).
mod events;
pub use events::{
    apply, DomainEvent, EventRecord, EventStore, ProjectedNode, ProjectedState, StoreError,
};

/// Entrega A2A faseada (W0-9) + contrato de fim-de-resposta (W0-10).
mod a2a;
pub use a2a::{
    build_paste, deliver_a2a, sanitize_paste, A2aError, DeliveryOutcome, EndDetector, EndOutcome,
    EndResult, GridSense, InjectPolicy,
};

/// W3-4: mailbox de arquivo (`.lina/`) — contrato `lina/msg@1` CLI↔supervisor.
mod mailbox;
pub use mailbox::{
    now_ms, parse_target, render_message_block, AgentPresence, MailMessage, Mailbox, TargetSpec,
    MAIL_SCHEMA_V1,
};

/// W3-4: roteador do supervisor (pipeline `route_message` com guardrails 0-4).
mod router;
pub use router::{
    AutonomyLevel, PlanOpError, RouteOutcome, Router, RouterConfig, DEDUPE_WINDOW_MS,
    DELEGATION_BUDGET, FANOUT_GATE, MAX_DEPTH,
};

/// W3-5: plano compartilhado (`.lina/plan.md`) — projeção event-sourced, escritor único supervisor.
mod plan;
pub use plan::{ItemState, Plan, PlanError, PlanItem, PLAN_SCHEMA_V1};

/// Envelope canônico de mensagem A2A (versionado — âncora de continuidade).
///
/// Campos definidos desde já (opcionais até o supervisor preenchê-los), para
/// evitar contrato fragmentado por feature (W0-4 + W3-3 + W3-7).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct A2aEnvelope {
    pub id: String,
    pub root_cause_id: Option<String>,
    pub from: NodeId,
    pub to: Recipient,
    pub intent: Option<String>,
    pub hops: u8,
    pub await_reply: bool,
    /// Caminho percorrido — base da detecção de loop de mensagem. Começa em `[from]`.
    pub trace: Vec<NodeId>,
    /// Hop-count restante (anti-loop). Ao chegar a 0, a mensagem é dropada.
    pub ttl: u8,
}

/// TTL default de um envelope (hop-count máximo antes de dropar por loop).
pub const DEFAULT_TTL: u8 = 6;

impl A2aEnvelope {
    /// Novo envelope com id `msg_<UUID v7>` (ordenável por tempo, semântica ULID) e
    /// `trace` iniciado em `from`.
    #[must_use]
    pub fn new(from: NodeId, to: Recipient, intent: Option<String>) -> Self {
        Self {
            id: format!("msg_{}", Uuid::now_v7()),
            root_cause_id: None,
            from,
            to,
            intent,
            hops: 0,
            await_reply: false,
            trace: vec![from],
            ttl: DEFAULT_TTL,
        }
    }

    /// Marca o envelope como bloqueante (espera `reply` correlacionado).
    #[must_use]
    pub fn awaiting(mut self) -> Self {
        self.await_reply = true;
        self
    }
}

/// Endereçamento de mensagem: nó específico, papel (late-bound) ou broadcast.
#[derive(Debug, Clone)]
pub enum Recipient {
    Node(NodeId),
    Role(String),
    Broadcast,
}

// ───────────────────────────── pty-host isolado (W0-3) ─────────────────────────────
//
// Cada terminal lê numa thread dedicada (fora de qualquer thread de UI futura). A
// thread drena o PTY (`lina-pty`), aplica os bytes ao `VtBackend` (`lina-vt`) em
// lotes e emite `GridDelta` (linhas sujas). Três proteções, no estilo VS Code:
//   1. event-batching — coalesce até ~64 KiB ou ~4 ms antes de aplicar.
//   2. flow-control high/low-watermark — para de drenar quando há bytes demais
//      ainda não consumidos pelo core; o PTY enche e o produtor (a CLI) bloqueia.
//   3. `catch_unwind` por loop — um panic mata o PAINEL (estado `Crashed`), nunca
//      o processo; os outros terminais seguem lendo.
// Refs: [[20 - Arquitetura Tecnica]] §1.3, [[47 - R2 LLM Engineer]] §1.

/// Erros do pty-host.
#[derive(Debug, Error)]
pub enum PtyHostError {
    /// Nenhum terminal registrado sob esse `NodeId`.
    #[error("terminal {0} não encontrado")]
    NotFound(NodeId),
    /// Erro vindo da camada de PTY (`lina-pty`).
    #[error("erro de PTY: {0}")]
    Pty(#[from] PtyError),
    /// Erro de I/O ao escrever no master ou ao subir a thread de leitura.
    #[error("erro de I/O no terminal {0}: {1}")]
    Io(NodeId, String),
}

/// Estado observável de um painel-terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalState {
    /// Lendo normalmente.
    Running,
    /// O loop de leitura entrou em panic e foi contido — o processo seguiu vivo.
    Crashed,
    /// O processo do terminal saiu (EOF) ou foi encerrado.
    Exited,
}

/// Configuração de batching e flow-control do pty-host.
#[derive(Debug, Clone, Copy)]
pub struct PtyHostConfig {
    /// Acima deste total de bytes não-consumidos (por terminal), o reader **para**
    /// de drenar o PTY (backpressure ao produtor) — o teto de RAM que gerenciamos.
    pub high_watermark: usize,
    /// Só retoma a drenagem quando os bytes não-consumidos caem até aqui (histerese).
    pub low_watermark: usize,
    /// Flush do lote quando ele atinge este tamanho.
    pub max_batch_bytes: usize,
    /// Flush do lote quando este tempo passa desde o último flush.
    pub max_batch_time: Duration,
    /// Tamanho de cada `read` do PTY.
    pub read_buf: usize,
}

impl Default for PtyHostConfig {
    fn default() -> Self {
        Self {
            high_watermark: 1 << 20,    // 1 MiB
            low_watermark: 256 * 1024,  // 256 KiB
            max_batch_bytes: 64 * 1024, // 64 KiB
            max_batch_time: Duration::from_millis(4),
            read_buf: 8 * 1024,
        }
    }
}

/// Delta emitido a cada flush: as linhas sujas do grid + os bytes que ele
/// representa (para o consumidor dar `ack` e liberar o flow-control). A ponte
/// para a UI (`HostEvent::GridDelta`) é ligada na W0-11; aqui o canal é o produto.
#[derive(Debug, Clone)]
pub struct GridDelta {
    pub node: NodeId,
    pub rows: Vec<usize>,
    pub bytes: usize,
    pub seq: u64,
}

/// Métricas observáveis por terminal (W0-3: bytes/s, ocupação do watermark, batches).
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub bytes_read: u64,
    pub batches: u64,
    pub inflight: u64,
    pub state: TerminalState,
}

#[derive(Default)]
struct MetricsInner {
    bytes_read: AtomicU64,
    batches: AtomicU64,
}

/// Estado compartilhado entre o pty-host e a thread de leitura de um terminal.
struct TermShared {
    vt: Mutex<Box<dyn VtBackend>>,
    /// Bytes aplicados-mas-ainda-não-consumidos (knob do flow-control).
    inflight: AtomicU64,
    stop: AtomicBool,
    panic_inject: AtomicBool,
    state: Mutex<TerminalState>,
    metrics: MetricsInner,
}

struct Terminal {
    shared: Arc<TermShared>,
    writer: Mutex<Box<dyn Write + Send>>,
    join: Option<JoinHandle<()>>,
    pty_key: lina_pty::NodeId,
}

/// O host isolado de PTYs. Dono de um `PtyManager` e de uma thread de leitura por
/// terminal. Emite `GridDelta` num canal; o consumidor faz `ack` para liberar o
/// flow-control.
pub struct PtyHost {
    manager: PtyManager,
    terminals: HashMap<NodeId, Terminal>,
    config: PtyHostConfig,
    delta_tx: Sender<GridDelta>,
    delta_rx: Option<Receiver<GridDelta>>,
    seq: Arc<AtomicU64>,
}

impl Default for PtyHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyHost {
    /// Host com a configuração padrão.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(PtyHostConfig::default())
    }

    /// Host com batching/flow-control customizados.
    #[must_use]
    pub fn with_config(config: PtyHostConfig) -> Self {
        let (delta_tx, delta_rx) = mpsc::channel();
        Self {
            manager: PtyManager::new(),
            terminals: HashMap::new(),
            config,
            delta_tx,
            delta_rx: Some(delta_rx),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Toma o receptor de `GridDelta` (uma vez). O consumidor o drena e dá `ack`.
    pub fn take_delta_receiver(&mut self) -> Option<Receiver<GridDelta>> {
        self.delta_rx.take()
    }

    /// Abre um terminal: spawna `cmd` num PTY, sobe a thread de leitura dedicada e
    /// devolve o `NodeId` (UUID v7, ordenável por tempo — âncora de continuidade).
    pub fn spawn(&mut self, cmd: PtyCommand, cols: u16, rows: u16) -> Result<NodeId, PtyHostError> {
        let node = Uuid::now_v7();
        let pty_key = lina_pty::NodeId::new(node.to_string());

        self.manager.spawn(pty_key.clone(), cmd, cols, rows)?;
        let writer = self.manager.take_writer(pty_key.clone())?;
        let reader = self.manager.clone_reader(pty_key.clone())?;

        let backend: Box<dyn VtBackend> = Box::new(AlacrittyBackend::new(cols, rows));
        let shared = Arc::new(TermShared {
            vt: Mutex::new(backend),
            inflight: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            panic_inject: AtomicBool::new(false),
            state: Mutex::new(TerminalState::Running),
            metrics: MetricsInner::default(),
        });

        let join = {
            let shared = Arc::clone(&shared);
            let config = self.config;
            let delta_tx = self.delta_tx.clone();
            let seq = Arc::clone(&self.seq);
            thread::Builder::new()
                .name(format!("lina-pty-reader-{node}"))
                .spawn(move || run_reader(reader, shared, config, delta_tx, node, seq))
                .map_err(|e| PtyHostError::Io(node, e.to_string()))?
        };

        self.terminals.insert(
            node,
            Terminal {
                shared,
                writer: Mutex::new(writer),
                join: Some(join),
                pty_key,
            },
        );
        Ok(node)
    }

    /// Escreve bytes no master do terminal (o core é o único dono do writer).
    pub fn write(&self, node: NodeId, bytes: &[u8]) -> Result<(), PtyHostError> {
        let term = self
            .terminals
            .get(&node)
            .ok_or(PtyHostError::NotFound(node))?;
        let mut writer = lock(&term.writer);
        writer
            .write_all(bytes)
            .map_err(|e| PtyHostError::Io(node, e.to_string()))?;
        writer
            .flush()
            .map_err(|e| PtyHostError::Io(node, e.to_string()))?;
        Ok(())
    }

    /// Redimensiona o PTY e o grid do terminal.
    pub fn resize(&mut self, node: NodeId, cols: u16, rows: u16) -> Result<(), PtyHostError> {
        let pty_key = self
            .terminals
            .get(&node)
            .ok_or(PtyHostError::NotFound(node))?
            .pty_key
            .clone();
        self.manager.resize(pty_key, cols, rows)?;
        if let Some(term) = self.terminals.get(&node) {
            lock(&term.shared.vt).resize(cols, rows);
        }
        Ok(())
    }

    /// O consumidor confirma que processou `bytes` deste terminal — libera o
    /// flow-control (saturating, nunca abaixo de zero).
    pub fn ack(&self, node: NodeId, bytes: u64) -> Result<(), PtyHostError> {
        let term = self
            .terminals
            .get(&node)
            .ok_or(PtyHostError::NotFound(node))?;
        let _ = term
            .shared
            .inflight
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(bytes))
            });
        Ok(())
    }

    /// Bytes aplicados-mas-não-consumidos do terminal (ocupação do watermark).
    #[must_use]
    pub fn inflight(&self, node: NodeId) -> Option<u64> {
        self.terminals
            .get(&node)
            .map(|t| t.shared.inflight.load(Ordering::Relaxed))
    }

    /// Estado atual do painel.
    #[must_use]
    pub fn state(&self, node: NodeId) -> Option<TerminalState> {
        self.terminals.get(&node).map(|t| *lock(&t.shared.state))
    }

    /// Snapshot de métricas do terminal.
    #[must_use]
    pub fn metrics(&self, node: NodeId) -> Option<Metrics> {
        self.terminals.get(&node).map(|t| Metrics {
            bytes_read: t.shared.metrics.bytes_read.load(Ordering::Relaxed),
            batches: t.shared.metrics.batches.load(Ordering::Relaxed),
            inflight: t.shared.inflight.load(Ordering::Relaxed),
            state: *lock(&t.shared.state),
        })
    }

    /// Acesso somente-leitura ao grid parseado do terminal (ex.: `last_nonempty_line`
    /// para a detecção de prompt-pronto do A2A — ler o grid, não OCR de pixel).
    #[must_use]
    pub fn with_grid<R>(&self, node: NodeId, f: impl FnOnce(&dyn VtBackend) -> R) -> Option<R> {
        let term = self.terminals.get(&node)?;
        let guard = lock(&term.shared.vt);
        Some(f(&**guard))
    }

    /// Quantos terminais estão em `Running`.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.terminals
            .values()
            .filter(|t| *lock(&t.shared.state) == TerminalState::Running)
            .count()
    }

    /// Encerra o terminal: sinaliza stop, manda SIGTERM→SIGKILL ao processo e dá
    /// join na thread de leitura.
    pub fn kill(&mut self, node: NodeId) -> Result<(), PtyHostError> {
        let pty_key = {
            let term = self
                .terminals
                .get(&node)
                .ok_or(PtyHostError::NotFound(node))?;
            term.shared.stop.store(true, Ordering::Relaxed);
            term.pty_key.clone()
        };
        let kill_res = self.manager.kill(pty_key, Duration::from_secs(2));

        if let Some(mut term) = self.terminals.remove(&node) {
            if let Some(join) = term.join.take() {
                let _ = join.join();
            }
            let mut st = lock(&term.shared.state);
            if *st == TerminalState::Running {
                *st = TerminalState::Exited;
            }
        }
        kill_res.map_err(PtyHostError::from)
    }

    /// **Fault injection (testes/diagnóstico):** força um panic no próximo passo do
    /// loop do terminal, para provar que o `catch_unwind` contém o dano.
    pub(crate) fn inject_panic(&self, node: NodeId) -> Result<(), PtyHostError> {
        let term = self
            .terminals
            .get(&node)
            .ok_or(PtyHostError::NotFound(node))?;
        term.shared.panic_inject.store(true, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for PtyHost {
    fn drop(&mut self) {
        let nodes: Vec<NodeId> = self.terminals.keys().copied().collect();
        for node in nodes {
            let _ = self.kill(node);
        }
    }
}

/// Recupera o guard mesmo se o mutex estiver envenenado (um terminal pode ter
/// entrado em panic segurando o lock; queremos seguir vivos, não propagar).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Normaliza um nome para o casamento tolerante (`node_by_name`): tira espaços e o `@` inicial,
/// minúsculas. `"@Dev Backend"` → `"dev backend"`.
fn normalize_name(s: &str) -> String {
    s.trim().trim_start_matches('@').trim().to_ascii_lowercase()
}

/// Wrapper da thread de leitura: roda o loop dentro de `catch_unwind` e marca o
/// estado final (Exited em saída limpa, Crashed se o loop entrou em panic).
fn run_reader(
    mut reader: Box<dyn Read + Send>,
    shared: Arc<TermShared>,
    config: PtyHostConfig,
    delta_tx: Sender<GridDelta>,
    node: NodeId,
    seq: Arc<AtomicU64>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reader_loop(&mut *reader, &shared, &config, &delta_tx, node, &seq);
    }));

    let final_state = match outcome {
        Ok(()) => TerminalState::Exited,
        Err(_) => {
            tracing::error!(%node, "loop do terminal entrou em panic — painel marcado como crashed; processo segue vivo");
            TerminalState::Crashed
        }
    };
    *lock(&shared.state) = final_state;
}

fn reader_loop(
    reader: &mut dyn Read,
    shared: &TermShared,
    config: &PtyHostConfig,
    delta_tx: &Sender<GridDelta>,
    node: NodeId,
    seq: &AtomicU64,
) {
    let mut buf = vec![0u8; config.read_buf.max(1)];
    let mut batch: Vec<u8> = Vec::with_capacity(config.max_batch_bytes);
    let mut last_flush = Instant::now();

    loop {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        check_panic(shared, node);

        // Flow-control com histerese: ao bater no high-watermark, NÃO drena o PTY
        // até o core consumir de volta ao low-watermark. Isso enche o buffer do
        // PTY e bloqueia o produtor (a CLI) — o teto de RAM é respeitado.
        if shared.inflight.load(Ordering::Relaxed) >= config.high_watermark as u64 {
            while shared.inflight.load(Ordering::Relaxed) > config.low_watermark as u64 {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                check_panic(shared, node);
                thread::sleep(Duration::from_millis(1));
            }
        }

        match reader.read(&mut buf) {
            Ok(0) => {
                flush(&mut batch, shared, delta_tx, node, seq, &mut last_flush);
                break; // EOF: o processo do terminal saiu
            }
            Ok(n) => {
                batch.extend_from_slice(&buf[..n]);
                shared
                    .metrics
                    .bytes_read
                    .fetch_add(n as u64, Ordering::Relaxed);
                // Flush ao atingir o teto de coalescing (carga) OU quando o produtor
                // pausa — um read curto (`n < buf`) indica que ele drenou, então
                // aplicamos já (baixa latência p/ saída pequena) — OU pelo tempo.
                let due = batch.len() >= config.max_batch_bytes
                    || n < buf.len()
                    || last_flush.elapsed() >= config.max_batch_time;
                if due {
                    flush(&mut batch, shared, delta_tx, node, seq, &mut last_flush);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                flush(&mut batch, shared, delta_tx, node, seq, &mut last_flush);
                break; // master fechado / erro de leitura
            }
        }
    }
}

#[inline]
fn check_panic(shared: &TermShared, node: NodeId) {
    if shared.panic_inject.load(Ordering::Relaxed) {
        panic!("panic injetado no loop do terminal {node} (fault injection W0-3)");
    }
}

/// Aplica o lote ao `VtBackend`, coleta as linhas sujas, contabiliza o inflight e
/// emite o `GridDelta`. Esvazia o lote.
fn flush(
    batch: &mut Vec<u8>,
    shared: &TermShared,
    delta_tx: &Sender<GridDelta>,
    node: NodeId,
    seq: &AtomicU64,
    last_flush: &mut Instant,
) {
    *last_flush = Instant::now();
    if batch.is_empty() {
        return;
    }
    let bytes = batch.len();

    let rows = {
        let mut vt = lock(&shared.vt);
        vt.advance(batch.as_slice());
        let dirty = vt.damaged_rows();
        vt.reset_damage();
        dirty
    };
    batch.clear();

    shared.inflight.fetch_add(bytes as u64, Ordering::Relaxed);
    shared.metrics.batches.fetch_add(1, Ordering::Relaxed);
    let s = seq.fetch_add(1, Ordering::Relaxed);
    // Se o consumidor sumiu (rx dropado), apenas seguimos — o flow-control ainda
    // segura via inflight (ninguém dá ack → o reader pausa).
    let _ = delta_tx.send(GridDelta {
        node,
        rows,
        bytes,
        seq: s,
    });
}

// ════════════════════════ Workspace Bus / Supervisor (W0-4) ════════════════════════
//
// O broker in-process que torna "estar no workspace = estar conectado": roster de nós
// (NodeRegistry), 1 fila SERIAL de escrita por terminal (humano e agente nunca
// interleavam bytes no master), locks lógicos de PTY com timeout, wait-for-graph
// (detecta ciclo de `ask` sem travar), endereçamento node|role|broadcast com papel
// late-bound, e pub/sub in-process (tokio::sync::broadcast). Ver [[20 - Arquitetura
// Tecnica]] §3 e [[47 - R2 LLM Engineer]] §1.

/// Estado de um nó no roster (presença = `status != Dead`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Starting,
    Running,
    Idle,
    Busy,
    Blocked,
    Dead,
}

impl NodeStatus {
    /// Presença: o nó está vivo (qualquer estado menos `Dead`).
    #[must_use]
    pub fn is_alive(self) -> bool {
        self != NodeStatus::Dead
    }
    /// Disponível para receber trabalho (preferido na resolução `first-idle`).
    #[must_use]
    pub fn is_available(self) -> bool {
        matches!(
            self,
            NodeStatus::Idle | NodeStatus::Running | NodeStatus::Starting
        )
    }
}

/// Linha do roster: id (UUID v7), nome, papel (lido do nome — doc 21) e status.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub name: String,
    pub role: Option<String>,
    pub status: NodeStatus,
}

/// Política de resolução de endereço por papel (late-bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolePolicy {
    /// Primeiro nó disponível (idle/running/starting); senão o primeiro vivo.
    FirstIdle,
    /// Rodízio entre os nós vivos do papel.
    RoundRobin,
    /// Todos os nós vivos do papel.
    All,
}

/// Quem escreve no master (para a fila serial e o lock lógico).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writer {
    Human,
    Agent(NodeId),
}

/// Operação de escrita serial num terminal. Humano e agente passam pela MESMA fila;
/// nunca interleavam bytes no master.
#[derive(Debug, Clone)]
pub enum WriteOp {
    /// Teclas digitadas pelo humano.
    HumanKeys(Vec<u8>),
    /// Texto injetado por um agente (em produção já em bracketed-paste sanitizado — W0-9).
    AgentText { from: NodeId, bytes: Vec<u8> },
    /// Enter como keystroke separado (`0x0D`) — o que de fato submete (W0-9).
    Submit { from: Option<NodeId> },
}

impl WriteOp {
    /// Bytes que esta op escreve no master.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            WriteOp::HumanKeys(b) => b.clone(),
            WriteOp::AgentText { bytes, .. } => bytes.clone(),
            WriteOp::Submit { .. } => vec![0x0D],
        }
    }
}

/// Eventos do barramento (pub/sub in-process).
#[derive(Debug, Clone)]
pub enum BusEvent {
    NodeSpawned {
        node: NodeId,
    },
    NodeDied {
        node: NodeId,
    },
    NodeStatus {
        node: NodeId,
        status: NodeStatus,
    },
    Message {
        id: String,
        from: NodeId,
        to: Recipient,
    },
}

/// Erros do supervisor.
#[derive(Debug, Error)]
pub enum SupervisorError {
    /// Nó não está no roster.
    #[error("nó {0} não está no workspace")]
    NodeNotFound(NodeId),
    /// Registrar este await fecharia um ciclo no wait-for-graph (deadlock de `ask`).
    #[error("ciclo de await detectado: {waiter} -> {target} fecharia o grafo")]
    CycleDetected { waiter: NodeId, target: NodeId },
    /// Não conseguiu o lock lógico do PTY dentro do timeout.
    #[error("timeout ao adquirir o lock lógico do PTY do nó {0}")]
    LockTimeout(NodeId),
}

/// Lock lógico de PTY com TTL — Arc-backed (o guard limpa no drop). Aquisição por
/// polling com timeout; um holder mais velho que o TTL (ex.: nó morto que o segurava)
/// é reclamável.
#[derive(Clone)]
pub struct PtyLock(Arc<Mutex<LockState>>);

struct LockState {
    holder: Option<Writer>,
    since: Instant,
    ttl: Duration,
}

/// Guarda do lock lógico; libera no drop.
pub struct PtyGuard {
    lock: PtyLock,
}

impl PtyLock {
    fn new(ttl: Duration) -> Self {
        Self(Arc::new(Mutex::new(LockState {
            holder: None,
            since: Instant::now(),
            ttl,
        })))
    }

    fn acquire(&self, who: Writer, timeout: Duration) -> Option<PtyGuard> {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let mut st = lock(&self.0);
                let expired = st.holder.is_some() && st.since.elapsed() >= st.ttl;
                if st.holder.is_none() || expired {
                    st.holder = Some(who);
                    st.since = Instant::now();
                    return Some(PtyGuard { lock: self.clone() });
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Quem detém o lock agora (diagnóstico/teste).
    #[must_use]
    pub fn holder(&self) -> Option<Writer> {
        lock(&self.0).holder
    }
}

impl Drop for PtyGuard {
    fn drop(&mut self) {
        lock(&self.lock.0).holder = None;
    }
}

/// Grafo wait-for: `waiter -> target` (cada nó bloqueado em `ask` espera 1 alvo).
#[derive(Default)]
struct WaitForGraph {
    awaiting: HashMap<NodeId, NodeId>,
}

impl WaitForGraph {
    /// Tenta registrar `waiter` esperando `target`. `Err` se fecharia um ciclo
    /// (`target` alcança `waiter` seguindo as arestas) — NÃO insere nesse caso.
    fn add(&mut self, waiter: NodeId, target: NodeId) -> Result<(), ()> {
        let mut cur = target;
        loop {
            if cur == waiter {
                return Err(()); // fecharia um ciclo
            }
            match self.awaiting.get(&cur) {
                Some(&next) => cur = next,
                None => break,
            }
        }
        self.awaiting.insert(waiter, target);
        Ok(())
    }

    fn remove(&mut self, waiter: NodeId) {
        self.awaiting.remove(&waiter);
    }
}

/// Configuração do supervisor.
#[derive(Debug, Clone, Copy)]
pub struct SupervisorConfig {
    /// TTL do lock lógico de PTY (um holder mais velho que isto é reclamável).
    pub lock_ttl: Duration,
    /// Capacidade do canal de pub/sub.
    pub event_capacity: usize,
    /// Tamanho máximo do log de ops aplicadas por terminal (observabilidade).
    pub applied_log_cap: usize,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            lock_ttl: Duration::from_secs(30),
            event_capacity: 1024,
            applied_log_cap: 4096,
        }
    }
}

struct TermChannel {
    tx: Sender<WriteOp>,
    lock: PtyLock,
    applied: Arc<Mutex<Vec<WriteOp>>>,
    consumer: Option<JoinHandle<()>>,
}

/// Consumidor SERIAL da fila de escrita de um terminal: drena `rx` e aplica cada op
/// ao writer do master, uma de cada vez — bytes nunca interleavam.
fn serial_writer(
    rx: Receiver<WriteOp>,
    mut writer: Box<dyn Write + Send>,
    applied: Arc<Mutex<Vec<WriteOp>>>,
    cap: usize,
) {
    while let Ok(op) = rx.recv() {
        let bytes = op.bytes();
        // Erro de escrita não derruba o supervisor — o terminal pode ter saído.
        if writer.write_all(&bytes).is_err() {
            break;
        }
        let _ = writer.flush();
        let mut log = lock(&applied);
        log.push(op);
        if log.len() > cap {
            let excess = log.len() - cap;
            log.drain(0..excess);
        }
    }
}

/// Broker in-process: roster, filas serial por terminal, locks lógicos, wait-for-graph,
/// endereçamento e pub/sub. `Send + Sync` — compartilhe via `Arc<Supervisor>`.
pub struct Supervisor {
    registry: Mutex<HashMap<NodeId, NodeInfo>>,
    order: Mutex<Vec<NodeId>>,
    terminals: Mutex<HashMap<NodeId, TermChannel>>,
    waitfor: Mutex<WaitForGraph>,
    rr: Mutex<HashMap<String, usize>>,
    events: broadcast::Sender<BusEvent>,
    config: SupervisorConfig,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    /// Supervisor com a configuração padrão.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(SupervisorConfig::default())
    }

    /// Supervisor com configuração customizada.
    #[must_use]
    pub fn with_config(config: SupervisorConfig) -> Self {
        let (events, _) = broadcast::channel(config.event_capacity);
        Self {
            registry: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            terminals: Mutex::new(HashMap::new()),
            waitfor: Mutex::new(WaitForGraph::default()),
            rr: Mutex::new(HashMap::new()),
            events,
            config,
        }
    }

    /// Assina o pub/sub. O assinante recebe os eventos publicados DEPOIS da assinatura.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.events.subscribe()
    }

    fn publish(&self, ev: BusEvent) {
        let _ = self.events.send(ev);
    }

    /// Registra um nó: gera UUID v7, sobe o consumidor serial de escrita ligado a
    /// `writer` (em produção, o writer do master do pty-host; em teste, um sink) e
    /// publica `NodeSpawned`. Status inicial `Starting`.
    pub fn register(
        &self,
        name: impl Into<String>,
        role: Option<String>,
        writer: Box<dyn Write + Send>,
    ) -> NodeId {
        let id = Uuid::now_v7();
        let (tx, rx) = mpsc::channel::<WriteOp>();
        let applied = Arc::new(Mutex::new(Vec::new()));
        let cap = self.config.applied_log_cap;

        let consumer = {
            let applied = Arc::clone(&applied);
            thread::Builder::new()
                .name(format!("lina-bus-writer-{id}"))
                .spawn(move || serial_writer(rx, writer, applied, cap))
                .ok()
        };

        lock(&self.registry).insert(
            id,
            NodeInfo {
                id,
                name: name.into(),
                role,
                status: NodeStatus::Starting,
            },
        );
        lock(&self.order).push(id);
        lock(&self.terminals).insert(
            id,
            TermChannel {
                tx,
                lock: PtyLock::new(self.config.lock_ttl),
                applied,
                consumer,
            },
        );
        self.publish(BusEvent::NodeSpawned { node: id });
        id
    }

    /// Marca um nó como `Dead`: para sua fila de escrita, limpa qualquer await
    /// de/para ele (não trava quem o esperava) e publica `NodeDied`. Mantém a linha
    /// no roster (como Dead) para observabilidade; a resolução o ignora.
    pub fn mark_dead(&self, node: NodeId) -> Result<(), SupervisorError> {
        {
            let mut reg = lock(&self.registry);
            let info = reg
                .get_mut(&node)
                .ok_or(SupervisorError::NodeNotFound(node))?;
            info.status = NodeStatus::Dead;
        }
        self.stop_terminal(node);
        {
            let mut wf = lock(&self.waitfor);
            wf.remove(node);
            wf.awaiting.retain(|_, target| *target != node);
        }
        self.publish(BusEvent::NodeDied { node });
        Ok(())
    }

    /// Remove um nó por completo (roster + fila). Publica `NodeDied`.
    pub fn unregister(&self, node: NodeId) -> Result<(), SupervisorError> {
        let existed = lock(&self.registry).remove(&node).is_some();
        lock(&self.order).retain(|n| *n != node);
        self.stop_terminal(node);
        {
            let mut wf = lock(&self.waitfor);
            wf.remove(node);
            wf.awaiting.retain(|_, target| *target != node);
        }
        if existed {
            self.publish(BusEvent::NodeDied { node });
            Ok(())
        } else {
            Err(SupervisorError::NodeNotFound(node))
        }
    }

    fn stop_terminal(&self, node: NodeId) {
        let tc = lock(&self.terminals).remove(&node);
        if let Some(mut tc) = tc {
            let handle = tc.consumer.take();
            drop(tc); // dropa o `tx` → `rx.recv()` retorna Err → consumidor encerra
            if let Some(h) = handle {
                let _ = h.join();
            }
        }
    }

    /// Atualiza o status de um nó e publica `NodeStatus`.
    pub fn set_status(&self, node: NodeId, status: NodeStatus) -> Result<(), SupervisorError> {
        {
            let mut reg = lock(&self.registry);
            let info = reg
                .get_mut(&node)
                .ok_or(SupervisorError::NodeNotFound(node))?;
            info.status = status;
        }
        self.publish(BusEvent::NodeStatus { node, status });
        Ok(())
    }

    /// Define/atualiza o papel de um nó.
    pub fn set_role(&self, node: NodeId, role: Option<String>) -> Result<(), SupervisorError> {
        let mut reg = lock(&self.registry);
        let info = reg
            .get_mut(&node)
            .ok_or(SupervisorError::NodeNotFound(node))?;
        info.role = role;
        Ok(())
    }

    /// Snapshot da linha de um nó.
    #[must_use]
    pub fn get(&self, node: NodeId) -> Option<NodeInfo> {
        lock(&self.registry).get(&node).cloned()
    }

    /// Roster completo, em ordem de registro (determinístico).
    #[must_use]
    pub fn list(&self) -> Vec<NodeInfo> {
        let reg = lock(&self.registry);
        lock(&self.order)
            .iter()
            .filter_map(|id| reg.get(id).cloned())
            .collect()
    }

    /// Quantidade de nós no roster (inclui Dead até `unregister`).
    #[must_use]
    pub fn count(&self) -> usize {
        lock(&self.registry).len()
    }

    /// W3-4: resolve um **nome** do roster para o `NodeId` de um nó **vivo**. A CLI (`lina ask`)
    /// só conhece nomes (de `.lina/bootstrap.json`); o supervisor faz a ponte nome→`NodeId`. Casa
    /// o nome exato primeiro (ordem de registro, determinístico); senão tenta uma forma tolerante
    /// (sem `@` à frente, case-insensitive) — o leigo/skill pode escrever `@B` para `@b`.
    #[must_use]
    pub fn node_by_name(&self, name: &str) -> Option<NodeId> {
        let reg = lock(&self.registry);
        let order = lock(&self.order);
        for id in order.iter() {
            if let Some(n) = reg.get(id) {
                if n.status.is_alive() && n.name == name {
                    return Some(*id);
                }
            }
        }
        let want = normalize_name(name);
        for id in order.iter() {
            if let Some(n) = reg.get(id) {
                if n.status.is_alive() && normalize_name(&n.name) == want {
                    return Some(*id);
                }
            }
        }
        None
    }

    /// Resolve um endereço em nós concretos. Papel é **late-bound** e ignora nós
    /// `Dead`: se o reviewer morre e outro nasce, `role:reviewer` continua válido.
    #[must_use]
    pub fn resolve(
        &self,
        to: &Recipient,
        policy: RolePolicy,
        exclude: Option<NodeId>,
    ) -> Vec<NodeId> {
        let reg = lock(&self.registry);
        let order = lock(&self.order);
        let role = match to {
            Recipient::Node(id) => {
                let ok = Some(*id) != exclude && reg.get(id).is_some_and(|n| n.status.is_alive());
                return if ok { vec![*id] } else { vec![] };
            }
            Recipient::Broadcast => {
                return order
                    .iter()
                    .copied()
                    .filter(|id| {
                        Some(*id) != exclude && reg.get(id).is_some_and(|n| n.status.is_alive())
                    })
                    .collect();
            }
            Recipient::Role(r) => r.as_str(),
        };

        let candidates: Vec<NodeId> = order
            .iter()
            .copied()
            .filter(|id| {
                Some(*id) != exclude
                    && reg
                        .get(id)
                        .is_some_and(|n| n.status.is_alive() && n.role.as_deref() == Some(role))
            })
            .collect();
        if candidates.is_empty() {
            return vec![];
        }

        match policy {
            RolePolicy::All => candidates,
            RolePolicy::FirstIdle => {
                let pick = candidates
                    .iter()
                    .copied()
                    .find(|id| reg.get(id).is_some_and(|n| n.status.is_available()))
                    .unwrap_or(candidates[0]);
                vec![pick]
            }
            RolePolicy::RoundRobin => {
                drop(reg);
                drop(order);
                let mut rr = lock(&self.rr);
                let counter = rr.entry(role.to_string()).or_insert(0);
                let pick = candidates[*counter % candidates.len()];
                *counter = counter.wrapping_add(1);
                vec![pick]
            }
        }
    }

    /// Adquire o lock lógico do PTY de `node` (timeout) para uma transação de escrita.
    pub fn lock_pty(
        &self,
        node: NodeId,
        who: Writer,
        timeout: Duration,
    ) -> Result<PtyGuard, SupervisorError> {
        let lk = lock(&self.terminals)
            .get(&node)
            .map(|t| t.lock.clone())
            .ok_or(SupervisorError::NodeNotFound(node))?;
        lk.acquire(who, timeout)
            .ok_or(SupervisorError::LockTimeout(node))
    }

    /// Enfileira um `WriteOp` na fila SERIAL do terminal (1 consumidor → sem
    /// interleave de bytes). Base da entrega A2A faseada de W0-9 (ver `a2a::deliver_a2a`).
    pub fn enqueue_write(&self, node: NodeId, op: WriteOp) -> Result<(), SupervisorError> {
        let map = lock(&self.terminals);
        let tc = map.get(&node).ok_or(SupervisorError::NodeNotFound(node))?;
        tc.tx
            .send(op)
            .map_err(|_| SupervisorError::NodeNotFound(node))
    }

    /// Escrita do humano: transação atômica (lock lógico → `HumanKeys` → solta).
    pub fn write_human(
        &self,
        node: NodeId,
        keys: &[u8],
        timeout: Duration,
    ) -> Result<(), SupervisorError> {
        let _g = self.lock_pty(node, Writer::Human, timeout)?;
        self.enqueue_write(node, WriteOp::HumanKeys(keys.to_vec()))
    }

    /// Injeção de agente SIMPLES (texto cru + Enter), atômica sob o lock lógico. A
    /// entrega A2A faseada completa (bracketed-paste sanitizado + `submit_delay` +
    /// Enter separado, conforme CLI Profile) é `a2a::deliver_a2a` (W0-9).
    pub fn inject_agent(
        &self,
        target: NodeId,
        from: NodeId,
        text: &[u8],
        timeout: Duration,
    ) -> Result<(), SupervisorError> {
        let _g = self.lock_pty(target, Writer::Agent(from), timeout)?;
        self.enqueue_write(
            target,
            WriteOp::AgentText {
                from,
                bytes: text.to_vec(),
            },
        )?;
        self.enqueue_write(target, WriteOp::Submit { from: Some(from) })
    }

    /// Log das ops já aplicadas no terminal (observabilidade/teste).
    #[must_use]
    pub fn applied_ops(&self, node: NodeId) -> Vec<WriteOp> {
        lock(&self.terminals)
            .get(&node)
            .map(|t| lock(&t.applied).clone())
            .unwrap_or_default()
    }

    /// Registra que `waiter` espera o `reply` de `target` (ask bloqueante). Retorna
    /// `CycleDetected` na hora (sem travar) se fecharia um ciclo no wait-for-graph.
    pub fn begin_await(&self, waiter: NodeId, target: NodeId) -> Result<(), SupervisorError> {
        lock(&self.waitfor)
            .add(waiter, target)
            .map_err(|()| SupervisorError::CycleDetected { waiter, target })
    }

    /// Encerra o await de `waiter` (chegou o reply ou deu timeout).
    pub fn end_await(&self, waiter: NodeId) {
        lock(&self.waitfor).remove(waiter);
    }

    /// Resolve os destinatários de um envelope aplicando anti-loop de mensagem
    /// (`ttl`/`trace`) e publica `Message`. Devolve os alvos que devem receber (sem o
    /// remetente nem nós já no `trace`). Vazio = dropado (loop/ttl) ou sem alvo vivo.
    pub fn route(&self, env: &A2aEnvelope, policy: RolePolicy) -> Vec<NodeId> {
        if env.ttl == 0 {
            return vec![];
        }
        let fresh: Vec<NodeId> = self
            .resolve(&env.to, policy, Some(env.from))
            .into_iter()
            .filter(|t| !env.trace.contains(t))
            .collect();
        if !fresh.is_empty() {
            self.publish(BusEvent::Message {
                id: env.id.clone(),
                from: env.from,
                to: env.to.clone(),
            });
        }
        fresh
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let nodes: Vec<NodeId> = lock(&self.terminals).keys().copied().collect();
        for node in nodes {
            self.stop_terminal(node);
        }
    }
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use serial_test::serial;

    fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        loop {
            if cond() {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn sink() -> Box<dyn Write + Send> {
        Box::new(std::io::sink())
    }

    const T: Duration = Duration::from_secs(5);

    /// Critério (a): 3 nós no registry; writes concorrentes humano + 2 agentes no
    /// MESMO terminal → ordem serial, sem interleave de bytes, e cada injeção de
    /// agente (texto+Enter) é atômica (lock lógico). `#[serial]` pela concorrência.
    #[test]
    #[serial]
    fn three_nodes_and_concurrent_writes_stay_serial_without_interleave() {
        let sup = Arc::new(Supervisor::new());
        let agent_a = sup.register("@Dev Frontend", Some("frontend".into()), sink());
        let agent_b = sup.register("@Dev Backend", Some("backend".into()), sink());
        let target = sup.register("@QA", Some("qa".into()), sink());
        assert_eq!(sup.count(), 3, "3 nós no registry");

        let iters = 60usize;
        let mut handles = Vec::new();

        // 1 humano martelando o terminal `target`.
        {
            let sup = Arc::clone(&sup);
            handles.push(thread::spawn(move || {
                for i in 0..iters {
                    sup.write_human(target, format!("h{i};").as_bytes(), T)
                        .expect("write_human");
                }
            }));
        }
        // 2 agentes injetando (texto + Enter) concorrentemente no MESMO terminal.
        for from in [agent_a, agent_b] {
            let sup = Arc::clone(&sup);
            handles.push(thread::spawn(move || {
                for i in 0..iters {
                    sup.inject_agent(target, from, format!("a{i};").as_bytes(), T)
                        .expect("inject_agent");
                }
            }));
        }
        for h in handles {
            h.join().expect("join produtor");
        }

        // humano: `iters` HumanKeys; cada agente: `iters` AgentText + `iters` Submit.
        let expected = iters + 2 * (iters + iters);
        assert!(
            poll_until(T, || sup.applied_ops(target).len() == expected),
            "todas as ops deveriam ter sido aplicadas em série"
        );

        let ops = sup.applied_ops(target);
        // (i) ATOMICIDADE: cada AgentText é IMEDIATAMENTE seguido pelo seu Submit
        //     (mesmo agente), sem nenhum HumanKeys no meio.
        for (i, op) in ops.iter().enumerate() {
            if let WriteOp::AgentText { from, .. } = op {
                match ops.get(i + 1) {
                    Some(WriteOp::Submit { from: Some(s) }) => {
                        assert_eq!(s, from, "o Submit deve ser do mesmo agente que o texto")
                    }
                    other => {
                        panic!("AgentText deveria ser seguido pelo seu Submit; veio {other:?}")
                    }
                }
            }
        }
        // (ii) contagem por tipo bate — nada perdido, duplicado ou embaralhado.
        let humans = ops
            .iter()
            .filter(|o| matches!(o, WriteOp::HumanKeys(_)))
            .count();
        let texts = ops
            .iter()
            .filter(|o| matches!(o, WriteOp::AgentText { .. }))
            .count();
        let submits = ops
            .iter()
            .filter(|o| matches!(o, WriteOp::Submit { .. }))
            .count();
        assert_eq!(humans, iters);
        assert_eq!(texts, 2 * iters);
        assert_eq!(submits, 2 * iters);
    }

    /// Critério (b): ciclo de await A→B→A dispara `CycleDetected` e NÃO trava (a 2ª
    /// chamada retorna em <2s, provado por thread + recv_timeout).
    #[test]
    fn await_cycle_is_detected_and_does_not_hang() {
        let sup = Arc::new(Supervisor::new());
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());

        sup.begin_await(a, b).expect("A→B deve registrar");

        // B→A fecharia A→B→A. Roda em thread e exige retorno rápido (não-bloqueante).
        let (tx, rx) = mpsc::channel();
        {
            let sup = Arc::clone(&sup);
            thread::spawn(move || {
                let _ = tx.send(sup.begin_await(b, a));
            });
        }
        let res = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("begin_await NÃO travou (retornou em <2s)");
        assert!(
            matches!(res, Err(SupervisorError::CycleDetected { .. })),
            "deveria detectar o ciclo"
        );

        // Sem a aresta de B: ao A liberar, B→A passa a ser válido.
        sup.end_await(a);
        sup.begin_await(b, a).expect("B→A ok após A liberar");
        sup.end_await(b);
    }

    /// Critério (c): `role:reviewer` resolve para o nó VIVO depois que o reviewer
    /// original é marcado `dead` (late-bound, ignora mortos).
    #[test]
    fn role_resolves_to_live_node_after_original_dies() {
        let sup = Supervisor::new();
        let r1 = sup.register("@Reviewer 1", Some("reviewer".into()), sink());
        let r2 = sup.register("@Reviewer 2", Some("reviewer".into()), sink());

        // Ambos vivos: `All` retorna os dois, em ordem de registro.
        assert_eq!(
            sup.resolve(&Recipient::Role("reviewer".into()), RolePolicy::All, None),
            vec![r1, r2]
        );

        // Mata o reviewer original.
        sup.mark_dead(r1).expect("mark_dead");

        // `role:reviewer` agora resolve só para o vivo (r2), em qualquer política.
        assert_eq!(
            sup.resolve(
                &Recipient::Role("reviewer".into()),
                RolePolicy::FirstIdle,
                None
            ),
            vec![r2],
            "first-idle deve cair no reviewer vivo"
        );
        assert_eq!(
            sup.resolve(
                &Recipient::Role("reviewer".into()),
                RolePolicy::RoundRobin,
                None
            ),
            vec![r2],
            "round-robin entre vivos também ignora o morto"
        );
        assert_eq!(
            sup.resolve(&Recipient::Role("reviewer".into()), RolePolicy::All, None),
            vec![r2]
        );
    }

    /// Pub/sub emite o ciclo de vida do nó (spawned → status → died). `try_recv` é
    /// síncrono; `send` é imediato — sem runtime tokio.
    #[test]
    fn pubsub_emits_node_lifecycle() {
        let sup = Supervisor::new();
        let mut rx = sup.subscribe();
        let n = sup.register("@Dev", Some("dev".into()), sink());
        sup.set_status(n, NodeStatus::Busy).expect("status");
        sup.mark_dead(n).expect("dead");

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BusEvent::NodeSpawned { node } if *node == n)),
            "deveria ter NodeSpawned"
        );
        assert!(
            events.iter().any(
                |e| matches!(e, BusEvent::NodeStatus { node, status: NodeStatus::Busy } if *node == n)
            ),
            "deveria ter NodeStatus(Busy)"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BusEvent::NodeDied { node } if *node == n)),
            "deveria ter NodeDied"
        );
    }

    /// Endereçamento por papel + anti-loop de mensagem (`trace`/`ttl`) no `route`.
    #[test]
    fn route_resolves_role_and_drops_loops() {
        let sup = Supervisor::new();
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", Some("reviewer".into()), sink());

        // role:reviewer → b (vivo).
        let env = A2aEnvelope::new(a, Recipient::Role("reviewer".into()), Some("ask".into()));
        assert_eq!(sup.route(&env, RolePolicy::FirstIdle), vec![b]);

        // alvo já no trace → dropado (anti-loop).
        let mut looped = A2aEnvelope::new(a, Recipient::Node(b), Some("ask".into()));
        looped.trace.push(b);
        assert!(sup.route(&looped, RolePolicy::FirstIdle).is_empty());

        // ttl=0 → dropado.
        let mut dead = A2aEnvelope::new(a, Recipient::Node(b), None);
        dead.ttl = 0;
        assert!(sup.route(&dead, RolePolicy::FirstIdle).is_empty());
    }
}
// TODO(W0-5): Event Store (rusqlite WAL + JSONL + snapshots; replay determinístico).
// TODO(W0-6): recuperação pós-crash visível (integrity_check -> preservar -> rebuild).

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Timeout generoso para toda espera-por-condição — robusto a contenção de CPU.
    const T: Duration = Duration::from_secs(5);

    /// Espera-POR-CONDIÇÃO com timeout (poll), nunca um sleep fixo de asserção.
    fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        loop {
            if cond() {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Espera o `bytes_read` ESTABILIZAR (duas amostras consecutivas iguais) — prova
    /// por CONDIÇÃO, não por timing absoluto, que o reader parou de drenar. Deve ser
    /// chamado só depois de `inflight >= high` (aí o reader está entrando na pausa, e
    /// `bytes_read` congela de verdade). Devolve o valor estável (ou o último, no timeout).
    fn wait_until_drained_stops(host: &PtyHost, node: NodeId, timeout: Duration) -> u64 {
        let start = Instant::now();
        let mut prev = host.metrics(node).map_or(0, |m| m.bytes_read);
        loop {
            thread::sleep(Duration::from_millis(30));
            let cur = host.metrics(node).map_or(0, |m| m.bytes_read);
            if cur == prev || start.elapsed() >= timeout {
                return cur;
            }
            prev = cur;
        }
    }

    fn last_line(host: &PtyHost, node: NodeId) -> Option<String> {
        host.with_grid(node, |vt| vt.last_nonempty_line())
    }

    /// Critério de aceite (a): sob flood (`yes`) sem ack, o flow-control LIMITA os
    /// bytes não-consumidos a um teto e o reader PARA de drenar (backpressure); outro
    /// terminal segue sendo servido. Mede o COMPORTAMENTO do watermark por condição,
    /// não timing absoluto. `#[serial]`: não compete por CPU com o outro flood-test.
    #[test]
    #[serial]
    fn flow_control_caps_memory_under_flood_and_others_stay_responsive() {
        let config = PtyHostConfig {
            high_watermark: 256 * 1024,
            low_watermark: 64 * 1024,
            max_batch_bytes: 64 * 1024,
            max_batch_time: Duration::from_millis(4),
            read_buf: 8 * 1024,
        };
        // Um flush pode passar do high em até (max_batch_bytes + read_buf); 2 lotes de folga.
        let cap = (config.high_watermark + 2 * config.max_batch_bytes) as u64;
        let mut host = PtyHost::with_config(config);

        let flood = host
            .spawn(PtyCommand::new("yes"), 80, 24)
            .expect("spawn yes");

        // (1) Bate no high-watermark (sem ack ninguém libera) — por condição.
        assert!(
            poll_until(T, || host.inflight(flood).unwrap_or(0)
                >= config.high_watermark as u64),
            "o flood deveria atingir o high-watermark"
        );
        // (2) O reader PARA de drenar: bytes_read estabiliza (condição, não timing).
        let stable = wait_until_drained_stops(&host, flood, T);
        // (3) E o inflight fica LIMITADO ao teto — a prova de RAM-cap.
        let inf = host.inflight(flood).expect("inflight");
        assert!(
            inf <= cap,
            "inflight {inf} excedeu o teto {cap} — RAM-cap furado"
        );
        // (4) Continua estável (não voltou a drenar sozinho, sem ack).
        assert_eq!(
            wait_until_drained_stops(&host, flood, Duration::from_secs(2)),
            stable,
            "sob backpressure o reader não deveria voltar a drenar sem ack"
        );

        // (5) Outro terminal é servido normalmente enquanto o flood está represado.
        let tick = host
            .spawn(
                PtyCommand::new("sh").arg("-c").arg("printf 'LINA_TICK\\n'"),
                80,
                24,
            )
            .expect("spawn tick");
        assert!(
            poll_until(T, || last_line(&host, tick).as_deref() == Some("LINA_TICK")),
            "o terminal 'tick' deveria ser lido enquanto o flood está represado"
        );

        // (6) Ao consumir (ack) o inflight, o reader cai abaixo do low e RETOMA.
        let to_ack = host.inflight(flood).expect("inflight");
        host.ack(flood, to_ack).expect("ack");
        assert!(
            poll_until(T, || host.metrics(flood).map_or(0, |m| m.bytes_read)
                > stable),
            "após ack, o reader deveria retomar a drenagem"
        );

        host.kill(flood).ok();
        host.kill(tick).ok();
    }

    /// Critério de aceite (b): um panic no loop de um terminal é CONTIDO (estado
    /// `Crashed`) sem derrubar o processo; outro terminal segue lendo e respondendo a
    /// writes. `cat` é controlado por write (sem netos/`sleep`), cleanup determinístico.
    #[test]
    #[serial]
    fn panic_in_one_loop_is_contained_and_others_survive() {
        let mut host = PtyHost::new();
        let a = host.spawn(PtyCommand::new("yes"), 80, 24).expect("spawn A");
        let b = host.spawn(PtyCommand::new("cat"), 80, 24).expect("spawn B");

        // A já lê; B ecoa um write (via cat) — ambos vivos. Tudo por condição.
        assert!(poll_until(T, || host
            .metrics(a)
            .map_or(0, |m| m.bytes_read)
            > 0));
        host.write(b, b"B_ALIVE_1\n").expect("write B");
        assert!(
            poll_until(T, || last_line(&host, b).as_deref() == Some("B_ALIVE_1")),
            "B deveria ecoar o primeiro write"
        );
        assert_eq!(host.state(a), Some(TerminalState::Running));
        assert_eq!(host.state(b), Some(TerminalState::Running));

        // Injeta panic no loop de A.
        host.inject_panic(a).expect("inject");

        // A vira Crashed; este teste segue executando ⇒ o PROCESSO sobreviveu.
        assert!(
            poll_until(T, || host.state(a) == Some(TerminalState::Crashed)),
            "o painel A deveria estar marcado como crashed"
        );

        // B segue Running e RESPONDE a um novo write após o panic de A.
        assert_eq!(host.state(b), Some(TerminalState::Running));
        host.write(b, b"B_ALIVE_2\n").expect("write B pós-panic");
        assert!(
            poll_until(T, || last_line(&host, b).as_deref() == Some("B_ALIVE_2")),
            "B deveria seguir lendo/escrevendo após o panic de A"
        );
        assert_eq!(host.running_count(), 1, "só B deve estar Running");

        host.kill(a).ok();
        host.kill(b).ok();
    }

    /// A saída de um comando chega ao grid parseado via a thread de leitura.
    /// `printf` one-shot (sem `sleep`/netos): cleanup determinístico.
    #[test]
    fn output_lands_in_parsed_grid() {
        let mut host = PtyHost::new();
        let node = host
            .spawn(
                PtyCommand::new("sh").arg("-c").arg("printf 'LINA_OK\\n'"),
                80,
                24,
            )
            .expect("spawn");
        assert!(
            poll_until(T, || last_line(&host, node).as_deref() == Some("LINA_OK")),
            "a saída do comando deveria aparecer no grid parseado"
        );
        host.kill(node).ok();
    }

    /// Operações em nó inexistente erram limpo (sem panic, sem unwrap).
    #[test]
    fn missing_node_errors_cleanly() {
        let host = PtyHost::new();
        let ghost = Uuid::now_v7();
        assert!(matches!(
            host.write(ghost, b"x"),
            Err(PtyHostError::NotFound(_))
        ));
        assert!(matches!(host.ack(ghost, 1), Err(PtyHostError::NotFound(_))));
        assert!(host.inflight(ghost).is_none());
        assert!(host.state(ghost).is_none());
        assert!(host.metrics(ghost).is_none());
    }
}
