//! A PONTE real core <-> shell (Onda 2). **Livre de tipos de toolkit** (zero gpui):
//! contém só a projeção (`SharedModel`), o `impl UiHost` (core→UI) e o `impl InputSink`
//! (UI→core). A âncora de continuidade do `CLAUDE.md` — trocar gpui↔Slint sem reescrever
//! o core — fica viva: um shell Slint reimplementa **só** a view, reusando esta ponte.
//!
//! ## Saída (render), tudo PELO pipeline do core — nunca lendo o PTY no shell
//! `PTY → pty-host (lina-core) → grid alacritty_terminal → GridDelta { dirty_rows } →`
//! `UiHost::on_event →` lê **só** as linhas sujas via `PtyHost::with_grid` (o grid
//! parseado PELO core) `→ SharedModel`. O shell gpui lê o `SharedModel` a cada frame.
//!
//! ## Entrada
//! `tecla no gpui → InputSink::submit(WriteOp::HumanKeys) → PtyHost::write` no master
//! (o pty-host é dono do writer nesta story; ver RESULTADO.md sobre o seam da MailQueue).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lina_core::{BusEvent, GridDelta, NodeStatus as CoreStatus, PtyHost, Recipient};
use lina_host::{BusTarget, HostEvent, InputSink, NodeId, NodeKind, NodeStatus, UiHost, WriteOp};
use tokio::sync::broadcast;

/// Viewport do terminal (cols x rows). Fixo nesta story (1 terminal).
pub const COLS: u16 = 100;
pub const ROWS: u16 = 30;

/// Recupera o guard mesmo se o mutex estiver envenenado (não propaga panic de um nó).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Projeção descartável de um nó-terminal — SÓ dados (nenhum tipo de toolkit).
#[derive(Debug, Clone)]
pub struct NodeView {
    pub kind: NodeKind,
    pub status: NodeStatus,
    /// Viewport linearizado: uma `String` por linha (índice = linha do viewport).
    pub rows: Vec<String>,
}

impl NodeView {
    fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            status: NodeStatus::Starting,
            rows: vec![String::new(); ROWS as usize],
        }
    }
}

/// Estado projetado que o shell gpui lê a cada frame. SÓ dados.
#[derive(Debug, Default)]
pub struct SharedModel {
    pub nodes: BTreeMap<NodeId, NodeView>,
    pub order: Vec<NodeId>,
    pub recovering: bool,
    /// Sobe a cada mutação — o shell pode pular o redraw quando inalterado (futuro).
    pub generation: u64,
}

impl SharedModel {
    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Handle compartilhado do estado projetado.
pub type Model = Arc<Mutex<SharedModel>>;

/// **core → UI.** Implementa `UiHost`: projeta cada `HostEvent` no `SharedModel`,
/// lendo as linhas sujas do grid **pelo core** (`PtyHost::with_grid`), nunca do PTY.
/// `Send + 'static` porque `PtyHost: Send` ⇒ `Arc<Mutex<PtyHost>>: Send`.
pub struct GpuiBridgeHost {
    model: Model,
    host: Arc<Mutex<PtyHost>>,
}

impl GpuiBridgeHost {
    #[must_use]
    pub fn new(model: Model, host: Arc<Mutex<PtyHost>>) -> Self {
        Self { model, host }
    }
}

impl UiHost for GpuiBridgeHost {
    fn on_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::NodeAdded { node, kind } => {
                let mut m = lock(&self.model);
                m.nodes.entry(node).or_insert_with(|| NodeView::new(kind));
                if !m.order.contains(&node) {
                    m.order.push(node);
                }
                m.touch();
            }
            HostEvent::NodeStatusChanged { node, status } => {
                let mut m = lock(&self.model);
                if let Some(nv) = m.nodes.get_mut(&node) {
                    nv.status = status;
                    m.touch();
                }
            }
            HostEvent::NodeRemoved { node } => {
                let mut m = lock(&self.model);
                m.nodes.remove(&node);
                m.order.retain(|n| *n != node);
                m.touch();
            }
            HostEvent::GridDelta { node, dirty_rows } => {
                // Lê SÓ as linhas sujas do grid parseado PELO core (with_grid) — não o PTY.
                let updates: Option<Vec<(usize, String)>> =
                    lock(&self.host).with_grid(node, |vt| {
                        dirty_rows
                            .iter()
                            .copied()
                            .filter(|&r| r < ROWS as usize)
                            .map(|r| (r, vt.row_text(r)))
                            .collect()
                    });
                if let Some(updates) = updates {
                    let mut m = lock(&self.model);
                    let is_new = !m.nodes.contains_key(&node);
                    {
                        let nv = m
                            .nodes
                            .entry(node)
                            .or_insert_with(|| NodeView::new(NodeKind::Terminal));
                        for (r, text) in updates {
                            if r < nv.rows.len() {
                                nv.rows[r] = text;
                            }
                        }
                    }
                    if is_new && !m.order.contains(&node) {
                        m.order.push(node);
                    }
                    m.touch();
                }
            }
            HostEvent::Recovering => {
                let mut m = lock(&self.model);
                m.recovering = true;
                m.touch();
            }
            HostEvent::Recovered => {
                let mut m = lock(&self.model);
                m.recovering = false;
                m.touch();
            }
            // BusMessage (pulso A2A) e variantes futuras: feedback visual na próxima story.
            HostEvent::BusMessage { .. } => {}
            _ => {}
        }
    }
}

/// **UI → core.** O shell chama `submit`; roteia o `WriteOp` para o writer do master
/// via `PtyHost::write` (o pty-host é dono do writer nesta story). `Send + Sync`.
pub struct CoreInput {
    host: Arc<Mutex<PtyHost>>,
}

impl CoreInput {
    #[must_use]
    pub fn new(host: Arc<Mutex<PtyHost>>) -> Self {
        Self { host }
    }
}

impl InputSink for CoreInput {
    fn submit(&self, node: NodeId, op: WriteOp) {
        let bytes = match op {
            WriteOp::HumanKeys(b) => b,
            WriteOp::AgentText(s) => s.into_bytes(),
            WriteOp::Submit => vec![0x0D],
            _ => return,
        };
        if bytes.is_empty() {
            return;
        }
        if let Err(e) = lock(&self.host).write(node, &bytes) {
            // Não silenciar: uma escrita falha (terminal morto) precisa ser vista.
            eprintln!("lina-gpui: falha ao escrever input no nó {node}: {e}");
        }
    }
}

/// Mapeia o `NodeStatus` do core (Bus) para o `NodeStatus` do contrato de UI.
fn map_status(s: CoreStatus) -> NodeStatus {
    match s {
        CoreStatus::Starting => NodeStatus::Starting,
        CoreStatus::Running => NodeStatus::Running,
        CoreStatus::Idle => NodeStatus::Idle,
        CoreStatus::Busy => NodeStatus::Busy,
        // O contrato de UI não tem `Blocked` (await do A2A): bloqueado ≈ ocupado.
        CoreStatus::Blocked => NodeStatus::Busy,
        CoreStatus::Dead => NodeStatus::Dead,
    }
}

/// Traduz um `BusEvent` real do core em `HostEvent`, remapeando o `NodeId` do
/// Supervisor para o `NodeId` canônico do terminal (o do pty-host) via `remap` —
/// porque `PtyHost::spawn` e `Supervisor::register` cunham ids independentes.
pub fn bus_to_host(ev: &BusEvent, remap: &impl Fn(NodeId) -> NodeId) -> Option<HostEvent> {
    match ev {
        BusEvent::NodeSpawned { node } => Some(HostEvent::NodeAdded {
            node: remap(*node),
            kind: NodeKind::Terminal,
        }),
        BusEvent::NodeDied { node } => Some(HostEvent::NodeRemoved { node: remap(*node) }),
        BusEvent::NodeStatus { node, status } => Some(HostEvent::NodeStatusChanged {
            node: remap(*node),
            status: map_status(*status),
        }),
        BusEvent::Message { from, to, .. } => Some(HostEvent::BusMessage {
            from: remap(*from),
            to: match to {
                Recipient::Node(n) => BusTarget::Node(remap(*n)),
                Recipient::Role(r) => BusTarget::Role(r.clone()),
                Recipient::Broadcast => BusTarget::Broadcast,
            },
        }),
    }
}

/// Handle da thread-bomba (background) da ponte; encerra no `stop`/`Drop`.
pub struct Pump {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Pump {
    /// Sinaliza parada e dá join (idempotente).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Sobe a **thread-bomba** (o "laço do core" que chama `UiHost::on_event`, livre de gpui):
/// drena o bus (lifecycle/status/A2A, remapeado p/ o id canônico) e os `GridDelta` do
/// pty-host (render incremental + `ack` do flow-control), projetando no `SharedModel`.
pub fn spawn_pump(
    mut bridge: GpuiBridgeHost,
    host: Arc<Mutex<PtyHost>>,
    delta_rx: Receiver<GridDelta>,
    mut bus_rx: broadcast::Receiver<BusEvent>,
    remap: impl Fn(NodeId) -> NodeId + Send + 'static,
) -> std::io::Result<Pump> {
    let stop = Arc::new(AtomicBool::new(false));
    let join = {
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("lina-bridge-pump".into())
            .spawn(move || loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let mut worked = false;

                // 1) Eventos do Bus (lifecycle/status/A2A), remapeados ao id canônico.
                loop {
                    match bus_rx.try_recv() {
                        Ok(ev) => {
                            if let Some(h) = bus_to_host(&ev, &remap) {
                                bridge.on_event(h);
                                worked = true;
                            }
                        }
                        Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                        Err(_) => break, // Empty | Closed
                    }
                }

                // 2) Deltas de grid do pty-host: render incremental + ack do flow-control.
                while let Ok(gd) = delta_rx.try_recv() {
                    let node = gd.node;
                    let bytes = gd.bytes as u64;
                    bridge.on_event(HostEvent::GridDelta {
                        node,
                        dirty_rows: gd.rows,
                    });
                    let _ = lock(&host).ack(node, bytes);
                    worked = true;
                }

                if !worked {
                    thread::sleep(Duration::from_millis(2));
                }
            })?
    };
    Ok(Pump {
        stop,
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_core::PtyCommand;
    use std::time::Instant;

    /// O loop COMPLETO da ponte, sem display: input do humano (`InputSink`) → writer do
    /// master (pty-host) → `cat` ecoa → reader do pty-host → grid lina-vt → `GridDelta`
    /// → `UiHost::on_event` → `with_grid`/`row_text` → `SharedModel`. Prova que o texto
    /// chega à projeção **pelo pipeline do core**, nunca por leitura direta do PTY.
    #[test]
    fn grid_roundtrip_through_core() {
        let mut pty = PtyHost::new();
        let node = pty
            .spawn(PtyCommand::new("cat"), COLS, ROWS)
            .expect("spawn cat");
        let rx = pty.take_delta_receiver().expect("delta rx");
        let host = Arc::new(Mutex::new(pty));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let mut bridge = GpuiBridgeHost::new(Arc::clone(&model), Arc::clone(&host));
        bridge.on_event(HostEvent::NodeAdded {
            node,
            kind: NodeKind::Terminal,
        });

        // UI → core: o humano "digita" via o contrato InputSink.
        let input = CoreInput::new(Arc::clone(&host));
        input.submit(node, WriteOp::HumanKeys(b"lina-bridge-ok\r".to_vec()));

        // Drena os deltas e projeta (exatamente o que a thread-bomba faz).
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while Instant::now() < deadline && !found {
            while let Ok(gd) = rx.try_recv() {
                bridge.on_event(HostEvent::GridDelta {
                    node,
                    dirty_rows: gd.rows,
                });
                let _ = lock(&host).ack(node, gd.bytes as u64);
            }
            let joined = {
                let m = lock(&model);
                m.nodes
                    .get(&node)
                    .map(|n| n.rows.join("\n"))
                    .unwrap_or_default()
            };
            found = joined.contains("lina-bridge-ok");
            if !found {
                thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(
            found,
            "o texto digitado deve chegar ao SharedModel PELO pipeline do core (PTY→lina-vt→GridDelta→UiHost)"
        );
        let _ = lock(&host).kill(node);
    }

    /// `Recovering`/`Recovered` (vindos do EventStore em corrupção) alternam o banner.
    #[test]
    fn recovery_pair_toggles_banner() {
        let host = Arc::new(Mutex::new(PtyHost::new()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let mut bridge = GpuiBridgeHost::new(Arc::clone(&model), host);
        assert!(!lock(&model).recovering);
        bridge.on_event(HostEvent::Recovering);
        assert!(lock(&model).recovering, "banner de recuperação deve ligar");
        bridge.on_event(HostEvent::Recovered);
        assert!(
            !lock(&model).recovering,
            "banner deve desligar ao recuperar"
        );
    }

    /// `bus_to_host` traduz os `BusEvent` reais do Supervisor e remapeia o id do
    /// Supervisor para o id canônico do terminal (pty-host).
    #[test]
    fn bus_events_map_and_remap() {
        use uuid::Uuid;
        let sup_id = Uuid::now_v7();
        let term_id = Uuid::now_v7();
        let remap = |id: NodeId| if id == sup_id { term_id } else { id };

        match bus_to_host(&BusEvent::NodeSpawned { node: sup_id }, &remap) {
            Some(HostEvent::NodeAdded { node, kind }) => {
                assert_eq!(node, term_id, "NodeSpawned deve remapear p/ o id canônico");
                assert_eq!(kind, NodeKind::Terminal);
            }
            other => panic!("esperava NodeAdded remapeado, veio {other:?}"),
        }

        match bus_to_host(
            &BusEvent::NodeStatus {
                node: sup_id,
                status: CoreStatus::Idle,
            },
            &remap,
        ) {
            Some(HostEvent::NodeStatusChanged { node, status }) => {
                assert_eq!(node, term_id);
                assert_eq!(status, NodeStatus::Idle);
            }
            other => panic!("esperava NodeStatusChanged, veio {other:?}"),
        }
    }
}
