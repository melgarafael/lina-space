//! A PONTE real core <-> shell (Onda 2 · walking skeleton). **Livre de tipos de toolkit**
//! (zero gpui): projeção (`SharedModel`), `impl UiHost` (core→UI), `impl InputSink`
//! (UI→core), a fiação de 2 terminais reais e o gatilho de A2A com pulso. A âncora do
//! `CLAUDE.md` (trocar gpui↔Slint sem reescrever o core) fica viva — um shell Slint
//! reimplementa só a view, reusando tudo isto.
//!
//! ## Diferença para a story 1
//! Aqui o **Supervisor é dono dos writers** (`sup.register(..., writer)`), porque o A2A
//! (`deliver_a2a`) injeta pela **MailQueue serial** do Supervisor (`enqueue_write`). Logo
//! o id do Supervisor É o id canônico (sem remap). O pty-host (PtyHost) da story 1 não
//! serve aqui (ele toma o writer); reusamos `PtyManager` + reader threads (padrão do
//! `gate_onda0`) que **emitem `GridDelta`** para o `UiHost`. Todo o resto é REUSO do core:
//! `Supervisor`, `deliver_a2a`, `EventStore`, o pub/sub do Bus, `GridSense`, `VtBackend`.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lina_core::{
    deliver_a2a, A2aEnvelope, AlacrittyBackend, BusEvent, CliProfile, DeliveryOutcome, DomainEvent,
    EventStore, GridDelta, InjectPolicy, NodeStatus as CoreStatus, PtyCommand, PtyManager,
    Recipient, RolePolicy, Supervisor, VtBackend,
};
use lina_host::{BusTarget, HostEvent, InputSink, NodeId, NodeKind, NodeStatus, UiHost, WriteOp};
use tokio::sync::broadcast;

/// Viewport de cada terminal (cols x rows).
pub const COLS: u16 = 80;
pub const ROWS: u16 = 22;
/// Quanto tempo o pulso A→B fica visível na tela (efêmero).
pub const PULSE_MS: u64 = 1100;

/// Recupera o guard mesmo se o mutex estiver envenenado (não propaga panic de um nó).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Handle de grid compartilhado: a thread leitora o avança, o render lê (`row_text`) e o
/// `deliver_a2a` o usa como `GridSense` (detecção de prompt). Um único `Arc` para os três.
pub type Grid = Arc<Mutex<Box<dyn VtBackend>>>;

/// Projeção descartável de um nó-terminal — SÓ dados (nenhum tipo de toolkit).
#[derive(Debug, Clone)]
pub struct NodeView {
    pub name: String,
    pub kind: NodeKind,
    pub status: NodeStatus,
    pub rows: Vec<String>,
    /// Posição no canvas (lógica, antes do pan/zoom).
    pub x: f32,
    pub y: f32,
}

impl NodeView {
    pub fn new(name: impl Into<String>, kind: NodeKind, x: f32, y: f32) -> Self {
        Self {
            name: name.into(),
            kind,
            status: NodeStatus::Starting,
            rows: vec![String::new(); ROWS as usize],
            x,
            y,
        }
    }
}

/// Um pulso A→B efêmero (a metáfora "sem fios"): nasce ao escutar `BusEvent::Message`.
#[derive(Debug, Clone, Copy)]
pub struct Pulse {
    pub from: NodeId,
    pub to: NodeId,
    pub started: Instant,
}

impl Pulse {
    /// Progresso 0.0→1.0 do pacote viajando de A para B; `None` quando já expirou.
    #[must_use]
    pub fn progress(&self) -> Option<f32> {
        let t = self.started.elapsed().as_millis() as f32 / PULSE_MS as f32;
        (t <= 1.0).then_some(t)
    }
}

/// Estado projetado que o shell gpui lê a cada frame. SÓ dados.
#[derive(Debug, Default)]
pub struct SharedModel {
    pub nodes: BTreeMap<NodeId, NodeView>,
    pub order: Vec<NodeId>,
    pub recovering: bool,
    /// Pulso A→B atual (efêmero) — desenhado e some em ~`PULSE_MS`.
    pub pulse: Option<Pulse>,
    /// Selo "Time conectado" — liga após o primeiro A2A.
    pub connected: bool,
    /// Eventos persistidos no EventStore (prova de persistência, visível na tela).
    pub event_count: u64,
    pub generation: u64,
}

impl SharedModel {
    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    /// Insere um nó com nome/posição (o `main` semeia os 2 terminais).
    pub fn seed_node(&mut self, node: NodeId, view: NodeView) {
        self.nodes.entry(node).or_insert(view);
        if !self.order.contains(&node) {
            self.order.push(node);
        }
        self.touch();
    }
}

pub type Model = Arc<Mutex<SharedModel>>;

/// **core → UI.** Projeta cada `HostEvent` no `SharedModel`, lendo as linhas sujas do
/// grid (já parseado pelo core) via os handles `Grid`. `Send + 'static`.
pub struct GpuiBridgeHost {
    model: Model,
    grids: BTreeMap<NodeId, Grid>,
}

impl GpuiBridgeHost {
    #[must_use]
    pub fn new(model: Model, grids: BTreeMap<NodeId, Grid>) -> Self {
        Self { model, grids }
    }
}

impl UiHost for GpuiBridgeHost {
    fn on_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::NodeAdded { node, kind } => {
                let mut m = lock(&self.model);
                m.nodes
                    .entry(node)
                    .or_insert_with(|| NodeView::new("terminal", kind, 0.0, 0.0));
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
                // Lê SÓ as linhas sujas do grid parseado PELO core — nunca o PTY.
                let updates: Option<Vec<(usize, String)>> = self.grids.get(&node).map(|grid| {
                    let g = lock(grid);
                    dirty_rows
                        .iter()
                        .copied()
                        .filter(|&r| r < ROWS as usize)
                        .map(|r| (r, g.row_text(r)))
                        .collect()
                });
                if let Some(updates) = updates {
                    let mut m = lock(&self.model);
                    if let Some(nv) = m.nodes.get_mut(&node) {
                        for (r, text) in updates {
                            if r < nv.rows.len() {
                                nv.rows[r] = text;
                            }
                        }
                        m.touch();
                    }
                }
            }
            // O DIFERENCIAL: tráfego A2A no Bus → pulso efêmero + selo "Time conectado".
            HostEvent::BusMessage { from, to } => {
                let target = match to {
                    BusTarget::Node(n) => n,
                    _ => from,
                };
                let mut m = lock(&self.model);
                m.pulse = Some(Pulse {
                    from,
                    to: target,
                    started: Instant::now(),
                });
                m.connected = true;
                m.touch();
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
            _ => {}
        }
    }
}

/// **UI → core.** O shell chama `submit`; agora o **Supervisor** é dono do writer, então
/// o input do humano vai pela **MailQueue serial** (`write_human`) — o caminho canônico.
pub struct CoreInput {
    sup: Arc<Supervisor>,
}

impl CoreInput {
    #[must_use]
    pub fn new(sup: Arc<Supervisor>) -> Self {
        Self { sup }
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
        if let Err(e) = self.sup.write_human(node, &bytes, Duration::from_secs(2)) {
            eprintln!("lina-gpui: input do humano falhou no nó {node}: {e}");
        }
    }
}

/// Gatilho de A2A com pulso (o diferencial). Reúne tudo que `deliver_a2a` precisa + o
/// EventStore (persistência) + o model (contador de eventos). `Send + Sync`.
pub struct A2aTrigger {
    sup: Arc<Supervisor>,
    from: NodeId,
    to: NodeId,
    grid_to: Grid,
    profile: CliProfile,
    store: Arc<Mutex<EventStore>>,
    model: Model,
}

impl A2aTrigger {
    #[must_use]
    pub fn new(
        sup: Arc<Supervisor>,
        from: NodeId,
        to: NodeId,
        grid_to: Grid,
        profile: CliProfile,
        store: Arc<Mutex<EventStore>>,
        model: Model,
    ) -> Self {
        Self {
            sup,
            from,
            to,
            grid_to,
            profile,
            store,
            model,
        }
    }

    /// Dispara o A2A `from → to` numa thread (o `deliver_a2a` bloqueia ~submit_delay):
    /// (1) `sup.route` publica `BusEvent::Message` (→ o pulso, via o pub/sub) ; (2)
    /// `deliver_a2a` injeta o texto FASEADO no PTY do alvo (chega no grid de B) ; (3)
    /// persiste `BusMessageSent` no EventStore.
    pub fn fire(&self, text: String) {
        let sup = Arc::clone(&self.sup);
        let grid_to = Arc::clone(&self.grid_to);
        let store = Arc::clone(&self.store);
        let model = Arc::clone(&self.model);
        let profile = self.profile.clone();
        let from = self.from;
        let to = self.to;
        thread::spawn(move || {
            let env = A2aEnvelope::new(from, Recipient::Node(to), Some("a2a-demo".into()));
            let id = env.id.clone();
            // (1) publica BusEvent::Message → o pulso nasce ao escutar o Bus.
            let _targets = sup.route(&env, RolePolicy::All);
            // (2) injeta o texto faseado no PTY do alvo (bracketed-paste → delay → Enter).
            match deliver_a2a(
                &sup,
                to,
                from,
                &text,
                &profile,
                &grid_to,
                InjectPolicy::AllowAll,
            ) {
                Ok(DeliveryOutcome::Injected { .. } | DeliveryOutcome::SessionResume) => {}
                Err(e) => eprintln!("lina-gpui: deliver_a2a falhou: {e}"),
            }
            // (3) persiste o evento (estado sobrevive).
            let count = {
                let mut s = lock(&store);
                let _ = s.append(&DomainEvent::BusMessageSent {
                    id,
                    from,
                    to: format!("node:{to}"),
                });
                s.event_count().unwrap_or(0)
            };
            let mut m = lock(&model);
            m.event_count = count;
            m.touch();
        });
    }
}

/// `CliProfile` mínimo para a demo: injeção no PTY (`pty_inject`), prompt "pronto" sempre
/// que o grid tem conteúdo (`.`), Enter separado após 150 ms.
#[must_use]
pub fn demo_profile() -> CliProfile {
    let src = r#"
        id = "lina-demo"
        program = "sh"
        delivery = "pty_inject"
        submit_delay_ms = 150
        prompt_ready_regex = '.'
        idle_ms = 200
        ask_timeout_ms = 120000
        [end_signal]
        kind = "idle"
    "#;
    CliProfile::from_toml_str(src, "<demo>").expect("profile da demo deve parsear")
}

/// Mapeia o `NodeStatus` do core (Bus) para o `NodeStatus` do contrato de UI.
fn map_status(s: CoreStatus) -> NodeStatus {
    match s {
        CoreStatus::Starting => NodeStatus::Starting,
        CoreStatus::Running => NodeStatus::Running,
        CoreStatus::Idle => NodeStatus::Idle,
        CoreStatus::Busy => NodeStatus::Busy,
        CoreStatus::Blocked => NodeStatus::Busy,
        CoreStatus::Dead => NodeStatus::Dead,
    }
}

/// Traduz um `BusEvent` real do core em `HostEvent` (sem remap — o id do Supervisor é o
/// id canônico, pois ele é dono dos writers).
pub fn bus_to_host(ev: &BusEvent) -> Option<HostEvent> {
    match ev {
        BusEvent::NodeSpawned { node } => Some(HostEvent::NodeAdded {
            node: *node,
            kind: NodeKind::Terminal,
        }),
        BusEvent::NodeDied { node } => Some(HostEvent::NodeRemoved { node: *node }),
        BusEvent::NodeStatus { node, status } => Some(HostEvent::NodeStatusChanged {
            node: *node,
            status: map_status(*status),
        }),
        BusEvent::Message { from, to, .. } => Some(HostEvent::BusMessage {
            from: *from,
            to: match to {
                Recipient::Node(n) => BusTarget::Node(*n),
                Recipient::Role(r) => BusTarget::Role(r.clone()),
                Recipient::Broadcast => BusTarget::Broadcast,
            },
        }),
    }
}

/// Cabeia um terminal REAL no core: spawna o PTY, dá o **writer ao Supervisor** (input +
/// A2A pela MailQueue), e sobe a thread leitora que avança o grid e **emite `GridDelta`**
/// (o contrato de render). Devolve o `NodeId` canônico (do Supervisor) e o handle do grid
/// (compartilhado: render + `GridSense` do A2A). Reusa o padrão do `gate_onda0`.
pub fn wire_terminal(
    pty: &mut PtyManager,
    sup: &Supervisor,
    delta_tx: &Sender<GridDelta>,
    key: &str,
    name: &str,
    cmd: PtyCommand,
) -> Result<(NodeId, Grid), String> {
    pty.spawn(key, cmd, COLS, ROWS).map_err(|e| e.to_string())?;
    let writer = pty.take_writer(key).map_err(|e| e.to_string())?;
    let node = sup.register(name, Some("terminal".into()), writer);
    let mut reader = pty.clone_reader(key).map_err(|e| e.to_string())?;

    let grid: Grid = Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(COLS, ROWS))));
    {
        let grid = Arc::clone(&grid);
        let delta_tx = delta_tx.clone();
        thread::Builder::new()
            .name(format!("lina-reader-{key}"))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break, // EOF/erro: o processo saiu
                        Ok(n) => {
                            let dirty = {
                                let mut g = lock(&grid);
                                g.advance(&buf[..n]);
                                let d = g.damaged_rows();
                                g.reset_damage();
                                d
                            };
                            if !dirty.is_empty() {
                                // GridDelta no contrato do core (W0-3): índices das linhas sujas.
                                let _ = delta_tx.send(GridDelta {
                                    node,
                                    rows: dirty,
                                    bytes: n,
                                    seq: 0,
                                });
                            }
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok((node, grid))
}

/// Handle da thread-bomba (background) da ponte; encerra no `stop`/`Drop`.
pub struct Pump {
    stop: Arc<std::sync::atomic::AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Pump {
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

/// Sobe a **thread-bomba** (o laço do core que chama `UiHost::on_event`, livre de gpui):
/// drena o Bus (lifecycle/status + **`Message` → pulso**) e os `GridDelta` dos terminais.
pub fn spawn_pump(
    mut bridge: GpuiBridgeHost,
    delta_rx: Receiver<GridDelta>,
    mut bus_rx: broadcast::Receiver<BusEvent>,
) -> std::io::Result<Pump> {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let join = {
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("lina-bridge-pump".into())
            .spawn(move || loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let mut worked = false;
                loop {
                    match bus_rx.try_recv() {
                        Ok(ev) => {
                            if let Some(h) = bus_to_host(&ev) {
                                bridge.on_event(h);
                                worked = true;
                            }
                        }
                        Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                while let Ok(gd) = delta_rx.try_recv() {
                    bridge.on_event(HostEvent::GridDelta {
                        node: gd.node,
                        dirty_rows: gd.rows,
                    });
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

    /// **O GATE da Onda 2, sem display:** 2 terminais reais cabeados no Supervisor; A2A
    /// A→B via `deliver_a2a`; `sup.route` publica `BusEvent::Message` → o pulso liga + a
    /// aura "Time conectado"; o texto A2A CHEGA no grid de B (round-trip faseado real); e
    /// um evento é persistido no EventStore.
    #[test]
    fn a2a_roundtrip_pulse_and_persist() {
        let dir = std::env::temp_dir().join(format!("lina-ws2-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(EventStore::open(&dir).expect("open store")));
        let before = lock(&store).event_count().expect("count");

        let mut pty = PtyManager::new();
        let sup = Arc::new(Supervisor::new());
        let bus_rx = sup.subscribe();
        let (delta_tx, delta_rx) = std::sync::mpsc::channel::<GridDelta>();

        let (node_a, grid_a) = wire_terminal(
            &mut pty,
            &sup,
            &delta_tx,
            "A",
            "Terminal A",
            PtyCommand::new("cat"),
        )
        .expect("wire A");
        let (node_b, grid_b) = wire_terminal(
            &mut pty,
            &sup,
            &delta_tx,
            "B",
            "Terminal B",
            PtyCommand::new("cat"),
        )
        .expect("wire B");

        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        {
            let mut m = lock(&model);
            m.seed_node(
                node_a,
                NodeView::new("Terminal A", NodeKind::Terminal, 40.0, 80.0),
            );
            m.seed_node(
                node_b,
                NodeView::new("Terminal B", NodeKind::Terminal, 560.0, 80.0),
            );
        }
        let mut grids = BTreeMap::new();
        grids.insert(node_a, Arc::clone(&grid_a));
        grids.insert(node_b, Arc::clone(&grid_b));
        let mut bridge = GpuiBridgeHost::new(Arc::clone(&model), grids);

        // A2A A→B: route (publica Message) + deliver_a2a (injeta) + persist.
        let env = A2aEnvelope::new(node_a, Recipient::Node(node_b), Some("a2a".into()));
        let id = env.id.clone();
        let targets = sup.route(&env, RolePolicy::All);
        assert!(targets.contains(&node_b), "route deve resolver o alvo B");
        let out = deliver_a2a(
            &sup,
            node_b,
            node_a,
            "LINA_A2A_MARKER",
            &demo_profile(),
            &grid_b,
            InjectPolicy::AllowAll,
        )
        .expect("deliver_a2a");
        assert!(matches!(out, DeliveryOutcome::Injected { .. }));
        lock(&store)
            .append(&DomainEvent::BusMessageSent {
                id,
                from: node_a,
                to: format!("node:{node_b}"),
            })
            .expect("append");

        // Pump o Bus → bridge: o BusEvent::Message vira o pulso + a aura.
        let mut bus_rx = bus_rx;
        while let Ok(ev) = bus_rx.try_recv() {
            if let Some(h) = bus_to_host(&ev) {
                bridge.on_event(h);
            }
        }
        assert!(
            lock(&model).pulse.is_some(),
            "BusEvent::Message deve acender o pulso A→B"
        );
        assert!(lock(&model).connected, "a aura 'Time conectado' deve ligar");

        // Round-trip: o texto A2A chega no grid de B (via GridDelta → bridge).
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut found = false;
        while Instant::now() < deadline && !found {
            while let Ok(gd) = delta_rx.try_recv() {
                bridge.on_event(HostEvent::GridDelta {
                    node: gd.node,
                    dirty_rows: gd.rows,
                });
            }
            let joined = {
                let m = lock(&model);
                m.nodes
                    .get(&node_b)
                    .map(|n| n.rows.join("\n"))
                    .unwrap_or_default()
            };
            found = joined.contains("LINA_A2A_MARKER");
            if !found {
                thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(
            found,
            "o texto A2A deve CHEGAR no grid do terminal B (round-trip real)"
        );

        let after = lock(&store).event_count().expect("count");
        assert!(
            after > before,
            "o evento A2A deve ser persistido no EventStore"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `Recovering`/`Recovered` alternam o banner.
    #[test]
    fn recovery_pair_toggles_banner() {
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let mut bridge = GpuiBridgeHost::new(Arc::clone(&model), BTreeMap::new());
        assert!(!lock(&model).recovering);
        bridge.on_event(HostEvent::Recovering);
        assert!(lock(&model).recovering);
        bridge.on_event(HostEvent::Recovered);
        assert!(!lock(&model).recovering);
    }
}
