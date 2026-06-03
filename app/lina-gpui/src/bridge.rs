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
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// W3-2: motor de bootstrap turno-0 (gera o `<workspace>/CLAUDE.md` por terminal).
use lina_bootstrap::{Autonomy, BootstrapInput, Bootstrapper};

use lina_core::{
    deliver_a2a, lookup_action, now_ms, run_custody, A2aEnvelope, AgentPresence, AlacrittyBackend,
    BrokerOutcome, BrokerRequest, BusEvent, CliProfile, DeliveryOutcome, DomainEvent, EventStore,
    GridDelta, InjectPolicy, MailMessage, Mailbox, NodeStatus as CoreStatus, PtyCommand,
    PtyManager, Recipient, RolePolicy, RouteOutcome, Router, Supervisor, VtBackend,
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
    /// Último roster escrito no `agents.json` (evita reescrever a cada tick sem mudança).
    last_roster: Vec<AgentPresence>,
}

impl MailboxPump {
    /// Cria o pump enraizado na `mailbox` (o `.lina/` compartilhado do workspace).
    #[must_use]
    pub fn new(
        sup: Arc<Supervisor>,
        store: Arc<Mutex<EventStore>>,
        grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
        mailbox: Mailbox,
        model: Model,
    ) -> Self {
        Self {
            router: Router::new(Arc::clone(&sup), mailbox),
            sup,
            store,
            grids,
            profile: demo_profile(),
            model,
            last_roster: Vec::new(),
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

    /// Um tick: drena a mailbox e roteia. A entrega resolve o grid do alvo e injeta faseado.
    fn tick(&mut self) {
        let sup = Arc::clone(&self.sup);
        let grids = Arc::clone(&self.grids);
        let profile = self.profile.clone();
        let deliver = move |target: NodeId, from: NodeId, text: &str| {
            let grid = lock(&grids).get(&target).cloned();
            match grid {
                Some(g) => deliver_a2a(
                    &sup,
                    target,
                    from,
                    text,
                    &profile,
                    &g,
                    InjectPolicy::AllowAll,
                )
                .map_err(|e| e.to_string()),
                None => Err(format!("sem grid para o alvo {target}")),
            }
        };

        let results = {
            let mut store = lock(&self.store);
            self.router.pump(&mut store, now_ms(), deliver)
        };

        // Se algo roteou, atualiza o event_count da UI (e narra no stderr os bloqueios — nunca
        // falha em silêncio: o usuário/dev vê POR QUE uma mensagem não passou).
        let mut routed = false;
        for (id, outcome) in &results {
            match outcome {
                RouteOutcome::Delivered { .. } | RouteOutcome::Duplicate => routed = true,
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
    }
}

// ════════════════════════ W3-6c · ÚLTIMA MILHA — custódia VIVA no app (ADR 0004) ════════════════════════
//
// Os testes headless já provam `run_custody` (broker.rs). Aqui o APP passa a CHAMÁ-LA: drena a FILA
// DE BROKER (`<LINA_HOME>/broker/`, irmã do outbox A2A — o `Router` nunca a varre), aplica o GATE
// HUMANO (tecla na janela, inforjável pelo PTY do agente) e só então deixa o broker buscar o segredo
// no cofre e executar. O agente NUNCA tem o token (vive só no [`SecretVault`]); sem confirmação
// humana, não há caminho de execução — a custódia é o piso inquebrável.

/// Um pedido custodiado aguardando o gate humano (projetado no banner da topbar).
#[derive(Debug, Clone)]
pub struct PendingCustody {
    /// `id` da `MailMessage` do pedido (chave do gate).
    pub id: String,
    /// Ação custodiada (`deploy`/`pay`/`send`).
    pub action: String,
    /// Requisitante = origem AUTENTICADA (dir-dono carimbado pelo drain por-nó — A3).
    pub requester: String,
    /// Chave do segredo no cofre (ex.: `prod` para `deploy --env prod`).
    pub secret_key: String,
    /// Texto legível do banner ("🔒 … aguarda confirmação").
    pub display: String,
}

/// **Mesa de custódia compartilhada** entre a [`BrokerPump`] (produz o pendente + executa) e a UI
/// (que confirma com TECLA na janela — um agente no PTY não consegue sintetizá-la). A UI nunca toca
/// o cofre nem o broker: só sinaliza `confirm_requested`; quem executa é a pump (escritora única).
#[derive(Debug, Default)]
pub struct CustodyDesk {
    /// Pedido aguardando o gate humano. `None` = nada pendente.
    pub pending: Option<PendingCustody>,
    /// A UI sinalizou que o humano confirmou o pendente de `id`. A pump consome no próximo tick.
    pub confirm_requested: Option<String>,
    /// Último resultado (banner efêmero pós-execução): texto + quando nasceu.
    pub last_result: Option<(String, Instant)>,
}

impl CustodyDesk {
    /// Texto a exibir na topbar AGORA: o pendente (prioridade) ou o último resultado por ~6s.
    #[must_use]
    pub fn banner(&self) -> Option<String> {
        if let Some(p) = &self.pending {
            return Some(p.display.clone());
        }
        match &self.last_result {
            Some((txt, born)) if born.elapsed() < Duration::from_secs(6) => Some(txt.clone()),
            _ => None,
        }
    }
}

/// Handle compartilhado da mesa de custódia.
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

/// **Observador da FILA DE BROKER (`<LINA_HOME>/broker/`).** Numa thread própria, drena os pedidos
/// `broker.do`, dispara o gate humano e — só após confirmação na janela — chama [`run_custody`], que
/// obtém o segredo do `vault` e executa. Genérico sobre o backend do cofre (`MockStore` na demo,
/// `KeyringStore` em produção). Escritor único do estado de execução custodiada.
pub struct BrokerPump<S: SecretStore> {
    mailbox: Mailbox,
    store: Arc<Mutex<EventStore>>,
    vault: SecretVault<S>,
    desk: Desk,
    model: Model,
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
    ) -> Self {
        Self {
            mailbox,
            store,
            vault,
            desk,
            model,
        }
    }

    /// Sobe a thread que observa a fila de broker (gate humano + execução custodiada) a cada ~150 ms.
    pub fn spawn(mut self) -> JoinHandle<()> {
        thread::spawn(move || loop {
            self.tick();
            thread::sleep(Duration::from_millis(150));
        })
    }

    fn tick(&mut self) {
        // (1) Confirmação humana sinalizada pela UI? → executa COM o segredo do cofre.
        let confirm = lock(&self.desk).confirm_requested.take();
        if let Some(id) = confirm {
            self.execute_confirmed(&id);
        }
        // (2) Novos pedidos de broker (recuperável a crash via .inflight + ack).
        let msgs = self.mailbox.drain_to_inflight().unwrap_or_else(|e| {
            eprintln!("lina-gpui: broker pump nao drenou a fila (sera retentado): {e}");
            Vec::new()
        });
        for m in msgs {
            self.handle_request(&m);
            if let Err(e) = self.mailbox.ack_inflight(&m.id) {
                eprintln!(
                    "lina-gpui: broker pump falhou ao confirmar inflight {}: {e}",
                    m.id
                );
            }
        }
    }

    /// 1ª vista de um pedido `broker.do`: roda `run_custody(confirmed=false)` (ActionGated{ask} +
    /// BrokerDenied{unconfirmed}; o cofre NEM é tocado) e SURFAÇA o banner do gate humano.
    fn handle_request(&mut self, m: &MailMessage) {
        if m.intent != "broker.do" {
            return; // a fila é de broker; ignora ruído sem panicar (best-effort, VISÍVEL abaixo)
        }
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
        // requester = origem AUTENTICADA (o drain por-nó carimbou `from` = dir-dono).
        let req = BrokerRequest::new(custody, secret_key.clone(), m.from.clone());

        let outcome = {
            let mut store = lock(&self.store);
            // O app NÃO auto-confirma: 1ª passada SEM confirmação → loga o gate e bloqueia (custódia).
            run_custody(&req, false, &self.vault, &mut store, |_| Ok(()))
        };

        match outcome {
            Ok(BrokerOutcome::DeniedUnconfirmed) => {
                let pend = PendingCustody {
                    id: m.id.clone(),
                    action: action.to_string(),
                    requester: m.from.clone(),
                    secret_key,
                    display: format!(
                        "🔒 CUSTODIA: '{}' pedido por {} — aprove com ⌘⏎ (o agente NAO tem o token)",
                        custody.desc, m.from
                    ),
                };
                lock(&self.desk).pending = Some(pend);
                lock(&self.model).touch();
                eprintln!(
                    "lina-gpui: CUSTODIA — '{action}' de {} aguarda gate humano (⌘⏎). msg {}",
                    m.from, m.id
                );
            }
            Ok(other) => {
                eprintln!("lina-gpui: broker.do {action} 1a passada inesperada: {other:?}");
            }
            Err(e) => eprintln!("lina-gpui: broker.do {action} falhou no gate: {e}"),
        }
    }

    /// O humano confirmou (`id`): roda `run_custody(confirmed=true)` — o BROKER obtém o segredo do
    /// cofre e executa (o agente nunca o teve). Marca o disco como PROVA observável e atualiza o banner.
    fn execute_confirmed(&mut self, id: &str) {
        let pend = lock(&self.desk).pending.clone();
        let Some(pend) = pend.filter(|p| p.id == id) else {
            eprintln!("lina-gpui: confirmacao para {id} sem pendente correspondente — ignorada");
            return;
        };
        let Some(custody) = lookup_action(&pend.action) else {
            eprintln!(
                "lina-gpui: pendente com acao invalida {:?} — abortado",
                pend.action
            );
            lock(&self.desk).pending = None;
            return;
        };
        let req = BrokerRequest::new(custody, pend.secret_key.clone(), pend.requester.clone());
        let exec_root = self.mailbox.root().to_path_buf();
        let action_name = pend.action.clone();

        let outcome = {
            let mut store = lock(&self.store);
            run_custody(&req, true, &self.vault, &mut store, |secret| {
                // MVP: a ação externa real (deploy a um alvo) está fora do escopo; a PROVA é que o
                // executor recebeu o segredo DO COFRE (o agente nunca o viu). Marca o disco com o
                // TAMANHO do segredo (nunca o valor — invariante #2: o token jamais sai do cofre/executor).
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

        let banner = match outcome {
            Ok(BrokerOutcome::Executed) => format!(
                "✅ '{}' executado pelo broker — segredo veio do cofre; o agente nunca o viu",
                pend.action
            ),
            Ok(BrokerOutcome::DeniedNoSecret) => format!(
                "⛔ '{}' bloqueado — segredo AUSENTE do cofre (sem token, sem acao: custodia)",
                pend.action
            ),
            Ok(other) => format!("'{}' resultado inesperado: {other:?}", pend.action),
            Err(e) => format!("⛔ '{}' falhou na execucao: {e}", pend.action),
        };
        eprintln!("lina-gpui: CUSTODIA — {banner}");
        {
            let mut d = lock(&self.desk);
            d.pending = None;
            d.confirm_requested = None;
            d.last_result = Some((banner, Instant::now()));
        }
        lock(&self.model).touch();
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
        }
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

    // ─────────────── W3-6c última milha: custódia (fios 1 e 3) — unidades gpui-free ───────────────

    /// Fio 1: a chave do segredo é extraída de `--env <x>` (deploy); default `default` (pay/send).
    #[test]
    fn parse_secret_key_reads_env_flag_or_defaults() {
        assert_eq!(parse_secret_key("--env prod"), "prod");
        assert_eq!(parse_secret_key("algo --env staging mais"), "staging");
        assert_eq!(parse_secret_key(""), "default");
        assert_eq!(parse_secret_key("--env"), "default"); // sem valor após a flag
    }

    /// Fio 1: o banner prioriza o pendente; sem pendente, mostra o último resultado por uma janela.
    #[test]
    fn custody_desk_banner_prioritizes_pending_then_shows_result() {
        let mut desk = CustodyDesk::default();
        assert!(desk.banner().is_none(), "vazio → sem banner");

        desk.last_result = Some(("✅ deploy executado".into(), Instant::now()));
        assert_eq!(desk.banner().as_deref(), Some("✅ deploy executado"));

        desk.pending = Some(PendingCustody {
            id: "msg_1".into(),
            action: "deploy".into(),
            requester: "@Dev".into(),
            secret_key: "prod".into(),
            display: "🔒 aguarda".into(),
        });
        assert_eq!(
            desk.banner().as_deref(),
            Some("🔒 aguarda"),
            "o pendente vence o último resultado"
        );
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
        );

        // (1) 1º tick: drena → run_custody(confirmed=false) → bloqueia + banner pendente.
        pump.tick();
        let pend = lock(&desk).pending.clone().expect("pendente após o pedido");
        assert_eq!(pend.action, "deploy");
        assert_eq!(pend.requester, "@Dev", "origem autenticada (A3)");
        assert_eq!(pend.secret_key, "prod");
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

        // (2) Gate humano: a UI sinaliza a confirmação → 2º tick executa com o segredo do cofre.
        lock(&desk).confirm_requested = Some(pend.id.clone());
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

        // O pendente foi limpo e o banner agora mostra o resultado efêmero.
        assert!(
            lock(&desk).pending.is_none(),
            "pendente limpo após executar"
        );
        assert!(
            lock(&desk)
                .banner()
                .is_some_and(|b| b.contains("executado")),
            "banner de resultado visível"
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
