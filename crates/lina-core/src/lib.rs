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

use lina_pty::{PtyError, PtyManager};
use thiserror::Error;
use uuid::Uuid;

/// Re-exports do contrato de UI para os consumidores do core.
pub use lina_host::{HostEvent, NodeId, UiHost};
/// Re-exports da camada de PTY/VT para montar comandos e ler o grid.
pub use lina_pty::PtyCommand;
pub use lina_vt::VtBackend;

use lina_vt::AlacrittyBackend;

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
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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

// TODO(W0-4): Workspace Bus / Supervisor (NodeRegistry, MailQueues serial, wait-for-graph).
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
