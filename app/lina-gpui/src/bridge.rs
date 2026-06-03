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

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// W3-2: motor de bootstrap turno-0 (gera o `<workspace>/CLAUDE.md` por terminal).
use lina_bootstrap::{Autonomy, BootstrapInput, Bootstrapper};
// W4-2 (M2): deriva o papel do agente pelo nome ("Revisor" -> reviewer).
use lina_role_discovery::RoleRegistry;

use lina_core::{
    deliver_a2a, lookup_action, now_ms, run_custody, A2aEnvelope, AgentPresence, AlacrittyBackend,
    BrokerOutcome, BrokerRequest, BusEvent, CliProfile, CostLedger, DeliveryOutcome, DomainEvent,
    EventStore, GridDelta, InjectPolicy, MailMessage, Mailbox, NodeStatus as CoreStatus,
    PtyCommand, PtyManager, Recipient, RolePolicy, RouteOutcome, Router, RouterConfig, Supervisor,
    VtBackend,
};
use lina_host::{BusTarget, HostEvent, InputSink, NodeId, NodeKind, NodeStatus, UiHost, WriteOp};
// W3-6c (ADR 0004): cofre de segredos — o broker lê o token daqui (o agente nunca o tem no env).
use lina_secrets::{SecretStore, SecretVault};
// W2-4: tipos de mouse reporting do motor VT (não re-exportados pelo core) — para `encode_mouse`.
use lina_vt::{MouseButton as VtBtn, MouseEvent, MouseEventKind, MouseModifiers};
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
    /// W3-7c · TETO DE CUSTO: `true` quando o `CostLedger` está em `Paused` (uso ≥ teto). Espelhado
    /// pela `MailboxPump`; o render mostra o banner "teto atingido — pausado".
    pub cost_paused: bool,
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
                {
                    let mut m = lock(&self.model);
                    m.pulse = Some(Pulse {
                        from,
                        to: target,
                        started: Instant::now(),
                    });
                    m.connected = true;
                    m.touch();
                }
                // INSTRUMENTAÇÃO: nascimento via Bus/projeção (o `A2aTrigger` demo OU um Router que
                // publique `BusEvent::Message`). Distingue, no stderr, do caminho `deliver_fn` real.
                eprintln!(
                    "lina-gpui: pulso NASCEU via=bus/projection from={from} to={target} os_reduce_motion={}",
                    crate::a11y::os_reduce_motion()
                );
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

/// **W3-4 · O supervisor OBSERVA a mailbox.** Numa thread de fundo, drena o `.lina/outbox/`
/// (onde a CLI `lina ask` deposita), roteia pelos guardrails do [`Router`] e ENTREGA ao PTY do
/// alvo via `deliver_a2a` (faseado) usando o grid+perfil de cada terminal. É o **escritor único**
/// de `.lina/` (também reescreve o `agents.json` quando o roster muda). Roda numa thread só → o
/// estado de dedupe/orçamento do `Router` é local (sem locks).
pub struct MailboxPump {
    router: Router,
    sup: Arc<Supervisor>,
    store: Arc<Mutex<EventStore>>,
    grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    profile: CliProfile,
    model: Model,
    /// W4-3: estado do FREIO compartilhado com a UI (que pede pausa/retoma; a pump aplica no Router).
    brake: crate::wiring::Brake,
    /// Último roster escrito no `agents.json` (evita reescrever a cada tick sem mudança).
    last_roster: Vec<AgentPresence>,
    /// W3-7c: último `event_count` em que reconstruímos o `CostLedger` — só re-replaya quando há eventos
    /// novos (evita replay por tick quando nada mudou).
    last_cost_ec: u64,
}

impl MailboxPump {
    /// Cria o pump enraizado na `mailbox` (o `.lina/` compartilhado do workspace). **W4-3:** restaura
    /// o estado do freio do log (inv #6: um app reaberto após pausar continua pausado) e o espelha no
    /// `brake` para a UI.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        sup: Arc<Supervisor>,
        store: Arc<Mutex<EventStore>>,
        grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
        mailbox: Mailbox,
        model: Model,
        brake: crate::wiring::Brake,
        token_budget_day: u64,
    ) -> Self {
        // W3-7c (TETO DE CUSTO): `token_budget_day > 0` ARMA o teto no Router (0 = desligado). O
        // `CostLedger` soma os `TokenUsageReported` (emitidos pela bomba) e PAUSA na transição.
        let config = RouterConfig {
            token_budget_day,
            ..RouterConfig::default()
        };
        let mut router = Router::with_config(Arc::clone(&sup), mailbox, config);
        // W4-3: o freio é event-sourced — reconstrói `paused` do ÚLTIMO Orchestration{Paused,Resumed}.
        if let Err(e) = router.restore_orchestration_state(&lock(&store)) {
            eprintln!("lina-gpui: falha ao restaurar o estado do freio (segue despausado): {e}");
        }
        lock(&brake).paused = router.is_paused();
        Self {
            router,
            sup,
            store,
            grids,
            profile: demo_profile(),
            model,
            brake,
            last_roster: Vec::new(),
            last_cost_ec: 0,
        }
    }

    /// Constrói a closure de ENTREGA faseada (reusada por `pump` e `resume`). Captura clones `Arc`
    /// (own → `'static`). **W4-3:** a entrega REAL acende o PULSO efêmero `from→target` no model — a
    /// metáfora "sem fios" dirigida por evento REAL (`lina ask`), não pelo `A2aTrigger` demo.
    fn deliver_fn(&self) -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
        let sup = Arc::clone(&self.sup);
        let grids = Arc::clone(&self.grids);
        let profile = self.profile.clone();
        let model = Arc::clone(&self.model);
        move |target, from, text| {
            let Some(g) = lock(&grids).get(&target).cloned() else {
                return Err(format!("sem grid para o alvo {target}"));
            };
            let out = deliver_a2a(
                &sup,
                target,
                from,
                text,
                &profile,
                &g,
                InjectPolicy::AllowAll,
            )
            .map_err(|e| e.to_string())?;
            // Pulso efêmero da entrega real (some sozinho via `Pulse::progress`).
            {
                let mut m = lock(&model);
                m.pulse = Some(Pulse {
                    from,
                    to: target,
                    started: Instant::now(),
                });
                m.connected = true;
                m.touch();
            }
            // INSTRUMENTAÇÃO: prova que o `lina ask` REAL acende o pulso (caminho `deliver_fn`, NÃO o
            // `A2aTrigger` demo). O Maestro lê o stderr p/ confirmar o nascimento + `os_reduce_motion`
            // (que o render usa no gate `animate`). Esporádico (1×/entrega) → sem flood.
            eprintln!(
                "lina-gpui: pulso NASCEU via=lina-ask/real from={from} to={target} os_reduce_motion={}",
                crate::a11y::os_reduce_motion()
            );
            Ok(out)
        }
    }

    /// Sobe a thread que observa a mailbox (drena → roteia → entrega) a cada ~120 ms.
    pub fn spawn(mut self) -> JoinHandle<()> {
        thread::spawn(move || loop {
            self.refresh_agents();
            self.tick();
            thread::sleep(Duration::from_millis(120));
        })
    }

    /// Reescreve `agents.json` (o mapa do time que a skill `lina-agent-bus` lê) quando o roster
    /// mudar — o supervisor é o ESCRITOR ÚNICO de `.lina/`.
    fn refresh_agents(&mut self) {
        let roster: Vec<AgentPresence> = self
            .sup
            .list()
            .into_iter()
            .map(|n| AgentPresence {
                name: n.name,
                role: n.role,
                status: format!("{:?}", n.status),
            })
            .collect();
        if roster != self.last_roster {
            if let Err(e) = self.router.mailbox().write_agents(&roster) {
                eprintln!("lina-gpui: falha ao escrever agents.json: {e}");
            }
            self.last_roster = roster;
        }
    }

    /// Um tick: aplica o freio pedido pela UI, drena a mailbox e roteia. A entrega resolve o grid do
    /// alvo, injeta faseado e acende o pulso (via [`Self::deliver_fn`]).
    fn tick(&mut self) {
        // W4-3 FREIO: a UI pediu para alternar? → pausa OU retoma+DRENA a auto-orquestração no Router.
        let toggle = std::mem::take(&mut lock(&self.brake).toggle_requested);
        if toggle {
            {
                let mut store = lock(&self.store);
                if self.router.is_paused() {
                    let mut deliver = self.deliver_fn();
                    match self.router.resume(&mut store, now_ms(), &mut deliver) {
                        Ok(_) => eprintln!(
                            "lina-gpui: orquestração RETOMADA (freio solto) — fila represada drenada"
                        ),
                        Err(e) => eprintln!("lina-gpui: falha ao retomar a orquestração: {e}"),
                    }
                } else {
                    match self.router.pause(&mut store) {
                        Ok(_) => eprintln!(
                            "lina-gpui: orquestração PAUSADA (freio) — novas delegações enfileiram, nada se perde"
                        ),
                        Err(e) => eprintln!("lina-gpui: falha ao pausar a orquestração: {e}"),
                    }
                }
            }
            lock(&self.brake).paused = self.router.is_paused();
            lock(&self.model).touch();
        }

        let mut deliver = self.deliver_fn();
        let results = {
            let mut store = lock(&self.store);
            self.router.pump(&mut store, now_ms(), &mut deliver)
        };

        // Se algo roteou, atualiza o event_count da UI (e narra no stderr os bloqueios — nunca
        // falha em silêncio: o usuário/dev vê POR QUE uma mensagem não passou). `Queued` é ESPERADO
        // sob o freio (não é erro): a msg fica represada no `.inflight` até retomar.
        let mut routed = false;
        for (id, outcome) in &results {
            match outcome {
                RouteOutcome::Delivered { .. } | RouteOutcome::Duplicate => routed = true,
                RouteOutcome::Queued => {} // freio ativo: enfileirada de propósito, sem ruído
                RouteOutcome::Presence => {} // ping de presença (handshake/broadcast): absorvido, sem ruído
                other => eprintln!("lina-gpui: mensagem {id} não roteada: {other:?}"),
            }
        }
        if routed {
            let count = lock(&self.store).event_count().ok();
            let mut m = lock(&self.model);
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }
        self.refresh_cost_paused();
    }

    /// W3-7c: espelha `CostLedger::paused` (teto de custo atingido) no model p/ o banner. Só re-replaya
    /// quando há eventos NOVOS (compara `event_count`) — evita um replay do log a cada tick. Captura a
    /// transição feita pelo próprio Router (CostCeilingHit) E a retomada pelo gate humano (CostCeilingResumed).
    fn refresh_cost_paused(&mut self) {
        let ec = lock(&self.store).event_count().unwrap_or(self.last_cost_ec);
        if ec == self.last_cost_ec {
            return;
        }
        self.last_cost_ec = ec;
        let paused = CostLedger::replay(&lock(&self.store), now_ms())
            .map(|l| l.paused)
            .unwrap_or(false);
        let mut m = lock(&self.model);
        if m.cost_paused != paused {
            m.cost_paused = paused;
            m.touch();
        }
    }
}

// ════════════════════════ W3-6c · ÚLTIMA MILHA — custódia VIVA no app (ADR 0004) ════════════════════════
//
// Os testes headless já provam `run_custody` (broker.rs). Aqui o APP passa a CHAMÁ-LA: drena a FILA
// DE BROKER (`<LINA_HOME>/broker/`, irmã do outbox A2A — o `Router` nunca a varre), aplica o GATE
// HUMANO (tecla na janela, inforjável pelo PTY do agente) e só então deixa o broker buscar o segredo
// no cofre e executar. O agente NUNCA tem o token (vive só no [`SecretVault`]); sem confirmação
// humana, não há caminho de execução — a custódia é o piso inquebrável.

/// Um pedido aguardando o gate humano na FILA do [`CustodyDesk`]. Dois tipos passam pelo MESMO canal
/// humano (⌘⏎ na janela): custódia de segredo (`lina do`) e retomada do teto de custo (`lina resume`).
/// O `requester` é SEMPRE a ORIGEM AUTENTICADA (nome do dir-dono carimbado no drain por-nó), NUNCA o
/// campo `from` do conteúdo (hole 3 — o banner não pode exibir um requester forjável).
#[derive(Debug, Clone)]
pub enum PendingGate {
    /// `lina do <action>` — exige o segredo do cofre na execução (ADR 0004).
    Custody {
        id: String,
        action: String,
        secret_key: String,
        requester: String,
        display: String,
    },
    /// `lina resume` — o app aplica `CostCeilingResumed` SÓ após este gate (o agente não des-pausa).
    Resume {
        id: String,
        requester: String,
        display: String,
    },
}

impl PendingGate {
    /// `id` da `MailMessage` de origem (chave do gate; casa com `confirm_requested`).
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Custody { id, .. } | Self::Resume { id, .. } => id,
        }
    }
    /// Texto legível do banner.
    #[must_use]
    pub fn display(&self) -> &str {
        match self {
            Self::Custody { display, .. } | Self::Resume { display, .. } => display,
        }
    }
    /// Origem AUTENTICADA (dir-dono). Exposto para teste/observabilidade do gate.
    #[must_use]
    pub fn requester(&self) -> &str {
        match self {
            Self::Custody { requester, .. } | Self::Resume { requester, .. } => requester,
        }
    }
}

/// **Teto de pedidos pendentes no gate humano** (hole 2 + DoS, Round 5b). Um humano tria poucos por
/// vez; acima disto novos pedidos são RECUSADOS (sem `run_custody`, sem escrita durável) — limita
/// memória, amplificação de log e o *burial* da fila por flood de um agente.
pub const MAX_PENDING_GATES: usize = 32;

/// **Teto de pedidos drenados por tick** da fila de broker (DoS, Round 5b — espelha o `MAX_DRAIN_BATCH`
/// do core, que este drain por-nó não reusa). Limita syscalls/contenção de lock por tick sob flood.
const MAX_BROKER_DRAIN: usize = 64;

/// `true` se `id` é um id de mensagem SEGURO (`msg_` + `[A-Za-z0-9_-]`), espelhando o `valid_msg_id`
/// do core (privado lá). Revalidado no drain do app porque um agente que escreve DIRETO no subdir
/// controla o `id` do conteúdo (chave do gate/confirm) E o nome do arquivo (chave FIFO) — hole 2.
fn valid_broker_id(id: &str) -> bool {
    match id.strip_prefix("msg_") {
        Some(rest) => {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }
        None => false,
    }
}

/// **Mesa do gate humano compartilhada** entre a [`BrokerPump`] (enfileira o pendente + executa) e a
/// UI (que confirma/recusa com TECLA na janela — um agente no PTY não consegue sintetizá-la). A UI
/// nunca toca o cofre nem o broker: só sinaliza `confirm_requested`/`reject_requested`; quem executa
/// é a pump (escritora única).
///
/// **Hole 2 (swap-antes-de-confirmar + flood):** os pendentes vivem numa FILA FIFO CAPEADA
/// ([`MAX_PENDING_GATES`]) — um 2º pedido vai para o FIM, NUNCA sobrescreve a frente; acima do teto é
/// recusado. O humano vê/confirma (⌘⏎) OU RECUSA (⌘⇧⏎) a frente; nada se perde por sobrescrita e um
/// flood não enterra indefinidamente nem deixa o usuário sem saída (há dismiss).
#[derive(Debug, Default)]
pub struct CustodyDesk {
    /// Fila FIFO de pedidos aguardando o gate humano (frente = o que o humano vê/confirma).
    pub queue: VecDeque<PendingGate>,
    /// A UI sinalizou que o humano CONFIRMOU o pedido da FRENTE (id). A pump consome no próximo tick.
    pub confirm_requested: Option<String>,
    /// A UI sinalizou que o humano RECUSOU/dispensou o pedido da FRENTE (id) — sai da fila SEM executar.
    pub reject_requested: Option<String>,
    /// Último resultado (banner efêmero pós-execução): texto + quando nasceu.
    pub last_result: Option<(String, Instant)>,
}

impl CustodyDesk {
    /// O pedido na frente da fila (o que o humano vê/confirma), se houver.
    #[must_use]
    pub fn front(&self) -> Option<&PendingGate> {
        self.queue.front()
    }

    /// Texto a exibir na topbar AGORA: a FRENTE da fila (com "(+N na fila)" se houver mais) ou, sem
    /// pendentes, o último resultado por ~6s.
    #[must_use]
    pub fn banner(&self) -> Option<String> {
        if let Some(p) = self.queue.front() {
            let extra = self.queue.len().saturating_sub(1);
            let suffix = if extra > 0 {
                format!("   (+{extra} na fila)")
            } else {
                String::new()
            };
            return Some(format!("{}{suffix}", p.display()));
        }
        match &self.last_result {
            Some((txt, born)) if born.elapsed() < Duration::from_secs(6) => Some(txt.clone()),
            _ => None,
        }
    }
}

/// Handle compartilhado da mesa do gate humano.
pub type Desk = Arc<Mutex<CustodyDesk>>;

/// Extrai a chave do segredo do payload do pedido (`--env <x>`); default `"default"` para ações
/// sem ambiente (pay/send). Determinístico, ZERO interpretação semântica (invariante #1).
fn parse_secret_key(payload: &str) -> String {
    let toks: Vec<&str> = payload.split_whitespace().collect();
    for w in toks.windows(2) {
        if w[0] == "--env" {
            return w[1].to_string();
        }
    }
    "default".to_string()
}

/// **Round 5c — sanitiza um valor ATACANTE-CONTROLÁVEL** (ex.: `--env <x>`) ANTES de exibi-lo no banner
/// do gate. Achata controles/quebras de linha e os overrides de direção Unicode (RTL: LRE/RLE/PDF/LRO/
/// RLO/LRI/RLI/FSI/PDI) + LS/PS → um agente não injeta linha falsa, texto de "aprovado" ou inversão de
/// direção no que o humano LÊ; limita o tamanho. SÓ para exibição — o valor CRU segue sendo a chave de
/// lookup no cofre (sanitizar a chave mudaria a busca).
fn sanitize_for_banner(s: &str) -> String {
    const MAX: usize = 48;
    let mut out: String = s
        .chars()
        .map(|c| {
            let bad = c.is_control()
                || c == '\u{2028}'
                || c == '\u{2029}'
                || ('\u{202a}'..='\u{202e}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c);
            if bad {
                '·'
            } else {
                c
            }
        })
        .take(MAX)
        .collect();
    if s.chars().count() > MAX {
        out.push('…');
    }
    out
}

/// **Observador da FILA DE BROKER (`<LINA_HOME>/broker/`).** Numa thread própria, drena os pedidos
/// privilegiados (`broker.do` = custódia; `resume.request` = retomada do teto), enfileira-os no gate
/// humano e — só após confirmação na janela — executa: custódia chama [`run_custody`] (segredo do
/// cofre), resume aplica `CostCeilingResumed`. Genérico sobre o backend do cofre. Escritor único do
/// estado de execução do gate.
///
/// **Hole 3 (requester forjável):** drena SÓ os subdirs POR-NÓ ([`Self::drain_broker_authenticated`])
/// — `from` = nome do dir-dono — E cruza essa origem com o ROSTER VIVO (`sup`): um subdir cujo nome
/// não é um nó real (ex.: `@Admin` plantado) é REJEITADO. Arquivos FLAT no outbox também são rejeitados.
/// Assim o banner só pode exibir o nome de um participante real, nunca um nome de autoridade inventado.
pub struct BrokerPump<S: SecretStore> {
    mailbox: Mailbox,
    store: Arc<Mutex<EventStore>>,
    vault: SecretVault<S>,
    desk: Desk,
    model: Model,
    /// Roster vivo — para validar que a origem autenticada (dir-dono) é um NÓ REAL (hole 3).
    sup: Arc<Supervisor>,
}

impl<S: SecretStore + Send + 'static> BrokerPump<S> {
    /// Cria a pump enraizada na `mailbox` da fila de broker.
    #[must_use]
    pub fn new(
        mailbox: Mailbox,
        store: Arc<Mutex<EventStore>>,
        vault: SecretVault<S>,
        desk: Desk,
        model: Model,
        sup: Arc<Supervisor>,
    ) -> Self {
        Self {
            mailbox,
            store,
            vault,
            desk,
            model,
            sup,
        }
    }

    /// Sobe a thread que observa a fila de broker (gate humano + execução) a cada ~150 ms.
    pub fn spawn(mut self) -> JoinHandle<()> {
        thread::spawn(move || loop {
            self.tick();
            thread::sleep(Duration::from_millis(150));
        })
    }

    fn tick(&mut self) {
        // (1) Recusa humana? → tira a FRENTE da fila SEM executar (dismiss; hole 2 — saída do flood).
        let reject = lock(&self.desk).reject_requested.take();
        if let Some(id) = reject {
            self.reject_front(&id);
        }
        // (2) Confirmação humana? → executa a FRENTE da fila (se casar o id).
        let confirm = lock(&self.desk).confirm_requested.take();
        if let Some(id) = confirm {
            self.execute_confirmed(&id);
        }
        // (3) Novos pedidos — SÓ por-nó autenticado+roster, capeado por tick (flat/forja/flood barrados).
        for m in self.drain_broker_authenticated() {
            self.handle_request(&m);
        }
    }

    /// O humano RECUSOU a frente (⌘⇧⏎): tira-a da fila SEM executar (custódia não roda; resume não
    /// des-pausa). Dá ao usuário uma saída para limpar um flood/erro sem disparar a ação (hole 2).
    fn reject_front(&mut self, id: &str) {
        let dismissed = {
            let mut d = lock(&self.desk);
            match d.queue.front() {
                Some(p) if p.id() == id => d.queue.pop_front(),
                _ => None,
            }
        };
        let Some(pend) = dismissed else {
            eprintln!("lina-gpui: recusa para {id} nao casa a frente da fila — ignorada");
            return;
        };
        let banner = format!(
            "🚫 pedido de {} RECUSADO no gate humano (nao executado)",
            pend.requester()
        );
        eprintln!("lina-gpui: GATE — {banner}");
        lock(&self.desk).last_result = Some((banner, Instant::now()));
        lock(&self.model).touch();
    }

    /// **drain AUTENTICADO por-nó da fila de broker** (holes 2/3, Round 5c). Lê SÓ os `*.json` dos
    /// subdirs por-nó (`outbox/<node>/`) e carimba `from` = nome CANÔNICO do roster (não a string crua
    /// do dir — anti-spoof hole 3); a origem tem de RESOLVER a um nó real (roster-check). Ordena por
    /// FAIR-SHARE round-robin ENTRE nós (anti front-running hole 2 — o id/filename é do agente), capeado
    /// por tick ([`MAX_BROKER_DRAIN`]). Rejeita: arquivos FLAT, ids fora de `msg_<alnum>`, origens
    /// fora do roster, não-regulares (anti-symlink), oversize. Consome (read-delete): o gate é
    /// interativo; um crash antes da confirmação só faz o humano re-rodar (o registro do agente persiste).
    fn drain_broker_authenticated(&self) -> Vec<MailMessage> {
        let outbox = self.mailbox.outbox_dir();
        let rd = match std::fs::read_dir(&outbox) {
            Ok(rd) => rd,
            Err(_) => return Vec::new(), // outbox ainda não existe → nada a drenar
        };
        let mut collected: Vec<(String, std::path::PathBuf)> = Vec::new(); // (node, json_path)
        for entry in rd.flatten() {
            let path = entry.path();
            // FLAT (`*.json` no topo do outbox) NUNCA é legítimo na fila de broker (o CLI usa
            // `enqueue_as` estrito). Só uma forja o planta → rejeita + remove + WARN, sem processar.
            if path.is_file() {
                if path.extension().is_some_and(|x| x == "json") {
                    eprintln!(
                        "lina-gpui: REJEITADO arquivo flat na fila de broker (possivel forja de origem): {}",
                        path.display()
                    );
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            let Some(node) = entry.file_name().to_str().map(str::to_owned) else {
                continue; // nome não-UTF8 → não é nó legítimo
            };
            // Mesma validação do core (`valid_node_name`) via API pública: exclui `.inflight`/ocultos/
            // traversal. Defense-in-depth contra um subdir com nome perigoso.
            if self.mailbox.outbox_node_dir(&node).is_none() {
                continue;
            }
            if let Ok(files) = std::fs::read_dir(&path) {
                for f in files.flatten() {
                    let fp = f.path();
                    if fp.extension().is_some_and(|x| x == "json") {
                        collected.push((node.clone(), fp));
                    }
                }
            }
        }
        // **Hole 2 (front-running) — Round 5c:** NÃO ordenar por nome de arquivo. O nome (`<id>.json`)
        // é controlado pelo agente, e `valid_broker_id` aceita `msg_<alnum>` (sem exigir v7) — então um
        // `msg_000000` plantado venceria qualquer id v7 (`msg_019…`) e enterraria o pedido legítimo de
        // um colega. Em vez disso: FAIR-SHARE round-robin ENTRE nós (1 pedido de cada nó por rodada) —
        // nenhum nó monopoliza a frente; o pedido de um colega nunca fica atrás do flood INTEIRO de outro.
        // O teto MAX_BROKER_DRAIN é aplicado DURANTE o round-robin; o excedente fica no disco p/ o próximo.
        let total = collected.len();
        let mut by_node: BTreeMap<String, VecDeque<std::path::PathBuf>> = BTreeMap::new();
        for (node, fp) in collected {
            by_node.entry(node).or_default().push_back(fp);
        }
        // Ordem intra-nó: por nome de arquivo (um nó ordena os SEUS próprios pedidos — inofensivo).
        for files in by_node.values_mut() {
            let mut v: Vec<std::path::PathBuf> = files.drain(..).collect();
            v.sort();
            *files = v.into_iter().collect();
        }
        let mut collected: Vec<(String, std::path::PathBuf)> = Vec::new();
        let mut progressed = true;
        while progressed && collected.len() < MAX_BROKER_DRAIN {
            progressed = false;
            for (node, files) in by_node.iter_mut() {
                if collected.len() >= MAX_BROKER_DRAIN {
                    break;
                }
                if let Some(fp) = files.pop_front() {
                    collected.push((node.clone(), fp));
                    progressed = true;
                }
            }
        }
        if total > collected.len() {
            eprintln!(
                "lina-gpui: broker drenando {} de {total} pedidos neste tick (teto MAX_BROKER_DRAIN, fair-share por no); o restante fica para o proximo",
                collected.len()
            );
        }

        let mut out = Vec::with_capacity(collected.len());
        for (node, fp) in collected {
            match std::fs::symlink_metadata(&fp) {
                Ok(meta) if !meta.is_file() => {
                    eprintln!(
                        "lina-gpui: broker descartou entrada nao-regular (symlink?): {}",
                        fp.display()
                    );
                    let _ = std::fs::remove_file(&fp);
                    continue;
                }
                Ok(meta) if meta.len() > 256 * 1024 => {
                    eprintln!(
                        "lina-gpui: broker descartou mensagem oversize: {}",
                        fp.display()
                    );
                    let _ = std::fs::remove_file(&fp);
                    continue;
                }
                Ok(_) => {}
                Err(_) => continue, // erro transiente: deixa p/ o próximo tick
            }
            let Ok(data) = std::fs::read_to_string(&fp) else {
                continue;
            };
            let _ = std::fs::remove_file(&fp); // consumido (read-delete)
            match serde_json::from_str::<MailMessage>(&data) {
                Ok(mut msg) => {
                    // Hole 2: revalida o `id` (chave do gate/confirm) — um agente que escreve direto no
                    // subdir controla o conteúdo; um id fora de `msg_<alnum>` é descartado.
                    if !valid_broker_id(&msg.id) {
                        eprintln!(
                            "lina-gpui: broker descartou pedido com id invalido {:?}",
                            msg.id
                        );
                        continue;
                    }
                    // Hole 3 — ROSTER-CHECK + NOME CANÔNICO (Round 5c): a origem (dir-dono) tem de
                    // RESOLVER a um NÓ REAL do roster vivo; um `@Admin`/`@CEO` inventado não resolve →
                    // rejeitado. E o `from` exibido é o nome CANÔNICO do roster, NÃO a string crua do dir:
                    // fecha o spoof por casing/espaço/`@`/Unicode (ex.: "  terminal a  " normaliza-colide
                    // com "Terminal A" mas exibiria a string crua enganosa).
                    let Some(node_id) = self.sup.node_by_name(&node) else {
                        eprintln!(
                            "lina-gpui: REJEITADO pedido de broker de origem {node:?} que NAO resolve a um no do roster (possivel forja de autoridade)"
                        );
                        continue;
                    };
                    let Some(canonical) = self.sup.get(node_id).map(|n| n.name) else {
                        eprintln!(
                            "lina-gpui: REJEITADO pedido de broker — no {node:?} sumiu do roster"
                        );
                        continue;
                    };
                    // ORIGEM AUTENTICADA = nome CANÔNICO do roster (vence o `from` de conteúdo E a string
                    // crua do dir). (Residual: impersonar um nó REAL escrevendo no subdir DELE depende de
                    // isolamento de cwd a nível de SO — fronteira do app, fora do MVP; ver entrega.)
                    msg.from = canonical;
                    out.push(msg);
                }
                Err(e) => eprintln!("lina-gpui: broker descartou mensagem ilegivel: {e}"),
            }
        }
        out
    }

    /// 1ª vista de um pedido: ENFILEIRA no gate humano (FIFO, sem sobrescrever — hole 2). Custódia
    /// também roda `run_custody(confirmed=false)` (ActionGated{ask}+BrokerDenied{unconfirmed}; o cofre
    /// nem é tocado). O banner usa `m.from` = origem AUTENTICADA (hole 3).
    fn handle_request(&mut self, m: &MailMessage) {
        // DoS/hole 2: fila CHEIA → RECUSA novos pedidos SEM `run_custody` (sem escrita durável) nem
        // push — bound de memória + amplificação de log + burial. Visível (nunca silencioso).
        if lock(&self.desk).queue.len() >= MAX_PENDING_GATES {
            eprintln!(
                "lina-gpui: fila do gate humano CHEIA ({MAX_PENDING_GATES}) — pedido {} de {} RECUSADO (aprove/recuse os pendentes antes)",
                m.id, m.from
            );
            return;
        }
        match m.intent.as_str() {
            "broker.do" => self.enqueue_custody(m),
            "resume.request" => self.enqueue_resume(m),
            other => eprintln!(
                "lina-gpui: intent {other:?} desconhecido na fila de broker (msg {}) — ignorado",
                m.id
            ),
        }
    }

    /// Enfileira um pedido de custódia (`lina do`) após registrar o gate (run_custody sem confirmação).
    fn enqueue_custody(&mut self, m: &MailMessage) {
        let Some(action) = m.ref_id.as_deref().and_then(|r| r.strip_prefix("do:")) else {
            eprintln!(
                "lina-gpui: broker.do sem ref 'do:<action>' (msg {}) — ignorado",
                m.id
            );
            return;
        };
        let Some(custody) = lookup_action(action) else {
            eprintln!(
                "lina-gpui: broker.do com acao nao custodiada {action:?} (msg {}) — ignorado",
                m.id
            );
            return;
        };
        let secret_key = parse_secret_key(&m.payload);
        let req = BrokerRequest::new(custody, secret_key.clone(), m.from.clone());

        let outcome = {
            let mut store = lock(&self.store);
            run_custody(&req, false, &self.vault, &mut store, |_| Ok(()))
        };
        match outcome {
            Ok(BrokerOutcome::DeniedUnconfirmed) => {
                let pend = PendingGate::Custody {
                    id: m.id.clone(),
                    action: action.to_string(),
                    // Fidelidade do gate (Round 5b): mostra o ALVO (`--env <x>`) ao humano — ele decide
                    // vendo prod vs staging. O env é ATACANTE-CONTROLÁVEL → sanitizado p/ exibição (5c):
                    // não injeta controle/RTL/"aprovado" no banner. A chave CRUA (vault) fica em `secret_key`.
                    display: format!(
                        "🔒 CUSTODIA: '{}' [{}/{}] pedido por {} — ⌘⏎ aprova · ⌘⇧⏎ recusa (o agente NAO tem o token)",
                        custody.desc,
                        custody.scope,
                        sanitize_for_banner(&secret_key),
                        m.from
                    ),
                    secret_key,
                    requester: m.from.clone(), // AUTENTICADA
                };
                lock(&self.desk).queue.push_back(pend);
                lock(&self.model).touch();
                eprintln!(
                    "lina-gpui: CUSTODIA — '{action}' de {} na fila do gate humano (⌘⏎). msg {}",
                    m.from, m.id
                );
            }
            Ok(other) => {
                eprintln!("lina-gpui: broker.do {action} 1a passada inesperada: {other:?}")
            }
            Err(e) => eprintln!("lina-gpui: broker.do {action} falhou no gate: {e}"),
        }
    }

    /// Enfileira um pedido de retomada do teto (`lina resume`). O agente NÃO des-pausa: só o gate humano
    /// (⌘⏎) faz o app aplicar `CostCeilingResumed` (hole 1).
    fn enqueue_resume(&mut self, m: &MailMessage) {
        let pend = PendingGate::Resume {
            id: m.id.clone(),
            requester: m.from.clone(), // AUTENTICADA
            display: format!(
                "⏸ RETOMADA do teto pedida por {} — ⌘⏎ aprova · ⌘⇧⏎ recusa (o agente NAO des-pausa sozinho)",
                m.from
            ),
        };
        lock(&self.desk).queue.push_back(pend);
        lock(&self.model).touch();
        eprintln!(
            "lina-gpui: RESUME — retomada do teto pedida por {} na fila do gate humano (⌘⏎). msg {}",
            m.from, m.id
        );
    }

    /// O humano confirmou (`id`): tira a FRENTE da fila (se casar o id) e executa conforme o tipo —
    /// custódia (segredo do cofre + marcador) ou retomada (CostCeilingResumed se pausado).
    fn execute_confirmed(&mut self, id: &str) {
        // Só a FRENTE é confirmável (é o que o humano vê). Defensivo: confere o id.
        let pend = {
            let mut d = lock(&self.desk);
            match d.queue.front() {
                Some(p) if p.id() == id => d.queue.pop_front(),
                _ => None,
            }
        };
        let Some(pend) = pend else {
            eprintln!("lina-gpui: confirmacao para {id} nao casa a frente da fila — ignorada");
            return;
        };
        // Auditoria: registra a ORIGEM AUTENTICADA (dir-dono) do pedido confirmado — nunca um `from`
        // de conteúdo forjável (hole 3).
        eprintln!(
            "lina-gpui: GATE confirmado — pedido {} de origem autenticada {}",
            pend.id(),
            pend.requester()
        );
        let banner = match pend {
            PendingGate::Custody {
                action,
                secret_key,
                requester,
                ..
            } => self.execute_custody(&action, &secret_key, &requester),
            PendingGate::Resume { requester, .. } => self.execute_resume(&requester),
        };
        eprintln!("lina-gpui: GATE — {banner}");
        {
            let mut d = lock(&self.desk);
            d.last_result = Some((banner, Instant::now()));
        }
        lock(&self.model).touch();
    }

    /// Executa a custódia confirmada: `run_custody(confirmed=true)` → o BROKER obtém o segredo do cofre
    /// e age (o agente nunca o teve). Marcador no disco como PROVA observável (só o TAMANHO do segredo).
    fn execute_custody(&self, action: &str, secret_key: &str, requester: &str) -> String {
        let Some(custody) = lookup_action(action) else {
            return format!("⛔ pendente com acao invalida {action:?} — abortado");
        };
        let req = BrokerRequest::new(custody, secret_key.to_string(), requester.to_string());
        let exec_root = self.mailbox.root().to_path_buf();
        let action_name = action.to_string();
        let outcome = {
            let mut store = lock(&self.store);
            run_custody(&req, true, &self.vault, &mut store, |secret| {
                let marker = exec_root.join(format!("executed-{action_name}.txt"));
                std::fs::write(
                    &marker,
                    format!(
                        "broker executou '{action_name}' com um segredo de {} bytes obtido do cofre (o agente nunca o teve)\n",
                        secret.len()
                    ),
                )
                .map_err(|e| format!("escrever marcador de execucao: {e}"))
            })
        };
        match outcome {
            Ok(BrokerOutcome::Executed) => format!(
                "✅ '{action}' executado pelo broker — segredo veio do cofre; o agente nunca o viu"
            ),
            Ok(BrokerOutcome::DeniedNoSecret) => format!(
                "⛔ '{action}' bloqueado — segredo AUSENTE do cofre (sem token, sem acao: custodia)"
            ),
            Ok(other) => format!("'{action}' resultado inesperado: {other:?}"),
            Err(e) => format!("⛔ '{action}' falhou na execucao: {e}"),
        }
    }

    /// Executa a retomada confirmada (hole 1): reconstrói o ledger do store REAL do app e, SE pausado,
    /// apenda `CostCeilingResumed`. O agente nunca chega aqui — só o gate humano dispara isto.
    fn execute_resume(&self, requester: &str) -> String {
        let mut store = lock(&self.store);
        let ledger = match CostLedger::replay(&store, now_ms()) {
            Ok(l) => l,
            Err(e) => return format!("⛔ retomada falhou ao reconstruir o ledger: {e}"),
        };
        if !ledger.paused {
            return format!("ℹ retomada de {requester}: o teto nao esta pausado (nada a retomar)");
        }
        match store.append(&DomainEvent::CostCeilingResumed { day: ledger.day }) {
            Ok(_) => format!(
                "✅ teto de custo RETOMADO (gate humano) a pedido de {requester} — nova janela contabil"
            ),
            Err(e) => format!("⛔ retomada falhou ao apendar CostCeilingResumed: {e}"),
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
    role: &str,
    cmd: PtyCommand,
    cols: u16,
    rows: u16,
) -> Result<(NodeId, Grid), String> {
    pty.spawn(key, cmd, cols, rows).map_err(|e| e.to_string())?;
    let writer = pty.take_writer(key).map_err(|e| e.to_string())?;
    // W4-2 (M2): o nó nasce com o PAPEL no roster do Supervisor → flui para `agents.json` (lina list).
    let node = sup.register(name, Some(role.to_string()), writer);
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

/// Candidatos a raiz do repo (onde vivem `assets/` e `target/`), em ordem de preferência: o
/// derivado do `CARGO_MANIFEST_DIR` (resolvido pelo cargo em build — não é literal da máquina) e
/// os ancestrais do executável atual (cobre o binário realocado em runtime). Quem chama filtra por
/// existência do recurso, então tolera candidatos inexistentes.
fn repo_root_candidates() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    // CARGO_MANIFEST_DIR = `<repo>/app/lina-gpui` → `app` → `<repo>`.
    if let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
    {
        roots.push(repo.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe() {
        roots.extend(exe.ancestors().map(Path::to_path_buf));
    }
    roots
}

/// Nome do executável `lina` no SO atual (`lina.exe` no Windows).
fn lina_exe_name() -> &'static str {
    if cfg!(windows) {
        "lina.exe"
    } else {
        "lina"
    }
}

/// Diretório que contém o binário `lina`, para prependê-lo ao `PATH` do shell filho — assim
/// `lina ask`/`handshake`/`plan` rodam DENTRO do terminal sem setup manual. Tenta, em ordem:
/// (1) o diretório de `LINA_BIN`, se este for um caminho (override explícito); (2) ao lado do
/// executável atual; (3) `target/{debug,release}` do repo — onde o crate `lina-bootstrap` builda o
/// bin `lina` (workspace separado do app, logo NÃO fica ao lado do `lina-gpui`). Devolve o 1º que
/// existe; `None` se nada resolver (o shell ainda herda o `PATH` do pai → sem regressão).
fn lina_bin_dir() -> Option<PathBuf> {
    let exe = lina_exe_name();
    let has_lina = |dir: &Path| dir.join(exe).is_file();

    // 1) LINA_BIN como caminho → use seu diretório. Nome puro ("lina") cai para os próximos.
    if let Some(bin) = std::env::var_os("LINA_BIN") {
        let p = PathBuf::from(&bin);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                return Some(parent.to_path_buf());
            }
        }
    }

    // 2) Ao lado do executável atual.
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            if has_lina(dir) {
                return Some(dir.to_path_buf());
            }
        }
    }

    // 3) target/{debug,release} do repo.
    for repo in repo_root_candidates() {
        for profile in ["debug", "release"] {
            let cand = repo.join("target").join(profile);
            if has_lina(&cand) {
                return Some(cand);
            }
        }
    }

    None
}

/// Prepõe `dir` ao `PATH` `current` sem duplicar (idempotente), com o separador do SO (`;` no
/// Windows, `:` no resto) via [`std::env::split_paths`]/[`std::env::join_paths`]. Função PURA — é o
/// ponto coberto pelo teste unitário do builder do env.
///
/// # Errors
/// Propaga `JoinPathsError` se algum componente contiver o separador de `PATH`.
fn prepend_dir_to_path(
    dir: &Path,
    current: Option<&OsStr>,
) -> Result<OsString, std::env::JoinPathsError> {
    let mut dirs: Vec<PathBuf> = vec![dir.to_path_buf()];
    if let Some(cur) = current {
        dirs.extend(std::env::split_paths(cur).filter(|p| p.as_path() != dir));
    }
    std::env::join_paths(dirs)
}

/// Valor de `PATH` para o shell filho: o diretório do `lina` na frente do `PATH` herdado. `None`
/// se não há diretório do `lina` resolvível, ou se o `PATH` resultante não for UTF-8 (degradação
/// graciosa com erro VISÍVEL — o `PtyCommand` só carrega env UTF-8).
fn lina_path_value() -> Option<String> {
    let dir = lina_bin_dir()?;
    let current = std::env::var_os("PATH");
    match prepend_dir_to_path(&dir, current.as_deref()) {
        Ok(joined) => joined.into_string().map_or_else(
            |_| {
                eprintln!("lina-gpui: PATH com bytes não-UTF8; `lina` não foi prependido ao shell");
                None
            },
            Some,
        ),
        Err(e) => {
            eprintln!("lina-gpui: não montei o PATH do shell (lina pode faltar no terminal): {e}");
            None
        }
    }
}

/// Caminho relativo da skill `lina-agent-bus` dentro de `assets/`.
const AGENT_BUS_SKILL_REL: &str = "lina-skills/lina-agent-bus";

/// Raiz da skill `lina-agent-bus` (o dir que contém `SKILL.md`), resolvida sem caminho absoluto
/// hardcoded: (1) `$LINA_ASSETS/lina-skills/lina-agent-bus` (override do dir `assets/`); (2)
/// `<repo>/assets/...` por [`repo_root_candidates`]. `None` se não achar (instalação é best-effort).
fn agent_bus_skill_src() -> Option<PathBuf> {
    let has_skill = |dir: &Path| dir.join("SKILL.md").is_file();

    if let Some(assets) = std::env::var_os("LINA_ASSETS") {
        let cand = PathBuf::from(assets).join(AGENT_BUS_SKILL_REL);
        if has_skill(&cand) {
            return Some(cand);
        }
    }
    for repo in repo_root_candidates() {
        let cand = repo.join("assets").join(AGENT_BUS_SKILL_REL);
        if has_skill(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Copia recursivamente o conteúdo de `src` para `dest`, sobrescrevendo (idempotente). Função pura
/// de I/O — coberta pelo teste do skill installer.
///
/// # Errors
/// Propaga o 1º erro de I/O (ler diretório, criar diretório, copiar arquivo).
fn copy_skill_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_skill_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Instala a skill `lina-agent-bus` em `<cwd>/.claude/skills/lina-agent-bus/` (idempotente). É o
/// que faz o terminal carregar o A2A automático no turno 0 — sem o setup manual da demo. Best-effort
/// com erro VISÍVEL (`eprintln`): nunca trava o app (mesma postura do bootstrap/mailbox).
fn install_agent_bus_skill(cwd: &Path) {
    let Some(src) = agent_bus_skill_src() else {
        eprintln!(
            "lina-gpui: skill `lina-agent-bus` não encontrada (defina LINA_ASSETS p/ o dir assets/); \
             {} sobe sem A2A automático",
            cwd.display()
        );
        return;
    };
    let dest = cwd.join(".claude").join("skills").join("lina-agent-bus");
    if let Err(e) = copy_skill_tree(&src, &dest) {
        eprintln!(
            "lina-gpui: copiar a skill `lina-agent-bus` para {} falhou: {e}",
            dest.display()
        );
    }
}

/// Fábrica do comando de um nó: um **SHELL INTERATIVO REAL** — o do usuário (`$SHELL`, com
/// fallback `/bin/sh`), idêntico para TODOS os nós (aceita teclado, roda claude/vim/…). Mostra
/// um banner neutro e dá `exec` no shell (sem mock/`cat`). É gpui-free (vive aqui, não no main).
/// Prepõe o diretório do `lina` ao `PATH` do filho para o A2A funcionar sem setup manual.
#[must_use]
pub fn shell_cmd(name: &str) -> PtyCommand {
    let cmd = PtyCommand::new("sh")
        .arg("-c")
        .arg(format!(
            "printf '{name} — shell interativo (digite comandos; rode claude/vim/…).\\r\\n'; \
             exec \"${{SHELL:-/bin/sh}}\" -i"
        ))
        .env("TERM", "xterm-256color");
    // Garante o `lina` no PATH do shell filho (A2A sem setup manual): prepõe o dir do binário
    // `lina` ao PATH herdado. Best-effort — sem resolução, o shell ainda herda o PATH do pai.
    match lina_path_value() {
        Some(path) => cmd.env("PATH", path),
        None => cmd,
    }
}

/// **W3-6c fio 3 — env limpo do PTY do agente.** Nomes de variáveis de ambiente portadoras de
/// segredo que NÃO podem vazar para o env de um terminal do agente: o token de ação externa vive SÓ
/// no [`SecretVault`] do Lina (invariante #2 + ADR 0004 "o segredo nunca entra no env do PTY").
/// `LINA_DEPLOY_TOKEN` é a semente do cofre na demo — removida do env logo após semear.
pub const SECRET_ENV_DENYLIST: &[&str] = &[
    "LINA_DEPLOY_TOKEN",
    "DEPLOY_TOKEN",
    "DEPLOY_KEY",
    "AWS_SECRET_ACCESS_KEY",
    "STRIPE_SECRET_KEY",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "NPM_TOKEN",
];

/// Remove do **próprio env do processo do app** (que TODO PTY filho herda no spawn) as variáveis
/// portadoras de segredo da denylist. Como os filhos herdam o env do app no momento do spawn, scrubar
/// AQUI — antes de qualquer `wire_terminal` — garante que nenhum terminal do agente nasça com o token.
/// Retorna os nomes efetivamente removidos (observabilidade/teste).
pub fn scrub_pty_secret_env() -> Vec<&'static str> {
    let mut removed = Vec::new();
    for &key in SECRET_ENV_DENYLIST {
        if std::env::var_os(key).is_some() {
            std::env::remove_var(key);
            removed.push(key);
        }
    }
    removed
}

/// Fábrica injetável de comando por nome: o app passa [`shell_cmd`] (shell real); os testes
/// passam um comando leve (`cat`) p/ serem determinísticos e headless.
pub type CmdFactory = Arc<dyn Fn(&str) -> PtyCommand + Send + Sync>;

/// Registry de papéis (W3-1), construído UMA vez (regexes compiladas) e cacheado. `None` se o YAML
/// embutido falhar (não deve ocorrer) → o caller cai num papel genérico, nunca panica.
///
/// **W4-2:** aplica um OVERLAY app-side adicionando o papel `reviewer` (o `default-roles.yaml` da crate
/// role-discovery não o tem) — assim "Revisor" deriva `reviewer` (critério de aceite) SEM tocar aquela
/// crate. Falha de overlay é best-effort (segue com o default).
fn role_registry() -> Option<&'static RoleRegistry> {
    /// Papéis que o registry default (lina-role-discovery) não cobre, mas o canvas T4 precisa.
    const APP_ROLE_OVERLAY: &str = "\
roles:
  - role: reviewer
    patterns: [revisor, reviewer, review, revisao]
    skills: [pr-review-toolkit:review-pr]
";
    static REG: OnceLock<Option<RoleRegistry>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut reg = RoleRegistry::with_defaults()
            .map_err(|e| {
                eprintln!("lina-gpui: role-registry default inválido ({e}); papel genérico")
            })
            .ok()?;
        if let Err(e) = reg.overlay_yaml_str(APP_ROLE_OVERLAY, "<app-role-overlay>") {
            eprintln!(
                "lina-gpui: overlay de papel 'reviewer' falhou ({e}); seguindo com o default"
            );
        }
        Some(reg)
    })
    .as_ref()
}

/// **W4-2 (M2) — deriva o PAPEL canônico pelo nome do agente** ("Revisor" → `reviewer`, "@Dev" →
/// backend, …) via role-discovery (W3-1). Determinístico e infalível: nome desconhecido cai no
/// fallback do registry; registry indisponível → `"generalist"`.
fn infer_agent_role(name: &str) -> String {
    match role_registry() {
        Some(reg) => reg.infer_role(name).role,
        None => "generalist".to_string(),
    }
}

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

// ─────────────────────────── W2-4: seleção + mouse reporting ───────────────────────────

/// Métricas da célula monoespaçada (Menlo 13px). Definem cols×rows (`fit_dims`) E a conversão
/// tela→célula da seleção/mouse — uma só fonte da verdade.
pub const CELL_W: f32 = 7.84;
pub const CELL_H: f32 = 17.0;

/// Origem X do grid DENTRO do card, em px NÃO escalados: borda(2) + padding do corpo(4).
const GRID_OX: f32 = 6.0;
/// Parte FIXA da origem Y: borda(2) + padding vertical do título(16) + padding do corpo(4). A
/// linha de texto do título escala com o zoom → soma-se `CELL_H * zoom` em [`screen_to_cell`].
const GRID_OY_FIXED: f32 = 22.0;

/// **Tela → célula do viewport** (0-based), ciente de pan/zoom: inverte a transformação da
/// câmera + a origem do grid no card. Satura na borda do grid. Casa o clique/arraste do mouse
/// com a célula certa para `set_selection`/`encode_mouse`.
#[must_use]
pub fn screen_to_cell(
    cam: &Camera,
    card_world: (f32, f32),
    screen_pt: (f32, f32),
    dims: (usize, usize),
) -> (usize, usize) {
    let z = cam.zoom;
    let (sx, sy) = cam.world_to_screen(card_world);
    let gx = sx + GRID_OX;
    let gy = sy + GRID_OY_FIXED + CELL_H * z;
    let col = (((screen_pt.0 - gx) / (CELL_W * z)).floor()).clamp(0.0, (dims.0.max(1) - 1) as f32);
    let row = (((screen_pt.1 - gy) / (CELL_H * z)).floor()).clamp(0.0, (dims.1.max(1) - 1) as f32);
    (col as usize, row as usize)
}

/// Normaliza uma seleção `anchor → head` para ordem de leitura: `(sc, sr, ec, er)` com
/// `(sr,sc) <= (er,ec)`. A direção do arraste não importa (igual ao `set_selection` do backend).
#[must_use]
pub fn normalize_sel(anchor: (usize, usize), head: (usize, usize)) -> (usize, usize, usize, usize) {
    let ((ac, ar), (hc, hr)) = (anchor, head);
    if (ar, ac) <= (hr, hc) {
        (ac, ar, hc, hr)
    } else {
        (hc, hr, ac, ar)
    }
}

/// A célula `(col,row)` está na seleção **stream** (não-retangular) `(sc,sr)..(ec,er)`? Espelha
/// o que o `selection_text()` extrai: linha inicial de `sc` ao fim, linhas do meio inteiras,
/// linha final do início a `ec`. Usado p/ DESTACAR as células no render.
#[must_use]
pub fn cell_in_selection(
    col: usize,
    row: usize,
    sc: usize,
    sr: usize,
    ec: usize,
    er: usize,
) -> bool {
    if row < sr || row > er {
        false
    } else if sr == er {
        col >= sc && col <= ec
    } else if row == sr {
        col >= sc
    } else if row == er {
        col <= ec
    } else {
        true
    }
}

/// Ação de ponteiro que a UI traduz em **mouse reporting** (subconjunto usado pelo app).
#[derive(Clone, Copy, Debug)]
pub enum PtrAction {
    Press,
    Release,
    Motion,
    WheelUp,
    WheelDown,
}

/// Codifica uma ação de ponteiro na célula `cell` para o PTY **se** o TUI pediu mouse reporting;
/// senão `None` (a UI então seleciona/rola). Reusa `VtBackend::encode_mouse` (SGR 1006) — não
/// reimplementa o protocolo. `mods` = `(shift, alt, control)`.
#[must_use]
pub fn encode_pointer(
    grid: &dyn VtBackend,
    action: PtrAction,
    cell: (usize, usize),
    mods: (bool, bool, bool),
) -> Option<Vec<u8>> {
    let (button, kind) = match action {
        PtrAction::Press => (Some(VtBtn::Left), MouseEventKind::Press),
        PtrAction::Release => (Some(VtBtn::Left), MouseEventKind::Release),
        PtrAction::Motion => (Some(VtBtn::Left), MouseEventKind::Motion),
        PtrAction::WheelUp => (Some(VtBtn::WheelUp), MouseEventKind::Press),
        PtrAction::WheelDown => (Some(VtBtn::WheelDown), MouseEventKind::Press),
    };
    grid.encode_mouse(MouseEvent {
        button,
        kind,
        col: cell.0,
        row: cell.1,
        mods: MouseModifiers {
            shift: mods.0,
            alt: mods.1,
            control: mods.2,
        },
    })
}

/// **W3-2 · Escritor de bootstrap por terminal (gpui-free).** Mantém o renderer + a config do
/// workspace (vault, autonomia, caminho do bin `lina`) e escreve, no **cwd de cada terminal**
/// (`<ws_root>/<key>/`), os arquivos dos DOIS canais do design §1: `CLAUDE.md` (doutrina/8 blocos,
/// canal à prova de falhas), `.lina/bootstrap.json` (estado vivo p/ o `lina whoami`) e
/// `.claude/settings.json` (hook `SessionStart`, só Claude Code). **Reescreve TODOS a cada mudança
/// de roster** — o arquivo é a âncora.
pub struct BootstrapWriter {
    bootstrapper: Bootstrapper,
    ws_root: PathBuf,
    vault_path: String,
    autonomy: Autonomy,
    lina_bin: String,
}

impl BootstrapWriter {
    /// # Errors
    /// Falha se o registry de papéis default não construir (não deve ocorrer em produção).
    pub fn new(
        ws_root: PathBuf,
        vault_path: String,
        autonomy: Autonomy,
        lina_bin: String,
    ) -> Result<Self, String> {
        Ok(Self {
            bootstrapper: Bootstrapper::new().map_err(|e| e.to_string())?,
            ws_root,
            vault_path,
            autonomy,
            lina_bin,
        })
    }

    /// Diretório (cwd) do terminal `key`.
    #[must_use]
    pub fn dir_for(&self, key: &str) -> PathBuf {
        self.ws_root.join(key)
    }

    /// Escreve os arquivos de bootstrap de **UM** terminal (`key`/`name`) com o `roster` dado.
    /// Usado tanto no `rewrite_all` quanto no write-before-spawn do `add_node` (o nó novo já sobe
    /// com o `CLAUDE.md` no `cwd`). Erro de I/O é logado (best-effort): o app não trava por isto.
    pub fn write_one(&self, key: &str, name: &str, roster: &[String]) {
        let dir = self.dir_for(key);
        let input = BootstrapInput::new(
            name.to_string(),
            roster.to_vec(),
            self.vault_path.clone(),
            self.autonomy,
        );
        if let Err(e) = self
            .bootstrapper
            .write_terminal_files(&dir, &input, &self.lina_bin)
        {
            eprintln!("lina-gpui: bootstrap de {name} falhou: {e}");
        }
        // Instala a skill `lina-agent-bus` no cwd do terminal → A2A automático no turno 0
        // (idempotente, best-effort com erro visível). Cobre A/B no boot (via `rewrite_all`) E os
        // nós adicionados em runtime (write-before-spawn do `add_node`). Não duplica lógica.
        install_agent_bus_skill(&dir);
    }

    /// Reescreve os arquivos de TODOS os terminais `(key, name)` — o `roster` é o conjunto dos
    /// nomes. Chamar a cada mudança de roster (add/remove/renomear). Erros de I/O são logados
    /// (best-effort): o app não trava por causa do bootstrap.
    pub fn rewrite_all(&self, terminals: &[(String, String)]) {
        let roster: Vec<String> = terminals.iter().map(|(_, name)| name.clone()).collect();
        for (key, name) in terminals {
            self.write_one(key, name, &roster);
        }
    }
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
    /// W3-2: se presente, gera/reescreve o `CLAUDE.md` por terminal a cada mudança de roster.
    bootstrap: Option<BootstrapWriter>,
    /// W4-2 (M3/M4): o `.lina/` do workspace — onde os criadores persistem `notes/` e `folders/`.
    lina_dir: PathBuf,
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
        bootstrap: Option<BootstrapWriter>,
        lina_dir: PathBuf,
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
            bootstrap,
            lina_dir,
        }
    }

    /// **W4-2 · M3/M4 — cria uma NOTA ou PASTA** pelo nome e a torna VIVA no canvas (não só na
    /// projeção). Reusa [`crate::creators::CreatorForm::commit`] (apenda NodeAdded{Note|Folder} +
    /// Note/FolderCreated + NodeRenamed e persiste em `.lina/notes`|`.lina/folders`) e então SEMEIA o
    /// nó no model com o kind certo, posicionado sem sobrepor. Devolve o `NodeId` (p/ focar). O corpo
    /// da nota nasce vazio (editável depois no arquivo / P4).
    pub fn create_artifact(
        &self,
        kind: crate::creators::CreatorKind,
        title: &str,
    ) -> Result<NodeId, String> {
        let (x, y) = next_free_slot(&lock(&self.model));
        let form = crate::creators::CreatorForm {
            title: title.to_string(),
            body: String::new(),
        };
        let node = {
            let mut store = lock(&self.store);
            form.commit(
                kind,
                &mut store,
                &self.lina_dir,
                (f64::from(x), f64::from(y)),
            )
            .map_err(|e| e.to_string())?
        };
        let count = lock(&self.store).event_count().ok();
        let node_kind = match kind {
            crate::creators::CreatorKind::Note => NodeKind::Note,
            crate::creators::CreatorKind::Folder => NodeKind::Folder,
        };
        {
            let mut m = lock(&self.model);
            m.nodes
                .insert(node, NodeView::new(title.trim(), node_kind, x, y));
            if !m.order.contains(&node) {
                m.order.push(node);
            }
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }
        eprintln!(
            "lina-gpui: M3/M4 — {kind:?} '{}' criado (node {node})",
            title.trim()
        );
        Ok(node)
    }

    /// W3-2: snapshot `(key, name)` de todos os terminais vivos, na ordem do canvas.
    fn terminals_snapshot(&self) -> Vec<(String, String)> {
        let keys = lock(&self.keys);
        let m = lock(&self.model);
        m.order
            .iter()
            .filter_map(|n| {
                keys.get(n)
                    .and_then(|k| m.nodes.get(n).map(|v| (k.clone(), v.name.clone())))
            })
            .collect()
    }

    /// W3-2: reescreve o `CLAUDE.md`/estado de TODOS os terminais (o arquivo é a âncora a cada
    /// mudança de roster). No-op se o bootstrap não está configurado.
    pub fn rewrite_bootstrap(&self) {
        if let Some(bw) = &self.bootstrap {
            bw.rewrite_all(&self.terminals_snapshot());
        }
    }

    /// W3-2: cwd do terminal `key` (cria o diretório). `None` se o bootstrap não está configurado.
    fn ensure_cwd(&self, key: &str) -> Option<PathBuf> {
        let bw = self.bootstrap.as_ref()?;
        let dir = bw.dir_for(key);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("lina-gpui: criar cwd {} falhou: {e}", dir.display());
        }
        Some(dir)
    }

    /// W3-2: renomeia um nó — persiste `NodeRenamed`, atualiza o model e **reescreve** os
    /// `CLAUDE.md` (o roster mudou → todos os colegas veem o nome novo). Sem reiniciar o app.
    pub fn rename_node(&self, node: NodeId, new_name: &str) -> Result<(), String> {
        // Defesa-em-profundidade na fronteira: o renderer já sanitiza, mas rejeitar aqui um nome
        // vazio/controle/`{{`/`}}` dá erro cedo (em vez de um nome mutilado) e nunca persiste lixo.
        let trimmed = new_name.trim();
        if trimmed.is_empty()
            || trimmed.chars().any(char::is_control)
            || trimmed.contains("{{")
            || trimmed.contains("}}")
        {
            return Err(format!("nome de terminal inválido: {new_name:?}"));
        }
        let count = {
            let mut s = lock(&self.store);
            s.append(&DomainEvent::NodeRenamed {
                node,
                name: new_name.to_string(),
            })
            .map_err(|e| e.to_string())?;
            s.event_count().ok()
        };
        {
            let mut m = lock(&self.model);
            if let Some(v) = m.nodes.get_mut(&node) {
                v.name = new_name.to_string();
            }
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }
        self.rewrite_bootstrap();
        Ok(())
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
        // W3-2: o terminal spawna no SEU cwd (<ws_root>/<key>), onde fica o CLAUDE.md dele.
        let mut cmd = (*self.cmd_factory)(&name);
        if let Some(dir) = self.ensure_cwd(&key) {
            cmd = cmd.cwd(dir);
            // write-before-spawn: a doutrina do nó NOVO é gravada ANTES do PTY subir (o shell já
            // encontra o CLAUDE.md no cwd). Só o nó novo aqui — os existentes são atualizados no
            // `rewrite_bootstrap()` ao final; se o spawn falhar, eles não viram um colega fantasma.
            if let Some(bw) = &self.bootstrap {
                let mut roster: Vec<String> = self
                    .terminals_snapshot()
                    .into_iter()
                    .map(|(_, n)| n)
                    .collect();
                roster.push(name.clone());
                bw.write_one(&key, &name, &roster);
            }
        }

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
                "terminal",
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
        // W3-2: roster mudou → reescreve os CLAUDE.md (o novo nó + os existentes ganham o colega).
        self.rewrite_bootstrap();
        Ok(node)
    }

    /// **W4-2 · M2 "Novo Agente" — cria um agente PELO NOME.** Deriva o PAPEL do nome (role-discovery
    /// W3-1: "Revisor" → `reviewer`), spawna um terminal com CLI REAL **com `VIBE_ROLE` no env**, e
    /// **grava o papel** no roster (Supervisor) + no log (`NodeRoleAssigned`) → visível em
    /// `lina list --json` e no Inspetor (P4). Mesma disciplina do [`Self::add_node`]: persiste ANTES de
    /// projetar; se a persistência falhar, desfaz o PTY (`retire_pty`). Devolve o `NodeId` (p/ focar).
    pub fn create_agent(&self, name: &str) -> Result<NodeId, String> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().any(char::is_control)
            || name.contains("{{")
            || name.contains("}}")
        {
            return Err(format!("nome de agente inválido: {name:?}"));
        }
        let role = infer_agent_role(name);

        let seq = {
            let mut s = lock(&self.seq);
            let v = *s;
            *s = s.wrapping_add(1);
            v
        };
        let key = format!("t{seq}");
        // CLI real + cwd próprio (CLAUDE.md) + **VIBE_ROLE no env do PTY** (o agente sabe seu papel).
        let mut cmd = (*self.cmd_factory)(name).env("VIBE_ROLE", &role);
        if let Some(dir) = self.ensure_cwd(&key) {
            cmd = cmd.cwd(dir);
            if let Some(bw) = &self.bootstrap {
                let mut roster: Vec<String> = self
                    .terminals_snapshot()
                    .into_iter()
                    .map(|(_, n)| n)
                    .collect();
                roster.push(name.to_string());
                bw.write_one(&key, name, &roster);
            }
        }

        let (x, y) = next_free_slot(&lock(&self.model));
        let (node, grid) = {
            let mut p = lock(&self.pty);
            wire_terminal(
                &mut p,
                &self.sup,
                &self.delta_tx,
                &key,
                name,
                &role, // ← o papel derivado entra no roster do Supervisor
                cmd,
                self.cols,
                self.rows,
            )?
        };

        // PERSISTE (log = fonte da verdade) ANTES de projetar: NodeAdded + TerminalSpawned + o PAPEL.
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
                cli: name.to_string(),
            }) {
                drop(s);
                self.retire_pty(node, key);
                return Err(e.to_string());
            }
            // O PAPEL gravado no log (projeção `ProjectedNode.role`; sobrevive a replay/crash).
            if let Err(e) = s.append(&DomainEvent::NodeRoleAssigned {
                node,
                role: role.clone(),
            }) {
                drop(s);
                self.retire_pty(node, key);
                return Err(e.to_string());
            }
            s.event_count().ok()
        };

        {
            let mut m = lock(&self.model);
            m.nodes
                .insert(node, NodeView::new(name, NodeKind::Terminal, x, y));
            if !m.order.contains(&node) {
                m.order.push(node);
            }
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }
        let _ = self.sup.set_status(node, CoreStatus::Running);
        lock(&self.grids).insert(node, grid);
        lock(&self.keys).insert(node, key);
        self.rewrite_bootstrap();
        eprintln!(
            "lina-gpui: M2 — agente '{name}' criado (papel {role}, VIBE_ROLE no env). node {node}"
        );
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
        // W4-2 (M3/M4): nós-ARTEFATO (Note/Folder) NÃO têm PTY/key — `None` é legítimo (só não há PTY
        // a encerrar). Terminais têm key. Antes, `None` recusava a remoção (errava p/ Note/Folder).
        let key = lock(&self.keys).get(&node).cloned();

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

        // Encerra o PTY em background (não bloqueia o thread de UI por até 2s) — SÓ se houver (um
        // nó-artefato Note/Folder não tem PTY).
        if let Some(key) = key {
            self.retire_pty(node, key);
        }
        // W3-2: roster mudou → reescreve os CLAUDE.md (os colegas restantes não listam o removido).
        self.rewrite_bootstrap();
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

/// Sobe a thread-bomba: drena o Bus (lifecycle/status + **`Message` → pulso**) e os `GridDelta`.
///
/// **W3-7c · TETO DE CUSTO REAL:** cada `GridDelta` carrega `node` + `bytes` do output do PTY → alimenta
/// um [`crate::cost::CostMeter`]; ao fim de um turno (idle ≥ `idle_ms`) apenda `TokenUsageReported`
/// (estimado por bytes) no `store` AUTORITATIVO (ws2) — é o que o `CostLedger` soma p/ o teto. Sem isto,
/// o teto era INERTE (ninguém emitia uso).
pub fn spawn_pump(
    mut bridge: GpuiBridgeHost,
    delta_rx: Receiver<GridDelta>,
    mut bus_rx: broadcast::Receiver<BusEvent>,
    store: Arc<Mutex<EventStore>>,
    idle_ms: u64,
) -> std::io::Result<Pump> {
    let stop = Arc::new(AtomicBool::new(false));
    let join = {
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("lina-bridge-pump".into())
            .spawn(move || {
                let mut meter = crate::cost::CostMeter::new();
                // W4-6 gap2: nós atualmente em turno (produzindo). Dirige Busy↔Idle na ÁRVORE a11y SÓ na
                // transição (não a cada delta → sem busy-loop/churn de render). Idle = "resposta pronta".
                let mut active: std::collections::HashSet<NodeId> =
                    std::collections::HashSet::new();
                loop {
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
                    let now = now_ms();
                    // Mede o output de cada nó (node+bytes do GridDelta) — fonte do teto de custo.
                    while let Ok(delta) = delta_rx.try_recv() {
                        meter.record_output(delta.node, delta.bytes, now);
                        // W4-6 gap2: 1ª saída do turno → Busy (transição) → o `aria_label` do nó e o
                        // status refletem "rodando"; habilita o próximo →Idle a anunciar.
                        if active.insert(delta.node) {
                            bridge.on_event(HostEvent::NodeStatusChanged {
                                node: delta.node,
                                status: NodeStatus::Busy,
                            });
                        }
                        worked = true;
                    }
                    // Fim-de-turno por idle → apenda TokenUsageReported (estimado) no store autoritativo.
                    for (node, tokens) in meter.poll_finished_turns(now, idle_ms) {
                        if let Err(e) = lock(&store).append(&DomainEvent::TokenUsageReported {
                            node: node.to_string(),
                            tokens,
                        }) {
                            eprintln!("lina-gpui: falha ao apendar TokenUsageReported: {e}");
                        }
                        // W4-6 gap2: fim-de-turno → Idle → a `LiveRegion` computa "resposta pronta"
                        // (1×/turno) e o badge vira "💤 dormindo". Transição → sem churn.
                        active.remove(&node);
                        bridge.on_event(HostEvent::NodeStatusChanged {
                            node,
                            status: NodeStatus::Idle,
                        });
                    }
                    if !worked {
                        thread::sleep(Duration::from_millis(2));
                    }
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

    // ─────────────── W3-6c última milha: custódia (fios 1 e 3) — unidades gpui-free ───────────────

    /// Fio 1: a chave do segredo é extraída de `--env <x>` (deploy); default `default` (pay/send).
    #[test]
    fn parse_secret_key_reads_env_flag_or_defaults() {
        assert_eq!(parse_secret_key("--env prod"), "prod");
        assert_eq!(parse_secret_key("algo --env staging mais"), "staging");
        assert_eq!(parse_secret_key(""), "default");
        assert_eq!(parse_secret_key("--env"), "default"); // sem valor após a flag
    }

    /// Round 5c: o env exibido no banner é sanitizado — controles/quebras/RTL viram `·`; o valor cru
    /// (usado p/ lookup no cofre) NÃO é tocado. Sem injeção de linha falsa/"aprovado" no que o humano lê.
    #[test]
    fn sanitize_for_banner_neutralizes_injection() {
        assert_eq!(sanitize_for_banner("prod"), "prod", "valor benigno intacto");
        // Newline + texto de "aprovado" + override RTL → achatados.
        let evil = "prod\n✅ APROVADO\u{202e}gnirts";
        let s = sanitize_for_banner(evil);
        assert!(!s.contains('\n'), "sem quebra de linha");
        assert!(!s.contains('\u{202e}'), "sem override de direcao (RTL)");
        // Tamanho limitado (sem banner gigante).
        let long = "x".repeat(200);
        assert!(sanitize_for_banner(&long).chars().count() <= 49);
    }

    /// Helper de teste: um pendente de custódia mínimo com `id`/`display`.
    fn custody_pending(id: &str, display: &str) -> PendingGate {
        PendingGate::Custody {
            id: id.into(),
            action: "deploy".into(),
            secret_key: "prod".into(),
            requester: "@Dev".into(),
            display: display.into(),
        }
    }

    /// Banner: a FRENTE da fila vence o último resultado; sem fila, mostra o resultado por uma janela.
    #[test]
    fn custody_desk_banner_prioritizes_front_then_shows_result() {
        let mut desk = CustodyDesk::default();
        assert!(desk.banner().is_none(), "vazio → sem banner");

        desk.last_result = Some(("✅ deploy executado".into(), Instant::now()));
        assert_eq!(desk.banner().as_deref(), Some("✅ deploy executado"));

        desk.queue.push_back(custody_pending("msg_1", "🔒 aguarda"));
        assert_eq!(
            desk.banner().as_deref(),
            Some("🔒 aguarda"),
            "a frente da fila vence o último resultado"
        );
    }

    /// **Hole 2 (swap-antes-de-confirmar):** um 2º pedido vai para o FIM da fila — NUNCA apaga a frente.
    /// O banner segue mostrando o 1º (com "+1 na fila"); ao confirmar a frente, o 2º assume.
    #[test]
    fn custody_desk_queue_never_overwrites_front() {
        let mut desk = CustodyDesk::default();
        desk.queue
            .push_back(custody_pending("msg_1", "🔒 primeiro"));
        desk.queue.push_back(custody_pending("msg_2", "🔒 segundo"));

        assert_eq!(
            desk.queue.len(),
            2,
            "ambos os pedidos coexistem (sem sobrescrita)"
        );
        assert_eq!(
            desk.front().map(|p| p.id()),
            Some("msg_1"),
            "a frente é o 1º"
        );
        let banner = desk.banner().expect("banner");
        assert!(
            banner.contains("primeiro"),
            "o banner mostra o 1º, não o 2º"
        );
        assert!(
            banner.contains("+1 na fila"),
            "sinaliza que há mais um pendente"
        );

        // Confirma a frente (pop) → o 2º vira a frente; nada se perdeu.
        desk.queue.pop_front();
        assert_eq!(
            desk.front().map(|p| p.id()),
            Some("msg_2"),
            "o 2º assume após o 1º sair"
        );
        assert!(desk.banner().unwrap().contains("segundo"));
    }

    /// Fio 3: `scrub_pty_secret_env` remove do env do processo as vars portadoras de segredo (que os
    /// PTYs filhos herdariam). Prova que o token NÃO pode chegar ao env de um terminal do agente.
    #[test]
    fn scrub_removes_secret_env_vars_so_children_cannot_inherit() {
        std::env::set_var("LINA_DEPLOY_TOKEN", "TOKEN-SUPER-SECRETO");
        assert!(std::env::var_os("LINA_DEPLOY_TOKEN").is_some());

        let removed = scrub_pty_secret_env();
        assert!(
            removed.contains(&"LINA_DEPLOY_TOKEN"),
            "a var semeada deve constar dos removidos"
        );
        assert!(
            std::env::var_os("LINA_DEPLOY_TOKEN").is_none(),
            "após o scrub, nenhum PTY filho herda o token (ele vive só no cofre)"
        );
    }

    /// Roster vivo com os nós `names` registrados (writer = sink) — para o roster-check do hole 3.
    fn sup_with_nodes(names: &[&str]) -> Arc<Supervisor> {
        let sup = Arc::new(Supervisor::new());
        for n in names {
            let _ = sup.register(*n, None, Box::new(std::io::sink()));
        }
        sup
    }

    /// **Fio 1 (headless, prova de custódia VIVA no app):** com o token SÓ no cofre, um `broker.do`
    /// na fila de broker (1) BLOQUEIA sem confirmação (ActionGated{ask}+BrokerDenied{unconfirmed},
    /// banner pendente, executor NÃO roda) e (2) após o gate humano (confirm_requested = id) EXECUTA
    /// com o segredo do cofre (BrokerExecuted + marcador no disco) — o agente nunca teve o token.
    #[test]
    fn broker_pump_gate_blocks_then_executes_on_confirm() {
        let base = std::env::temp_dir().join(format!("lina-brokerpump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("abrir store"),
        ));

        // Token SÓ no cofre (o agente nunca o tem). Cofre em memória (MockStore).
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        vault
            .set("deploy", "prod", "DEPLOY-KEY-DO-COFRE")
            .expect("semear");

        // Pedido custodiado na fila de broker, origem autenticada @Dev (enqueue_as).
        let broker_mb = Mailbox::new(&broker_root);
        let req =
            MailMessage::new("@Dev", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        broker_mb.enqueue_as("@Dev", &req).expect("enqueue_as @Dev");

        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            Arc::clone(&model),
            sup_with_nodes(&["@Dev"]), // @Dev é um nó real → passa o roster-check
        );

        // (1) 1º tick: drena → run_custody(confirmed=false) → bloqueia + enfileira no gate.
        pump.tick();
        let (pend_id, pend_requester) = {
            let d = lock(&desk);
            let p = d.front().expect("pendente na frente após o pedido");
            assert!(
                matches!(p, PendingGate::Custody { action, secret_key, .. } if action == "deploy" && secret_key == "prod")
            );
            assert_eq!(p.requester(), "@Dev", "origem autenticada (A3)");
            (p.id().to_string(), p.requester().to_string())
        };
        assert_eq!(pend_requester, "@Dev");
        let kinds_after_block: Vec<String> = lock(&store)
            .events()
            .expect("eventos")
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert!(kinds_after_block.iter().any(|k| k == "ActionGated"));
        assert!(kinds_after_block.iter().any(|k| k == "BrokerDenied"));
        assert!(
            !kinds_after_block.iter().any(|k| k == "BrokerExecuted"),
            "sem confirmação NUNCA executa"
        );

        // (2) Gate humano: a UI sinaliza a confirmação da frente → 2º tick executa com o segredo do cofre.
        lock(&desk).confirm_requested = Some(pend_id);
        pump.tick();

        let events = lock(&store).events().expect("eventos");
        assert!(
            events
                .iter()
                .any(|r| r.kind == "BrokerExecuted" && r.payload["action"] == "deploy"),
            "após confirmação, o broker executa COM o segredo do cofre"
        );
        // O segredo NUNCA pode aparecer no log (invariante #2).
        let dump = format!(
            "{:?}",
            events.iter().map(|r| &r.payload).collect::<Vec<_>>()
        );
        assert!(
            !dump.contains("DEPLOY-KEY-DO-COFRE"),
            "o token não pode vazar ao log"
        );

        // Marcador de execução no disco (prova observável; carrega só o TAMANHO do segredo).
        let marker = broker_root.join("executed-deploy.txt");
        let body = std::fs::read_to_string(&marker).expect("marcador de execução");
        assert!(body.contains("deploy"));
        assert!(
            !body.contains("DEPLOY-KEY-DO-COFRE"),
            "o marcador não expõe o token"
        );

        // O pendente saiu da fila e o banner agora mostra o resultado efêmero.
        assert!(lock(&desk).front().is_none(), "fila vazia após executar");
        assert!(
            lock(&desk)
                .banner()
                .is_some_and(|b| b.contains("executado")),
            "banner de resultado visível"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Hole 1 (resume sem gate):** um `resume.request` na fila de broker NÃO des-pausa sozinho —
    /// fica pendente; o estado segue `Paused` até a confirmação humana (⌘⏎), quando o app apenda
    /// `CostCeilingResumed`. Prova que o agente, só pela fila, jamais retoma o teto.
    #[test]
    fn broker_pump_resume_requires_human_gate_to_unpause() {
        let base = std::env::temp_dir().join(format!("lina-resumegate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("abrir store"),
        ));
        // Semeia o estado PAUSED (último evento de teto = CostCeilingHit).
        lock(&store)
            .append(&DomainEvent::CostCeilingHit {
                day: "2026-06-02".into(),
                tokens: 100,
            })
            .expect("semear pause");
        assert!(
            CostLedger::replay(&lock(&store), now_ms())
                .expect("ledger")
                .paused,
            "pré-condição: workspace pausado"
        );

        let broker_mb = Mailbox::new(&broker_root);
        let req =
            MailMessage::new("@Dev", "broker", "resume.request", "").with_ref("resume:ceiling");
        broker_mb.enqueue_as("@Dev", &req).expect("enqueue_as");

        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            model,
            sup_with_nodes(&["@Dev"]),
        );

        // 1º tick: o pedido só ENFILEIRA — NÃO des-pausa (o agente não pode retomar).
        pump.tick();
        let pend_id = {
            let d = lock(&desk);
            let p = d.front().expect("resume pendente");
            assert!(matches!(p, PendingGate::Resume { .. }));
            assert_eq!(p.requester(), "@Dev", "origem autenticada");
            p.id().to_string()
        };
        assert!(
            CostLedger::replay(&lock(&store), now_ms())
                .expect("ledger")
                .paused,
            "APÓS o pedido do agente, o teto SEGUE pausado (sem gate humano)"
        );
        assert!(
            lock(&store)
                .events()
                .expect("eventos")
                .iter()
                .all(|r| r.kind != "CostCeilingResumed"),
            "nenhum CostCeilingResumed antes do gate humano"
        );

        // Gate humano: confirma a frente → o app apenda CostCeilingResumed e des-pausa.
        lock(&desk).confirm_requested = Some(pend_id);
        pump.tick();
        assert!(
            !CostLedger::replay(&lock(&store), now_ms())
                .expect("ledger")
                .paused,
            "após o gate humano, o teto é retomado"
        );
        assert!(
            lock(&store)
                .events()
                .expect("eventos")
                .iter()
                .any(|r| r.kind == "CostCeilingResumed"),
            "CostCeilingResumed apendado SÓ após o gate humano"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Hole 3 (requester forjável) — 3 vetores barrados:** o drain (a) carimba `from` = dir-dono,
    /// vencendo um `from` de conteúdo forjado; (b) REJEITA um arquivo FLAT plantado; e (c) — ROUND 5b —
    /// REJEITA um subdir POR-NÓ plantado com nome de AUTORIDADE inventado (`@Admin`) que NÃO é um nó do
    /// roster. Só a origem real (@Dev) entra; o banner nunca mostra um requester forjado.
    #[test]
    fn broker_drain_authenticates_origin_roster_and_rejects_forgeries() {
        let base = std::env::temp_dir().join(format!("lina-authorigin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let broker_mb = Mailbox::new(&broker_root);

        // (a) Pedido legítimo por-nó de @Dev (nó REAL), com `from` de CONTEÚDO forjado = @Admin.
        let forged =
            MailMessage::new("@Admin", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        broker_mb
            .enqueue_as("@Dev", &forged)
            .expect("enqueue_as @Dev");

        // (b) Arquivo FLAT plantado direto no outbox (bypassa o CLI) — deve ser rejeitado.
        let flat =
            MailMessage::new("@Admin", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        std::fs::create_dir_all(broker_mb.outbox_dir()).expect("mkdir outbox");
        std::fs::write(
            broker_mb.outbox_dir().join(format!("{}.json", flat.id)),
            serde_json::to_string(&flat).expect("ser"),
        )
        .expect("plantar flat");

        // (c) Subdir POR-NÓ plantado com nome de autoridade INVENTADO @Admin (NÃO é nó do roster).
        let planted =
            MailMessage::new("@Admin", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        broker_mb
            .enqueue_as("@Admin", &planted)
            .expect("plantar subdir @Admin");

        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        // Só @Dev é um nó REAL; @Admin NÃO está no roster.
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            model,
            sup_with_nodes(&["@Dev"]),
        );

        pump.tick();

        let d = lock(&desk);
        assert_eq!(
            d.queue.len(),
            1,
            "só o pedido do nó REAL @Dev entra; flat e subdir-@Admin-forjado são rejeitados"
        );
        let p = d.front().expect("pendente");
        assert_eq!(
            p.requester(),
            "@Dev",
            "a ORIGEM autenticada (dir @Dev, nó real) vence o `from` forjado @Admin"
        );
        assert!(
            p.display().contains("@Dev") && !p.display().contains("@Admin"),
            "o banner mostra a origem real, nunca um @Admin inventado"
        );
        drop(d);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Hole 2 / DoS (Round 5b):** a fila do gate é CAPEADA em [`MAX_PENDING_GATES`]; um flood de
    /// pedidos (mesmo de um nó REAL) NÃO enterra indefinidamente — acima do teto, novos são recusados
    /// SEM enfileirar nem escrever evento durável. A frente (1º pedido) é preservada.
    #[test]
    fn broker_queue_is_capped_against_flood() {
        let base = std::env::temp_dir().join(format!("lina-flood-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let broker_mb = Mailbox::new(&broker_root);

        // Um nó REAL (@Dev) faz FLOOD da própria fila com muito mais que o teto.
        let flood = MAX_PENDING_GATES + 20;
        for _ in 0..flood {
            let m =
                MailMessage::new("@Dev", "broker", "broker.do", "--env prod").with_ref("do:deploy");
            broker_mb.enqueue_as("@Dev", &m).expect("enqueue_as");
        }

        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            model,
            sup_with_nodes(&["@Dev"]),
        );

        // Vários ticks (o drain é capeado por tick) — a fila NUNCA passa do teto.
        for _ in 0..5 {
            pump.tick();
            assert!(
                lock(&desk).queue.len() <= MAX_PENDING_GATES,
                "a fila do gate nunca excede MAX_PENDING_GATES (anti-flood)"
            );
        }
        assert_eq!(
            lock(&desk).queue.len(),
            MAX_PENDING_GATES,
            "a fila satura no teto; o excedente é recusado sem enfileirar"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Hole 2 (dismiss):** o humano pode RECUSAR (⌘⇧⏎ → reject_requested) a frente — sai da fila SEM
    /// executar (sem BrokerExecuted, sem marcador), dando saída a um flood/erro. O 2º assume.
    #[test]
    fn broker_reject_front_dismisses_without_executing() {
        let base = std::env::temp_dir().join(format!("lina-reject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let broker_mb = Mailbox::new(&broker_root);
        let m1 =
            MailMessage::new("@Dev", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        let m2 =
            MailMessage::new("@Dev", "broker", "broker.do", "--env staging").with_ref("do:deploy");
        broker_mb.enqueue_as("@Dev", &m1).expect("enq1");
        broker_mb.enqueue_as("@Dev", &m2).expect("enq2");

        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            model,
            sup_with_nodes(&["@Dev"]),
        );

        pump.tick(); // enfileira os 2 (FIFO: m1 frente, m2 atrás)
        let front_id = lock(&desk).front().expect("frente").id().to_string();

        // RECUSA a frente.
        lock(&desk).reject_requested = Some(front_id.clone());
        pump.tick();

        // m1 saiu SEM executar; m2 assumiu a frente; nenhum BrokerExecuted.
        let d = lock(&desk);
        assert_eq!(d.queue.len(), 1, "a frente recusada saiu; resta o 2º");
        assert_ne!(d.front().unwrap().id(), front_id, "o 2º assumiu");
        // O resultado "recusado" foi registrado (last_result); o banner agora mostra o 2º (a frente).
        assert!(
            d.last_result
                .as_ref()
                .is_some_and(|(t, _)| t.to_lowercase().contains("recusado")),
            "o resultado da recusa foi registrado"
        );
        assert!(
            d.banner().unwrap().contains("CUSTODIA"),
            "o banner agora mostra o 2º pedido (a nova frente)"
        );
        drop(d);
        assert!(
            lock(&store)
                .events()
                .expect("eventos")
                .iter()
                .all(|r| r.kind != "BrokerExecuted"),
            "recusar NUNCA executa"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Hole 2 (front-running) — Round 5c:** um nó @Evil enche a fila com ids BAIXOS (`msg_000000…`)
    /// que venceriam qualquer id v7 numa ordenação por nome de arquivo; o FAIR-SHARE round-robin
    /// garante que o pedido legítimo de @Dev NÃO fica atrás do flood inteiro — está na 1ª rodada.
    #[test]
    fn broker_drain_round_robin_prevents_front_run_burial() {
        let base = std::env::temp_dir().join(format!("lina-rr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let broker_mb = Mailbox::new(&broker_root);

        // @Evil planta 8 pedidos com ids baixos (venceriam por nome de arquivo um id v7).
        for i in 0..8 {
            let mut m =
                MailMessage::new("x", "broker", "broker.do", "--env prod").with_ref("do:deploy");
            m.id = format!("msg_{i:06}"); // id baixo, válido (msg_<alnum>)
            broker_mb.enqueue_as("@Evil", &m).expect("enqueue_as @Evil");
        }
        // @Dev faz 1 pedido legítimo (id v7 alto, nasce depois — perderia numa ordenação por nome).
        let dev =
            MailMessage::new("@Dev", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        broker_mb.enqueue_as("@Dev", &dev).expect("enqueue_as @Dev");

        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            model,
            sup_with_nodes(&["@Evil", "@Dev"]),
        );

        pump.tick();

        let d = lock(&desk);
        let pos = d
            .queue
            .iter()
            .position(|p| p.requester() == "@Dev")
            .expect("@Dev na fila");
        assert!(
            pos <= 1,
            "fair-share: @Dev na 1a rodada (pos {pos}), nao enterrado atras do flood de 8 do @Evil"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Hole 3 (spoof por casing/espaço) — Round 5c:** um subdir cujo nome só NORMALIZA-colide com um
    /// nó real ("  terminal a  " vs "Terminal A") resolve ao nó, mas o banner mostra o nome CANÔNICO do
    /// roster — nunca a string crua enganosa.
    #[test]
    fn broker_drain_uses_canonical_roster_name_not_raw_dir() {
        let base = std::env::temp_dir().join(format!("lina-canon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let broker_root = base.join("broker");
        let broker_mb = Mailbox::new(&broker_root);

        // Subdir com casing/espaço enganosos que normaliza-colide com "Terminal A".
        let m = MailMessage::new("x", "broker", "broker.do", "--env prod").with_ref("do:deploy");
        broker_mb
            .enqueue_as("  terminal a  ", &m)
            .expect("enqueue_as spoof");

        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let vault = SecretVault::with_store("lina-space/teste", lina_secrets::MockStore::new());
        let mut pump = BrokerPump::new(
            broker_mb,
            Arc::clone(&store),
            vault,
            Arc::clone(&desk),
            model,
            sup_with_nodes(&["Terminal A"]),
        );

        pump.tick();

        let d = lock(&desk);
        let p = d.front().expect("pendente");
        assert_eq!(
            p.requester(),
            "Terminal A",
            "nome CANONICO do roster, nao a string crua '  terminal a  '"
        );
        assert!(
            p.display().contains("Terminal A") && !p.display().contains("terminal a"),
            "o banner mostra o canonico, nunca a variante crua enganosa"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

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
            "terminal",
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
            "terminal",
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

    /// **W4-3 (headless) — pulso na ENTREGA REAL + FREIO.** Um `ask A→B` na mailbox, drenado pela
    /// `MailboxPump`, ENTREGA e acende o PULSO (`from`=A, `to`=B) — evento real, não o A2aTrigger demo.
    /// O FREIO pausa: um `ask` novo fica ENFILEIRADO (Queued, não entregue → sem pulso); soltar o freio
    /// DRENA e entrega. Os pares Orchestration{Paused,Resumed} ficam no log.
    #[test]
    fn mailbox_pump_pulse_on_real_delivery_and_brake_queues() {
        let dir = std::env::temp_dir().join(format!("lina-w43-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(
            EventStore::open(dir.join("events")).expect("store"),
        ));

        let mut pty = PtyManager::new();
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _drx) = std::sync::mpsc::channel::<GridDelta>();
        let (node_a, grid_a) = wire_terminal(
            &mut pty,
            &sup,
            &delta_tx,
            "A",
            "Terminal A",
            "terminal",
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
            "terminal",
            PtyCommand::new("cat"),
            80,
            24,
        )
        .expect("wire B");
        let _ = sup.set_status(node_a, CoreStatus::Running);
        let _ = sup.set_status(node_b, CoreStatus::Running);

        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        lock(&grids).insert(node_a, grid_a);
        lock(&grids).insert(node_b, grid_b);

        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let brake = crate::wiring::new_brake();
        let mailbox_root = dir.join(".lina");
        let mut pump = MailboxPump::new(
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&grids),
            Mailbox::new(&mailbox_root),
            Arc::clone(&model),
            Arc::clone(&brake),
            0, // teto de custo desligado neste teste (freio/pulso)
        );
        let outbox = Mailbox::new(&mailbox_root); // handle separado p/ enfileirar (mesmo `.lina/`)
        let kinds = |s: &Arc<Mutex<EventStore>>| {
            lock(s)
                .events()
                .expect("eventos")
                .into_iter()
                .map(|r| r.kind)
                .collect::<Vec<_>>()
        };

        // (1) ask A→B → entrega REAL → pulso A→B aceso. Round 6: o A2A real usa o outbox POR-NÓ
        // (`enqueue_as`, origem autenticada); o flat é anônimo (`from`=""→UnknownSender), então NÃO usar.
        outbox
            .enqueue_as(
                "Terminal A",
                &MailMessage::new("Terminal A", "Terminal B", "ask", "oi 1"),
            )
            .expect("enfileira 1");
        pump.tick();
        assert!(
            lock(&model)
                .pulse
                .is_some_and(|p| p.from == node_a && p.to == node_b),
            "a entrega real do ask acende o pulso A→B"
        );

        // (2) FREIO: pausa.
        lock(&brake).toggle_requested = true;
        pump.tick();
        assert!(lock(&brake).paused, "o freio pausou a orquestração");
        assert!(
            kinds(&store).iter().any(|k| k == "OrchestrationPaused"),
            "OrchestrationPaused apendado no log"
        );

        // (3) Pausado: um ask novo fica ENFILEIRADO (não entrega → sem pulso novo).
        lock(&model).pulse = None;
        outbox
            .enqueue_as(
                "Terminal A",
                &MailMessage::new("Terminal A", "Terminal B", "ask", "oi 2"),
            )
            .expect("enfileira 2");
        pump.tick();
        assert!(
            lock(&model).pulse.is_none(),
            "sob o freio a delegação ENFILEIRA (não injeta → sem pulso)"
        );

        // (4) Solta o freio → DRENA a fila represada → entrega → pulso + OrchestrationResumed.
        lock(&brake).toggle_requested = true;
        pump.tick();
        assert!(!lock(&brake).paused, "o freio foi solto");
        assert!(
            kinds(&store).iter().any(|k| k == "OrchestrationResumed"),
            "OrchestrationResumed apendado no log"
        );
        assert!(
            lock(&model).pulse.is_some(),
            "ao retomar, a fila represada drena e entrega (o pulso acende)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **W3-7c (headless END-TO-END) — TETO DE CUSTO REAL deixa de ser INERTE.** Com o teto ARMADO
    /// (`token_budget_day=100`), o app EMITE `TokenUsageReported` (o que faltava) somando > teto → a
    /// PRÓXIMA delegação bate `CostCeilingHit` e o workspace fica PAUSADO (espelhado em `model.cost_paused`,
    /// visível). Só o gate humano (`CostCeilingResumed`) reabre. Prova o teto que era inerte.
    #[test]
    fn cost_ceiling_goes_live_when_app_feeds_token_usage() {
        let dir = std::env::temp_dir().join(format!("lina-teto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(
            EventStore::open(dir.join("events")).expect("store"),
        ));
        let mut pty = PtyManager::new();
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _drx) = std::sync::mpsc::channel::<GridDelta>();
        let (node_a, grid_a) = wire_terminal(
            &mut pty,
            &sup,
            &delta_tx,
            "A",
            "Terminal A",
            "terminal",
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
            "terminal",
            PtyCommand::new("cat"),
            80,
            24,
        )
        .expect("wire B");
        let _ = sup.set_status(node_a, CoreStatus::Running);
        let _ = sup.set_status(node_b, CoreStatus::Running);
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        lock(&grids).insert(node_a, grid_a);
        lock(&grids).insert(node_b, grid_b);
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let brake = crate::wiring::new_brake();
        let mailbox_root = dir.join(".lina");
        // Teto ARMADO em 100 tokens/dia.
        let mut pump = MailboxPump::new(
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&grids),
            Mailbox::new(&mailbox_root),
            Arc::clone(&model),
            Arc::clone(&brake),
            100,
        );
        let outbox = Mailbox::new(&mailbox_root);
        let kinds = |s: &Arc<Mutex<EventStore>>| {
            lock(s)
                .events()
                .expect("ev")
                .into_iter()
                .map(|r| r.kind)
                .collect::<Vec<_>>()
        };

        // (1) O app OBSERVA o fim-de-resposta e EMITE TokenUsageReported (aqui direto; em produção é a
        // bomba via CostMeter). 60 + 60 = 120 ≥ 100.
        {
            let mut s = lock(&store);
            for _ in 0..2 {
                s.append(&DomainEvent::TokenUsageReported {
                    node: node_a.to_string(),
                    tokens: 60,
                })
                .expect("uso");
            }
        }

        // (2) A PRÓXIMA delegação bate o teto: CostCeilingHit + Paused; NÃO entrega (sem pulso).
        outbox
            .enqueue_as(
                "Terminal A",
                &MailMessage::new("Terminal A", "Terminal B", "ask", "x"),
            )
            .expect("enfileira");
        pump.tick();
        assert!(
            kinds(&store).iter().any(|k| k == "CostCeilingHit"),
            "a delegação sobre o teto apenda CostCeilingHit"
        );
        assert!(
            lock(&model).cost_paused,
            "o model ESPELHA o teto pausado (banner visível)"
        );
        assert!(
            lock(&model).pulse.is_none(),
            "delegação barrada pelo teto NÃO entrega"
        );

        // (3) Só o gate humano reabre: CostCeilingResumed (lina resume → ⌘⏎) → cost_paused limpa.
        {
            let day = CostLedger::replay(&lock(&store), now_ms())
                .expect("ledger")
                .day;
            lock(&store)
                .append(&DomainEvent::CostCeilingResumed { day })
                .expect("resume");
        }
        pump.tick();
        assert!(
            !lock(&model).cost_paused,
            "após o gate humano (CostCeilingResumed), o teto reabre"
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
    fn test_manager(
        tag: &str,
        bootstrap: Option<BootstrapWriter>,
    ) -> (NodeManager, Arc<Mutex<EventStore>>, Model) {
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
                    "terminal",
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
            bootstrap,
            dir.join(".lina"),
        );
        (nm, store, model)
    }

    /// **GATE de ADD (headless, determinístico):** `add_node` sobe a contagem 2→3, dá um grid
    /// ao novo nó, persiste NodeAdded+TerminalSpawned, e o event log RECONSTRÓI o novo nó
    /// (`project()`) — provando "todo nó é shell real + o log é a fonte da verdade".
    #[test]
    fn add_node_grows_count_persists_and_reconstructs() {
        let (nm, store, model) = test_manager("add", None);
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

    /// **W4-2 (M2) — papel derivado pelo nome.** `infer_agent_role` mapeia "Revisor" → `reviewer`
    /// (via overlay app-side), nomes que casam o registry default → seu papel, e desconhecidos → o
    /// fallback `DEVELOPER`. Infalível (nunca panica).
    #[test]
    fn infer_agent_role_derives_from_name() {
        assert_eq!(infer_agent_role("Revisor"), "reviewer", "overlay app-side");
        assert_eq!(infer_agent_role("reviewer"), "reviewer");
        assert_eq!(infer_agent_role("Backend"), "BACKEND", "registry default");
        assert_eq!(infer_agent_role("QA do time"), "QA");
        assert_eq!(infer_agent_role("xyzzy-sem-papel"), "DEVELOPER", "fallback");
    }

    /// **W4-2 (M2) — GATE headless de "Novo Agente":** `create_agent("Revisor")` cria um nó-terminal
    /// (CLI real spawned via `cat` no teste), DERIVA o papel `reviewer`, GRAVA `NodeRoleAssigned` no
    /// log (→ projeção `ProjectedNode.role` = reviewer; sobrevive a replay), e projeta no model com o
    /// nome. Nome inválido é recusado sem efeito.
    #[test]
    fn create_agent_derives_role_records_and_projects() {
        let (nm, store, model) = test_manager("m2", None);
        assert_eq!(lock(&model).order.len(), 2, "começa com 2 nós");

        let id = nm.create_agent("Revisor").expect("M2 cria o agente");
        assert_eq!(lock(&model).order.len(), 3, "o agente entra no canvas");
        assert!(
            lock(&model)
                .nodes
                .get(&id)
                .is_some_and(|v| v.name == "Revisor"),
            "o nó tem o nome dado"
        );

        // O papel foi GRAVADO no log → reconstrói na projeção (fonte da verdade).
        let events = lock(&store).events().expect("eventos");
        assert!(
            events
                .iter()
                .any(|r| r.kind == "NodeRoleAssigned" && r.payload["role"] == "reviewer"),
            "NodeRoleAssigned{{role:reviewer}} apendado"
        );
        let projected = lock(&store).project().expect("project");
        assert_eq!(
            projected.nodes.get(&id).and_then(|n| n.role.clone()),
            Some("reviewer".to_string()),
            "a projeção tem o papel reviewer (sobrevive a replay)"
        );

        // Nome inválido → recusado, canvas inalterado.
        assert!(nm.create_agent("  ").is_err(), "nome vazio recusado");
        assert!(
            nm.create_agent("a{{b}}").is_err(),
            "nome com sentinela recusado"
        );
        assert_eq!(lock(&model).order.len(), 3, "recusas não criam nó");
    }

    /// **W4-2 · M3/M4 (hook ligado, headless):** `create_artifact` cria nota/pasta — apenda os eventos
    /// (Note/FolderCreated) E SEMEIA o nó VIVO no model com o kind certo (aparece sem reabrir o app);
    /// a projeção (replay) reconstrói o kind. E `remove_node` agora TOLERA o nó-artefato (sem PTY).
    #[test]
    fn create_artifact_seeds_live_node_logs_and_removes() {
        let (nm, store, model) = test_manager("artifact", None);
        let before = lock(&model).order.len();

        let note = nm
            .create_artifact(crate::creators::CreatorKind::Note, "Ideias")
            .expect("cria nota");
        assert_eq!(
            lock(&model).order.len(),
            before + 1,
            "a nota aparece VIVA no model"
        );
        assert!(
            lock(&model)
                .nodes
                .get(&note)
                .is_some_and(|v| v.name == "Ideias" && matches!(v.kind, NodeKind::Note)),
            "nó da nota semeado com kind Note + nome"
        );

        let folder = nm
            .create_artifact(crate::creators::CreatorKind::Folder, "Clientes")
            .expect("cria pasta");
        assert!(lock(&model)
            .nodes
            .get(&folder)
            .is_some_and(|v| matches!(v.kind, NodeKind::Folder)));

        // Eventos no log + projeção (replay) reconstrói o kind certo (fonte da verdade).
        let kinds: Vec<String> = lock(&store)
            .events()
            .expect("ev")
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert!(kinds.iter().any(|k| k == "NoteCreated"));
        assert!(kinds.iter().any(|k| k == "FolderCreated"));
        let proj = lock(&store).project().expect("project");
        assert_eq!(proj.nodes.get(&note).map(|n| n.kind.as_str()), Some("Note"));
        assert_eq!(
            proj.nodes.get(&folder).map(|n| n.kind.as_str()),
            Some("Folder")
        );

        // `remove_node` tolera o nó-ARTEFATO (sem PTY) — antes errava "sem PTY registrado".
        nm.remove_node(note)
            .expect("remover a nota (sem PTY) funciona");
        assert!(
            !lock(&model).nodes.contains_key(&note),
            "a nota saiu do model"
        );
    }

    /// **GATE de REMOVE (headless, determinístico):** após add (3 nós), `remove_node` desce a
    /// contagem 3→2, encerra o PTY e tira do grid, persiste NodeRemoved, e o replay fica SEM o
    /// nó. Invariante não-técnico: o ÚLTIMO nó nunca sai (o canvas jamais fica em branco). Sem
    /// panic do começo ao fim.
    #[test]
    fn remove_node_shrinks_count_persists_and_never_empties() {
        let (nm, store, model) = test_manager("remove", None);
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
        let (nm, _store, _model) = test_manager("cards", None);
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

    /// **GATE W2-4 — tela→célula (round-trip) ciente de pan/zoom:** o centro da célula
    /// `(col,row)` mapeia de volta para `(col,row)`.
    #[test]
    fn screen_to_cell_maps_under_pan_zoom() {
        let dims = (80usize, 24usize);
        let card = (30.0_f32, 96.0_f32);
        for cam in [
            Camera::default(),
            Camera {
                pan: (120.0, -40.0),
                zoom: 1.5,
            },
            Camera {
                pan: (-200.0, 60.0),
                zoom: 0.6,
            },
        ] {
            let z = cam.zoom;
            let (sx, sy) = cam.world_to_screen(card);
            let (gx, gy) = (sx + GRID_OX, sy + GRID_OY_FIXED + CELL_H * z);
            for &(col, row) in &[(0usize, 0usize), (10, 5), (40, 12), (79, 23)] {
                let px = gx + (col as f32 + 0.5) * CELL_W * z;
                let py = gy + (row as f32 + 0.5) * CELL_H * z;
                assert_eq!(
                    screen_to_cell(&cam, card, (px, py), dims),
                    (col, row),
                    "cam={cam:?} célula ({col},{row})"
                );
            }
        }
        // Fora do grid satura na borda (não estoura).
        assert_eq!(
            screen_to_cell(&Camera::default(), card, (-9999.0, -9999.0), dims),
            (0, 0)
        );
        assert_eq!(
            screen_to_cell(&Camera::default(), card, (9_999_999.0, 9_999_999.0), dims),
            (79, 23)
        );
    }

    /// **GATE W2-4 — membership da seleção stream** (espelha o `selection_text` do backend).
    #[test]
    fn selection_membership_is_stream_not_rectangle() {
        // 1 linha, direção invertida → normaliza.
        let (sc, sr, ec, er) = normalize_sel((5, 1), (2, 1));
        assert_eq!((sc, sr, ec, er), (2, 1, 5, 1));
        assert!(cell_in_selection(2, 1, sc, sr, ec, er) && cell_in_selection(5, 1, sc, sr, ec, er));
        assert!(
            !cell_in_selection(1, 1, sc, sr, ec, er) && !cell_in_selection(6, 1, sc, sr, ec, er)
        );
        assert!(!cell_in_selection(3, 0, sc, sr, ec, er));
        // Multi-linha (3,1)→(2,3): linha 1 de col>=3; linha 2 inteira; linha 3 col<=2.
        let (sc, sr, ec, er) = normalize_sel((3, 1), (2, 3));
        assert_eq!((sc, sr, ec, er), (3, 1, 2, 3));
        assert!(
            cell_in_selection(3, 1, sc, sr, ec, er) && cell_in_selection(79, 1, sc, sr, ec, er)
        );
        assert!(!cell_in_selection(2, 1, sc, sr, ec, er));
        assert!(
            cell_in_selection(0, 2, sc, sr, ec, er) && cell_in_selection(79, 2, sc, sr, ec, er)
        );
        assert!(
            cell_in_selection(2, 3, sc, sr, ec, er) && !cell_in_selection(3, 3, sc, sr, ec, er)
        );
    }

    /// **GATE W2-4 — seleção por células → `selection_text` correto** (usa a primitiva do core).
    #[test]
    fn selection_cells_yield_expected_text() {
        let mut b = AlacrittyBackend::new(20, 4);
        b.advance(b"\x1b[2J\x1b[H\x1b[1;1Hhello world");
        b.set_selection(0, 0, 4, 0);
        assert_eq!(b.selection_text().as_deref(), Some("hello"));
        b.clear_selection();
        assert_eq!(b.selection_text(), None);
    }

    /// **GATE W2-4 — roteamento mouse:** quando o TUI pede reporting, `encode_pointer` gera os
    /// bytes SGR certos; quando NÃO pede, devolve `None` (a UI então seleciona).
    #[test]
    fn pointer_routes_to_encode_when_reporting_else_selection() {
        let mut b = AlacrittyBackend::new(80, 24);
        b.advance(b"\x1b[2J\x1b[Hhello world");
        // SEM reporting → encode = None (a UI seleciona); set_selection segue funcionando.
        assert!(!b.is_mouse_reporting());
        assert_eq!(
            encode_pointer(&b, PtrAction::Press, (0, 0), (false, false, false)),
            None
        );
        b.set_selection(0, 0, 4, 0);
        assert_eq!(b.selection_text().as_deref(), Some("hello"));
        b.clear_selection();
        // COM reporting (?1000h + ?1006h) → press na (0,0) = CSI<0;1;1M.
        b.advance(b"\x1b[?1000h\x1b[?1006h");
        assert!(b.is_mouse_reporting());
        assert_eq!(
            encode_pointer(&b, PtrAction::Press, (0, 0), (false, false, false)).as_deref(),
            Some(&b"\x1b[<0;1;1M"[..])
        );
    }

    /// **GATE W3-2 (integração):** criar um terminal escreve o `<cwd>/CLAUDE.md`; uma mudança de
    /// roster (add/rename/remove) **REESCREVE** os arquivos dos colegas — o arquivo é a âncora.
    #[test]
    fn bootstrap_writes_and_rewrites_on_roster_change() {
        let ws = std::env::temp_dir().join(format!("lina-w32-int-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, _store, model) = test_manager("boot", Some(bw));
        nm.rewrite_bootstrap(); // seed inicial: escreve A e B.

        let a_md = ws.join("A").join("CLAUDE.md");
        let a1 = std::fs::read_to_string(&a_md).expect("CLAUDE.md de A");
        // Doutrina canônica: 8 BLOCOS marcados `<!-- ===== BLOCO N · … -->` (1ª e última presentes).
        assert!(
            a1.contains("===== BLOCO 1 ·") && a1.contains("===== BLOCO 8 ·"),
            "8 blocos canônicos"
        );
        assert!(a1.contains("Terminal B"), "A lista B como colega");

        // ADD: novo terminal C → CLAUDE.md próprio (no cwd dele) e o de A REESCRITO com C.
        let c = nm.add_node().expect("add C");
        assert!(
            ws.join("t2").join("CLAUDE.md").exists(),
            "C tem CLAUDE.md no seu cwd"
        );
        let a2 = std::fs::read_to_string(&a_md).expect("re-read A");
        assert!(a2.contains("Terminal C"), "A reescrito com o colega C");

        // RENAME: renomear B reescreve o CLAUDE.md de A (diff observável) sem reiniciar.
        let b_id = lock(&model).order.get(1).copied().expect("B existe");
        nm.rename_node(b_id, "@QA Tester").expect("rename B");
        let a3 = std::fs::read_to_string(&a_md).expect("re-read A pós-rename");
        assert_ne!(a2, a3, "rename mudou o CLAUDE.md de A");
        assert!(
            a3.contains("@QA Tester") && !a3.contains("Terminal B"),
            "nome novo no lugar do antigo"
        );

        // REMOVE: tirar C reescreve A sem C.
        nm.remove_node(c).expect("remove C");
        let a4 = std::fs::read_to_string(&a_md).expect("re-read A pós-remove");
        assert!(!a4.contains("Terminal C"), "A não lista mais C");

        let _ = std::fs::remove_dir_all(&ws);
    }

    /// **GATE (env builder do PATH):** o diretório do `lina` entra na FRENTE do `PATH`, preserva o
    /// herdado, usa o separador do SO (round-trip por `split_paths`) e é idempotente (não duplica).
    /// Função pura → headless e determinística.
    #[test]
    fn prepend_dir_to_path_puts_lina_dir_first_and_is_idempotent() {
        let dir = PathBuf::from("/opt/lina/bin");
        let other = PathBuf::from("/usr/bin");

        // Com PATH herdado: o dir do lina vem primeiro e o herdado é preservado.
        let cur = std::env::join_paths([&other]).expect("join cur");
        let got = prepend_dir_to_path(&dir, Some(cur.as_os_str())).expect("join");
        let parts: Vec<PathBuf> = std::env::split_paths(&got).collect();
        assert_eq!(parts.first(), Some(&dir), "o dir do lina vem primeiro");
        assert!(parts.contains(&other), "preserva o PATH herdado");

        // Idempotente: se o dir já está no PATH, não duplica.
        let cur2 = std::env::join_paths([&dir, &other]).expect("join cur2");
        let got2 = prepend_dir_to_path(&dir, Some(cur2.as_os_str())).expect("join2");
        let parts2: Vec<PathBuf> = std::env::split_paths(&got2).collect();
        assert_eq!(
            parts2.iter().filter(|p| **p == dir).count(),
            1,
            "não duplica o dir do lina"
        );
        assert_eq!(parts2.first(), Some(&dir), "ainda na frente após dedupe");

        // Sem PATH herdado: só o dir do lina.
        let got3 = prepend_dir_to_path(&dir, None).expect("join3");
        let parts3: Vec<PathBuf> = std::env::split_paths(&got3).collect();
        assert_eq!(parts3, vec![dir], "sem PATH herdado, só o dir do lina");
    }

    /// **GATE (skill installer):** instalar a skill cria `<cwd>/.claude/skills/lina-agent-bus/SKILL.md`
    /// (resolvendo a fonte via CARGO_MANIFEST_DIR → `<repo>/assets`) e é idempotente em recriação.
    /// Teste headless: copia para um tmpdir e confere o arquivo.
    #[test]
    fn installs_agent_bus_skill_into_cwd_idempotently() {
        let cwd = std::env::temp_dir().join(format!("lina-skill-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cwd);
        std::fs::create_dir_all(&cwd).expect("cwd");

        install_agent_bus_skill(&cwd);
        let skill = cwd
            .join(".claude")
            .join("skills")
            .join("lina-agent-bus")
            .join("SKILL.md");
        assert!(skill.is_file(), "SKILL.md instalada: {}", skill.display());
        let body = std::fs::read_to_string(&skill).expect("ler SKILL.md");
        assert!(
            body.contains("lina-agent-bus"),
            "o conteúdo da skill foi copiado"
        );

        // Idempotência: reinstalar sobre o existente não falha e mantém o arquivo.
        install_agent_bus_skill(&cwd);
        assert!(skill.is_file(), "idempotente em recriação");

        let _ = std::fs::remove_dir_all(&cwd);
    }
}
