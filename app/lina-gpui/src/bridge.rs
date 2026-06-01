//! A PONTE real core <-> shell (Onda 2 · render de terminal de verdade). **Livre de tipos
//! de toolkit** (zero gpui): projeção (`SharedModel`), `impl UiHost`, `impl InputSink`, a
//! fiação de 2 terminais reais e o gatilho de A2A com pulso.
//!
//! ## Render: snapshot do grid, não deltas empilhados
//! O conteúdo do terminal NÃO vive mais no `SharedModel` (não há `rows` de texto). O shell
//! lê o **snapshot completo do grid visível** (`VtBackend::screen()`, com cores e cursor)
//! direto do handle `Grid` **a cada frame**. Assim TUIs que redesenham a tela inteira
//! (Claude Code, vim, …) aparecem corretas. O `SharedModel` só carrega metadados do nó
//! (nome, status, posição), o pulso A2A e o contador de persistência.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Quanto tempo o pulso A→B fica visível na tela (efêmero).
pub const PULSE_MS: u64 = 1100;

/// Recupera o guard mesmo se o mutex estiver envenenado (não propaga panic de um nó).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Handle de grid compartilhado: a thread leitora o avança, o render lê o `screen()` e o
/// `deliver_a2a` o usa como `GridSense`. Um único `Arc` para os três.
pub type Grid = Arc<Mutex<Box<dyn VtBackend>>>;

/// Metadados projetados de um nó-terminal (SEM o conteúdo do grid — esse vem do `Grid`).
#[derive(Debug, Clone)]
pub struct NodeView {
    pub name: String,
    pub kind: NodeKind,
    pub status: NodeStatus,
    pub x: f32,
    pub y: f32,
}

impl NodeView {
    pub fn new(name: impl Into<String>, kind: NodeKind, x: f32, y: f32) -> Self {
        Self {
            name: name.into(),
            kind,
            status: NodeStatus::Starting,
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

/// Estado projetado (metadados + pulso + persistência). SÓ dados.
#[derive(Debug, Default)]
pub struct SharedModel {
    pub nodes: BTreeMap<NodeId, NodeView>,
    pub order: Vec<NodeId>,
    pub recovering: bool,
    pub pulse: Option<Pulse>,
    pub connected: bool,
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

/// **core → UI.** Projeta o lifecycle/status do nó + o pulso A2A no `SharedModel`. O
/// `GridDelta` é só drenado (o render lê o snapshot do grid direto). `Send + 'static`.
pub struct GpuiBridgeHost {
    model: Model,
}

impl GpuiBridgeHost {
    #[must_use]
    pub fn new(model: Model) -> Self {
        Self { model }
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
            // O render lê o snapshot do grid direto a cada frame; o delta só é drenado.
            HostEvent::GridDelta { .. } => {}
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

/// **UI → core.** Input do humano pela MailQueue serial do Supervisor (`write_human`).
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

/// Gatilho de A2A com pulso (o diferencial). `Send + Sync`.
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

    /// Dispara o A2A `from → to` numa thread: (1) `sup.route` publica `BusEvent::Message`
    /// (→ o pulso, via o pub/sub); (2) `deliver_a2a` injeta o texto FASEADO no PTY do alvo
    /// (chega no grid de B); (3) persiste `BusMessageSent` no EventStore.
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
            let _targets = sup.route(&env, RolePolicy::All);
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
            let count = {
                let mut s = lock(&store);
                let _ = s.append(&DomainEvent::BusMessageSent {
                    id,
                    from,
                    to: format!("node:{to}"),
                });
                s.event_count().ok()
            };
            let mut m = lock(&model);
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        });
    }

    /// O gatilho A→B só faz sentido com **ambos** os nós vivos. A UI usa isto para esconder o
    /// botão ⚡ quando o usuário fecha A ou B (nunca um botão que falha em silêncio).
    #[must_use]
    pub fn ready(&self) -> bool {
        let m = lock(&self.model);
        m.nodes.contains_key(&self.from) && m.nodes.contains_key(&self.to)
    }
}

/// `CliProfile` mínimo para a demo: injeção no PTY (`pty_inject`), pronto sempre que o
/// grid tem conteúdo (`.`), Enter separado após 150 ms.
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

/// Cabeia um terminal REAL: spawna o PTY no tamanho `cols×rows`, dá o **writer ao
/// Supervisor** (input + A2A), e sobe a thread leitora que avança o grid e **emite
/// `GridDelta`** (sinal de "mudou"). Devolve o `NodeId` canônico + o handle do grid.
#[allow(clippy::too_many_arguments)]
pub fn wire_terminal(
    pty: &mut PtyManager,
    sup: &Supervisor,
    delta_tx: &Sender<GridDelta>,
    key: &str,
    name: &str,
    cmd: PtyCommand,
    cols: u16,
    rows: u16,
) -> Result<(NodeId, Grid), String> {
    pty.spawn(key, cmd, cols, rows).map_err(|e| e.to_string())?;
    let writer = pty.take_writer(key).map_err(|e| e.to_string())?;
    let node = sup.register(name, Some("terminal".into()), writer);
    let mut reader = pty.clone_reader(key).map_err(|e| e.to_string())?;

    let grid: Grid = Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(cols, rows))));
    {
        let grid = Arc::clone(&grid);
        let delta_tx = delta_tx.clone();
        thread::Builder::new()
            .name(format!("lina-reader-{key}"))
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let dirty = {
                                let mut g = lock(&grid);
                                g.advance(&buf[..n]);
                                let d = g.damaged_rows();
                                g.reset_damage();
                                d
                            };
                            if !dirty.is_empty() {
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

/// Layout do card no canvas (fonte única — o render em `main` reusa estes valores).
pub const CARD_W: f32 = 680.0;
pub const CARD_H: f32 = 500.0;

/// Fábrica do comando de um nó: um **SHELL INTERATIVO REAL** — o do usuário (`$SHELL`, com
/// fallback `/bin/sh`), idêntico para TODOS os nós (aceita teclado, roda claude/vim/…). Mostra
/// um banner neutro e dá `exec` no shell (sem mock/`cat`). É gpui-free (vive aqui, não no main).
#[must_use]
pub fn shell_cmd(name: &str) -> PtyCommand {
    PtyCommand::new("sh")
        .arg("-c")
        .arg(format!(
            "printf '{name} — shell interativo (digite comandos; rode claude/vim/…).\\r\\n'; \
             exec \"${{SHELL:-/bin/sh}}\" -i"
        ))
        .env("TERM", "xterm-256color")
}

/// Fábrica injetável de comando por nome: o app passa [`shell_cmd`] (shell real); os testes
/// passam um comando leve (`cat`) p/ serem determinísticos e headless.
pub type CmdFactory = Arc<dyn Fn(&str) -> PtyCommand + Send + Sync>;

/// Rótulo amigável do `seq`-ésimo terminal: A, B, C… Z, depois `#27`, `#28`, …
fn node_label(seq: u32) -> String {
    if seq < 26 {
        ((b'A' + seq as u8) as char).to_string()
    } else {
        format!("#{}", seq + 1)
    }
}

/// Primeira posição numa grade que NÃO sobrepõe nenhum card existente (lê o `model`). Garante
/// o invariante "posicionar sem sobrepor"; o pan do canvas revela colunas/linhas além da tela.
#[must_use]
fn next_free_slot(model: &SharedModel) -> (f32, f32) {
    const MARGIN_X: f32 = 30.0;
    const MARGIN_Y: f32 = 96.0;
    const GAP: f32 = 30.0;
    const COLS: u32 = 4;
    let existing: Vec<(f32, f32)> = model
        .order
        .iter()
        .filter_map(|n| model.nodes.get(n))
        .map(|v| (v.x, v.y))
        .collect();
    for k in 0u32..10_000 {
        let x = MARGIN_X + (k % COLS) as f32 * (CARD_W + GAP);
        let y = MARGIN_Y + (k / COLS) as f32 * (CARD_H + GAP);
        let collides = existing.iter().any(|&(ex, ey)| {
            x < ex + CARD_W + GAP
                && x + CARD_W + GAP > ex
                && y < ey + CARD_H + GAP
                && y + CARD_H + GAP > ey
        });
        if !collides {
            return (x, y);
        }
    }
    (MARGIN_X, MARGIN_Y)
}

/// Limites de zoom do canvas (escala). Fora deles o zoom satura.
pub const ZOOM_MIN: f32 = 0.4;
pub const ZOOM_MAX: f32 = 2.0;

/// **Câmera 2D do canvas (gpui-free → testável):** `pan` (translação em px de TELA) + `zoom`
/// (escala). É o coração do scene-graph 2D: `screen = world * zoom + pan`. O render aplica isto
/// por card; o culling e o hit-test usam a MESMA transformação (uma só fonte da verdade).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub pan: (f32, f32),
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: (0.0, 0.0),
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Mundo → tela.
    #[must_use]
    pub fn world_to_screen(&self, w: (f32, f32)) -> (f32, f32) {
        (w.0 * self.zoom + self.pan.0, w.1 * self.zoom + self.pan.1)
    }

    /// Tela → mundo (inverso exato de [`Self::world_to_screen`]).
    #[must_use]
    pub fn screen_to_world(&self, s: (f32, f32)) -> (f32, f32) {
        // O zoom é sempre clampado em [ZOOM_MIN, ZOOM_MAX] (>0) via `zoom_by`/`reset`/`default`;
        // o assert documenta o invariante e pega regressão de quem mexer no campo direto.
        debug_assert!(
            self.zoom > 0.0 && self.zoom.is_finite(),
            "Camera::zoom deve ser positivo e finito"
        );
        (
            (s.0 - self.pan.0) / self.zoom,
            (s.1 - self.pan.1) / self.zoom,
        )
    }

    /// Volta a câmera ao "home": pan zerado, zoom 1.0 (⌘0 / botão 🏠) — resgata a vista quando
    /// o usuário se perde no canvas (invariante não-técnico: nunca ficar sem nada visível).
    pub fn reset(&mut self) {
        *self = Camera::default();
    }

    /// Zoom multiplicando por `factor`, mantendo o ponto de MUNDO sob `cursor` (em tela) fixo —
    /// é o que impede o zoom "sob o cursor" de escorregar. Clampa em [`ZOOM_MIN`, `ZOOM_MAX`].
    pub fn zoom_by(&mut self, cursor: (f32, f32), factor: f32) {
        let new_zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let w = self.screen_to_world(cursor);
        self.zoom = new_zoom;
        self.pan = (cursor.0 - w.0 * new_zoom, cursor.1 - w.1 * new_zoom);
    }

    /// Ajuste MÍNIMO do `pan` para trazer um card (canto sup-esq em coords de MUNDO) inteiro
    /// para dentro da `viewport`, na escala atual, respeitando a barra superior. É o auto-pan
    /// que faz o nó recém-criado aparecer (o `next_free_slot` pode pôr o card além da janela).
    pub fn reveal(&mut self, card_world: (f32, f32), card_size: (f32, f32), viewport: (f32, f32)) {
        const MARGIN: f32 = 16.0;
        const TOPBAR: f32 = 44.0;
        let (cw, ch) = (card_size.0 * self.zoom, card_size.1 * self.zoom);
        // 2 iterações convergem nas DUAS bordas: o caso normal resolve na 1ª; se o card for
        // maior que (viewport - margens) — ex.: janela encolhida — a 2ª prioriza esq/topo.
        for _ in 0..2 {
            let (sx, sy) = self.world_to_screen(card_world);
            if sx + cw > viewport.0 - MARGIN {
                self.pan.0 -= sx + cw - (viewport.0 - MARGIN);
            }
            let sx = self.world_to_screen(card_world).0;
            if sx < MARGIN {
                self.pan.0 += MARGIN - sx;
            }
            if sy + ch > viewport.1 - MARGIN {
                self.pan.1 -= sy + ch - (viewport.1 - MARGIN);
            }
            let sy = self.world_to_screen(card_world).1;
            if sy < TOPBAR + MARGIN {
                self.pan.1 += TOPBAR + MARGIN - sy;
            }
        }
    }
}

/// **CULLING:** o card (canto sup-esq em mundo, tamanho `card_size`) intersecta a viewport
/// visível sob a câmera? O render pula os invisíveis — com muitos nós, só os visíveis trabalham.
#[must_use]
pub fn card_visible(
    cam: &Camera,
    world_pos: (f32, f32),
    card_size: (f32, f32),
    viewport: (f32, f32),
) -> bool {
    let (sx, sy) = cam.world_to_screen(world_pos);
    let (sw, sh) = (card_size.0 * cam.zoom, card_size.1 * cam.zoom);
    sx < viewport.0 && sx + sw > 0.0 && sy < viewport.1 && sy + sh > 0.0
}

/// **HIT-TEST:** qual card está sob `screen_pt`? Recebe os cards em ordem de z CRESCENTE
/// (último = topo); devolve o de MAIOR z cujo retângulo de tela contém o ponto (resolução de
/// sobreposição). `None` se o ponto caiu no fundo do canvas.
#[must_use]
pub fn hit_test(
    cam: &Camera,
    screen_pt: (f32, f32),
    cards_z_asc: &[(NodeId, (f32, f32))],
    card_size: (f32, f32),
    viewport: (f32, f32),
) -> Option<NodeId> {
    let (sw, sh) = (card_size.0 * cam.zoom, card_size.1 * cam.zoom);
    cards_z_asc
        .iter()
        .rev()
        // Só cards VISÍVEIS são hittable: um card cullado (off-screen) não tem elemento na tela,
        // logo não pode "roubar" o clique/scroll e criar uma zona morta de pan/zoom.
        .filter(|(_, pos)| card_visible(cam, *pos, card_size, viewport))
        .find_map(|(id, world_pos)| {
            let (sx, sy) = cam.world_to_screen(*world_pos);
            let inside = screen_pt.0 >= sx
                && screen_pt.0 <= sx + sw
                && screen_pt.1 >= sy
                && screen_pt.1 <= sy + sh;
            inside.then_some(*id)
        })
}

/// **Fábrica de nós em runtime (gpui-free → testável headless).** Detém os handles do core
/// (`PtyManager`, `Supervisor`, `EventStore`), a projeção (`SharedModel`) e os grids, e sabe
/// ADICIONAR e REMOVER nós-terminal mantendo o **event log como fonte da verdade** (emite
/// `NodeAdded`/`TerminalSpawned`/`NodeRemoved` persistidos e reconstruíveis). Sem `unwrap` em
/// caminho de produção; sem panic ao remover (o reader fecha sozinho no EOF do PTY morto).
pub struct NodeManager {
    pty: Arc<Mutex<PtyManager>>,
    sup: Arc<Supervisor>,
    store: Arc<Mutex<EventStore>>,
    pub model: Model,
    pub grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    keys: Mutex<BTreeMap<NodeId, String>>,
    delta_tx: Sender<GridDelta>,
    cmd_factory: CmdFactory,
    cols: u16,
    rows: u16,
    seq: Mutex<u32>,
}

impl NodeManager {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        pty: Arc<Mutex<PtyManager>>,
        sup: Arc<Supervisor>,
        store: Arc<Mutex<EventStore>>,
        model: Model,
        grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
        keys: BTreeMap<NodeId, String>,
        delta_tx: Sender<GridDelta>,
        cols: u16,
        rows: u16,
        seq_start: u32,
        cmd_factory: CmdFactory,
    ) -> Self {
        Self {
            pty,
            sup,
            store,
            model,
            grids,
            keys: Mutex::new(keys),
            delta_tx,
            cmd_factory,
            cols,
            rows,
            seq: Mutex::new(seq_start),
        }
    }

    /// Nº de nós vivos (projeção).
    #[must_use]
    pub fn count(&self) -> usize {
        lock(&self.model).order.len()
    }

    /// A lista ORDENADA de cards a renderizar — exatamente o que o `render` itera. O conjunto
    /// de cards na tela DERIVA daqui (não de nós fixos), então add/remove refletem na UI.
    #[must_use]
    pub fn cards(&self) -> Vec<(NodeId, NodeView)> {
        let m = lock(&self.model);
        m.order
            .iter()
            .filter_map(|n| m.nodes.get(n).map(|v| (*n, v.clone())))
            .collect()
    }

    /// ADICIONA um nó: um shell real novo (idêntico aos demais) com PTY + grid próprios,
    /// registrado no Supervisor, posicionado **sem sobrepor**, com `NodeAdded` +
    /// `TerminalSpawned` persistidos. Devolve o `NodeId` (para focar).
    ///
    /// Ordem: spawna o PTY → **persiste primeiro** (event log = fonte da verdade) → só então
    /// projeta no model. Se a persistência falhar, **desfaz** o PTY recém-criado (`retire_pty`)
    /// e devolve `Err` — nada de nó visível sem evento no log.
    pub fn add_node(&self) -> Result<NodeId, String> {
        let seq = {
            let mut s = lock(&self.seq);
            let v = *s;
            *s = s.wrapping_add(1);
            v
        };
        let name = format!("Terminal {}", node_label(seq));
        let key = format!("t{seq}");
        let cmd = (*self.cmd_factory)(&name);

        // Slot calculado ANTES de o nó existir: imune ao placeholder (0,0) que o pump cria.
        let (x, y) = next_free_slot(&lock(&self.model));

        let (node, grid) = {
            let mut p = lock(&self.pty);
            wire_terminal(
                &mut p,
                &self.sup,
                &self.delta_tx,
                &key,
                &name,
                cmd,
                self.cols,
                self.rows,
            )?
        };

        // PERSISTE antes de tocar o model. Se falhar, desfaz o nó (sem ghost no model/log).
        let count = {
            let mut s = lock(&self.store);
            if let Err(e) = s.append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: f64::from(x),
                y: f64::from(y),
            }) {
                drop(s);
                self.retire_pty(node, key);
                return Err(e.to_string());
            }
            if let Err(e) = s.append(&DomainEvent::TerminalSpawned {
                node,
                cli: name.clone(),
            }) {
                drop(s);
                self.retire_pty(node, key);
                return Err(e.to_string());
            }
            s.event_count().ok()
        };

        // Projeção: INSERE sobrescrevendo qualquer placeholder do pump (corrida do Bus).
        {
            let mut m = lock(&self.model);
            m.nodes
                .insert(node, NodeView::new(name.clone(), NodeKind::Terminal, x, y));
            if !m.order.contains(&node) {
                m.order.push(node);
            }
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }

        // Running (emite NodeStatus → o pump pinta o ponto de status do card).
        let _ = self.sup.set_status(node, CoreStatus::Running);

        lock(&self.grids).insert(node, grid);
        lock(&self.keys).insert(node, key);
        Ok(node)
    }

    /// REMOVE um nó: **persiste `NodeRemoved` primeiro** (event log = fonte da verdade; se
    /// falhar, nada muda e a operação é retriável), depois tira do model/grids/keys (feedback
    /// visual instantâneo) e encerra o PTY em **thread de fundo** via [`Self::retire_pty`] (não
    /// congela a UI; o reader fecha sozinho no EOF). **NUNCA** remove o último — o canvas jamais
    /// fica em branco (invariante não-técnico). Sem panic.
    pub fn remove_node(&self, node: NodeId) -> Result<(), String> {
        if self.count() <= 1 {
            return Err("o canvas nunca fica em branco: não removo o último nó".into());
        }
        let key = lock(&self.keys)
            .get(&node)
            .cloned()
            .ok_or_else(|| format!("nó {node} sem PTY registrado"))?;

        // PERSISTE primeiro: o replay reconstrói SEM este nó. Falha aqui não muda nada.
        let count = {
            let mut s = lock(&self.store);
            s.append(&DomainEvent::NodeRemoved { node })
                .map_err(|e| e.to_string())?;
            s.event_count().ok()
        };

        // Projeção: tira do model, dos grids e das keys (instantâneo).
        {
            let mut m = lock(&self.model);
            m.nodes.remove(&node);
            m.order.retain(|n| *n != node);
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }
        lock(&self.grids).remove(&node);
        lock(&self.keys).remove(&node);

        // Encerra o PTY em background (não bloqueia o thread de UI por até 2s).
        self.retire_pty(node, key);
        Ok(())
    }

    /// Encerra um PTY já fora do canvas **sem bloquear o thread de UI**: desregistra do
    /// Supervisor na hora e mata o processo (SIGTERM→SIGKILL, até 2s) numa thread de fundo.
    fn retire_pty(&self, node: NodeId, key: String) {
        if let Err(e) = self.sup.unregister(node) {
            eprintln!("lina-gpui: unregister do nó {node} falhou: {e}");
        }
        let pty = Arc::clone(&self.pty);
        if let Err(e) = thread::Builder::new()
            .name(format!("lina-kill-{key}"))
            .spawn(move || {
                if let Err(e) = lock(&pty).kill(key.as_str(), Duration::from_secs(2)) {
                    eprintln!("lina-gpui: kill do PTY {key} falhou: {e}");
                }
                let _ = lock(&pty).remove(key.as_str());
            })
        {
            eprintln!("lina-gpui: thread de kill não pôde subir: {e}");
        }
    }
}

/// Handle da thread-bomba (background) da ponte; encerra no `stop`/`Drop`.
pub struct Pump {
    stop: Arc<AtomicBool>,
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

/// Sobe a thread-bomba: drena o Bus (lifecycle/status + **`Message` → pulso**) e os
/// `GridDelta` (apenas para não vazar o canal — o render lê o snapshot do grid direto).
pub fn spawn_pump(
    mut bridge: GpuiBridgeHost,
    delta_rx: Receiver<GridDelta>,
    mut bus_rx: broadcast::Receiver<BusEvent>,
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
                while delta_rx.try_recv().is_ok() {
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

    /// Texto visível do grid (via o NOVO acessor `screen()`), p/ asserts.
    fn screen_text(grid: &Grid) -> String {
        let s = lock(grid).screen();
        (0..s.rows)
            .map(|r| s.row(r).iter().map(|c| c.c).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **O GATE da Onda 2 (render):** 2 terminais reais; A2A A→B via `deliver_a2a` →
    /// `BusEvent::Message` acende o pulso + a aura; o texto A2A CHEGA no **snapshot do
    /// grid de B** (`screen()`); um evento é persistido; e o **resize** muda as dims.
    #[test]
    fn a2a_roundtrip_pulse_persist_and_screen() {
        let dir = std::env::temp_dir().join(format!("lina-ws2r-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(EventStore::open(&dir).expect("open store")));
        let before = lock(&store).event_count().expect("count");

        let mut pty = PtyManager::new();
        let sup = Arc::new(Supervisor::new());
        let bus_rx = sup.subscribe();
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();

        let (node_a, _grid_a) = wire_terminal(
            &mut pty,
            &sup,
            &delta_tx,
            "A",
            "Terminal A",
            PtyCommand::new("cat"),
            80,
            24,
        )
        .expect("wire A");
        let (node_b, grid_b) = wire_terminal(
            &mut pty,
            &sup,
            &delta_tx,
            "B",
            "Terminal B",
            PtyCommand::new("cat"),
            80,
            24,
        )
        .expect("wire B");

        // RESIZE: o PTY e o grid acompanham (prova o caminho PtyManager::resize + VtBackend::resize).
        pty.resize("B", 100, 30).expect("resize PTY");
        lock(&grid_b).resize(100, 30);
        assert_eq!(
            lock(&grid_b).dims(),
            (100, 30),
            "o grid deve acompanhar o resize"
        );

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
        let mut bridge = GpuiBridgeHost::new(Arc::clone(&model));

        // A2A A→B: route (publica Message) + deliver_a2a (injeta) + persist.
        let env = A2aEnvelope::new(node_a, Recipient::Node(node_b), Some("a2a".into()));
        let id = env.id.clone();
        let targets = sup.route(&env, RolePolicy::All);
        assert!(targets.contains(&node_b));
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
            "BusEvent::Message deve acender o pulso"
        );
        assert!(lock(&model).connected, "a aura 'Time conectado' deve ligar");

        // Round-trip: o texto A2A chega no SNAPSHOT do grid de B (o novo render path).
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut found = false;
        while Instant::now() < deadline && !found {
            found = screen_text(&grid_b).contains("LINA_A2A_MARKER");
            if !found {
                thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(
            found,
            "o texto A2A deve aparecer no snapshot screen() do terminal B"
        );

        assert!(
            lock(&store).event_count().expect("count") > before,
            "evento deve persistir"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `Recovering`/`Recovered` alternam o banner.
    #[test]
    fn recovery_pair_toggles_banner() {
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let mut bridge = GpuiBridgeHost::new(Arc::clone(&model));
        assert!(!lock(&model).recovering);
        bridge.on_event(HostEvent::Recovering);
        assert!(lock(&model).recovering);
        bridge.on_event(HostEvent::Recovered);
        assert!(!lock(&model).recovering);
    }

    /// **GATE do render, com uma TUI controlada (determinístico):** alimenta ANSI conhecido
    /// (clear + cursor home + texto VERMELHO + NEGRITO) e asserta o `screen()`: caractere,
    /// **cor** (fg vermelho), **negrito** e **posição do cursor**. Depois prova o **scroll**
    /// do scrollback (display_offset sobe e volta).
    #[test]
    fn screen_renders_colors_cursor_and_scroll() {
        use lina_core::AlacrittyBackend;
        let mut b = AlacrittyBackend::new(20, 4);
        // CSI: ESC[2J limpa, ESC[H home, ESC[31m fg vermelho, ESC[0m reset, ESC[1m negrito.
        b.advance(b"\x1b[2J\x1b[H\x1b[31mRED\x1b[0m \x1b[1mBOLD\x1b[0m");
        let s = b.screen();
        let row0: String = s.row(0).iter().map(|c| c.c).collect();
        assert!(row0.starts_with("RED BOLD"), "linha 0 = {row0:?}");
        // Cor: o 'R' (col 0) é vermelho (ANSI 1 = 0xcd0000).
        assert_eq!(s.row(0)[0].c, 'R');
        assert_eq!(
            s.row(0)[0].fg,
            lina_core::VtRgb {
                r: 0xcd,
                g: 0x00,
                b: 0x00
            }
        );
        // Negrito: o 'B' de BOLD (col 4) é negrito; o ' ' (col 3) não.
        assert!(s.row(0)[4].bold, "o B de BOLD deve ser negrito");
        assert!(!s.row(0)[3].bold);
        // Cursor: após "RED BOLD" (8 chars), na col 8 da linha 0, visível.
        assert!(s.cursor.visible, "cursor deve estar visível");
        assert_eq!((s.cursor.line, s.cursor.col), (0, 8));

        // Redraw + SCROLL: enche o scrollback e rola pro passado e de volta.
        for i in 0..50 {
            b.advance(format!("\r\nlinha {i}").as_bytes());
        }
        assert_eq!(b.screen().display_offset, 0, "ao vivo: offset 0");
        b.scroll(5);
        assert_eq!(b.screen().display_offset, 5, "scroll sobe 5 no histórico");
        b.scroll(-100);
        assert_eq!(
            b.screen().display_offset,
            0,
            "scroll desce de volta ao fundo"
        );
    }

    /// Monta um `NodeManager` headless (PTYs `cat`, sem gpui/pump) com 2 nós seed (A, B),
    /// como o `main` faz, e devolve também `store`/`model` para os asserts.
    fn test_manager(tag: &str) -> (NodeManager, Arc<Mutex<EventStore>>, Model) {
        let dir = std::env::temp_dir().join(format!("lina-nm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(EventStore::open(&dir).expect("open store")));

        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        // Fábrica leve: terminais de teste rodam `cat` (determinístico, sem shell real).
        let cmd_factory: CmdFactory = Arc::new(|_name: &str| PtyCommand::new("cat"));

        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let mut keys: BTreeMap<NodeId, String> = BTreeMap::new();

        for (k, name) in [("A", "Terminal A"), ("B", "Terminal B")] {
            let (node, grid) = {
                let mut p = lock(&pty);
                wire_terminal(
                    &mut p,
                    &sup,
                    &delta_tx,
                    k,
                    name,
                    (*cmd_factory)(name),
                    80,
                    24,
                )
                .expect("wire seed")
            };
            lock(&model).seed_node(node, NodeView::new(name, NodeKind::Terminal, 0.0, 0.0));
            lock(&grids).insert(node, grid);
            keys.insert(node, k.to_string());
        }

        let nm = NodeManager::new(
            Arc::clone(&pty),
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&model),
            Arc::clone(&grids),
            keys,
            delta_tx,
            80,
            24,
            2,
            cmd_factory,
        );
        (nm, store, model)
    }

    /// **GATE de ADD (headless, determinístico):** `add_node` sobe a contagem 2→3, dá um grid
    /// ao novo nó, persiste NodeAdded+TerminalSpawned, e o event log RECONSTRÓI o novo nó
    /// (`project()`) — provando "todo nó é shell real + o log é a fonte da verdade".
    #[test]
    fn add_node_grows_count_persists_and_reconstructs() {
        let (nm, store, model) = test_manager("add");
        assert_eq!(lock(&model).order.len(), 2, "começa com 2 nós (A, B)");
        let before = lock(&store).event_count().expect("count");

        let id = nm.add_node().expect("add_node cria um nó");

        assert_eq!(
            lock(&model).order.len(),
            3,
            "add_node sobe a contagem para 3"
        );
        assert!(
            lock(&model).nodes.contains_key(&id),
            "o novo nó está no model"
        );
        assert!(
            lock(&nm.grids).contains_key(&id),
            "o novo nó tem grid próprio"
        );
        assert_eq!(
            lock(&store).event_count().expect("count"),
            before + 2,
            "persiste NodeAdded + TerminalSpawned"
        );
        let projected = lock(&store).project().expect("project");
        assert!(
            projected.nodes.contains_key(&id),
            "o event log reconstrói o novo nó (fonte da verdade)"
        );
    }

    /// **GATE de REMOVE (headless, determinístico):** após add (3 nós), `remove_node` desce a
    /// contagem 3→2, encerra o PTY e tira do grid, persiste NodeRemoved, e o replay fica SEM o
    /// nó. Invariante não-técnico: o ÚLTIMO nó nunca sai (o canvas jamais fica em branco). Sem
    /// panic do começo ao fim.
    #[test]
    fn remove_node_shrinks_count_persists_and_never_empties() {
        let (nm, store, model) = test_manager("remove");
        let id = nm.add_node().expect("add p/ ter o que remover");
        assert_eq!(lock(&model).order.len(), 3);
        let before = lock(&store).event_count().expect("count");

        nm.remove_node(id).expect("remove_node encerra o nó");

        assert_eq!(
            lock(&model).order.len(),
            2,
            "remove_node desce a contagem para 2"
        );
        assert!(!lock(&model).nodes.contains_key(&id), "o nó sai do model");
        assert!(!lock(&nm.grids).contains_key(&id), "o grid é removido");
        assert_eq!(
            lock(&store).event_count().expect("count"),
            before + 1,
            "persiste NodeRemoved"
        );
        let projected = lock(&store).project().expect("project");
        assert!(
            !projected.nodes.contains_key(&id),
            "o replay reconstrói SEM o nó removido"
        );

        // Invariante não-técnico: tenta esvaziar; o ÚLTIMO nó nunca sai.
        let remaining: Vec<NodeId> = lock(&model).order.clone();
        for n in remaining {
            let _ = nm.remove_node(n);
        }
        assert_eq!(
            lock(&model).order.len(),
            1,
            "sempre sobra ao menos 1 nó (nunca tela em branco)"
        );
        let last = lock(&model).order[0];
        assert!(
            nm.remove_node(last).is_err(),
            "remover o último deve ser recusado"
        );
    }

    /// **Regressão do bug do fundador (➕ não mostra card):** o conjunto de cards que o `render`
    /// desenha DERIVA do NodeManager (não de 2 fixos). `cards()` é exatamente o que o render
    /// itera — add entra na lista, remove sai.
    #[test]
    fn rendered_cards_derive_from_node_manager() {
        let (nm, _store, _model) = test_manager("cards");
        let initial: Vec<NodeId> = nm.cards().iter().map(|(id, _)| *id).collect();
        assert_eq!(initial.len(), 2, "começa com 2 cards (A, B)");

        let id = nm.add_node().expect("add");
        let after_add: Vec<NodeId> = nm.cards().iter().map(|(id, _)| *id).collect();
        assert_eq!(after_add.len(), 3, "o card novo entra na lista renderizada");
        assert!(
            after_add.contains(&id),
            "o card renderizado novo é o nó adicionado"
        );

        nm.remove_node(id).expect("remove");
        let after_remove: Vec<NodeId> = nm.cards().iter().map(|(id, _)| *id).collect();
        assert_eq!(
            after_remove.len(),
            2,
            "o card removido sai da lista renderizada"
        );
        assert!(
            !after_remove.contains(&id),
            "o card removido não é mais renderizado"
        );
    }

    /// **Regressão da CAUSA-RAIZ (card novo nasce fora da viewport):** na janela 1450×640 o 3º
    /// card cai em x≈1450 — totalmente FORA da tela com pan=0 (era por isso que "nada aparecia").
    /// `Camera::reveal` traz o card inteiramente para dentro do viewport — destrava o ➕.
    #[test]
    fn new_card_slot_is_offscreen_and_reveal_fixes_it() {
        let viewport = (1450.0_f32, 640.0_f32);

        // Reproduz o layout: 2 nós em (30,96) e (740,96) → próximo slot livre é o do 3º card.
        let mut model = SharedModel::default();
        model.seed_node(
            uuid::Uuid::from_u128(1),
            NodeView::new("Terminal A", NodeKind::Terminal, 30.0, 96.0),
        );
        model.seed_node(
            uuid::Uuid::from_u128(2),
            NodeView::new("Terminal B", NodeKind::Terminal, 740.0, 96.0),
        );
        let (cx, cy) = next_free_slot(&model);

        // BUG: com pan=(0,0) o card [cx, cx+CARD_W] passa da borda direita da viewport.
        let visible_at_zero =
            cx >= 0.0 && cx + CARD_W <= viewport.0 && cy >= 0.0 && cy + CARD_H <= viewport.1;
        assert!(
            !visible_at_zero,
            "com pan=0 o card novo (x={cx}, +{CARD_W}) NÃO cabe na viewport {} — é o bug",
            viewport.0
        );

        // FIX: Camera::reveal traz o card INTEIRAMENTE para dentro da viewport (escala atual).
        let mut cam = Camera::default();
        cam.reveal((cx, cy), (CARD_W, CARD_H), viewport);
        let (left, top) = cam.world_to_screen((cx, cy));
        assert!(left >= 0.0, "borda esquerda dentro (left={left})");
        assert!(
            left + CARD_W <= viewport.0,
            "borda direita dentro (right={})",
            left + CARD_W
        );
        assert!(top >= 0.0, "borda de cima dentro (top={top})");
        assert!(
            top + CARD_H <= viewport.1,
            "borda de baixo dentro (bottom={})",
            top + CARD_H
        );
    }

    /// **GATE W2-2 — transform da câmera (round-trip tela↔mundo):** `screen_to_world` desfaz
    /// `world_to_screen` para vários pan/zoom.
    #[test]
    fn camera_transform_roundtrips() {
        for &(pan, zoom) in &[
            ((0.0, 0.0), 1.0_f32),
            ((120.0, -45.0), 0.5),
            ((-300.0, 80.0), 1.8),
        ] {
            let cam = Camera { pan, zoom };
            for &w in &[(0.0_f32, 0.0_f32), (740.0, 96.0), (1450.0, 626.0)] {
                let s = cam.world_to_screen(w);
                let back = cam.screen_to_world(s);
                assert!(
                    (back.0 - w.0).abs() < 0.01 && (back.1 - w.1).abs() < 0.01,
                    "round-trip falhou: w={w:?} pan={pan:?} zoom={zoom} -> back={back:?}"
                );
            }
        }
    }

    /// **GATE W2-2 — zoom mantém o ponto sob o cursor estável + clampa nos limites.**
    #[test]
    fn zoom_keeps_cursor_point_stable_and_clamps() {
        let mut cam = Camera::default();
        let cursor = (700.0, 300.0);
        let world_before = cam.screen_to_world(cursor);
        cam.zoom_by(cursor, 1.5);
        let world_after = cam.screen_to_world(cursor);
        assert!(
            (world_before.0 - world_after.0).abs() < 0.01
                && (world_before.1 - world_after.1).abs() < 0.01,
            "o ponto de mundo sob o cursor deve ficar estável no zoom"
        );
        for _ in 0..50 {
            cam.zoom_by(cursor, 2.0);
        }
        assert!(
            (cam.zoom - ZOOM_MAX).abs() < 1e-4,
            "zoom satura em ZOOM_MAX (={ZOOM_MAX})"
        );
        for _ in 0..50 {
            cam.zoom_by(cursor, 0.5);
        }
        assert!(
            (cam.zoom - ZOOM_MIN).abs() < 1e-4,
            "zoom satura em ZOOM_MIN (={ZOOM_MIN})"
        );
    }

    /// **GATE W2-2 — culling: cards fora da viewport não geram render.**
    #[test]
    fn culling_marks_offscreen_cards() {
        let cam = Camera::default();
        let vp = (1450.0, 640.0);
        assert!(
            card_visible(&cam, (30.0, 96.0), (CARD_W, CARD_H), vp),
            "card on-screen é visível"
        );
        assert!(
            !card_visible(&cam, (5000.0, 5000.0), (CARD_W, CARD_H), vp),
            "card muito longe é cullado"
        );
        let panned = Camera {
            pan: (-3000.0, 0.0),
            zoom: 1.0,
        };
        assert!(
            !card_visible(&panned, (30.0, 96.0), (CARD_W, CARD_H), vp),
            "pan tira o card da viewport"
        );
    }

    /// **GATE W2-2 — hit-test: sobreposição resolve pelo MAIOR z; fora de tudo é None.**
    #[test]
    fn hit_test_resolves_overlap_by_z_top() {
        let cam = Camera::default();
        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        // a e b sobrepostos. Ordem z-CRESCENTE [a, b] → b é o topo.
        let vp = (1450.0_f32, 640.0_f32);
        let cards = vec![(a, (100.0_f32, 100.0_f32)), (b, (140.0_f32, 130.0_f32))];
        let pt = (200.0, 200.0); // dentro de ambos
        assert_eq!(
            hit_test(&cam, pt, &cards, (CARD_W, CARD_H), vp),
            Some(b),
            "o de maior z (topo) recebe o hit"
        );
        let cards_rev = vec![(b, (140.0_f32, 130.0_f32)), (a, (100.0_f32, 100.0_f32))];
        assert_eq!(
            hit_test(&cam, pt, &cards_rev, (CARD_W, CARD_H), vp),
            Some(a),
            "o topo muda com o z"
        );
        assert_eq!(
            hit_test(&cam, (1000.0, 600.0), &cards, (CARD_W, CARD_H), vp),
            None,
            "fora de todos os cards é None"
        );
        // Card CULLADO (fora da viewport) NÃO é hittable — sem zona morta de pan/zoom.
        let far = vec![(a, (5000.0_f32, 5000.0_f32))];
        assert_eq!(
            hit_test(&cam, (5010.0, 5010.0), &far, (CARD_W, CARD_H), vp),
            None,
            "card cullado (off-screen) não recebe hit"
        );
    }

    /// **GATE W2-2 — `reset` volta a câmera ao home** (resgate da vista: ⌘0 / 🏠).
    #[test]
    fn camera_reset_goes_home() {
        let mut cam = Camera {
            pan: (-2500.0, 800.0),
            zoom: 1.7,
        };
        cam.reset();
        assert_eq!(cam, Camera::default());
        assert_eq!(cam.pan, (0.0, 0.0));
        assert!((cam.zoom - 1.0).abs() < 1e-6);
    }
}
