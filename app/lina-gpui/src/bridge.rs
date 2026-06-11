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
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// W3-2: motor de bootstrap turno-0 (gera o `<workspace>/CLAUDE.md` por terminal).
use lina_bootstrap::{Autonomy, BootstrapInput, Bootstrapper};
// W4-2 (M2): deriva o papel do agente pelo nome ("Revisor" -> reviewer).

use lina_core::{
    build_paste, deliver_a2a, discover_clis, lookup_action, now_ms, run_custody, A2aEnvelope,
    AgentPresence, AlacrittyBackend, ApprovalDecision, ApprovalExecutor, ApprovalGesture,
    ApprovalKeys, ApprovalPort, BrokerOutcome, BrokerRequest, BusEvent, CliProfile, CostLedger,
    DeliveryOutcome, DiscoveredCli, DomainEvent, EventRecord, EventStore, GridDelta, GridSense,
    MailMessage, Mailbox, NodeStatus as CoreStatus, PortError, PortOutcome, ProjectedState,
    PtyCommand, PtyManager, Recipient, ResolutionVia, RolePolicy, RouteOutcome, Router,
    RouterConfig, Supervisor, SupervisorError, VtBackend, WorkspaceTrust, PROMPT_REGION_ROWS,
};
// A2A UNIVERSAL: o registry de TODOS os CLI Profiles — a entrega resolve o profile do ALVO
// por aqui (CliProfile vem re-exportado do core; ProfileRegistry é o índice por `id`).
use lina_cli_profiles::ProfileRegistry;
// F1-5-2 / F1-4-3: o store durável de scrollback do workspace + o job único de durabilidade.
// O app é PtyManager-direto (não usa `PtyHost`), então o cabo `append-on-scroll` é replicado
// em `wire_terminal_capturing` (addendo do Maestro) — a lógica de harvest vive no core (REUSO).
use lina_core::scrollback::{FlushGuard, FlushGuardConfig, ScrollbackStore};
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

/// W5-5: os `NodeId` dos membros **vivos** do Espaço — a fonte de verdade dos pares de
/// injeção confiados (invariante #5, *pertencimento = conexão*). A allow-list de produção
/// ([`WorkspaceTrust`]) deriva daqui: um id fora deste roster vivo (desconhecido/forjado)
/// não compõe par algum e, por construção, nunca recebe injeção (default-deny).
fn live_member_ids(sup: &Supervisor) -> Vec<NodeId> {
    sup.list()
        .into_iter()
        .filter(|n| n.status.is_alive())
        .map(|n| n.id)
        .collect()
}

// ════════════════════════ F1-2-4 · RE-INJEÇÃO DE DOUTRINA (terminal vivo) ════════════════════════
//
// Ao ACEITAR (gate humano no M6-E) a mudança de papel de um nó vivo, além de reescrever o CLAUDE.md,
// a doutrina nova é re-injetada no PTY vivo — **sem matar o terminal, sem trocar silenciosa**.
//
// COSTURA (mapeada contra o disco): o caminho que a story sugere ("deliver_a2a p/ o próprio nó") NÃO
// existe como drop-in — `route_message` EXCLUI o sender em `resolve()` (auto-entrega → `NoTarget`) e
// `WorkspaceTrust::from_members` EXCLUI pares self (auto-injeção → `InjectionDenied`); usar
// `InjectPolicy::AllowAll` seria a regressão de segurança que a W5-5 fechou. A re-injeção é, por
// natureza, **classe de confiança do INPUT HUMANO** (gatilho = confirmação no modal), não
// agente↔agente — então NÃO passa pelo allow-list de injeção A2A nem toca `deliver_a2a`/`Router`
// (a suíte de segurança do router fica intacta por construção). Entrega-se pela **fila serial de
// escrita do Supervisor** (o mesmo cano do teclado humano: `lock_pty` + `enqueue_write`), FASEADA
// (`build_paste` → `submit_delay` → Enter separado), respeitando o FREIO W4-3.
//
// FIX-A3 (ALTA — red-team do gate F1-1): a re-injeção de doutrina NÃO usa mais uma mailbox de
// FILESYSTEM. A antiga `<.lina>/reinject/<nó>/` era uma DROP-ZONE gravável por QUALQUER agente do
// mesmo usuário de SO: um agente malicioso escrevia `reinject/<peer>/x.json` com `payload`
// arbitrário, o drain carimbava `from`=nome-do-subdir=peer e INJETAVA o texto arbitrário no PTY do
// peer — sem 2ª camada (router/WorkspaceTrust), ao contrário do A2A. Perms de arquivo não resolvem
// (mesmo usuário). A correção (decisão do Maestro): FILA EM-PROCESSO, sem superfície de FS. Só o
// app (`assign_role`, pós-confirmação humana) enfileira; o `target` é o `NodeId` AUTENTICADO do
// roster (não um nome re-resolvido) e o TEXTO é REGENERADO no consumidor a partir do `role` —
// nenhum texto arbitrário trafega. O FREIO W4-3 segue: pausado não drena (represa em RAM).
//
// TRADEOFF (aceito pelo Maestro): o aviso inline deixa de ser crash-durável (fila em RAM). A
// mudança de PAPEL em si segue durável (`NodeRoleAssigned` + CLAUDE.md reescrito); o restore do
// log NÃO re-deriva a re-injeção (evita re-injetar a cada boot).

/// Item da fila EM-PROCESSO de re-injeção de doutrina (FIX-A3). `target` é o `NodeId` AUTENTICADO
/// do roster, cunhado dentro do `assign_role` (que já o tem — sem round-trip por nome forjável);
/// `role` REGENERA o texto no consumidor (nenhum texto arbitrário é transportado).
#[derive(Debug, Clone)]
pub struct ReinjectItem {
    /// Id de auditoria (`reinject_<uuid v7>`) → vira o `id` do `BusMessageSent`.
    id: String,
    /// Alvo AUTENTICADO (NodeId do roster).
    target: NodeId,
    /// Papel novo — fonte da doutrina REGENERADA no consumidor.
    role: String,
}

/// Fila EM-PROCESSO de re-injeção, compartilhada (clone de `Arc`) entre o produtor
/// (`NodeManager::assign_role`) e o consumidor (`MailboxPump::drain_reinject`). Substitui a antiga
/// mailbox de filesystem (FIX-A3): zero superfície que um processo/agente externo alcance.
pub type ReinjectQueue = Arc<Mutex<VecDeque<ReinjectItem>>>;

/// Cria uma fila de re-injeção vazia (em-processo).
#[must_use]
pub(crate) fn new_reinject_queue() -> ReinjectQueue {
    Arc::new(Mutex::new(VecDeque::new()))
}

/// O texto de doutrina re-injetado no PTY vivo ao trocar de papel. Conciso (uma linha — a doutrina
/// completa vive no `CLAUDE.md` reescrito): aponta o papel novo e que o CLAUDE.md foi atualizado.
/// É DADO transportado (jamais autoridade): o que injeta é o gate humano + o app confiável.
fn doctrine_reinjection_text(role: &str) -> String {
    format!(
        "[Lina · atualização de papel] Seu papel agora é \"{role}\". A doutrina do time foi \
         atualizada no seu CLAUDE.md — aja conforme esse papel a partir de agora."
    )
}

/// Empurra uma re-injeção na fila EM-PROCESSO. `target` é o `NodeId` AUTENTICADO (já resolvido no
/// `assign_role`); `role` regenera o texto no consumidor. SEM I/O e SEM filesystem — nenhum agente
/// alcança esta fila (FIX-A3). Infalível (push em memória).
fn enqueue_doctrine_reinjection(queue: &ReinjectQueue, target: NodeId, role: &str) {
    lock(queue).push_back(ReinjectItem {
        id: format!("reinject_{}", uuid::Uuid::now_v7()),
        target,
        role: role.to_string(),
    });
}

/// **FREIO W4-3 aplicado à fila EM-PROCESSO.** Pausado → devolve `[]` SEM tocar a fila (nada se
/// injeta; a doutrina fica enfileirada em RAM até o "Retomar"). Solto → drena TODOS os itens.
/// (FIX-A3: não é mais crash-durável — tradeoff aceito; o papel em si segue durável via
/// `NodeRoleAssigned`+CLAUDE.md, e o restore NÃO re-deriva a re-injeção.)
fn drain_reinject_active(queue: &ReinjectQueue, paused: bool) -> Vec<ReinjectItem> {
    if paused {
        return Vec::new();
    }
    lock(queue).drain(..).collect()
}

/// Injeção FASEADA da doutrina no PTY vivo de `target`, pela fila serial de escrita do Supervisor
/// (1 consumidor → sem interleave de bytes; `lock_pty` segura a sequência inteira → sem keystroke
/// humano no meio). Classe de confiança do INPUT HUMANO (`Writer::Human`): NÃO é A2A agente↔agente,
/// logo NÃO passa pelo `WorkspaceTrust`/`deliver_a2a`. `build_paste` sanitiza/embrulha conforme o
/// modo bracketed-paste do alvo; Enter separado submete (W0-9).
///
/// # Errors
/// Timeout do lock lógico ou falha ao enfileirar o write (nó saiu).
fn inject_doctrine_phased(
    sup: &Supervisor,
    target: NodeId,
    grid: &Grid,
    text: &str,
    profile: &CliProfile,
) -> Result<(), String> {
    let _guard = sup
        .lock_pty(target, lina_core::Writer::Human, Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let bracketed = grid.mode().bracketed_paste;
    let paste = build_paste(text, bracketed);
    sup.enqueue_write(target, lina_core::WriteOp::HumanKeys(paste))
        .map_err(|e| e.to_string())?;
    std::thread::sleep(profile.submit_delay());
    sup.enqueue_write(target, lina_core::WriteOp::HumanKeys(vec![0x0D]))
        .map_err(|e| e.to_string())?;
    Ok(())
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
    /// Rodada 360 (ADR 0022 §4): `true` = agente em pasta própria SEM o kit de integração
    /// (sem consentimento no modal, ou kit recusado por arquivo pré-existente do usuário) —
    /// o card pinta o badge "sem doutrina/observabilidade". NUNCA silencioso. Estado de UI
    /// por sessão (não persistido; nós reconstruídos por replay nascem `false`).
    pub kit_missing: bool,
    /// FIX-1 (dogfooding 2026-06-10): `true` = este agente divide o cwd de TRABALHO (a pasta
    /// `UserDir` escolhida pelo usuário) com outro nó vivo — o caso de uso "time inteiro no
    /// projeto do usuário" (default F1-4-1). A identidade A2A é isolada pelo env (ADR 0026);
    /// o badge "vários agentes nesta pasta" só torna o compartilhamento VISÍVEL ao leigo
    /// (nunca surpresa silenciosa). Estado de UI por sessão (replay nasce `false`).
    pub cwd_shared: bool,
    /// FIX-3 (costura per-Agente, dogfooding 2026-06-10): a autonomia EFETIVA deste nó — o
    /// `LINA_AUTONOMY` carimbado no env do PTY, que o guard (`lina guard --pretooluse`) honra.
    /// O badge do card lê DAQUI (não mais o nível do Espaço): UI e enforcement nunca divergem.
    /// `new` nasce `Assisted` (default do produto); `admit_node` sobrescreve com a escolha do
    /// plano. Estado de UI por sessão (replay nasce no default).
    pub autonomy: Autonomy,
}

impl NodeView {
    pub fn new(name: impl Into<String>, kind: NodeKind, x: f32, y: f32) -> Self {
        Self {
            name: name.into(),
            kind,
            status: NodeStatus::Starting,
            x,
            y,
            kit_missing: false,
            cwd_shared: false,
            autonomy: Autonomy::Assisted,
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
    /// F1-4-3: badge honesto do restore por nó RE-ERGUIDO neste boot («Sessão retomada» /
    /// «Novo começo…», copy-f1-4 §4). Efêmero por sessão (some na 1ª interação — o render
    /// remove); nós não-restaurados não aparecem aqui.
    pub restore_badges: BTreeMap<NodeId, RestoreBadge>,
}

impl SharedModel {
    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    /// Insere um nó com nome/posição. Rodada 360 (ADR 0022): produção semeia SÓ pelo funil
    /// `admit_node` — este helper sobrevive para os testes headless montarem cenários.
    #[cfg(test)]
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
    /// A2A UNIVERSAL: profile de FALLBACK (global); o ⚡ resolve o profile do ALVO pelo `registry`.
    profile: CliProfile,
    /// A2A UNIVERSAL: registry de todos os profiles — o ⚡ demo também resolve o profile do ALVO.
    registry: Arc<ProfileRegistry>,
    store: Arc<Mutex<EventStore>>,
    model: Model,
}

impl A2aTrigger {
    // A2A UNIVERSAL: +1 arg (registry) levou de 7→8 — mesma natureza do `MailboxPump::new`.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        sup: Arc<Supervisor>,
        from: NodeId,
        to: NodeId,
        grid_to: Grid,
        profile: CliProfile,
        registry: Arc<ProfileRegistry>,
        store: Arc<Mutex<EventStore>>,
        model: Model,
    ) -> Self {
        Self {
            sup,
            from,
            to,
            grid_to,
            profile,
            registry,
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
        // A2A UNIVERSAL: o ⚡ demo também resolve o profile do ALVO; `profile` global vira o fallback.
        let registry = Arc::clone(&self.registry);
        let fallback = self.profile.clone();
        let from = self.from;
        let to = self.to;
        thread::spawn(move || {
            let env = A2aEnvelope::new(from, Recipient::Node(to), Some("a2a-demo".into()));
            let id = env.id.clone();
            let _targets = sup.route(&env, RolePolicy::All);
            // W5-5: allow-list REAL — pares confiados = membros vivos do mesmo Espaço
            // (default-deny par fora do Espaço); substitui o `InjectPolicy::AllowAll` (dead code).
            let trust = WorkspaceTrust::from_members(&live_member_ids(&sup));
            // A2A UNIVERSAL: profile_id do ALVO pela projeção (`CliProfileSet`) → profile DELE;
            // fallback ao global se ausente/desconhecido. Projeta ANTES do lock de append abaixo
            // (o guard do `project` é solto ao fim deste statement — sem double-lock).
            let to_cli = lock(&store)
                .project()
                .ok()
                .and_then(|st| st.nodes.get(&to).and_then(|n| n.cli.clone()));
            let profile = target_profile(to_cli.as_deref(), &registry, &fallback);
            match deliver_a2a(&sup, to, from, &text, profile, &grid_to, trust.policy()) {
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

/// Candidatos ao `claude-code.toml` REAL, em ordem de preferência (F1-0-2 critério 5 — a
/// produção colhe o fix de prontidão calibrado):
/// 1. `$LINA_PROFILES/claude-code.toml` (override de dev — mesmo env do modal M6);
/// 2. `<lina_home>/profiles/claude-code.toml` (o dir que o modal semeia e o USUÁRIO edita —
///    coerência com o critério 7 da F1-2-2: o TOML do usuário governa);
/// 3. `profiles/claude-code.toml` ao lado do EXECUTÁVEL e ancestrais (bundle .app
///    auto-contido — ANTES do caminho do repo, p/ o bundle valer fora da máquina de dev);
/// 4. `<repo>/profiles/claude-code.toml` (dev, via `CARGO_MANIFEST_DIR`).
fn injection_profile_candidates(lina_home: Option<&Path>) -> Vec<PathBuf> {
    const REL: &str = "claude-code.toml";
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("LINA_PROFILES") {
        out.push(PathBuf::from(dir).join(REL));
    }
    if let Some(home) = lina_home {
        out.push(home.join("profiles").join(REL));
    }
    if let Ok(exe) = std::env::current_exe() {
        out.extend(
            exe.ancestors()
                .skip(1) // o próprio arquivo do exe não tem `profiles/`
                .map(|a| a.join("profiles").join(REL)),
        );
    }
    if let Some(repo) = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
    {
        out.push(repo.join("profiles").join(REL));
    }
    out
}

/// Resolve o 1º candidato que EXISTE e PARSEIA. TOML inválido ganha warning legível e cai
/// para o próximo (nunca panic). Puro sobre a lista — testável.
fn resolve_profile(candidates: &[PathBuf]) -> Option<(CliProfile, PathBuf)> {
    candidates.iter().find_map(|p| {
        if !p.is_file() {
            return None;
        }
        match CliProfile::load_file(p) {
            Ok(prof) => Some((prof, p.clone())),
            Err(e) => {
                eprintln!(
                    "lina-gpui: AVISO — profile inválido em {} ({e}); tentando o próximo",
                    p.display()
                );
                None
            }
        }
    })
}

/// Carrega o perfil sobre `candidates`, com **fallback-com-warning** para o demo embutido
/// (nunca panic — inv #6). Loga o perfil carregado NO BOOT (caminho + os campos do fix
/// F1-0-2) para inspeção. Separado de [`load_injection_profile`] p/ o fallback ser testável.
fn injection_profile_from(candidates: &[PathBuf]) -> CliProfile {
    match resolve_profile(candidates) {
        Some((prof, path)) => {
            eprintln!(
                "lina-gpui: CLI Profile de injeção CARREGADO: id={} de {} · ready_timeout_ms={:?} · busy_markers={:?} · submit_delay_ms={}",
                prof.id,
                path.display(),
                prof.ready_timeout_ms,
                prof.busy_markers,
                prof.submit_delay_ms,
            );
            prof
        }
        None => {
            eprintln!(
                "lina-gpui: AVISO — nenhum profiles/claude-code.toml carregável ({} candidato(s) tentados);                  usando o perfil DEMO embutido: timings genéricos, SEM o fix de prontidão calibrado da F1-0-2",
                candidates.len()
            );
            demo_profile()
        }
    }
}

/// **F1-0-2 critério 5 — o app de PRODUÇÃO usa o CLI Profile real** (`ready_timeout_ms`/
/// `busy_markers` calibrados), não mais o `demo_profile()` hardcoded. Resolução em
/// [`injection_profile_candidates`]; fallback-com-warning em [`injection_profile_from`].
#[must_use]
pub fn load_injection_profile(lina_home: Option<&Path>) -> CliProfile {
    injection_profile_from(&injection_profile_candidates(lina_home))
}

/// **A2A UNIVERSAL.** Resolve o CLI Profile do **ALVO** para a entrega faseada. Cada nó guarda
/// seu `profile_id` no campo `cli` da projeção do log (`CliProfileSet`, event-sourced — inv #4);
/// esse id é procurado no `registry` de TODOS os profiles carregados. Fallback ao `fallback` (o
/// profile global de injeção) quando o nó não tem profile_id (`None` — shell puro / log antigo) OU
/// o id é desconhecido no registry: a entrega **NUNCA** falha por falta de profile (degrada para
/// timings genéricos, jamais para "não entrega").
///
/// O bug que isto corrige: a entrega usava UM profile global (`claude-code`) para TODO alvo, então
/// o `prompt_ready_regex`/`busy_markers` do Claude eram aplicados à TUI do **Codex** (glifo `›`,
/// não `>`/`❯`) — que nunca casava → o alvo "nunca ficava pronto" → timeout, mensagem não injetada.
///
/// **Fronteira de segurança:** o profile decide só *COMO* injetar (prontidão/timing). *QUEM* pode
/// injetar em *QUEM* continua sendo decidido pela allow-list da [`WorkspaceTrust`] (intocada) —
/// trocar o profile do alvo não abre nenhuma porta de autorização (doutrina §segurança do CLAUDE.md).
fn target_profile<'a>(
    target_cli_id: Option<&str>,
    registry: &'a ProfileRegistry,
    fallback: &'a CliProfile,
) -> &'a CliProfile {
    target_cli_id
        .and_then(|id| registry.get(id))
        .unwrap_or(fallback)
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
    /// A2A UNIVERSAL: profile de injeção de FALLBACK (o global, ex.: claude-code) — usado só quando
    /// o alvo não tem profile_id ou seu id não está no `registry`. A entrega resolve o profile do
    /// ALVO pelo `registry`; este é a rede de segurança que garante "nunca falhar por falta de profile".
    profile: CliProfile,
    /// A2A UNIVERSAL: registry de TODOS os CLI Profiles (claude-code, codex, gemini, antigravity, …).
    /// A entrega resolve o profile do ALVO pelo `profile_id` do nó (projeção `CliProfileSet`) aqui —
    /// fim do profile global único que aplicava o `prompt_ready` do Claude à TUI do Codex (→ timeout).
    registry: Arc<ProfileRegistry>,
    model: Model,
    /// W4-3: estado do FREIO compartilhado com a UI (que pede pausa/retoma; a pump aplica no Router).
    brake: crate::wiring::Brake,
    /// FIX-A3: fila EM-PROCESSO de RE-INJEÇÃO de doutrina (substitui a mailbox de FS, que era
    /// drop-zone gravável por qualquer agente). O `assign_role` enfileira; a pump (escritor único)
    /// drena+injeta no PTY vivo SÓ com o freio solto (pausado = represa em RAM).
    reinject_queue: ReinjectQueue,
    /// Último roster escrito no `agents.json` (evita reescrever a cada tick sem mudança).
    last_roster: Vec<AgentPresence>,
    /// W3-7c: último `event_count` em que reconstruímos o `CostLedger` — só re-replaya quando há eventos
    /// novos (evita replay por tick quando nada mudou).
    last_cost_ec: u64,
    /// SEAM-1: o funil de admissão (ADR 0022) — `SpawnApproved` chama `nodes.execute_spawn` aqui.
    /// `Arc` compartilhado com o gpui View (escritor único de nós; `admit_node` é gpui-free e usa só
    /// estado `Arc<Mutex>`, como o `deliver_fn` já muta o model cross-thread).
    nodes: Arc<NodeManager>,
    /// SEAM-1: spawns cujo binding de cascata (M4) + 1º prompt o pump JÁ fiou nesta sessão (keyed em
    /// `spawn_id`). Em memória — no restart, re-fiar é idempotente (seed via P1; 1º prompt com id
    /// determinístico → dedupe durável do ledger).
    seeded_spawns: std::collections::HashSet<String>,
    /// SEAM-1: `event_count` do último `post_process_spawns` — só re-varre o log quando há evento
    /// novo (padrão do `last_cost_ec`; evita ler o log a cada tick sem mudança).
    last_spawn_ec: u64,
}

/// SEAM-1: detalhes INFORJÁVEIS de um pedido de spawn (do `SpawnRequested`), usados por
/// `post_process_spawns` p/ fiar o binding de cascata (M4) + o 1º prompt.
struct SpawnReq {
    root: String,
    hops: u8,
    prompt: String,
    requested_by: Option<NodeId>,
}

/// SEAM-1 (M2): tradução `lina_bootstrap::Autonomy` → `lina_core::AutonomyLevel`. O core NÃO depende
/// do bootstrap (o app traduz, por design — `router.rs` deixa explícito). Mapeamento 1:1.
fn autonomy_to_level(a: lina_bootstrap::Autonomy) -> lina_core::AutonomyLevel {
    match a {
        lina_bootstrap::Autonomy::Manual => lina_core::AutonomyLevel::Manual,
        lina_bootstrap::Autonomy::Assisted => lina_core::AutonomyLevel::Assisted,
        lina_bootstrap::Autonomy::Autonomous => lina_core::AutonomyLevel::Autonomous,
    }
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
        // FIX-A3: a fila EM-PROCESSO de re-injeção de doutrina — compartilhada (clone de Arc) com o
        // `NodeManager` (produtor via `assign_role`); esta pump drena (escritor único) sob o freio.
        reinject_queue: ReinjectQueue,
        model: Model,
        brake: crate::wiring::Brake,
        token_budget_day: u64,
        // F1-0-2 critério 5: o perfil de INJEÇÃO vem de fora (produção = `load_injection_profile`,
        // o claude-code.toml real; testes = demo/fixture) — fim do `demo_profile()` hardcoded.
        // A2A UNIVERSAL: agora é o FALLBACK; o profile do ALVO vem do `registry` (abaixo).
        profile: CliProfile,
        // A2A UNIVERSAL: registry de TODOS os profiles — a entrega resolve o do ALVO por aqui.
        registry: Arc<ProfileRegistry>,
        // SEAM-1 (M2): a autonomia REAL do workspace, FIADA no `RouterConfig` (mata o `..default()`→
        // Assisted que deixava o ramo manual/cascata do gate de spawn FAKE em produção).
        autonomy: lina_bootstrap::Autonomy,
        // SEAM-1: o funil de admissão (ADR 0022) — `SpawnApproved` cria o terminal por aqui.
        nodes: Arc<NodeManager>,
    ) -> Self {
        // W3-7c (TETO DE CUSTO): `token_budget_day > 0` ARMA o teto no Router (0 = desligado). O
        // `CostLedger` soma os `TokenUsageReported` (emitidos pela bomba) e PAUSA na transição.
        // SEAM-1 (M2): `autonomy` REAL fiada — `Manual` agora bloqueia spawn/delegação DE FATO.
        let config = RouterConfig {
            token_budget_day,
            autonomy: autonomy_to_level(autonomy),
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
            profile,
            registry,
            model,
            brake,
            reinject_queue,
            last_roster: Vec::new(),
            last_cost_ec: 0,
            nodes,
            seeded_spawns: std::collections::HashSet::new(),
            last_spawn_ec: 0,
        }
    }

    /// **SEAM-1 — fia o binding de cascata (M4) + entrega o 1º prompt de spawns recém-admitidos.**
    /// Observa `SpawnAdmitted{id, node}` no log (origem admitida pelo pump OU banner-aprovado na
    /// thread de UI) e, para cada um ainda não fiado nesta sessão:
    /// - **M4:** `router.seed_delivered_root(node, root, hops)` (do `SpawnRequested{id}` — INFORJÁVEL)
    ///   → o filho NASCE com cadeia → seu `lina spawn` é CASCATA (anti-fork-bomb end-to-end);
    /// - **1º prompt:** enfileira (do solicitante p/ o filho) com id DETERMINÍSTICO — entregue com
    ///   segurança no próximo tick (retenção ACHADO-2 se o CLI do filho ainda não estiver pronto);
    ///   o id determinístico + o ledger durável evitam 1º-prompt duplicado num restart.
    ///
    /// Só re-varre o log quando o `event_count` muda (gate `last_spawn_ec`). O `seeded_spawns`
    /// em-memória evita re-trabalho no mesmo processo; o seed é idempotente (P1) e o 1º prompt é
    /// deduplicado pelo ledger — re-fiar pós-restart é seguro.
    fn post_process_spawns(&mut self, now_ms: u64) {
        let ec = lock(&self.store).event_count().unwrap_or(0);
        if ec == self.last_spawn_ec {
            return; // sem evento novo → nada a fiar
        }
        self.last_spawn_ec = ec;
        let recs = match lock(&self.store).events() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("lina-gpui: post_process_spawns não leu o log (re-tenta): {e}");
                return;
            }
        };
        // Indexa os pedidos por id (a fonte INFORJÁVEL de root/hops/prompt/solicitante).
        let mut req: std::collections::HashMap<String, SpawnReq> = std::collections::HashMap::new();
        for r in &recs {
            if r.kind == "SpawnRequested" {
                if let (Some(id), Some(root), Some(prompt)) = (
                    r.payload.get("id").and_then(serde_json::Value::as_str),
                    r.payload
                        .get("root_cause_id")
                        .and_then(serde_json::Value::as_str),
                    r.payload.get("prompt").and_then(serde_json::Value::as_str),
                ) {
                    let hops = r
                        .payload
                        .get("hops")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u8;
                    let requested_by = r
                        .payload
                        .get("requested_by")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|s| s.parse().ok());
                    req.insert(
                        id.to_string(),
                        SpawnReq {
                            root: root.to_string(),
                            hops,
                            prompt: prompt.to_string(),
                            requested_by,
                        },
                    );
                }
            }
        }
        for r in &recs {
            if r.kind != "SpawnAdmitted" {
                continue;
            }
            let Some(id) = r.payload.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if self.seeded_spawns.contains(id) {
                continue;
            }
            let Some(node): Option<NodeId> = r
                .payload
                .get("node")
                .and_then(serde_json::Value::as_str)
                .and_then(|s| s.parse().ok())
            else {
                continue;
            };
            let Some(info) = req.get(id) else {
                // SpawnAdmitted sem o SpawnRequested correspondente (log truncado?) — não fia às
                // cegas; marca como visto p/ não re-tentar em loop.
                self.seeded_spawns.insert(id.to_string());
                continue;
            };
            // M4: binding de cascata (root/hops EFETIVOS do gate — inforjáveis).
            self.router
                .seed_delivered_root(node, info.root.clone(), info.hops, now_ms);
            // 1º prompt: do solicitante p/ o filho, id determinístico (dedupe durável no ledger).
            if let (Some(spawner), Some(child_name)) = (
                info.requested_by.and_then(|rb| self.sup.get(rb)),
                self.sup.get(node),
            ) {
                let prompt_id = format!("msg_spawn1st-{}", id.strip_prefix("msg_").unwrap_or(id));
                let mut msg = MailMessage::new(
                    spawner.name.clone(),
                    child_name.name.clone(),
                    "ask",
                    info.prompt.clone(),
                );
                msg.id = prompt_id;
                if let Err(e) = self.router.mailbox().enqueue_as(&spawner.name, &msg) {
                    eprintln!("lina-gpui: 1º prompt do spawn {id} não enfileirado: {e}");
                }
            }
            self.seeded_spawns.insert(id.to_string());
        }
    }

    /// **A2A UNIVERSAL.** Projeta o mapa `node → profile_id` do event log (campo `cli` de cada nó,
    /// vindo de `CliProfileSet`). Chamado UMA vez por tick, **ANTES** de travar o `store` para o
    /// `pump`/`resume` — o `deliver_fn` roda DENTRO desse lock, então projetar lá dentro re-travaria
    /// o mesmo `Mutex` (deadlock). Best-effort: falha de projeção → mapa vazio → todo alvo cai no
    /// fallback (degrada, nunca trava).
    fn project_cli_by_node(&self) -> BTreeMap<NodeId, String> {
        match lock(&self.store).project() {
            Ok(state) => state
                .nodes
                .into_iter()
                .filter_map(|(id, n)| n.cli.map(|cli| (id, cli)))
                .collect(),
            Err(e) => {
                eprintln!(
                    "lina-gpui: A2A — projeção node→profile falhou ({e}); usando fallback global"
                );
                BTreeMap::new()
            }
        }
    }

    /// Constrói a closure de ENTREGA faseada (reusada por `pump` e `resume`). Captura clones `Arc`
    /// (own → `'static`). **W4-3:** a entrega REAL acende o PULSO efêmero `from→target` no model — a
    /// metáfora "sem fios" dirigida por evento REAL (`lina ask`), não pelo `A2aTrigger` demo.
    fn deliver_fn(
        &self,
        cli_by_node: &BTreeMap<NodeId, String>,
    ) -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
        let sup = Arc::clone(&self.sup);
        let grids = Arc::clone(&self.grids);
        // A2A UNIVERSAL: registry de todos os profiles + fallback global + o mapa node→profile_id
        // (projetado ANTES do lock do store por `project_cli_by_node` — evita re-travar o `Mutex`
        // dentro do `pump`/`resume`, que daria deadlock). A resolução per-alvo é abaixo.
        let registry = Arc::clone(&self.registry);
        let fallback = self.profile.clone();
        let cli_by_node = cli_by_node.clone();
        let model = Arc::clone(&self.model);
        move |target, from, text| {
            let Some(g) = lock(&grids).get(&target).cloned() else {
                return Err(format!("sem grid para o alvo {target}"));
            };
            // W5-5: allow-list REAL derivada da topologia viva a cada entrega (o roster muda);
            // default-deny par fora do Espaço. Substitui o `InjectPolicy::AllowAll` (dead code).
            let trust = WorkspaceTrust::from_members(&live_member_ids(&sup));
            // A2A UNIVERSAL: o profile do ALVO (o `prompt_ready_regex`/`busy_markers` DELE), não
            // mais o global — destrava a entrega ao Codex/Gemini/Antigravity sem timeout. A
            // autorização (QUEM→QUEM) segue na `WorkspaceTrust` acima; o profile só decide o COMO.
            let profile = target_profile(
                cli_by_node.get(&target).map(String::as_str),
                &registry,
                &fallback,
            );
            let out = deliver_a2a(&sup, target, from, text, profile, &g, trust.policy())
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
        // A2A UNIVERSAL: projeta `node → profile_id` ANTES de qualquer lock do `store` — o
        // `deliver_fn` roda DENTRO do lock do `pump`/`resume`, então projetar lá dentro re-travaria
        // o mesmo `Mutex` (deadlock, pois `std::sync::Mutex` não é reentrante). O mapa é capturado
        // (clonado) por cada `deliver_fn` e resolve o profile do alvo sem mais I/O no caminho quente.
        let cli_by_node = self.project_cli_by_node();
        // W4-3 FREIO: a UI pediu para alternar? → pausa OU retoma+DRENA a auto-orquestração no Router.
        let toggle = std::mem::take(&mut lock(&self.brake).toggle_requested);
        if toggle {
            {
                let mut store = lock(&self.store);
                if self.router.is_paused() {
                    let mut deliver = self.deliver_fn(&cli_by_node);
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

        let mut deliver = self.deliver_fn(&cli_by_node);
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
                // F1-0-4/ACHADO-2: alvo ocupado/retry é ESPERADO (não-erro) — sem ruído de "não roteada".
                RouteOutcome::Retained { .. } | RouteOutcome::RetryBackoff { .. } => {}
                // SEAM-1: spawn APROVADO (origem) → cria o terminal pelo FUNIL ÚNICO (idempotente M3).
                // O binding de cascata (M4) + 1º prompt são fiados em `post_process_spawns` (abaixo),
                // ao OBSERVAR o `SpawnAdmitted` que `execute_spawn` apenda — desacopla da thread.
                RouteOutcome::SpawnApproved {
                    name,
                    role,
                    requested_by,
                    root_cause_id,
                    ..
                } => {
                    match self
                        .nodes
                        .execute_spawn(root_cause_id, name, role, *requested_by)
                    {
                        Ok(child) => eprintln!(
                            "lina-gpui: spawn aprovado — terminal '{name}' ({role}) NASCEU: {child}"
                        ),
                        Err(e) => eprintln!("lina-gpui: spawn '{name}' falhou ao admitir: {e}"),
                    }
                    routed = true;
                }
                // SEAM-1: CASCATA já está no log (`SpawnGated{cascade}`) → a fila de atenção projeta o
                // banner (gate humano). Bloqueado (manual) é recusa terminal. Sem ruído de "não roteada".
                RouteOutcome::SpawnGated { .. } | RouteOutcome::SpawnBlocked => {}
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
        // SEAM-1: fia o binding de cascata (M4) + o 1º prompt de spawns recém-admitidos (origem aqui
        // OU banner-aprovado na thread de UI) — pelo router/mailbox, dos quais o pump é dono único.
        self.post_process_spawns(now_ms());
        // F1-2-4: entrega as re-injeções de doutrina represadas (respeita o freio: pausado = não drena).
        self.drain_reinject();
        self.refresh_cost_paused();
    }

    /// **F1-2-4 / FIX-A3 — drena+injeta as re-injeções de doutrina represadas, respeitando o FREIO
    /// W4-3.** Pausado → não toca a fila (`drain_reinject_active` devolve `[]`): a doutrina fica
    /// enfileirada em RAM até o "Retomar" (nada se injeta sob pausa). Solto → para cada item: o alvo
    /// é o `NodeId` AUTENTICADO cunhado no `assign_role` (FIX-A3 — nenhum campo forjável decide
    /// identidade), o TEXTO é REGENERADO de `role` no consumidor (jamais um payload transportado),
    /// injeta faseado pela fila serial de escrita e LOGA `BusMessageSent` (log-verificável: a doutrina
    /// só aparece no log DEPOIS do `OrchestrationResumed`). Removido da fila ao injetar com sucesso
    /// (idempotente); nó sem grid (subindo) ou falha de entrega → RE-ENFILEIRA p/ o próximo tick
    /// (nunca silenciosa). TRADEOFF FIX-A3: fila em RAM não é crash-durável — a mudança de papel em si
    /// segue durável (`NodeRoleAssigned`+CLAUDE.md) e o restore NÃO re-deriva a re-injeção.
    fn drain_reinject(&mut self) {
        let items = drain_reinject_active(&self.reinject_queue, self.router.is_paused());
        let mut retry: Vec<ReinjectItem> = Vec::new();
        for item in items {
            // O alvo é o `NodeId` AUTENTICADO cunhado no `assign_role` — nenhum campo forjável
            // decide identidade. Se o nó saiu do roster vivo, descarta (não há onde injetar).
            let Some(info) = self.sup.get(item.target) else {
                eprintln!(
                    "lina-gpui: reinject — alvo {:?} fora do roster vivo; descarta",
                    item.target
                );
                continue;
            };
            let Some(grid) = lock(&self.grids).get(&item.target).cloned() else {
                // Sem grid (nó ainda subindo): re-enfileira p/ o próximo tick (nada se perde).
                retry.push(item);
                continue;
            };
            // TEXTO REGENERADO no consumidor a partir do papel — jamais um payload transportado.
            let text = doctrine_reinjection_text(&item.role);
            match inject_doctrine_phased(&self.sup, item.target, &grid, &text, &self.profile) {
                Ok(()) => {
                    let count = {
                        let mut s = lock(&self.store);
                        let _ = s.append(&DomainEvent::BusMessageSent {
                            id: item.id.clone(),
                            from: item.target,
                            to: format!("doctrine:{}", info.name),
                        });
                        s.event_count().ok()
                    };
                    let mut model = lock(&self.model);
                    if let Some(c) = count {
                        model.event_count = c;
                    }
                    model.touch();
                }
                Err(e) => {
                    // Falha de entrega (lock/IO): re-enfileira p/ retry no próximo tick (nunca silenciosa).
                    eprintln!(
                        "lina-gpui: reinject de doutrina falhou (retry no próximo tick): {e}"
                    );
                    retry.push(item);
                }
            }
        }
        // Re-enfileira NA FRENTE (preserva a ordem relativa) os itens que pediram retry.
        if !retry.is_empty() {
            let mut q = lock(&self.reinject_queue);
            for item in retry.into_iter().rev() {
                q.push_front(item);
            }
        }
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

// ════════════════════ F1-1-7 · fiação da FILA DE ATENÇÃO (AttentionQueue ↔ app) ════════════════════
//
// A projeção `lina_core::AttentionQueue` (F1-1-7 core) é a FONTE ÚNICA de ordenação
// (custódia > permissão, round-robin por nó) e de escalação (≥5 min). Aqui mora só a
// fiação: replay do log no boot e a cada evento novo (padrão `CostLedger` — o
// `from_record` é interno ao core), espelho da custódia VIVA do [`CustodyDesk`] (com
// `first_seen` estável por id — a escalação não reseta entre re-replays), e os comandos
// do gesto humano (resolve/dismiss/mute) que apendam o evento devolvido pelo core e
// re-alimentam o fold. **NENHUM byte vai ao PTY por este caminho** (ADR 0021 §6 /
// AC-0021.6): o hub não tem handle de PTY nem de `InputSink` — o write é F1-1-8.

/// F1-1-8 / ADR 0024 — **a porta única de escrita validada, pelo caminho do app.** Implementa
/// `lina_core::ApprovalPort` (a trait que o `ApprovalExecutor` aciona) traduzindo o `node_id`
/// (que, na detecção do app, é o NOME canônico do roster — bridge.rs `AppPermissionWatch::scan`)
/// para o `NodeId` via a AUTORIDADE do roster (`node_by_name` — nunca por texto do grid), achando
/// o grid daquele nó e delegando ao `Supervisor::deliver_approval_with_grid` (check de tela + write
/// atômicos, ordem grid→writer). `approval.rs` permanece INALTERADO.
///
/// **Defesa em profundidade:** se `node_by_name` resolvesse o nó errado (homônimo), o
/// `check_screen` do core abortaria (o hash da tela não casa) — zero bytes, fail-safe.
struct SupervisorApprovalPort<'a> {
    sup: &'a Supervisor,
    grids: &'a Mutex<BTreeMap<NodeId, Grid>>,
}

impl ApprovalPort for SupervisorApprovalPort<'_> {
    fn deliver(
        &mut self,
        node_id: &str,
        expected_hash: &str,
        keys: &[u8],
    ) -> Result<PortOutcome, PortError> {
        // Identidade re-derivada pela autoridade única (roster), não pelo texto do pedido.
        let node = self
            .sup
            .node_by_name(node_id)
            .ok_or_else(|| PortError::NotFound(node_id.to_string()))?;
        let grid = lock(self.grids)
            .get(&node)
            .cloned()
            .ok_or_else(|| PortError::NotFound(node_id.to_string()))?;
        match self.sup.deliver_approval_with_grid(
            node,
            &grid,
            expected_hash,
            keys,
            PROMPT_REGION_ROWS,
        ) {
            Ok(outcome) => Ok(outcome),
            Err(SupervisorError::NodeNotFound(_)) => Err(PortError::NotFound(node_id.to_string())),
            // Erro de I/O no write: efeito NÃO confirmado → o executor audita `port_error`.
            Err(e) => Err(PortError::Io(e.to_string())),
        }
    }
}

/// F1-1-8 / ADR 0024 — o que o hub precisa para DIGITAR a aprovação (a fila F1-1-7 não tinha
/// porta; este é o ganho novo da F1-1-8). `keys` é o `approval_keys` do CLI Profile do app.
pub struct ApprovalWiring {
    sup: Arc<Supervisor>,
    grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    keys: ApprovalKeys,
}

impl ApprovalWiring {
    #[must_use]
    pub fn new(
        sup: Arc<Supervisor>,
        grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
        keys: ApprovalKeys,
    ) -> Self {
        Self { sup, grids, keys }
    }
}

/// Fiação F1-1-7/F1-1-8: `AttentionQueue` do core cabeada ao log + à mesa de custódia, e a porta
/// de escrita validada (F1-1-8 / ADR 0024 — o gesto humano e o auto-deny do SLA digitam por aqui).
pub struct AttentionHub {
    queue: Mutex<lina_core::AttentionQueue>,
    store: Arc<Mutex<EventStore>>,
    desk: Desk,
    model: Model,
    /// `event_count` do último replay (re-replay SÓ com evento novo — padrão CostLedger).
    last_ec: Mutex<u64>,
    /// Espelho da custódia: id → (nó autenticado, display, first_seen ms). O first_seen
    /// é cunhado no 1º sync que viu o id e SOBREVIVE a re-replays (escalação estável).
    custody_seen: Mutex<BTreeMap<String, (String, String, u64)>>,
    /// F1-1-8 (ADR 0024): a porta de escrita validada. O executor é reconstruído do LOG a cada
    /// gesto/auto-deny (binding `node_of` + dedup vêm do log, JAMAIS de campo da UI/fila).
    approval: ApprovalWiring,
    /// SEAM-1: o funil de admissão (ADR 0022). APROVAR um banner de spawn → `nodes.execute_spawn`
    /// (cria o terminal, idempotente M3); a `MailboxPump` fia o binding + 1º prompt ao observar o
    /// `SpawnAdmitted`. `Arc` compartilhado com o gpui View/pump.
    nodes: Arc<NodeManager>,
}

impl AttentionHub {
    /// Cria o hub reconstruindo a fila do LOG (crash com pendência → reabrir reconstrói
    /// — critério 5 da story; o replay é a mesma rota do live: fold único do core).
    #[must_use]
    pub fn new(
        store: Arc<Mutex<EventStore>>,
        desk: Desk,
        model: Model,
        approval: ApprovalWiring,
        // SEAM-1: o funil de admissão — aprovar um banner de spawn cria o terminal por aqui.
        nodes: Arc<NodeManager>,
    ) -> Self {
        let (queue, ec) = {
            let s = lock(&store);
            let records = s.events().unwrap_or_default();
            (
                lina_core::AttentionQueue::replay(&records),
                s.event_count().unwrap_or(0),
            )
        };
        Self {
            queue: Mutex::new(queue),
            store,
            desk,
            model,
            last_ec: Mutex::new(ec),
            custody_seen: Mutex::new(BTreeMap::new()),
            approval,
            nodes,
        }
    }

    /// **SEAM-1 — decisão humana sobre um BANNER de spawn (cascata).** `approve` → cria o terminal
    /// pelo funil único (`execute_spawn`, idempotente M3); a `MailboxPump` fia o binding de cascata
    /// (M4) + 1º prompt ao observar o `SpawnAdmitted`. `!approve` → apenda `SpawnDeclined` (nada
    /// nasce). `false` se `id` não é um banner de spawn pendente (idempotência: 2º clique é no-op).
    /// O nome/papel/solicitante vêm do `SpawnRequested{id}` no LOG (INFORJÁVEL — nunca campo da UI).
    pub fn resolve_spawn(&self, id: &str, approve: bool) -> bool {
        // Só age sobre um banner de spawn PENDENTE (a fila é a autoridade do que está aberto).
        if !lock(&self.queue).is_pending_spawn(id) {
            return false;
        }
        if !approve {
            if let Err(e) = lock(&self.store).append(&DomainEvent::SpawnDeclined { id: id.into() })
            {
                eprintln!("lina-gpui: SpawnDeclined({id}) não logado: {e}");
                return false;
            }
            return true;
        }
        // APROVA: re-lê o pedido TIPADO do log (name/role/requested_by inforjáveis) e admite.
        let Some((name, role, requested_by)) = self.spawn_request_of(id) else {
            eprintln!(
                "lina-gpui: banner de spawn {id} sem SpawnRequested no log — não admito às cegas"
            );
            return false;
        };
        match self.nodes.execute_spawn(id, &name, &role, requested_by) {
            Ok(child) => {
                eprintln!(
                    "lina-gpui: banner aprovado — terminal '{name}' ({role}) NASCEU: {child}"
                );
                true
            }
            Err(e) => {
                eprintln!("lina-gpui: banner de spawn '{name}' falhou ao admitir: {e}");
                false
            }
        }
    }

    /// SEAM-1: `(name, role, requested_by)` do `SpawnRequested{id}` no log (último vence). INFORJÁVEL
    /// — a identidade do solicitante é o `requested_by` carimbado pelo gate, nunca um campo da UI.
    fn spawn_request_of(&self, id: &str) -> Option<(String, String, NodeId)> {
        let recs = lock(&self.store).events().ok()?;
        recs.iter().rev().find_map(|r| {
            if r.kind != "SpawnRequested"
                || r.payload.get("id").and_then(serde_json::Value::as_str) != Some(id)
            {
                return None;
            }
            let name = r.payload.get("name").and_then(serde_json::Value::as_str)?;
            let role = r.payload.get("role").and_then(serde_json::Value::as_str)?;
            let rb = r
                .payload
                .get("requested_by")
                .and_then(serde_json::Value::as_str)
                .and_then(|s| s.parse().ok())?;
            Some((name.to_string(), role.to_string(), rb))
        })
    }

    /// Sincroniza e devolve a fila ORDENADA para exibição: (1) evento novo no log →
    /// re-replay + re-espelho da custódia conhecida; (2) diff da custódia VIVA do desk
    /// (novos ids entram com `first_seen = now`; resolvidos pelo ⌘⏎ saem). Chamado pelo
    /// heartbeat da view (~4 Hz) — re-replay só quando o `event_count` mudou.
    pub fn sync(&self, now_ms: u64) -> Vec<lina_core::AttentionItem> {
        let ec = lock(&self.store).event_count().unwrap_or(0);
        let stale = {
            let mut last = lock(&self.last_ec);
            let stale = *last != ec;
            *last = ec;
            stale
        };
        if stale {
            let records = lock(&self.store).events().unwrap_or_default();
            let mut q = lina_core::AttentionQueue::replay(&records);
            for (id, (node, display, ts)) in lock(&self.custody_seen).iter() {
                q.custody_enqueued(id.clone(), node.clone(), display.clone(), *ts);
            }
            *lock(&self.queue) = q;
        }
        // Custódia viva (origem AUTENTICADA do drain — `requester()`, nunca payload).
        let live: Vec<(String, String, String)> = lock(&self.desk)
            .queue
            .iter()
            .map(|p| {
                (
                    p.id().to_string(),
                    p.requester().to_string(),
                    p.display().to_string(),
                )
            })
            .collect();
        let mut seen = lock(&self.custody_seen);
        let mut q = lock(&self.queue);
        for (id, node, display) in &live {
            let (n, d, ts) = seen
                .entry(id.clone())
                .or_insert_with(|| (node.clone(), display.clone(), now_ms));
            q.custody_enqueued(id.clone(), n.clone(), d.clone(), *ts);
        }
        let gone: Vec<String> = seen
            .keys()
            .filter(|k| !live.iter().any(|(id, _, _)| id == *k))
            .cloned()
            .collect();
        for id in gone {
            seen.remove(&id);
            q.custody_resolved(&id);
        }
        q.items(now_ms)
    }

    /// Decisão humana sobre uma PERMISSÃO (clique do toast/fila). `false` = id
    /// desconhecido/já resolvido (clique duplo é no-op sem 2º evento — idempotência da fila),
    /// item de CUSTÓDIA (decide pelo ⌘⏎ existente — zero regressão), ou `prompt_kind` não-`Yn`
    /// (`Choice`/`Trust` alertam+focam, não aprovam — gate do core).
    ///
    /// **F1-1-8 / ADR 0024 (esta rodada):** o gesto agora DIGITA pela porta única. Branch por
    /// propriedade-do-item, NÃO por wiring:
    /// - **com Captura 1** (`vt_snapshot_hash` do `PermissionAsked` no LOG): dirige o
    ///   [`ApprovalExecutor`] → `PermissionResolved` → write validado contra a tela
    ///   (`Injected`) ou `ApprovalAborted` (tela mudou/erro);
    /// - **sem Captura 1** (origem hook-only — sem baseline de tela): fail-safe (ADR 0021 §1) —
    ///   registra a decisão (audit), JAMAIS digita às cegas; o humano vai ao terminal.
    ///
    /// INVARIANTE: `node_id` (binding) e `expected_hash` (Captura 1) vêm do LOG
    /// (`ApprovalLedger.node_of` / projeção `AttentionQueue`), JAMAIS de campo da UI — o clique
    /// só carrega o `stable_id` (uma referência, não autoridade).
    pub fn resolve(
        &self,
        stable_id: &str,
        decision: lina_core::ApprovalDecision,
        via: lina_core::ResolutionVia,
    ) -> bool {
        // 1) Gate da FILA (comando PURO — não muta): item existe + é `Yn` + dá o `stable_id`
        //    CANÔNICO (resolve aliases do merge entre camadas). Custódia/desconhecido/já-saiu/
        //    não-`Yn` ⇒ `None` ⇒ no-op (idempotência do clique duplo intacta).
        let Some(resolved_ev) = lock(&self.queue).resolve(stable_id, decision, via) else {
            return false;
        };
        let DomainEvent::PermissionResolved {
            stable_id: canonical,
            ..
        } = &resolved_ev
        else {
            return false; // `resolve` só devolve PermissionResolved (defensivo)
        };
        let canonical = canonical.clone();

        // 2) Captura 1 do item — do LOG (projeção da fila), nunca campo escrito pela UI.
        let expected_hash = lock(&self.queue)
            .items(now_ms())
            .into_iter()
            .find(|i| i.stable_id == canonical)
            .and_then(|i| i.vt_snapshot_hash);

        match expected_hash {
            // 3a) Com Captura 1 → porta única (write validado contra a tela).
            Some(hash) => self.deliver_via_executor(&canonical, decision, via, &hash),
            // 3b) Sem Captura 1 (hook-only) → fail-safe: registra a decisão, ZERO bytes.
            None => self.commit(resolved_ev, &canonical, "PermissionResolved"),
        }
    }

    /// Reconstrói o [`ApprovalExecutor`] do LOG (o binding `node_of` + a dedup do executor são
    /// projeções PURAS do log — reconstruí-las jamais escreve; a porta entra só no `deliver`).
    /// Rota rara (gesto humano / tick de auto-deny), então o replay-por-chamada é barato.
    fn rebuilt_executor(&self) -> ApprovalExecutor {
        let records = lock(&self.store).events().unwrap_or_default();
        ApprovalExecutor::replay(&records)
    }

    /// Drena UM gesto (humano ou auto-deny) pela porta única do executor: binding e dedup do
    /// LOG, write atômico via [`SupervisorApprovalPort`], e apenda o desfecho (Resolved → write →
    /// Injected/Aborted) no log + re-alimenta o fold. `false` só se o pedido não tem binding no
    /// log (não deveria — veio do log; fail-safe sem escrever).
    fn deliver_via_executor(
        &self,
        canonical: &str,
        decision: lina_core::ApprovalDecision,
        via: lina_core::ResolutionVia,
        expected_hash: &str,
    ) -> bool {
        let mut exec = self.rebuilt_executor();
        let Some(target) = exec.ledger().node_of(canonical).map(str::to_string) else {
            eprintln!("lina-gpui: [ATT] resolve sem binding no log p/ {canonical:?} — abortado (fail-safe)");
            return false;
        };
        let gesture = ApprovalGesture {
            stable_id: canonical,
            target_node: &target,
            decision,
            via,
            expected_hash,
            keys: &self.approval.keys,
        };
        let mut port = SupervisorApprovalPort {
            sup: &self.approval.sup,
            grids: &self.approval.grids,
        };
        let outcome = exec.deliver(&gesture, &mut port);
        self.append_and_observe(&outcome.events);
        eprintln!(
            "lina-gpui: [ATT] gesto via executor: {canonical:?} via={via:?} → {:?}",
            outcome.kind
        );
        true
    }

    /// Apenda os eventos do desfecho do executor NO LOG (fonte da verdade), na ordem, e
    /// re-alimenta o fold da fila (efeito imediato; o re-replay converge ao mesmo estado). Append
    /// que falha loga ALTO e segue (best-effort de auditoria) — nunca inventa estado.
    fn append_and_observe(&self, events: &[DomainEvent]) {
        for ev in events {
            if let Err(e) = lock(&self.store).append(ev) {
                eprintln!(
                    "lina-gpui: [ATT] evento de aprovação NÃO registrado (append falhou): {e}"
                );
                continue;
            }
            lock(&self.queue).observe(ev, now_ms());
        }
        lock(&self.model).touch();
    }

    /// **F1-1-8 / ADR 0021 §3 — driver de auto-deny.** Identifica as pendências `Yn` vencidas
    /// (≥ 10 min) pela projeção FRESCA do LOG (fonte da verdade — independe do `sync` da view) e
    /// drena CADA uma pelo MESMO pipeline validado do executor com `Deny/Timeout`. Como o
    /// candidato NÃO carrega decisão (a decisão é fixada `Deny` aqui), por construção NENHUM
    /// caminho produz approve por timeout (sem knob — anti-regressão AC-0021.5). Relógio injetado
    /// (`now_ms`); chamado pelo tick do pump.
    pub fn drive_auto_deny(&self, now_ms: u64) {
        let records = lock(&self.store).events().unwrap_or_default();
        let due = lina_core::AttentionQueue::replay(&records).auto_deny_due(now_ms);
        if due.is_empty() {
            return;
        }
        let mut exec = ApprovalExecutor::replay(&records);
        let mut port = SupervisorApprovalPort {
            sup: &self.approval.sup,
            grids: &self.approval.grids,
        };
        for d in due {
            let Some(target) = exec.ledger().node_of(&d.stable_id).map(str::to_string) else {
                continue; // sem binding no log — fail-safe (não escreve)
            };
            // Captura 1 ausente (hook-only) ⇒ sentinela "" nunca casa o hash real (64-hex) → o
            // check de tela aborta (deny-não-entregue, zero bytes). Decisão FIXA `Deny`.
            let hash = d.vt_snapshot_hash.clone().unwrap_or_default();
            let gesture = ApprovalGesture {
                stable_id: &d.stable_id,
                target_node: &target,
                decision: ApprovalDecision::Deny,
                via: ResolutionVia::Timeout,
                expected_hash: &hash,
                keys: &self.approval.keys,
            };
            let outcome = exec.deliver(&gesture, &mut port);
            self.append_and_observe(&outcome.events);
            eprintln!(
                "lina-gpui: [ATT] auto-deny (SLA 10min) {:?} → {:?}",
                d.stable_id, outcome.kind
            );
        }
    }

    /// «Não era um pedido» (mitigação de FP — decisão do Maestro): apenda o
    /// `PermissionDismissed` (NÃO é deny; alimenta a telemetria de FP do detector).
    pub fn dismiss(&self, stable_id: &str) -> bool {
        let Some(ev) = lock(&self.queue).dismiss(stable_id) else {
            return false;
        };
        self.commit(ev, stable_id, "PermissionDismissed")
    }

    /// «Silenciar detecção deste terminal» (allowlist por nó): apenda o
    /// `NodeDetectionMuted` (reversível — último evento vence).
    pub fn set_node_muted(&self, node_id: &str, muted: bool) -> bool {
        let ev = lock(&self.queue).set_node_muted(node_id, muted);
        self.commit(ev, node_id, "NodeDetectionMuted")
    }

    /// O fallback de grid está silenciado para o nó? (rótulo do botão na fila).
    #[must_use]
    pub fn is_node_muted(&self, node_id: &str) -> bool {
        lock(&self.queue).is_node_muted(node_id)
    }

    /// Porta única de escrita do hub: apenda no log (fonte da verdade) e re-alimenta o
    /// fold — o efeito na fila é imediato; o próximo re-replay converge para o mesmo
    /// estado (fold único). Falha de append loga ALTO e NÃO muda a fila (o gesto não
    /// "pegou" — o humano re-tenta; nunca estado fantasma sem evento).
    fn commit(&self, ev: DomainEvent, subject: &str, kind: &str) -> bool {
        if let Err(e) = lock(&self.store).append(&ev) {
            eprintln!("lina-gpui: [ATT] {kind} de {subject:?} NÃO registrado (append falhou): {e}");
            return false;
        }
        lock(&self.queue).observe(&ev, now_ms());
        lock(&self.model).touch();
        eprintln!("lina-gpui: [ATT] {kind} registrado no log para {subject:?}");
        true
    }

    /// (teste) Apenda um evento cru no store do hub — simula o detector da F1-1-6.
    #[cfg(test)]
    pub fn store_append_for_test(&self, ev: &DomainEvent) {
        lock(&self.store).append(ev).expect("append de teste");
    }

    /// (teste) Os registros do log do hub — p/ asserts de auditoria.
    #[cfg(test)]
    pub fn events_for_test(&self) -> Vec<EventRecord> {
        lock(&self.store).events().expect("events de teste")
    }
}

/// Mapeia o `NodeStatus` do core (Bus) para o `NodeStatus` do contrato de UI.
fn map_status(s: CoreStatus) -> NodeStatus {
    match s {
        CoreStatus::Starting => NodeStatus::Starting,
        CoreStatus::Running => NodeStatus::Running,
        // Costura F1-0-3 (coordenada via Maestro): UI trata Ready≈Idle até F1-1-5 exibir estados ricos.
        CoreStatus::Ready => NodeStatus::Idle,
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
///
/// Caminho legado SEM persistência de scrollback (delega a [`wire_terminal_capturing`] com
/// `None`) — os callers de teste históricos não precisam conhecer o cabo de durabilidade.
// Consumido só pelos harnesses de teste (produção usa `wire_terminal_capturing` via `admit_node`);
// mantido como o overload sem-scrollback, daí `dead_code` no build do binário.
#[allow(clippy::too_many_arguments, dead_code)]
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
    wire_terminal_capturing(pty, sup, delta_tx, key, name, role, cmd, cols, rows, None)
}

/// **F1-5-2 — `wire_terminal` com o cabo `append-on-scroll` ligado.** Quando `scrollback`
/// é `Some`, o backend nasce CAPTURANDO (cap = `store.cap()`, fonte única do teto da janela
/// viva) e a thread leitora drena `take_scrollback()` após cada `advance`, persistindo cada
/// linha que sai do viewport no [`ScrollbackStore`] do workspace sob a chave do `NodeId`
/// (painel) — o MESMO mecanismo do `PtyHost` do core (addendo do Maestro: o app é
/// `PtyManager`-direto, então o cabo é replicado aqui, mas a lógica de harvest/persistência
/// continua no core, sem duplicação). `None` = caminho legado intacto (sem persistência).
#[allow(clippy::too_many_arguments)]
pub fn wire_terminal_capturing(
    pty: &mut PtyManager,
    sup: &Supervisor,
    delta_tx: &Sender<GridDelta>,
    key: &str,
    name: &str,
    role: &str,
    cmd: PtyCommand,
    cols: u16,
    rows: u16,
    scrollback: Option<Arc<Mutex<ScrollbackStore>>>,
) -> Result<(NodeId, Grid), String> {
    pty.spawn(key, cmd, cols, rows).map_err(|e| e.to_string())?;
    let writer = pty.take_writer(key).map_err(|e| e.to_string())?;
    // W4-2 (M2): o nó nasce com o PAPEL no roster do Supervisor → flui para `agents.json` (lina list).
    let node = sup.register(name, Some(role.to_string()), writer);

    // ADR 0022 §5 / inv#4 (ATOMICIDADE): a partir do `register` o nó JÁ está no roster vivo
    // (agents.json / `lina list` / AppPermissionWatch). Toda falha ANTES do `Ok` desta função
    // tem que COMPENSAR (unregister + kill do PTY meio-criado) — senão sobra um nó no roster
    // com ZERO eventos no log (a compensação do `admit_node` só envolve o append-loop, que roda
    // DEPOIS daqui; e o NodeId nasce nesta função, o caller não o recebe em erro). Red-team 360
    // (inv 4, ALTA): `clone_reader`/spawn-da-thread podiam falhar pós-register sem compensar.
    #[cfg(test)]
    if tests::take_wire_fault() {
        compensate_wire(pty, sup, node, key);
        return Err("wire_terminal: falha injetada após register (teste)".to_string());
    }

    let mut reader = match pty.clone_reader(key) {
        Ok(r) => r,
        Err(e) => {
            compensate_wire(pty, sup, node, key);
            return Err(e.to_string());
        }
    };

    // F1-5-2: com store ligado, o backend nasce CAPTURANDO (cap = cap do store) e a chave do
    // painel é o `NodeId` (estável e derivável da projeção no restore). Senão, caminho legado.
    let grid: Grid = match &scrollback {
        Some(store) => {
            let cap = lock(store).cap();
            // eprintln ÚNICO por spawn (o Maestro valida o cabo por log) — addendo F1-5-2.
            eprintln!("[SB] store anexado painel={name}");
            Arc::new(Mutex::new(Box::new(
                AlacrittyBackend::with_scrollback_capture(cols, rows, cap),
            )))
        }
        None => Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(cols, rows)))),
    };
    let reader_grid = Arc::clone(&grid);
    let reader_tx = delta_tx.clone();
    let sink = scrollback.map(|store| (store, node.to_string()));
    let spawned = thread::Builder::new()
        .name(format!("lina-reader-{key}"))
        .spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Drena dentro do lock do grid (não aninha grid↔store); o
                        // `take_scrollback` vem vazio quando o backend não captura.
                        let (dirty, scrolled) = {
                            let mut g = lock(&reader_grid);
                            g.advance(&buf[..n]);
                            let d = g.damaged_rows();
                            g.reset_damage();
                            let s = g.take_scrollback();
                            (d, s)
                        };
                        // F1-5-2: persiste FORA do lock do grid. Em erro de disco a linha fica
                        // no cache da cauda do store e o próximo flush re-tenta (nada some).
                        if let Some((store, panel)) = &sink {
                            if !scrolled.is_empty() {
                                let mut s = lock(store);
                                for line in scrolled {
                                    if let Err(e) = s.push_line(panel, line) {
                                        eprintln!(
                                            "lina-gpui: scrollback push falhou painel={panel}: \
                                             {e} (linha fica no cache da cauda, re-tenta)"
                                        );
                                    }
                                }
                            }
                        }
                        if !dirty.is_empty() {
                            let _ = reader_tx.send(GridDelta {
                                node,
                                rows: dirty,
                                bytes: n,
                                seq: 0,
                            });
                        }
                    }
                }
            }
        });
    if let Err(e) = spawned {
        compensate_wire(pty, sup, node, key);
        return Err(e.to_string());
    }
    Ok((node, grid))
}

/// **COMPENSAÇÃO da janela de [`wire_terminal`] (ADR 0022 §5 / inv#4).** Após `sup.register`
/// o nó está no roster vivo; se uma operação seguinte falha, desfaz a meia-fiação ANTES de
/// propagar o erro: `unregister` (tira do roster/`agents.json`) + `kill`+`remove` do PTY
/// meio-criado. Síncrono — é caminho de erro raro (exaustão de FD/threads do SO), não há UI a
/// não-bloquear. Erros da própria limpeza são logados (best-effort): não há mais o que fazer.
fn compensate_wire(pty: &mut PtyManager, sup: &Supervisor, node: NodeId, key: &str) {
    if let Err(e) = sup.unregister(node) {
        eprintln!("lina-gpui: compensação wire_terminal — unregister do nó {node} falhou: {e}");
    }
    if let Err(e) = pty.kill(key, Duration::from_secs(2)) {
        eprintln!("lina-gpui: compensação wire_terminal — kill do PTY {key} falhou: {e}");
    }
    let _ = pty.remove(key);
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
/// Sentinela da nota de localização (#9) — a checagem de idempotência ancora aqui.
const WORKDIR_NOTE_MARK: &str = "Sua pasta de trabalho (cwd) é";

/// **#9 dogfooding r2 — a doutrina do nó gerenciado CITA o cwd dele.** O spawnado nasce numa
/// pasta NOVA (`<ws_root>/n-<uuid>`) que ele não conhece; um caminho RELATIVO vindo do
/// orquestrador quebra silenciosamente (evidência da sessão: correção manual via `lina ask`).
/// Appenda 1 linha com o caminho ABSOLUTO aos 3 arquivos de doutrina (neutralidade multi-CLI,
/// inv. #3) DEPOIS de o gerador (`lina-bootstrap`) escrevê-los — o gerador é de outro dono
/// nesta rodada; a linha vive no wrapper do app (o MESMO `write_one` que o `admit_node` usa).
/// Idempotente ([`WORKDIR_NOTE_MARK`]); best-effort com erro VISÍVEL (postura do kit).
fn append_workdir_note(dir: &Path) {
    let note = format!(
        "\n> 📍 {WORKDIR_NOTE_MARK} `{}` (caminho ABSOLUTO). Ao citar arquivos para um colega \
         de outro terminal, use SEMPRE caminhos absolutos — um caminho relativo de OUTRA pasta \
         não vale aqui.\n",
        dir.display()
    );
    for f in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let p = dir.join(f);
        match std::fs::read_to_string(&p) {
            // Já anotado (re-render do roster reescreve a base e o append re-aplica; este
            // ramo só protege contra append duplo no MESMO conteúdo).
            Ok(cur) if cur.contains(WORKDIR_NOTE_MARK) => {}
            Ok(cur) => {
                if let Err(e) = std::fs::write(&p, format!("{cur}{note}")) {
                    eprintln!("lina-gpui: nota de cwd em {} falhou: {e}", p.display());
                }
            }
            // Doutrina ausente = o write do kit já falhou (erro logado lá); nada a anotar.
            Err(_) => {}
        }
    }
}

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
/// fallback `/bin/sh`), idêntico para TODOS os nós (aceita teclado, roda claude/vim/…). Dá `exec`
/// direto no shell, **sem banner/intro** (pedido do fundador: o `+ Terminal` deve abrir como o
/// Terminal.app — shell limpo). É gpui-free (vive aqui, não no main). Prepõe o diretório do `lina`
/// ao `PATH` do filho para o A2A funcionar sem setup manual. O `_name` é ignorado (o shell puro
/// não imprime mais o nome); fica na assinatura só pelo contrato da [`CmdFactory`].
#[must_use]
pub fn shell_cmd(_name: &str) -> PtyCommand {
    let cmd = platform_shell_cmd().env("TERM", "xterm-256color");
    // Garante o `lina` no PATH do shell filho (A2A sem setup manual): prepõe o dir do binário
    // `lina` ao PATH herdado. Best-effort — sem resolução, o shell ainda herda o PATH do pai.
    match lina_path_value() {
        Some(path) => cmd.env("PATH", path),
        None => cmd,
    }
}

/// **ADR 0026 — identidade do nó no ENV do PTY filho (autoridade do APP no spawn).** Único
/// ponto que carimba `VIBE_ROLE` + `LINA_NODE_NAME` + `LINA_NODE_ID` + `LINA_AUTONOMY` (FIX-3)
/// no comando de um nó —
/// chamado pelo funil único `admit_node`. `LINA_NODE_ID` leva a `key` durável (`n-<uuidv7>`,
/// a identidade de filesystem do nó, cunhada ANTES do spawn; o `NodeId` canônico só nasce no
/// `register`, DEPOIS do PTY subir). O CLI `lina` resolve `terminal_name` por
/// `LINA_NODE_NAME` antes da ficha por-cwd — ver ADR 0026 §segurança: um agente pode mascarar
/// o PRÓPRIO whoami exportando a var num subshell, mas o `from` A2A final é re-carimbado pelo
/// supervisor no drain (dir-dono); nenhuma autoridade nova nasce aqui.
fn node_identity_env(
    cmd: PtyCommand,
    name: &str,
    role: &str,
    key: &str,
    autonomy: Autonomy,
) -> PtyCommand {
    cmd.env("VIBE_ROLE", role)
        .env("LINA_NODE_NAME", name)
        .env("LINA_NODE_ID", key)
        // FIX-3 (costura per-Agente): a autonomia ESCOLHIDA no modal vira env por-Agente — o
        // guard (`lina guard --pretooluse`) passa a honrar o nível DESTE nó, não só o do
        // Espaço. O `label()` (`manual`/`assistido`/`autonomo`) casa exatamente com o
        // `lina_core::guard::parse_autonomy`. Mesma postura de segurança do ADR 0026: o env é
        // autoridade do APP no spawn; um agente que reexporte a var só afeta o próprio gate
        // local (a custódia de ações irreversíveis — ADR 0004 — não muda).
        .env("LINA_AUTONOMY", autonomy.label())
}

#[cfg(windows)]
fn platform_shell_cmd() -> PtyCommand {
    // Shell interativo limpo, sem a linha de intro (`/K` sem `echo`): o `cmd.exe` já abre
    // aceitando comandos. Como o Terminal padrão do Windows.
    PtyCommand::new("cmd.exe")
}

#[cfg(not(windows))]
fn platform_shell_cmd() -> PtyCommand {
    // `exec` direto no shell de login do usuário (com fallback `/bin/sh`), SEM intro.
    PtyCommand::new("sh")
        .arg("-c")
        .arg("exec \"${SHELL:-/bin/sh}\" -i")
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

/// **R2c — env de ORQUESTRADOR ESTRANGEIRO (bug de tela 21:40 do fundador).** O Lina.app
/// lançado de um terminal hospedado no Maestri herda `MAESTRI_CLI` (& afins) no env do
/// processo; como TODO PTY filho herda esse env, a skill GLOBAL do Maestri
/// (`~/.claude/skills/maestri`) vira "funcional" aos olhos do modelo — o agente do ⌘N
/// tenta `maestri list` (exit 127) e `"$MAESTRI_CLI"` (exit 126: a VAR EXISTE).
///
/// Casa por **PREFIXO** (não nomes exatos): toda `MAESTRI_*` é plumbing do Maestri.
/// Estruturado p/ novos orquestradores entrarem por esta lista (inv #3 — a
/// neutralidade é também *não confundir* o agente com ferramentas de fora do Espaço).
/// **Não** mira vars de auth (`ANTHROPIC_*`/`CLAUDE_*`) nem básicas (`PATH`/`HOME`/`TERM`)
/// — o agente precisa delas para funcionar.
pub const FOREIGN_ORCHESTRATOR_ENV_PREFIXES: &[&str] = &["MAESTRI_"];

/// `true` se `key` é uma variável de plumbing de orquestrador estrangeiro (a ser
/// scrubada do env do agente). PURA — testável sem tocar o env global.
#[must_use]
pub fn is_foreign_orchestrator_var(key: &str) -> bool {
    FOREIGN_ORCHESTRATOR_ENV_PREFIXES
        .iter()
        .any(|p| key.starts_with(p))
}

/// As variáveis estrangeiras dentre `names` (seleção PURA — base do scrub real, testável
/// sem mutar o env do processo).
#[must_use]
pub fn foreign_orchestrator_keys<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    names
        .into_iter()
        .filter(|k| is_foreign_orchestrator_var(k))
        .map(str::to_owned)
        .collect()
}

/// Remove do **próprio env do processo do app** (herdado por TODO PTY filho) as variáveis
/// de orquestradores estrangeiros (`MAESTRI_*`). Chamado AQUI — antes de qualquer
/// `wire_terminal` — para nenhum agente nascer enxergando o plumbing do Maestri. Coleta
/// os nomes ANTES de remover (não dá para `remove_var` enquanto itera `vars_os`); a
/// seleção é a função pura [`foreign_orchestrator_keys`] (mesma lógica que os testes
/// exercitam sem tocar o env global). Retorna os nomes efetivamente removidos.
pub fn scrub_foreign_orchestrator_env() -> Vec<String> {
    let names: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .collect();
    let present = foreign_orchestrator_keys(names.iter().map(String::as_str));
    for key in &present {
        std::env::remove_var(key);
    }
    present
}

/// Fábrica injetável de comando por nome: o app passa [`shell_cmd`] (shell real); os testes
/// passam um comando leve (`cat`) p/ serem determinísticos e headless.
pub type CmdFactory = Arc<dyn Fn(&str) -> PtyCommand + Send + Sync>;

/// **F1-2-2 (M6)** — o MOTOR escolhido no modal: comando vindo do CLI Profile TOML (quando o
/// profile existe) ou do binário descoberto (W4-1). `label` é o rótulo LEIGO ("Claude Code") que
/// vai p/ `TerminalSpawned.cli`; nada disso atravessa para o core (inv. #7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEngine {
    pub program: String,
    pub args: Vec<String>,
    pub profile_id: Option<String>,
    pub label: String,
}

/// **#7 dogfooding r2 — fábrica do MOTOR de um spawn agente-pede.** Produção descobre o CLI
/// default do workspace no momento do spawn ([`default_spawn_engine`]); os testes injetam uma
/// fixa (a discovery real leria o `PATH` do runner e lançaria o CLI de verdade no PTY do
/// teste). Mesmo padrão injetável do [`CmdFactory`].
pub type SpawnEngineFactory = Arc<dyn Fn() -> Option<AgentEngine> + Send + Sync>;

/// **#7 dogfooding r2 — o motor DEFAULT de um spawn agente-pede**, derivado da descoberta de
/// CLIs (W4-1). `lina spawn` cria um COLEGA (agente), mas o nó nascia da fábrica do workspace
/// (shell puro) e `TerminalSpawned.cli` recebia o NOME do nó — poluindo as projeções por-CLI
/// (detecção/perfis/custo; evidência: seq 20326 `cli:"@Especialista em Licenças"` vs ⌘N
/// `cli:"Claude Code"`). Paridade com o modal ⌘N: `claude` primeiro (a MESMA pré-seleção de
/// `agent_modal::engines_from`), senão o 1º descoberto. Máquina sem CLI → `None` (fábrica do
/// workspace — comportamento histórico, degradação honesta). PURA (testável sem `PATH`).
/// `profile_id` fica `None`: o `ProfileRegistry` vive no pump, não no funil — sem ele não sai
/// `CliProfileSet` (igual a hoje; costura registrada na entrega).
fn default_spawn_engine(found: &[DiscoveredCli]) -> Option<AgentEngine> {
    let pick = found
        .iter()
        .find(|d| d.id == "claude")
        .or_else(|| found.first())?;
    Some(AgentEngine {
        program: pick.path.clone(),
        args: Vec::new(),
        profile_id: None,
        label: spawn_cli_label(&pick.id),
    })
}

/// Rótulo leigo do CLI descoberto — ESPELHO de `agent_modal::engine_label` (o modal importa
/// DESTE módulo; importar de lá criaria ciclo). O teste de paridade
/// `spawn_cli_label_matches_modal_engine_label` trava as duas cópias juntas (drift = vermelho).
fn spawn_cli_label(id: &str) -> String {
    match id {
        "claude" => "Claude Code".to_string(),
        "codex" => "Codex".to_string(),
        "gemini" => "Gemini CLI".to_string(),
        "opencode" => "OpenCode".to_string(),
        "copilot" => "Copilot CLI".to_string(),
        "agy" => "Antigravity".to_string(),
        other => {
            let mut s = other.to_string();
            if let Some(f) = s.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            s
        }
    }
}

/// **W4-2 (M2) — deriva o PAPEL canônico pelo nome do agente** ("Revisor" → `reviewer`, "@Dev" →
/// backend, …) via role-discovery (W3-1). Determinístico e infalível: nome desconhecido cai no
/// fallback do registry; registry indisponível → `"generalist"`. O registry (com o overlay
/// `reviewer`) é o MESMO do co-piloto (F1-2-2: fonte única em `role_suggester::role_registry` —
/// sugestão na tela e papel gravado no log nunca divergem).
fn infer_agent_role(name: &str) -> String {
    match crate::role_suggester::role_registry() {
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

/// **BUG B — caminho(s) de arquivo solto(s) no terminal → string CITADA** pronta p/ injetar no PTY
/// pelo MESMO caminho do teclado humano (`WriteOp::HumanKeys`), espelhando o Maestri. Cada caminho
/// vai entre aspas SIMPLES (POSIX: tudo é literal dentro de `'…'`, então espaços e metacaracteres
/// ficam seguros), com a aspa simples INTERNA escapada como `'\''` (fecha-escapa-reabre). Vários
/// caminhos são separados por espaço; a string TERMINA com um espaço, deixando o cursor pronto p/ o
/// próximo path/digitação. Sem caminhos → string vazia (o chamador NÃO escreve no PTY). Pura e
/// testável; o drop VISUAL = PENDENTE TELA DO FUNDADOR (gpui não roda headless).
#[must_use]
pub fn quote_dropped_paths(paths: &[std::path::PathBuf]) -> String {
    let mut out = String::new();
    for p in paths {
        out.push('\'');
        out.push_str(&p.to_string_lossy().replace('\'', "'\\''"));
        out.push_str("' ");
    }
    out
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
    /// F1-1-3 (wiring): listener de hooks compartilhado do app. `None` = degradação
    /// limpa (CLI sem `capabilities.hooks` OU bind falhou) → settings SEM hooks, como hoje.
    hooks: Option<Arc<HooksShared>>,
}

/// **F1-1-3 (wiring) — o listener de hooks do app, ÚNICO por processo.** Faz o
/// `HookListener::bind()` 1x no boot (porta efêmera em loopback — inv#2) e mantém o
/// runtime tokio dedicado VIVO (1 worker; o `serve` do axum roda nele). Cada spawn
/// pede um token por nó ([`Self::register`]); a timeline consome [`Self::subscribe`].
pub struct HooksShared {
    /// Mantém o serve vivo — dropar o runtime mataria o listener.
    _rt: tokio::runtime::Runtime,
    listener: lina_hooks::HookListener,
}

impl HooksShared {
    /// Sobe runtime + listener. Falha NUNCA derruba o app: `None` + stderr (degradação
    /// VISÍVEL — sem hooks o app funciona exatamente como antes desta fiação).
    pub fn start() -> Option<Arc<Self>> {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!(
                    "lina-gpui: [DASH] runtime de hooks indisponível ({e}) — seguindo sem hooks"
                );
                return None;
            }
        };
        match rt.block_on(lina_hooks::HookListener::bind()) {
            Ok(listener) => {
                eprintln!(
                    "lina-gpui: [DASH] hook listener vivo em 127.0.0.1:{}",
                    listener.local_addr().port()
                );
                Some(Arc::new(Self { _rt: rt, listener }))
            }
            Err(e) => {
                eprintln!(
                    "lina-gpui: [DASH] bind do hook listener falhou ({e}) — seguindo sem hooks"
                );
                None
            }
        }
    }

    /// Porta efêmera do listener (entra no settings de cada terminal via `HookWiring`).
    #[must_use]
    pub fn port(&self) -> u16 {
        self.listener.local_addr().port()
    }

    /// Token de atribuição do nó `key` — a ÚNICA fonte de identidade do POST.
    #[must_use]
    pub fn register(&self, key: &str) -> String {
        self.listener.register_node(key)
    }

    /// Stream dos eventos normalizados (a timeline do dashboard drena daqui).
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<lina_hooks::HookEvent> {
        self.listener.subscribe()
    }

    /// Drena o stream NO runtime do próprio listener (sem thread extra): cada evento
    /// vai ao callback (timeline + sonda `[DASH]`). Lag é CONTADO no stderr, nunca
    /// silencioso; o loop morre junto com o listener.
    pub fn spawn_drain(&self, mut on_event: impl FnMut(lina_hooks::HookEvent) + Send + 'static) {
        let mut rx = self.subscribe();
        self._rt.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => on_event(ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("lina-gpui: [DASH] timeline perdeu {n} eventos (lag)");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
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
            hooks: None,
        })
    }

    /// F1-1-3 (wiring): liga os hooks de observabilidade nos PRÓXIMOS writes — chamado
    /// no boot quando o perfil do CLI declara `capabilities.hooks=true` (TOML, F1-1-1)
    /// E o listener subiu. Sem esta chamada = degradação (settings como hoje).
    pub fn set_hooks(&mut self, hooks: Arc<HooksShared>) {
        self.hooks = Some(hooks);
    }

    /// Diretório (cwd) do terminal `key`.
    #[must_use]
    pub fn dir_for(&self, key: &str) -> PathBuf {
        self.ws_root.join(key)
    }

    /// F1-4-3: `true` se `dir` está DIRETAMENTE sob o `ws_root` com o shape das keys
    /// gerenciadas (`n-<uuid>` atual, `t<seq>` legado) — só a própria Lina cria dirs assim;
    /// o kit dentro deles é nosso por construção, mesmo sem carimbo (gerações pré-carimbo).
    fn is_managed_namespace(&self, dir: &Path) -> bool {
        dir.parent() == Some(self.ws_root.as_path())
            && dir.file_name().and_then(OsStr::to_str).is_some_and(|s| {
                s.strip_prefix("n-").is_some_and(|rest| !rest.is_empty())
                    || s.strip_prefix('t').is_some_and(|rest| {
                        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
                    })
            })
    }

    /// Vault primário EFETIVO a injetar na doutrina. O que o usuário linkou no onboarding
    /// (`<ws_root>/.lina/vault.json`) tem PRIORIDADE; só cai no `vault_path` do boot (fallback) se ainda
    /// não houver vault linkado. **Re-lido a cada escrita** — corrige o bug de ORDEM: o `vault_path` era
    /// congelado no boot (ANTES de o onboarding confirmar os vaults), então as doutrinas geradas depois
    /// apontavam para o fallback `<ws_root>/vault` (inexistente) em vez do vault real do usuário.
    fn effective_vault(&self) -> String {
        crate::obsidian::read_primary_vault(&self.ws_root.join(".lina"))
            .unwrap_or_else(|| self.vault_path.clone())
    }

    /// Escreve os arquivos de bootstrap de **UM** terminal (`key`/`name`) com o `roster` dado.
    /// Usado tanto no `rewrite_all` quanto no write-before-spawn do `add_node` (o nó novo já sobe
    /// com o `CLAUDE.md` no `cwd`). Erro de I/O é logado (best-effort): o app não trava por isto.
    pub fn write_one(&self, key: &str, name: &str, roster: &[String]) {
        let dir = self.dir_for(key);
        let input = BootstrapInput::new(
            name.to_string(),
            roster.to_vec(),
            self.effective_vault(),
            self.autonomy,
        );
        // F1-1-3 (wiring): com o listener vivo, o terminal ganha settings de hooks com
        // TOKEN próprio (atribuição por spawn — nunca pelo payload). O token registra o
        // NOME do terminal (identidade A2A única do roster) — é o `node_id` que chega na
        // timeline e casa com `NodeView.name` no painel. `None` = degradação limpa:
        // settings sem hooks, exatamente como antes da fiação.
        let wiring = self.hooks.as_ref().map(|h| lina_bootstrap::HookWiring {
            port: h.port(),
            token: h.register(name),
        });
        if let Err(e) = self.bootstrapper.write_terminal_files_with_hooks(
            &dir,
            &input,
            &self.lina_bin,
            wiring.as_ref(),
        ) {
            eprintln!("lina-gpui: bootstrap de {name} falhou: {e}");
        }
        // #9 dogfooding r2: o spawnado nasce num dir gerenciado NOVO que ele não conhece — a
        // doutrina cita o cwd ABSOLUTO (1 linha, nos 3 CLIs). Sem isto, um 1º prompt com
        // caminho RELATIVO do orquestrador quebra silencioso (correção manual na sessão).
        append_workdir_note(&dir);
        // Instala a skill `lina-agent-bus` no cwd do terminal → A2A automático no turno 0
        // (idempotente, best-effort com erro visível). Cobre o boot (via `rewrite_bootstrap`) E os
        // nós adicionados em runtime (write-before-spawn do funil de admissão). Não duplica lógica.
        install_agent_bus_skill(&dir);
    }

    /// **Rodada 360 (ADR 0022 §4) — kit de integração na pasta REAL do usuário**, com escrita
    /// MERGE-SAFE: um arquivo pré-existente que NÃO foi gerado pela Lina (sem o marcador) é
    /// **recusado com aviso** — jamais sobrescrito. Os nossos próprios (marcados) são
    /// reescritos normalmente a cada mudança de roster. `.lina/` e `.claude/skills/` são
    /// namespace da Lina (aditivos, sempre escritos). Devolve o que foi recusado, para o
    /// chamador degradar VISIVELMENTE (badge) — nunca silencioso.
    pub fn write_user_dir(&self, dir: &Path, name: &str, roster: &[String]) -> KitOutcome {
        // F1-4-3 (bug da porta morta): um dir DIRETAMENTE sob o `ws_root` com o shape das keys
        // gerenciadas só pode ter sido criado pela própria Lina — o kit nele é NOSSO por
        // construção, mesmo sem carimbo (gerações antigas, pré-carimbo). Sem esta migração, o
        // restore re-entrava nesses dirs como `UserDir` e PRESERVAVA um settings com a porta de
        // hooks de um boot morto → ECONNREFUSED em todo hook (visto na tela em 2026-06-11).
        let managed_ns = self.is_managed_namespace(dir);
        let input = BootstrapInput::new(
            name.to_string(),
            roster.to_vec(),
            self.effective_vault(),
            self.autonomy,
        );
        let wiring = self.hooks.as_ref().map(|h| lina_bootstrap::HookWiring {
            port: h.port(),
            token: h.register(name),
        });
        let mut refused: Vec<String> = Vec::new();
        let mut guarded =
            |rel: &str, is_ours: &dyn Fn(&str) -> bool, content: String| match write_guarded(
                &dir.join(rel),
                is_ours,
                &content,
            ) {
                Ok(true) => {}
                Ok(false) => {
                    eprintln!(
                        "lina-gpui: {rel} pré-existente do usuário em {} — NÃO sobrescrevo \
                         (kit parcial; o card mostra o aviso)",
                        dir.display()
                    );
                    refused.push(rel.to_string());
                }
                Err(e) => {
                    eprintln!("lina-gpui: escrever {rel} em {} falhou: {e}", dir.display());
                    refused.push(rel.to_string());
                }
            };
        // Doutrina (3 tiers): CARIMBADA com o marcador de máquina na 1ª linha; a posse é
        // `starts_with` — um arquivo do usuário que apenas CITE a doutrina não vira nosso
        // (red-team da rodada: `contains` do heading sobrescreveria citação). Em dir
        // GERENCIADO a posse é por construção (ver `managed_ns` acima).
        let ours_md = |existing: &str| managed_ns || existing.starts_with(LINA_MANAGED_MARKER);
        let stamped = |body: String| format!("{LINA_MANAGED_MARKER}\n{body}");
        guarded(
            "CLAUDE.md",
            &ours_md,
            stamped(self.bootstrapper.doctrine(&input)),
        );
        guarded(
            "AGENTS.md",
            &ours_md,
            stamped(self.bootstrapper.agents_doctrine(&input)),
        );
        guarded(
            "GEMINI.md",
            &ours_md,
            stamped(self.bootstrapper.gemini_doctrine(&input)),
        );
        // Settings/hooks (observabilidade + gate): posse por CHAVE ESTRUTURADA
        // (`_lina_managed: true`, parseada e checada por VALOR — não substring do comando do
        // hook, que o usuário poderia legitimamente ter). Um settings DO USUÁRIO sem a chave é
        // preservado (recusa + badge). Red-team 360 (inv 1, MÉDIA): `contains("whoami --bootstrap")`
        // sobrescreveria um settings do usuário que apenas contivesse aquela substring.
        guarded(
            ".claude/settings.json",
            &|existing: &str| managed_ns || settings_is_ours(existing),
            lina_bootstrap::stamp_managed_settings(
                &lina_bootstrap::hook_settings_json_with_observability(
                    &self.lina_bin,
                    wiring.as_ref(),
                ),
            ),
        );
        // `.lina/bootstrap.json` é namespace da Lina — escrita direta (estado vivo do `lina whoami`).
        let lina_dir = dir.join(".lina");
        let json = serde_json::to_string_pretty(&input).unwrap_or_else(|_| String::from("{}"));
        if let Err(e) = std::fs::create_dir_all(&lina_dir)
            .and_then(|()| std::fs::write(lina_dir.join("bootstrap.json"), json))
        {
            eprintln!(
                "lina-gpui: .lina/bootstrap.json em {} falhou: {e}",
                dir.display()
            );
        }
        // Skill A2A turno-0 — namespace `.claude/skills/` (idempotente, best-effort visível).
        install_agent_bus_skill(dir);
        if refused.is_empty() {
            KitOutcome::Full
        } else {
            KitOutcome::Partial(refused)
        }
    }
}

/// Marcador de MÁQUINA carimbado na 1ª linha dos markdown que a Lina escreve em pasta do
/// usuário (comentário HTML — invisível no render). A posse é `starts_with`: só um arquivo
/// que COMEÇA com ele é nosso; citações do marcador/doutrina no corpo de um arquivo do
/// usuário não transferem a posse.
const LINA_MANAGED_MARKER: &str = "<!-- lina-space:gerado — a Lina reescreve este arquivo a \
cada mudança de roster; edições manuais serão perdidas -->";
/// `true` se o `settings.json` existente é GERENCIADO pela Lina: a chave estruturada
/// [`lina_bootstrap::LINA_MANAGED_SETTINGS_KEY`] está presente e é `true` (parse + checagem
/// por valor, jamais substring). JSON ilegível/sem a chave → do usuário (preserve). O carimbo
/// em si mora no crate (`stamp_managed_settings` — F1-4-3: o kit por-nó também sai carimbado).
fn settings_is_ours(existing: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(existing)
        .ok()
        .and_then(|v| {
            v.get(lina_bootstrap::LINA_MANAGED_SETTINGS_KEY)
                .and_then(serde_json::Value::as_bool)
        })
        == Some(true)
}

/// Resultado da entrega do kit de integração numa pasta do usuário (ADR 0022 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KitOutcome {
    /// Tudo entregue (arquivos ausentes ou já nossos).
    Full,
    /// Estes arquivos pré-existentes do usuário foram PRESERVADOS (recusa com aviso).
    Partial(Vec<String>),
}

/// Escrita guardada (merge-safe): grava `content` em `path` SÓ se o arquivo não existe ou se
/// `is_ours(conteúdo atual)` (= foi a Lina que o gerou — o critério é do CHAMADOR, por tipo de
/// arquivo). `Ok(false)` = pré-existente do usuário, preservado. Cria os diretórios-pai
/// (ex.: `.claude/`).
fn write_guarded(
    path: &Path,
    is_ours: &dyn Fn(&str) -> bool,
    content: &str,
) -> std::io::Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(existing) if !is_ours(&existing) => return Ok(false),
        // Nosso (critério do chamador) → segue para a escrita.
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Existe mas não deu para ler (binário/permissão): trate como DO USUÁRIO — preserve.
        Err(_) => return Ok(false),
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// **F1-2-3 p2 — decode PURO dos papéis custom** (testável headless): filtra os
/// `RoleTemplateSaved` de `records`, EM ORDEM DE LOG, SEM dedup (o modal aplica o
/// last-wins por `name` — doutrina do evento no core). Campos ausentes do payload →
/// `""` (espírito `serde(default)`: shape antigo/parcial nunca quebra o replay).
#[must_use]
fn role_templates_from_records(records: &[EventRecord]) -> Vec<crate::agent_modal::RoleTemplate> {
    let field = |p: &serde_json::Value, k: &str| -> String {
        p.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    records
        .iter()
        .filter(|r| r.kind == "RoleTemplateSaved")
        .map(|r| crate::agent_modal::RoleTemplate {
            name: field(&r.payload, "name"),
            description: field(&r.payload, "description"),
            doctrine: field(&r.payload, "doctrine"),
        })
        .collect()
}

/// O HOME do usuário (`$HOME`, com fallback `$USERPROFILE` no Windows e, em último caso,
/// `std::env::temp_dir()`) — o cwd do terminal PURO (`+ Terminal`), como o Terminal.app. Nunca
/// panica (sem `unwrap`/`expect`): se o ambiente não trouxer nenhum, cai num diretório válido.
#[must_use]
fn user_home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// **Rodada 360 (ADR 0022 §4) — política de pasta de trabalho de uma admissão.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdPolicy {
    /// Dir gerenciado `<ws_root>/n-<uuid>` — kit de integração completo, como sempre (AGENTES).
    Managed,
    /// **Terminal PURO (`+ Terminal` / ⌘T).** cwd = HOME do usuário (`path`), como o
    /// Terminal.app: abre no `$HOME`, ZERO arquivos do Lina escritos (nunca um `CLAUDE.md`/
    /// `.lina`/`.claude` no home — não poluir o diretório pessoal). Sem dir gerenciado, sem kit,
    /// sem badge de degradação (não é agente meio-cidadão: é um shell limpo por design).
    UserHome { path: PathBuf },
    /// Pasta REAL do usuário (picker do M6). `consent` = o usuário autorizou no modal a
    /// Lina criar os arquivos de orquestração nela; sem consentimento o nó nasce com a
    /// degradação VISÍVEL (badge "sem doutrina/observabilidade") — nunca silenciosa.
    UserDir { path: PathBuf, consent: bool },
}

/// FIX-1 (dogfooding 2026-06-10): o path de TRABALHO que conta para a detecção de "cwd
/// compartilhado" — apenas `UserDir` (a pasta do usuário escolhida no M6, que recebe
/// kit/observabilidade e onde N agentes de um time se encontram). `Managed` é dir ÚNICO por
/// key (`n-<uuid>`, nunca colide por construção) e `UserHome` é o `$HOME` do terminal puro
/// (sempre compartilhado, sem kit: não é o bug). `None` ⇒ não participa da detecção.
fn shared_cwd_path(policy: &CwdPolicy) -> Option<&Path> {
    match policy {
        CwdPolicy::UserDir { path, .. } => Some(path.as_path()),
        CwdPolicy::Managed | CwdPolicy::UserHome { .. } => None,
    }
}

/// **Rodada 360 (ADR 0022 §1) — o PLANO de admissão**: a intenção "quero um terminal"
/// normalizada. Os entry points são tradutores finos de intenção → este plano; quem o
/// executa é SEMPRE o [`NodeManager::admit_node`]. Toda porta futura (API, CLI,
/// inteligência da Lina) entra por aqui ou não entra.
#[derive(Debug, Clone)]
pub struct NodeAdmission {
    /// `None` = nome automático "Terminal A/B/C…" derivado do seq alocado no funil (porta ⌘T).
    pub name: Option<String>,
    /// O papel canônico do nó — "terminal" também é papel (fim da classe dos 31-sem-papel).
    pub role: String,
    /// Motor escolhido no M6 (`None` = fábrica default `shell_cmd`).
    pub engine: Option<AgentEngine>,
    pub cwd: CwdPolicy,
    /// Posição FIXA no canvas (porta seed/demo); `None` = próximo slot livre.
    pub position: Option<(f64, f64)>,
    /// SEAM-1 (F1-3-6): NodeId do agente que PEDIU este spawn (`lina spawn`). `None` nas portas de
    /// ORIGEM HUMANA (⌘T/seed). `Some(spawner)` na admissão de spawn agente-pede → carimba
    /// `NodeAdded.requested_by` (auditoria intent-vs-action + base do anti-fork-bomb).
    pub requested_by: Option<NodeId>,
    /// FIX-3 (costura per-Agente): a autonomia ESCOLHIDA para este nó (do modal M6 →
    /// `CreatePlan.autonomy`). `admit_node` a carimba em `LINA_AUTONOMY` (env do PTY, lido pelo
    /// guard) e na projeção (`NodeView.autonomy`, fonte do badge). Os construtores ⌘T/seed/spawn
    /// usam `Assisted` (default do produto); só ⌘N (`create_agent_with_autonomy`) propaga a
    /// escolha do usuário.
    pub autonomy: Autonomy,
}

impl NodeAdmission {
    /// Porta ⌘T / botão +: terminal PURO, nome automático, cwd = HOME do usuário (como o
    /// Terminal.app). NÃO é agente: não recebe kit/doutrina, e ZERO arquivos do Lina são escritos
    /// no home (`CwdPolicy::UserHome`). Agentes (motor escolhido no M6) seguem `Managed`/`UserDir`.
    #[must_use]
    pub fn default_terminal() -> Self {
        Self {
            name: None,
            role: "terminal".into(),
            engine: None,
            cwd: CwdPolicy::UserHome {
                path: user_home_dir(),
            },
            position: None,
            requested_by: None,
            autonomy: Autonomy::Assisted,
        }
    }

    /// Porta seed/demo: terminal NOMEADO em posição fixa do roteiro (LINA_DEMO).
    #[must_use]
    pub fn seeded_terminal(name: impl Into<String>, x: f64, y: f64) -> Self {
        Self {
            name: Some(name.into()),
            role: "terminal".into(),
            engine: None,
            cwd: CwdPolicy::Managed,
            position: Some((x, y)),
            requested_by: None,
            autonomy: Autonomy::Assisted,
        }
    }

    /// **SEAM-1: porta de SPAWN agente-pede** (`lina spawn` aprovado, origem OU banner-aprovado). O
    /// terminal nasce gerenciado (cwd `Managed` → kit/doutrina do papel), com `requested_by` =
    /// spawner (carimba `NodeAdded.requested_by` p/ auditoria + anti-fork-bomb). O `engine` default
    /// (`None`) usa a fábrica do workspace — mesma do ⌘T.
    #[must_use]
    pub fn spawned_agent(
        name: impl Into<String>,
        role: impl Into<String>,
        requested_by: NodeId,
        // #7 dogfooding r2: o MOTOR do colega spawnado (CLI default do workspace, resolvido
        // pelo `execute_spawn` via `SpawnEngineFactory`). `None` = máquina sem CLI → fábrica
        // do workspace (comportamento histórico; `TerminalSpawned.cli` degrada para o nome).
        engine: Option<AgentEngine>,
    ) -> Self {
        Self {
            name: Some(name.into()),
            role: role.into(),
            engine,
            cwd: CwdPolicy::Managed,
            position: None,
            requested_by: Some(requested_by),
            autonomy: Autonomy::Assisted,
        }
    }
}

/// **A sequência CANÔNICA e COMPLETA de eventos de uma admissão (ADR 0022 §2)** — função
/// PURA (testável headless; o teste de paridade das 3 portas assenta aqui): `NodeAdded` +
/// `TerminalSpawned { cli, cwd REAL }` + `NodeRenamed` (F1-4-3: o nome no log, fonte do
/// restore) + `NodeRoleAssigned` SEMPRE + `CliProfileSet` quando o profile é conhecido.
/// Qualquer porta de admissão produz EXATAMENTE isto, módulo parâmetros.
#[allow(clippy::too_many_arguments)] // espelho 1:1 dos campos da admissão (sequência canônica ADR 0022)
fn admission_events(
    node: NodeId,
    x: f64,
    y: f64,
    name: &str,
    role: &str,
    engine: Option<&AgentEngine>,
    cwd: Option<&Path>,
    // SEAM-1: quem PEDIU o spawn (`Some` na porta agente-pede; `None` na origem humana ⌘T/seed).
    requested_by: Option<NodeId>,
) -> Vec<DomainEvent> {
    let mut events = vec![
        DomainEvent::NodeAdded {
            node,
            kind: "Terminal".into(),
            x,
            y,
            // SEAM-1 (F1-3-6): `Some(spawner)` p/ spawn agente-pede; `None` p/ origem humana.
            requested_by,
        },
        DomainEvent::TerminalSpawned {
            node,
            // Rótulo do MOTOR quando escolhido no M6 ("Claude Code"); senão o nome (legado M2).
            cli: engine.map_or_else(|| name.to_string(), |e| e.label.clone()),
            // O binding node↔cwd persistido (§3): o cwd REAL do spawn. `None` SÓ quando o
            // spawn herdou o cwd do app (bootstrap desligado) — degradação honesta, sem chute.
            cwd: cwd.map(|p| p.display().to_string()),
        },
        // F1-4-3: o NOME resolvido da admissão entra no LOG — `ProjectedNode.name` só nasce de
        // `NodeRenamed`, e sem ele o restore re-erguia todo mundo como "Terminal N" do contador
        // (que reinicia a cada boot). Mesmo evento do rename manual; último-vence na projeção.
        DomainEvent::NodeRenamed {
            node,
            name: name.to_string(),
        },
        DomainEvent::NodeRoleAssigned {
            node,
            role: role.to_string(),
        },
    ];
    if let Some(pid) = engine.and_then(|e| e.profile_id.as_deref()) {
        events.push(DomainEvent::CliProfileSet {
            node,
            profile: pid.to_string(),
        });
    }
    events
}

// ─────────────────────────── F1-4-3 · restore de terminais vivos ───────────────────────────

/// **Badge honesto do restore (F1-4-3).** Strings CONGELADAS de `copy-f1-4.md §4`, condicionadas
/// ao que o sistema OBSERVOU — nunca ao que o perfil prometeu (risco #5 da onda: "restore que
/// mente destrói a confiança"). O scrollback na tela persiste SEMPRE; o que o badge distingue é
/// se a MEMÓRIA do agente foi retomada (resume aplicado de fato) ou recomeça do zero.
//
// API pública consumida pela FIAÇÃO de boot do `main.rs` (costura externa F1-4-3: chamar
// `plan_restore` no startup, re-spawnar e renderizar o badge) — daí `dead_code` no binário
// até a costura entrar. Os testes headless (`bridge::tests`) já exercem todo o caminho.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreBadge {
    /// A retomada ACONTECEU: o motor declara resume E havia sessão anterior a retomar.
    Resumed,
    /// Motor sem resume — OU retomada não-aplicável (sem sessão anterior observável).
    FreshStart,
    /// 2026-06-11: o MOTOR não spawnou nesta abertura (ambiente — ex.: PATH sem o binário).
    /// O Agente re-ergue como terminal comum SEM perder a linhagem do CLI no log; o próximo
    /// boot re-tenta o motor. Antes disto o nó era DESCARTADO em silêncio (Dead sem sucessor
    /// = sumia do Espaço para sempre).
    EngineMissing,
}

#[allow(dead_code)] // ver nota em `RestoreBadge` (consumido pela costura de boot externa).
impl RestoreBadge {
    /// String congelada do badge no card (copy-f1-4 §4).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RestoreBadge::Resumed => "Sessão retomada",
            RestoreBadge::FreshStart => "Novo começo — o Agente não lembra da conversa anterior",
            RestoreBadge::EngineMissing => "A IA deste Agente não religou — reabra o Espaço",
        }
    }

    /// Texto ao passar o mouse (copy-f1-4 §4).
    #[must_use]
    pub fn hover(self) -> &'static str {
        match self {
            RestoreBadge::Resumed => "O Agente continua de onde vocês pararam.",
            RestoreBadge::FreshStart => {
                "A conversa de antes continua guardada aqui na tela — é só rolar para cima."
            }
            RestoreBadge::EngineMissing => {
                "O programa de IA não foi encontrado nesta abertura. Nada se perdeu: \
                 feche e reabra o Espaço para religar."
            }
        }
    }
}

/// **F1-4-3 — um terminal a re-erguer no quit limpo.** Derivado SÓ de estado durável: posição/
/// nome/papel/cwd do log (projeção), janela viva re-hidratada do `ScrollbackStore`, comando de
/// spawn do perfil (com o verbo de resume do TOML quando aplicável) e o badge honesto. Nenhum
/// PTY é tocado ao construir isto — é a ESPECIFICAÇÃO do restore, executável depois via `admit_node`.
#[allow(dead_code)] // ver nota em `RestoreBadge` (consumido pela costura de boot externa).
#[derive(Debug, Clone, PartialEq)]
pub struct RestoredTerminal {
    /// `NodeId` canônico do log — chave estável que sobrevive ao quit (mesma do scrollback).
    pub node: NodeId,
    pub name: String,
    pub role: Option<String>,
    pub x: f64,
    pub y: f64,
    pub cwd: Option<String>,
    /// `id` do CLI Profile resolvido no registry; `None` = shell puro / motor sem perfil.
    pub profile_id: Option<String>,
    /// Comando de spawn do restore: `[program, args…, resume_args…]`. Os `resume_args` SÓ
    /// entram quando `badge == Resumed` (observado). Vazio quando não há perfil (shell puro).
    pub command: Vec<String>,
    pub badge: RestoreBadge,
    /// Janela viva re-hidratada do store (ordem cronológica). Vazia se não havia histórico.
    pub scrollback_tail: Vec<String>,
    /// 2026-06-11: gerações `Dead` MAIS ANTIGAS do MESMO nome — aposentadas junto com o nó
    /// re-erguido (`NodeRemoved`). Sem isto, fechar o card depois ressuscitaria a próxima
    /// geração da fila no boot seguinte (cadeia de zumbis do resgate de órfãos).
    pub shadows: Vec<NodeId>,
}

/// **F1-4-3 — o PLANO de restore do quit limpo, derivável SÓ de estado durável.** Dado o estado
/// projetado do log (`proj` = `store.project()`), o registry de perfis e — opcional — o store de
/// scrollback, devolve os terminais a re-erguer: posições/nomes/papéis do log, scrollback
/// re-hidratado do disco, comando de spawn (com o verbo de resume do TOML quando o motor declara
/// E há sessão a retomar) e o badge honesto. **Função PURA** (sem efeito colateral além de leitura
/// do store): satisfaz "limpar projeções → replay → mesmo restore" (critério 4) e é testável
/// headless (critérios 1/2/3). A FIAÇÃO no boot (chamar isto e re-spawnar) + o render do badge na
/// tela moram no `main.rs` (território EXTERNO) — registrados como costura na entrega.
#[allow(dead_code)] // ver nota em `RestoreBadge` (consumido pela costura de boot externa).
#[must_use]
pub fn plan_restore(
    proj: &ProjectedState,
    registry: &ProfileRegistry,
    scrollback: Option<&Arc<Mutex<ScrollbackStore>>>,
) -> Vec<RestoredTerminal> {
    let mut out = Vec::new();
    let is_dead_terminal = |info: &lina_core::ProjectedNode| {
        info.kind.eq_ignore_ascii_case("terminal")
            && info.status.as_deref() == Some(lina_core::NodeStatus::Dead.as_str())
    };
    // 2026-06-11 (tela do fundador, 2ª reabertura) — RESGATE DE ÓRFÃOS por identidade-nome:
    // um `Dead` NOMEADO sem sucessor vivo de mesmo nome é uma VÍTIMA — a admissão dele falhou
    // num boot anterior (ex.: motor invisível no PATH degradado) e ninguém o re-ergueu nem o
    // aposentou. Pular TODO `Dead` apagava esses nós do Espaço PARA SEMPRE (23 terminais do
    // fundador). A identidade que atravessa gerações é o NOME (único no roster vivo por
    // construção): geração fechada DE VERDADE tem sucessor vivo de mesmo nome (e o supersede
    // a remove via `NodeRemoved`) — essa segue no passado; o órfão MAIS NOVO de cada nome
    // volta ao plano. `Dead` sem nome (gerações pré-ADR-0022, o nome nunca foi ao log) segue
    // fora — não há identidade para resgatar.
    let live_names: std::collections::BTreeSet<String> = proj
        .nodes
        .values()
        .filter(|i| i.kind.eq_ignore_ascii_case("terminal"))
        .filter(|i| i.status.as_deref() != Some(lina_core::NodeStatus::Dead.as_str()))
        .filter_map(|i| i.name.clone())
        .collect();
    let mut orphan_by_name: std::collections::BTreeMap<String, NodeId> =
        std::collections::BTreeMap::new();
    for (node, info) in &proj.nodes {
        if !is_dead_terminal(info) {
            continue;
        }
        let Some(name) = info.name.clone() else {
            continue;
        };
        if live_names.contains(&name) {
            continue;
        }
        let slot = orphan_by_name.entry(name).or_insert(*node);
        // NodeId é UUID v7 (cresce no tempo) → a geração MAIS NOVA do nome vence.
        if *node > *slot {
            *slot = *node;
        }
    }
    let rescued: std::collections::BTreeSet<NodeId> = orphan_by_name.into_values().collect();
    // Mapa nome → TODAS as gerações Dead (para as sombras do plano — ver `RestoredTerminal`).
    let mut dead_named: std::collections::BTreeMap<String, Vec<NodeId>> =
        std::collections::BTreeMap::new();
    for (node, info) in &proj.nodes {
        if is_dead_terminal(info) {
            if let Some(name) = info.name.clone() {
                dead_named.entry(name).or_default().push(*node);
            }
        }
    }
    for (node, info) in &proj.nodes {
        // Só TERMINAIS re-erguem aqui (notas/pastas têm outro ciclo de vida). Case-insensitive:
        // a admissão de produção grava "Terminal" (ADR 0022 §2), mas o filtro tolera variação.
        if !info.kind.eq_ignore_ascii_case("terminal") {
            continue;
        }
        // Geração já FECHADA no log (morte póstuma do `close_previous_generation` em boots
        // anteriores) fica no passado — a foto do boot é tirada ANTES do fechamento desta
        // geração, então quem estava vivo no último quit ainda NÃO está `Dead` aqui. Sem este
        // filtro, cada reabertura re-erguia TODAS as gerações empilhadas na mesma posição
        // (bug da tela de 2026-06-11: 23 cards em x=30,y=96, shell puro por baixo). A EXCEÇÃO
        // é o órfão resgatado acima — vítima, não geração fechada.
        if is_dead_terminal(info) && !rescued.contains(node) {
            continue;
        }
        let panel = node.to_string();
        // Re-hidrata a janela viva do disco (≥ janela viva, byte-idêntico). Erro de leitura
        // degrada para vazio (= sem histórico → caminho "novo começo"), nunca silencioso.
        let scrollback_tail = match scrollback {
            Some(s) => {
                let store = lock(s);
                let cap = store.cap();
                match store.tail(&panel, cap) {
                    Ok(lines) => lines,
                    Err(e) => {
                        eprintln!(
                            "lina-gpui: restore não leu scrollback do painel {panel}: {e} \
                             (re-hidrata vazio; badge cai para 'novo começo')"
                        );
                        Vec::new()
                    }
                }
            }
            None => Vec::new(),
        };
        let has_prior_session = !scrollback_tail.is_empty();

        // `info.cli` é o profile id quando houve `CliProfileSet` (reducer: último vence); senão
        // é o RÓTULO do motor ("Claude Code", gerações pré-CliProfileSet) ou o nome. Fallback
        // pelo slug do rótulo ("Claude Code" → "claude-code"): sem ele, um claude de geração
        // antiga re-erguia como shell puro silencioso (metade do bug "terminal empilhado").
        let resolved_id = info.cli.as_deref().and_then(|c| {
            if registry.get(c).is_some() {
                Some(c.to_string())
            } else {
                let slug = c.trim().to_lowercase().replace(' ', "-");
                registry.get(&slug).is_some().then_some(slug)
            }
        });
        let profile = resolved_id.as_deref().and_then(|id| registry.get(id));
        let profile_id = resolved_id;

        // Badge CONDICIONADO AO OBSERVADO (copy-f1-4 §4): declarar resume é NECESSÁRIO, nunca
        // SUFICIENTE — só «Sessão retomada» se há sessão anterior de fato (scrollback no disco).
        let resumes = profile.is_some_and(CliProfile::can_resume) && has_prior_session;
        let badge = if resumes {
            RestoreBadge::Resumed
        } else {
            RestoreBadge::FreshStart
        };

        let command = match profile {
            Some(p) => {
                let mut c = vec![p.program.clone()];
                c.extend(p.args.iter().cloned());
                if resumes {
                    c.extend(p.resume_args.iter().cloned());
                }
                c
            }
            None => Vec::new(),
        };

        let shadows: Vec<NodeId> = info
            .name
            .as_deref()
            .and_then(|n| dead_named.get(n))
            .map(|gens| gens.iter().copied().filter(|s| s != node).collect())
            .unwrap_or_default();
        out.push(RestoredTerminal {
            node: *node,
            name: info.name.clone().unwrap_or_default(),
            role: info.role.clone(),
            x: info.x,
            y: info.y,
            cwd: info.cwd.clone(),
            profile_id,
            command,
            badge,
            scrollback_tail,
            shadows,
        });
    }
    out
}

/// **F1-4-3 — EXECUTA o plano de restore (a outra metade de [`plan_restore`]).** Para cada
/// [`RestoredTerminal`]: re-admite pelo funil único (`admit_node` — mesma sequência canônica
/// de eventos do ⌘T), re-hidrata o grid com a janela viva persistida (a conversa de antes
/// "continua na tela — é só rolar para cima", copy-f1-4 §4) e registra o badge honesto na
/// projeção (`SharedModel::restore_badges`). Falha de UM terminal degrada com log e segue
/// (inv#6 — o restore parcial vale mais que nenhum). Devolve quantos re-ergueram.
impl NodeManager {
    /// O store de scrollback do workspace (compartilhado com a fiação de boot — `plan_restore`
    /// lê a janela viva dele). `None` = boot degradado sem persistência de histórico.
    #[must_use]
    pub fn scrollback(&self) -> Option<Arc<Mutex<ScrollbackStore>>> {
        self.scrollback.clone()
    }

    pub fn restore_terminals(&self, plans: &[RestoredTerminal]) -> usize {
        let mut up = 0;
        // 2026-06-11 — re-layout anti-colisão: o log herdou posições da era pré-fix (49 cards
        // em 8 coordenadas; até 10 no MESMO ponto — "um terminal em cima do outro" na tela do
        // fundador). O PRIMEIRO ocupante de cada coordenada a mantém; os demais caem para
        // `next_free_slot` (position=None na admissão) e a posição NOVA persiste no
        // `NodeAdded` — a correção é durável, não cosmética.
        let mut taken_slots: std::collections::BTreeSet<(i64, i64)> =
            std::collections::BTreeSet::new();
        for plan in plans {
            let slot = (plan.x.round() as i64, plan.y.round() as i64);
            let position = taken_slots.insert(slot).then_some((plan.x, plan.y));
            // Comando do restore → motor (mesmo caminho do M6): `[program, args…]` do perfil,
            // já com o verbo de resume quando o badge observou sessão anterior. Vazio = shell
            // puro (fábrica do workspace).
            let engine = plan
                .command
                .split_first()
                .map(|(program, args)| AgentEngine {
                    program: program.clone(),
                    args: args.to_vec(),
                    profile_id: plan.profile_id.clone(),
                    label: plan.profile_id.clone().unwrap_or_else(|| plan.name.clone()),
                });
            let had_engine = engine.is_some();
            let build_admission = |engine: Option<AgentEngine>| NodeAdmission {
                name: (!plan.name.is_empty()).then(|| plan.name.clone()),
                role: plan.role.clone().unwrap_or_else(|| "terminal".into()),
                engine,
                // cwd da sessão anterior já foi consentido quando o nó nasceu (o log o guarda);
                // re-entrar na MESMA pasta não é um consentimento novo.
                cwd: match &plan.cwd {
                    Some(p) => CwdPolicy::UserDir {
                        path: PathBuf::from(p),
                        consent: true,
                    },
                    None => CwdPolicy::Managed,
                },
                position, // colisão de coordenada → None → `next_free_slot` (re-layout durável)
                requested_by: None, // restore é gesto do BOOT (origem humana: reabrir o app)
                autonomy: Autonomy::Assisted,
            };
            // Reescrita do kit em LOTE (1× após o loop): com N restores, o rewrite por-admissão
            // era O(N²) de I/O — o vilão do boot lento medido em 2026-06-11 (~80 nós).
            let mut badge = plan.badge;
            let node = match self.admit_node_inner(build_admission(engine), false) {
                Ok(n) => n,
                // 2026-06-11 (tela do fundador): o spawn do MOTOR falha quando o ambiente
                // degrada (PATH sem o binário — hidratação do shell de login estourou o
                // timeout). Falha TRANSITÓRIA não pode virar dano permanente: re-ergue como
                // terminal comum, preservando nome/papel/cwd/posição; a linhagem do CLI é
                // re-apendada abaixo e o próximo boot re-tenta o motor.
                Err(e) if had_engine => {
                    eprintln!(
                        "lina-gpui: restore de '{}' com o motor falhou ({e}); re-erguendo \
                         como terminal comum (o motor volta no próximo boot)",
                        plan.name
                    );
                    match self.admit_node_inner(build_admission(None), false) {
                        Ok(n) => {
                            badge = RestoreBadge::EngineMissing;
                            n
                        }
                        Err(e2) => {
                            eprintln!(
                                "lina-gpui: restore de '{}' falhou ({e2}); seguindo sem ele",
                                plan.name
                            );
                            continue;
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "lina-gpui: restore de '{}' falhou ({e}); seguindo sem ele",
                        plan.name
                    );
                    continue;
                }
            };
            // Linhagem do CLI do nó DEGRADADO: o log continua declarando o profile (intenção
            // do nó), senão o próximo `plan_restore` o veria como shell PARA SEMPRE.
            if badge == RestoreBadge::EngineMissing {
                if let Some(pid) = &plan.profile_id {
                    if let Err(e) = lock(&self.store).append(&DomainEvent::CliProfileSet {
                        node,
                        profile: pid.clone(),
                    }) {
                        eprintln!(
                            "lina-gpui: CRÍTICO — linhagem de CLI de '{}' não persistiu ({e}); \
                             o próximo boot pode re-erguê-lo sem motor",
                            plan.name
                        );
                    }
                }
            }
            // SUPERSEDE: o nó da geração anterior é APOSENTADO no log — o re-erguido é um nó
            // NOVO (Dead é terminal, ADR 0020) e sem o `NodeRemoved` o antigo re-erguia DE NOVO
            // a cada boot (canvas dobrando, cards empilhados). Auditável: o remove fica no log.
            if let Err(e) = lock(&self.store).append(&DomainEvent::NodeRemoved { node: plan.node })
            {
                eprintln!(
                    "lina-gpui: CRÍTICO — '{}' re-erguido (node {node}) mas o antigo {} NÃO foi \
                     aposentado no log: {e} (pode reaparecer duplicado no próximo boot)",
                    plan.name, plan.node
                );
            }
            // Sombras: gerações Dead mais antigas do MESMO nome se aposentam JUNTO — senão
            // fechar o card depois ressuscita a próxima da fila no boot seguinte.
            for shadow in &plan.shadows {
                if let Err(e) =
                    lock(&self.store).append(&DomainEvent::NodeRemoved { node: *shadow })
                {
                    eprintln!(
                        "lina-gpui: sombra {shadow} de '{}' não aposentada no log ({e})",
                        plan.name
                    );
                }
            }
            // Re-hidrata o grid: a janela viva entra ANTES do output novo do shell — o leigo
            // reabre e a conversa está lá. CRLF por linha (o VT trata como output normal).
            if !plan.scrollback_tail.is_empty() {
                if let Some(grid) = lock(&self.grids).get(&node) {
                    let mut bytes = Vec::new();
                    for line in &plan.scrollback_tail {
                        bytes.extend_from_slice(line.as_bytes());
                        bytes.extend_from_slice(b"\r\n");
                    }
                    lock(grid).advance(&bytes);
                }
            }
            // Badge honesto na projeção (render lê daqui; chave = nó NOVO re-erguido).
            {
                let mut m = lock(&self.model);
                m.restore_badges.insert(node, badge);
                m.touch();
            }
            eprintln!(
                "lina-gpui: [RESTORE] '{}' re-erguido ({}; {} linhas re-hidratadas)",
                plan.name,
                badge.label(),
                plan.scrollback_tail.len()
            );
            up += 1;
        }
        // A reescrita adiada do lote: TODOS os kits ganham o roster completo (e a porta de
        // hooks DESTE boot) numa passada só — ver nota no `admit_node_inner(…, false)` acima.
        if up > 0 {
            self.rewrite_bootstrap();
        }
        up
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
    /// Rodada 360 (ADR 0022 §4): a POLÍTICA de cwd de cada nó admitido — o `rewrite_bootstrap`
    /// itera por ela (kit no cwd REAL para `UserDir` consentido; nada para não-consentido —
    /// FIM do dir-fantasma `t{seq}`). Nó ausente do mapa (seeds legados de teste) = `Managed`.
    cwds: Mutex<BTreeMap<NodeId, CwdPolicy>>,
    delta_tx: Sender<GridDelta>,
    cmd_factory: CmdFactory,
    cols: u16,
    rows: u16,
    seq: Mutex<u32>,
    /// W3-2: se presente, gera/reescreve o `CLAUDE.md` por terminal a cada mudança de roster.
    bootstrap: Option<BootstrapWriter>,
    /// W4-2 (M3/M4): o `.lina/` do workspace — onde os criadores persistem `notes/` e `folders/`.
    lina_dir: PathBuf,
    /// FIX-A3: fila EM-PROCESSO de re-injeção de doutrina (produtor: `assign_role`). Compartilhada
    /// com a `MailboxPump` (consumidor) via [`NodeManager::reinject_queue`] — substitui a mailbox
    /// de filesystem, que era drop-zone gravável por qualquer agente do mesmo usuário de SO.
    reinject_queue: ReinjectQueue,
    /// #7 dogfooding r2: o MOTOR de spawns agente-pede. Default de produção = discovery
    /// ([`default_spawn_engine`]); os testes injetam fixa via [`Self::set_spawn_engine_factory`].
    spawn_engine: SpawnEngineFactory,
    /// F1-5-2: o store durável de scrollback do workspace (`<lina_dir>/scrollback.db`). Todo
    /// terminal admitido a partir daqui nasce capturando — o cabo `append-on-scroll` é ligado
    /// em [`wire_terminal_capturing`]. `None` = boot degradado (abertura do store falhou; o app
    /// segue sem persistência de histórico, nunca crasha — degradação honesta como o bootstrap).
    scrollback: Option<Arc<Mutex<ScrollbackStore>>>,
    /// F1-5-6: o job ÚNICO de durabilidade sobre o `scrollback` (idle-drain + sinais fatais +
    /// retenção diária F1-5-9). Vive aqui porque o `NodeManager` é dono do ciclo de vida do
    /// store no app; para no Drop. `None` quando não há store (nada a proteger).
    #[allow(dead_code)] // mantido vivo só pelo Drop (RAII): para o job ao encerrar o manager.
    flush_guard: Option<FlushGuard>,
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
        // F1-5-2: abre o store durável de scrollback no MESMO lar de dados do workspace
        // (`<lina_dir>/scrollback.db`, arquivo separado do events log) e sobe o job único de
        // durabilidade (F1-5-6). Best-effort: falha de disco degrada (sem persistência), nunca
        // crasha o boot — mesma doutrina do `bootstrap` desligado. Os handlers de sinal só
        // entram em produção (`!cfg!(test)`): num binário de teste, instalar/desinstalar
        // handlers processo-globais a cada `NodeManager` é ruído desnecessário.
        let (scrollback, flush_guard) = match ScrollbackStore::open_default(&lina_dir) {
            Ok(store) => {
                let store = Arc::new(Mutex::new(store));
                let guard = FlushGuard::start(
                    Arc::clone(&store),
                    FlushGuardConfig {
                        handle_signals: !cfg!(test),
                        ..FlushGuardConfig::default()
                    },
                )
                .map_err(|e| {
                    eprintln!(
                        "lina-gpui: scrollback flush-guard não subiu ({e}); histórico ainda \
                         persiste por flush/drop, mas sem idle-drain/sinais neste boot"
                    );
                })
                .ok();
                (Some(store), guard)
            }
            Err(e) => {
                eprintln!(
                    "lina-gpui: scrollback store não abriu em {} ({e}); boot sem persistência \
                     de histórico (degradação honesta)",
                    lina_dir.display()
                );
                (None, None)
            }
        };
        Self {
            pty,
            sup,
            store,
            model,
            grids,
            keys: Mutex::new(keys),
            cwds: Mutex::new(BTreeMap::new()),
            delta_tx,
            cmd_factory,
            cols,
            rows,
            seq: Mutex::new(seq_start),
            bootstrap,
            lina_dir,
            reinject_queue: new_reinject_queue(),
            // #7: produção resolve o motor do spawn por DISCOVERY no momento do spawn (sem
            // mudança de assinatura — o main não precisa de fiação para o default correto).
            spawn_engine: Arc::new(|| default_spawn_engine(&discover_clis())),
            scrollback,
            flush_guard,
        }
    }

    /// **#7 — injeta a fábrica do motor de spawn** (testes: determinismo — a discovery real
    /// leria o `PATH` do runner e lançaria o CLI de verdade; fiação futura: o boot pode fixar
    /// o motor resolvido com o `ProfileRegistry` para o spawn ganhar `CliProfileSet`).
    // Chamadores hoje: só os harnesses de teste (o default de produção já é o correto, sem
    // fiação no main) — `allow` consciente até a costura do registry, não dead code esquecido.
    #[allow(dead_code)]
    pub fn set_spawn_engine_factory(&mut self, f: SpawnEngineFactory) {
        self.spawn_engine = f;
    }

    /// FIX-A3: a fila EM-PROCESSO de re-injeção de doutrina, compartilhada (clone de `Arc`) com a
    /// [`MailboxPump`] (consumidor). O `main.rs` a obtém daqui e a passa ao pump após criar o
    /// manager — é o canal produtor→consumidor sem superfície de filesystem.
    #[must_use]
    pub(crate) fn reinject_queue(&self) -> ReinjectQueue {
        Arc::clone(&self.reinject_queue)
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

    /// Rodada 360: como [`Self::terminals_snapshot`], com a POLÍTICA de cwd de cada nó
    /// (ausente do mapa = `Managed`, o comportamento histórico dos seeds).
    fn terminals_snapshot_with_policy(&self) -> Vec<(String, String, CwdPolicy)> {
        let keys = lock(&self.keys);
        let cwds = lock(&self.cwds);
        let m = lock(&self.model);
        m.order
            .iter()
            .filter_map(|n| {
                let k = keys.get(n)?;
                let v = m.nodes.get(n)?;
                let policy = cwds.get(n).cloned().unwrap_or(CwdPolicy::Managed);
                Some((k.clone(), v.name.clone(), policy))
            })
            .collect()
    }

    /// W3-2: reescreve o `CLAUDE.md`/estado de TODOS os terminais (o arquivo é a âncora a cada
    /// mudança de roster). No-op se o bootstrap não está configurado.
    ///
    /// Rodada 360 (ADR 0022 §4): itera o binding node→cwd-REAL — nó `UserDir` consentido recebe
    /// o kit merge-safe NA PASTA DELE; não-consentido não recebe NADA (fim do dir-fantasma
    /// `<ws_root>/t{seq}` órfão que ninguém lia). `Managed` segue o caminho histórico.
    pub fn rewrite_bootstrap(&self) {
        let Some(bw) = &self.bootstrap else { return };
        let snapshot = self.terminals_snapshot_with_policy();
        // O roster lista TODOS os colegas (inclusive os sem kit — eles existem no time;
        // o que lhes falta é a doutrina local, não a cidadania).
        let roster: Vec<String> = snapshot.iter().map(|(_, n, _)| n.clone()).collect();
        for (key, name, policy) in &snapshot {
            match policy {
                CwdPolicy::Managed => bw.write_one(key, name, &roster),
                CwdPolicy::UserDir {
                    path,
                    consent: true,
                } => {
                    let _ = bw.write_user_dir(path, name, &roster);
                }
                CwdPolicy::UserDir { consent: false, .. } => {}
                // Terminal PURO: NUNCA escreve no HOME do usuário (nem na admissão, nem aqui no
                // rewrite por mudança de roster). Continua listado como colega no `roster`, mas
                // o home dele fica intocado — invariante do `+ Terminal`.
                CwdPolicy::UserHome { .. } => {}
            }
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

    /// **F1-2-4 (M6-E)** — atribui/troca o PAPEL de um nó vivo: persiste `NodeRoleAssigned` (a
    /// projeção/`lina list --json` refletem), reescreve os `CLAUDE.md` (roster) e **ENFILEIRA a
    /// re-injeção da doutrina** na fila serial de escrita do nó (`reinject`). Quem ENTREGA é a
    /// [`MailboxPump`] (escritor único), respeitando o FREIO W4-3 — pausado, a re-injeção fica
    /// represada (durável, nada se perde, nada se injeta sob pausa). É chamado SÓ no fluxo de
    /// edição confirmada (M6-E pergunta antes de trocar papel — nunca troca silenciosa).
    ///
    /// O `NodeId` NÃO respawna (o PID é preservado — critério 1): a mudança é só evento + roster.
    pub fn assign_role(&self, node: NodeId, role: &str) -> Result<(), String> {
        let role = role.trim();
        if role.is_empty() || role.chars().any(char::is_control) {
            return Err(format!("papel inválido: {role:?}"));
        }
        let count = {
            let mut s = lock(&self.store);
            s.append(&DomainEvent::NodeRoleAssigned {
                node,
                role: role.to_string(),
            })
            .map_err(|e| e.to_string())?;
            s.event_count().ok()
        };
        {
            let mut m = lock(&self.model);
            if let Some(c) = count {
                m.event_count = c;
            }
            m.touch();
        }
        self.rewrite_bootstrap();
        // FIX-A3: enfileira a re-injeção na FILA EM-PROCESSO com o `NodeId` AUTENTICADO (`node`, que
        // já temos aqui — sem round-trip por nome/string forjável). O texto é REGENERADO no consumidor
        // a partir do `role`. Nenhuma superfície de filesystem: nenhum agente alcança esta fila. O nó
        // acabou de ser persistido (`NodeRoleAssigned`); se sumir do roster antes do drain, o consumidor
        // descarta com log (nunca injeta às cegas).
        enqueue_doctrine_reinjection(&self.reinject_queue, node, role);
        Ok(())
    }

    /// O `.lina/` do Espaço vivo (F1-2-2: os CLI Profiles do usuário moram em `<aqui>/profiles`).
    #[must_use]
    pub fn lina_home(&self) -> &std::path::Path {
        &self.lina_dir
    }

    /// Handle do event store vivo (clone do `Arc`) — p/ a janela de Ajustes aberta SOB DEMANDA
    /// (fix de tela F1-2-1: o painel era env-gated e o tema ficava inalcançável em produção).
    #[must_use]
    pub fn store_handle(&self) -> Arc<Mutex<EventStore>> {
        Arc::clone(&self.store)
    }

    /// **F1-2-2 (M6-E)** — o papel CANÔNICO corrente de um nó, lido da PROJEÇÃO do log (o
    /// `NodeView` de UI não carrega role/cli — projection-only, lição do W4-2). Replay sob
    /// demanda: chamado só ao ABRIR o modal de edição, nunca por frame.
    #[must_use]
    pub fn node_role(&self, node: NodeId) -> Option<String> {
        lock(&self.store)
            .project()
            .ok()
            .and_then(|st| st.nodes.get(&node).and_then(|n| n.role.clone()))
    }

    /// **F1-2-3 p2 (seam p/ o modal de papéis)** — persiste um papel CUSTOM do usuário:
    /// append de `RoleTemplateSaved` no event log (fonte da verdade; o modal reconstrói a
    /// biblioteca varrendo o log). Idioma de erro do [`Self::create_agent_with`]: mensagem
    /// LEIGA no `Err`, detalhe técnico no stderr.
    pub fn save_role_template(
        // SEAM-1: `&self` (era `&mut self` desnecessário) — só apenda via `store` (Arc<Mutex>,
        // mutabilidade interior); destrava o uso por `Arc<NodeManager>` compartilhado.
        &self,
        name: &str,
        description: &str,
        doctrine: &str,
    ) -> Result<(), String> {
        let mut s = lock(&self.store);
        if let Err(e) = s.append(&DomainEvent::RoleTemplateSaved {
            name: name.to_string(),
            description: description.to_string(),
            doctrine: doctrine.to_string(),
            ts: now_ms(),
        }) {
            eprintln!("lina-gpui: save_role_template — append falhou: {e}");
            return Err("não consegui salvar seu papel personalizado".to_string());
        }
        Ok(())
    }

    /// **F1-2-3 p2 (seam p/ o modal de papéis)** — os papéis CUSTOM como REAPARECEM do log:
    /// EM ORDEM DE LOG, SEM dedup (o modal aplica o last-wins — doutrina do evento no core).
    /// Replay sob demanda (ao abrir o modal), nunca por frame. Log ilegível → lista vazia
    /// (best-effort, detalhe no stderr — o modal segue só com a biblioteca embutida).
    #[must_use]
    pub fn role_templates(&self) -> Vec<crate::agent_modal::RoleTemplate> {
        match lock(&self.store).events() {
            Ok(records) => role_templates_from_records(&records),
            Err(e) => {
                eprintln!("lina-gpui: role_templates — leitura do log falhou: {e}");
                Vec::new()
            }
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
    /// Rodada 360 (ADR 0022): estado projetado do log para consumidores de UI — o painel P6
    /// correlaciona node↔cwd↔sessão via `ProjectedNode.cwd`. Replay completo (snapshot + tail):
    /// o CHAMADOR cacheia por [`Self::store_event_count`] (padrão `refresh_cost_paused`) —
    /// não chame por frame.
    pub fn projected(&self) -> Option<ProjectedState> {
        lock(&self.store).project().ok()
    }

    /// `event_count` do log — chave de cache barata para consumidores de [`Self::projected`]
    /// (re-replay SÓ quando há evento novo).
    pub fn store_event_count(&self) -> Option<u64> {
        lock(&self.store).event_count().ok()
    }

    /// **Porta ⌘T / botão + (tradutor fino — ADR 0022 §1).** Terminal default com nome
    /// automático ("Terminal A/B/C…", derivado do seq DENTRO do funil). Toda a admissão —
    /// eventos, kit, projeção, compensação — vive no [`Self::admit_node`].
    pub fn add_node(&self) -> Result<NodeId, String> {
        self.admit_node(NodeAdmission::default_terminal())
    }

    /// **Porta ⌘N "Novo Agente" (tradutor fino — ADR 0022 §1).** Traduz o `CreatePlan` do
    /// modal M6 em [`NodeAdmission`]: deriva o PAPEL do nome (role-discovery W3-1) salvo
    /// `role_override`; `engine`/`cwd` viram plano. `kit_consent` = o usuário autorizou no
    /// modal a Lina criar os arquivos de orquestração na pasta DELE (ADR 0022 §4) — sem
    /// efeito quando `cwd = None` (pasta gerenciada sempre recebe o kit completo).
    // FIX-3 moveu o modal p/ `create_agent_with_autonomy`; este wrapper segue como porta dos
    // call-sites de teste/seed (`allow` consciente — não dead code esquecido).
    #[allow(dead_code)]
    pub fn create_agent_with(
        &self,
        name: &str,
        engine: Option<&AgentEngine>,
        cwd: Option<&std::path::Path>,
        role_override: Option<&str>,
        kit_consent: bool,
    ) -> Result<NodeId, String> {
        // Autonomia DEFAULT do produto (`Assisted`); a porta ⌘N com a escolha EXPLÍCITA do
        // usuário é [`Self::create_agent_with_autonomy`] (usada pelo modal M6). Este wrapper
        // mantém os call-sites existentes (seed/testes) intocados — tradutor fino (ADR 0022 §1).
        self.create_agent_with_autonomy(
            name,
            engine,
            cwd,
            role_override,
            kit_consent,
            Autonomy::Assisted,
        )
    }

    /// **Porta ⌘N com a AUTONOMIA escolhida no modal (FIX-3 costura per-Agente).** Igual a
    /// [`Self::create_agent_with`], mas propaga `autonomy` ao plano → o funil `admit_node`
    /// carimba `LINA_AUTONOMY` no env do PTY (o guard `lina guard --pretooluse` honra o nível
    /// DESTE nó) e a projeção reflete o nível no badge do card (`NodeView.autonomy`).
    pub fn create_agent_with_autonomy(
        &self,
        name: &str,
        engine: Option<&AgentEngine>,
        cwd: Option<&std::path::Path>,
        role_override: Option<&str>,
        kit_consent: bool,
        autonomy: Autonomy,
    ) -> Result<NodeId, String> {
        let name = name.trim();
        let role = match role_override.map(str::trim).filter(|r| !r.is_empty()) {
            Some(r) => r.to_string(),
            None => infer_agent_role(name),
        };
        self.admit_node(NodeAdmission {
            name: Some(name.to_string()),
            role,
            engine: engine.cloned(),
            cwd: match cwd {
                Some(p) => CwdPolicy::UserDir {
                    path: p.to_path_buf(),
                    consent: kit_consent,
                },
                None => CwdPolicy::Managed,
            },
            position: None,
            requested_by: None, // criação pelo modal humano (⌘N) — não é spawn agente-pede
            autonomy,
        })
    }

    /// FIX-1: os nós VIVOS cujo cwd de TRABALHO coincide com o do `plan` (mesmo projeto do
    /// usuário). Só `UserDir` conta ([`shared_cwd_path`]); vazio quando o `plan` não é
    /// `UserDir` ou ninguém mais está na pasta. Lido ANTES de o novo nó entrar em `self.cwds`
    /// (passo 7 do funil) → enxerga só os JÁ vivos.
    fn live_nodes_sharing_cwd(&self, plan_cwd: &CwdPolicy) -> Vec<NodeId> {
        let Some(candidate) = shared_cwd_path(plan_cwd) else {
            return Vec::new();
        };
        lock(&self.cwds)
            .iter()
            .filter_map(|(node, policy)| {
                (shared_cwd_path(policy) == Some(candidate)).then_some(*node)
            })
            .collect()
    }

    /// **O FUNIL ÚNICO de admissão (ADR 0022 §1) — o único lugar do app que transforma a
    /// intenção "quero um terminal" em nó vivo.** Encapsula:
    ///
    /// - validação de nome + alocação de seq/key;
    /// - política de cwd (§4): `Managed` = kit completo no dir gerenciado; `UserDir` = kit
    ///   merge-safe no cwd REAL mediante consentimento, senão badge VISÍVEL (nunca silencioso);
    /// - write-before-spawn + `wire_terminal`;
    /// - a sequência CANÔNICA e COMPLETA de eventos (§2): `NodeAdded`, `TerminalSpawned`
    ///   com o cwd REAL, `NodeRoleAssigned` SEMPRE ("terminal" também é papel) e
    ///   `CliProfileSet` quando o profile é conhecido;
    /// - persiste-antes-de-projetar com COMPENSAÇÃO em falha de append (§5): `retire_pty`
    ///   (unregister + kill) — o nó nunca existe no roster vivo sem existir no log;
    /// - projeção atômica (model/grids/keys/cwds) + rewrite do roster.
    ///
    /// Os entry points ([`Self::add_node`], [`Self::create_agent_with`], o seed do demo no
    /// `main`) são tradutores finos de intenção → [`NodeAdmission`] — proibidos de apendar
    /// eventos ou tocar o Supervisor diretamente.
    pub fn admit_node(&self, plan: NodeAdmission) -> Result<NodeId, String> {
        self.admit_node_inner(plan, true)
    }

    /// O corpo do funil. `rewrite_roster=false` SÓ no restore (F1-4-3): N admissões em lote
    /// adiam a reescrita dos kits para UMA passada ao final (`restore_terminals`) — por-admissão
    /// era O(N²) de I/O no boot. Qualquer outra porta usa `admit_node` (reescreve sempre).
    fn admit_node_inner(
        &self,
        plan: NodeAdmission,
        rewrite_roster: bool,
    ) -> Result<NodeId, String> {
        // 1) seq/key — a identidade local do PTY (o NodeId canônico nasce no register).
        let seq = {
            let mut s = lock(&self.seq);
            let v = *s;
            *s = s.wrapping_add(1);
            v
        };
        // O dir-casa gerenciado (`dir_for(key) = <ws_root>/<key>`) é a CHAVE de identidade de
        // projeto do CLI (ex.: `~/.claude/projects/<slug-do-cwd>` + histórico). Por isso a key
        // precisa ser ÚNICA e NÃO-RECICLÁVEL entre boots: `seq` REINICIA em 0 a cada abertura do
        // app (main: `seq_start=0`), mas os dirs PERSISTEM com o conteúdo da sessão anterior —
        // um `t{seq}` reusado faria o agente novo `ls` e CONTINUAR o projeto errado. UUID v7
        // (monotônico+único globalmente; já dep do app) nasce VIRGEM por construção. O `seq`
        // fica SÓ para o rótulo cosmético (`node_label` → "Terminal A/B/C…"), nunca para a
        // identidade de filesystem. O cwd REAL persiste no `TerminalSpawned.cwd` (restore lê
        // de lá, não recalcula a key).
        let key = format!("n-{}", uuid::Uuid::now_v7());

        // 2) Validação de nome (regra ÚNICA do funil — antes espalhada por entry point).
        let name = match &plan.name {
            Some(n) => {
                // #6 dogfooding r2: o `@` é ENDEREÇAMENTO, não nome — o `lina spawn` chega
                // com o alvo "@Nome" (msg.to) e o nó nascia com o sigil EMBUTIDO (seq 20326:
                // "@Especialista em Licenças"; superfícies exibiam "@@Nome" ao endereçar).
                // Normaliza no FUNIL (vale para TODAS as portas) ANTES de validar; "@"/"@@"
                // puros viram vazio e caem na recusa de nome inválido.
                let n = n.trim().trim_start_matches('@').trim();
                if n.is_empty()
                    || n.chars().any(char::is_control)
                    || n.contains("{{")
                    || n.contains("}}")
                {
                    return Err(format!("nome de agente inválido: {n:?}"));
                }
                n.to_string()
            }
            // Porta ⌘T: nome automático derivado do seq (rótulo cosmético; a key de
            // filesystem é única/não-reciclável — ver o bloco da `key` acima). F1-4-3: o seq
            // reinicia a cada boot, mas o restore re-ergue nós que JÁ se chamam "Terminal X"
            // (nome persistido no log) — pula os tomados, senão o outbox/A2A ficam ambíguos.
            None => {
                let taken: std::collections::BTreeSet<String> = self
                    .terminals_snapshot()
                    .into_iter()
                    .map(|(_, n)| n)
                    .collect();
                let mut candidate = format!("Terminal {}", node_label(seq));
                while taken.contains(&candidate) {
                    let next = {
                        let mut s = lock(&self.seq);
                        let v = *s;
                        *s = s.wrapping_add(1);
                        v
                    };
                    candidate = format!("Terminal {}", node_label(next));
                }
                candidate
            }
        };
        let role = plan.role.trim().to_string();
        if role.is_empty() || role.chars().any(char::is_control) {
            return Err(format!("papel inválido: {role:?}"));
        }

        // 3) Comando: motor escolhido (M6) ou fábrica default — TODO nó nasce com a IDENTIDADE
        //    no env (ADR 0026): `VIBE_ROLE` ("terminal" também é papel) + `LINA_NODE_NAME`/
        //    `LINA_NODE_ID`. O env do spawn é autoridade do APP — com N nós no MESMO cwd
        //    (F1-4-1 `default_cwd`), a ficha `.lina/bootstrap.json` é último-escritor-vence;
        //    a identidade POR-PROCESSO do env não colide (bug de dogfooding: o MAESTRO se via
        //    como "Automatizador" no whoami/outbox).
        let mut cmd = node_identity_env(
            match &plan.engine {
                Some(e) => {
                    let mut c = PtyCommand::new(&e.program);
                    for a in &e.args {
                        c = c.arg(a);
                    }
                    c
                }
                None => (*self.cmd_factory)(&name),
            },
            &name,
            &role,
            &key,
            plan.autonomy,
        );

        // 4) Política de cwd (§4) + kit write-before-spawn (o shell já encontra o CLAUDE.md).
        //    `cwd_real` é o que o `TerminalSpawned` persiste — o binding node↔cwd↔sessão (§3).
        let mut kit_missing = false;
        let cwd_real: Option<PathBuf> = match &plan.cwd {
            CwdPolicy::Managed => {
                let dir = self.ensure_cwd(&key);
                match &dir {
                    Some(dir) => {
                        cmd = cmd.cwd(dir.clone());
                        // Só o nó novo aqui — os existentes são atualizados no `rewrite_bootstrap()`
                        // ao final; se o spawn falhar, eles não ganham um colega fantasma.
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
                    None => {
                        // inv#6 (red-team 360, MÉDIA): `ensure_cwd == None` ⟺ `self.bootstrap`
                        // é `None` — boot degradado (`BootstrapWriter::new` falhou no main, `.ok()`).
                        // O nó nasce sem kit e com cwd herdado do app; o fundador PRECISA ver isso
                        // (mesmo badge dos ramos UserDir), nunca um Espaço meio-degradado invisível.
                        kit_missing = true;
                        eprintln!(
                            "lina-gpui: agente '{name}' (pasta gerenciada) sem bootstrap neste \
                             boot — sem doutrina/observabilidade; o card mostra o aviso"
                        );
                    }
                }
                dir
            }
            CwdPolicy::UserHome { path } => {
                // Terminal PURO (pedido do fundador): abre no HOME do usuário, como o
                // Terminal.app. ZERO arquivos do Lina escritos — NÃO cria dir gerenciado,
                // NÃO escreve kit/doutrina/`.claude`/`settings.json` no home (não poluir o
                // diretório pessoal). `kit_missing` fica FALSE: não é um agente degradado, é um
                // shell limpo por design (sem badge "sem doutrina/observabilidade"). O cwd REAL
                // (= home) ainda persiste no `TerminalSpawned.cwd` (binding §3).
                cmd = cmd.cwd(path.clone());
                Some(path.clone())
            }
            CwdPolicy::UserDir { path, consent } => {
                if let Err(e) = std::fs::create_dir_all(path) {
                    return Err(format!("não consigo trabalhar nessa pasta: {e}"));
                }
                cmd = cmd.cwd(path.clone());
                if *consent {
                    match &self.bootstrap {
                        Some(bw) => {
                            let mut roster: Vec<String> = self
                                .terminals_snapshot()
                                .into_iter()
                                .map(|(_, n)| n)
                                .collect();
                            roster.push(name.clone());
                            // Kit no cwd REAL, merge-safe: arquivo pré-existente do usuário é
                            // PRESERVADO (aviso + badge) — jamais sobrescrito (§4).
                            if bw.write_user_dir(path, &name, &roster) != KitOutcome::Full {
                                kit_missing = true;
                            }
                        }
                        None => {
                            // Consentiu mas o bootstrap do boot DEGRADOU → o kit não tem como
                            // ser entregue. Badge liga do mesmo jeito — nunca silencioso (§4).
                            kit_missing = true;
                            eprintln!(
                                "lina-gpui: agente '{name}' consentiu o kit, mas o bootstrap \
                                 está desativado neste boot — sem doutrina/observabilidade"
                            );
                        }
                    }
                } else {
                    // Sem consentimento → degradação VISÍVEL (badge no card), nunca silenciosa.
                    kit_missing = true;
                    eprintln!(
                        "lina-gpui: agente '{name}' em pasta própria SEM kit de integração \
                         (sem consentimento) — sem doutrina/observabilidade nesta pasta"
                    );
                }
                Some(path.clone())
            }
        };

        // FIX-1 (dogfooding 2026-06-10): o cwd de TRABALHO já pertence a outro nó VIVO? O caso
        // de uso central — um time inteiro no projeto do usuário (default F1-4-1) — põe N
        // agentes no MESMO `UserDir`. A identidade A2A é isolada pelo env (ADR 0026); aqui a
        // DEFESA ESTRUTURAL DETECTA o compartilhamento e o SINALIZA ao leigo (badge no card),
        // nunca surpresa silenciosa. (O isolamento por-nó da OBSERVABILIDADE — token HTTP no
        // `.claude/settings.json` lido por cwd — é story dedicada: toca o contrato F1-1-3 + a
        // neutralidade multi-CLI; ver `.entrega-fix1.md`.)
        let cwd_sharers = self.live_nodes_sharing_cwd(&plan.cwd);
        let cwd_shared = !cwd_sharers.is_empty();

        // 5) Slot calculado ANTES de o nó existir: imune ao placeholder (0,0) que o pump cria.
        //    Posição FIXA só na porta seed/demo (roteiro determinístico).
        let (x, y) = match plan.position {
            Some((px, py)) => (px as f32, py as f32),
            None => next_free_slot(&lock(&self.model)),
        };

        let (node, grid) = {
            let mut p = lock(&self.pty);
            wire_terminal_capturing(
                &mut p,
                &self.sup,
                &self.delta_tx,
                &key,
                &name,
                &role, // ← o papel entra no roster do Supervisor (agents.json / lina list)
                cmd,
                self.cols,
                self.rows,
                // F1-5-2: o terminal nasce capturando no store do workspace (cabo append-on-scroll).
                self.scrollback.clone(),
            )?
        };

        // 6) PERSISTE a sequência CANÔNICA (§2) antes de projetar — log = fonte da verdade.
        //    Falha em QUALQUER append → compensação (§5): `retire_pty` desregistra do roster
        //    vivo e mata o PTY — nada de nó no agents.json sem rastro no log.
        let events = admission_events(
            node,
            f64::from(x),
            f64::from(y),
            &name,
            &role,
            plan.engine.as_ref(),
            cwd_real.as_deref(),
            plan.requested_by, // SEAM-1: carimba quem pediu o spawn (None na origem humana)
        );
        let count = {
            let mut s = lock(&self.store);
            let mut appended = Ok(());
            for ev in &events {
                if let Err(e) = s.append(ev) {
                    appended = Err(e.to_string());
                    break;
                }
            }
            if let Err(e) = appended {
                drop(s);
                self.retire_pty(node, key);
                return Err(e);
            }
            s.event_count().ok()
        };

        // 7) Projeção ATÔMICA: model (INSERE sobrescrevendo qualquer placeholder do pump) +
        //    grids + keys + política de cwd (o `rewrite_bootstrap` itera por ela).
        {
            let mut m = lock(&self.model);
            let mut view = NodeView::new(name.clone(), NodeKind::Terminal, x, y);
            view.kit_missing = kit_missing;
            view.cwd_shared = cwd_shared;
            view.autonomy = plan.autonomy;
            m.nodes.insert(node, view);
            // FIX-1: os nós que JÁ estavam nesta pasta agora têm companhia → badge honesto
            // neles também (o compartilhamento é mútuo; nenhum card esconde que divide a pasta).
            for sharer in &cwd_sharers {
                if let Some(v) = m.nodes.get_mut(sharer) {
                    v.cwd_shared = true;
                }
            }
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
        lock(&self.cwds).insert(node, plan.cwd.clone());

        // 8) Roster mudou → reescreve as doutrinas (os existentes ganham o colega novo).
        //    No restore em lote a reescrita é ADIADA para o fim (ver `admit_node_inner`).
        if rewrite_roster {
            self.rewrite_bootstrap();
        }
        eprintln!(
            "lina-gpui: admissão — agente '{name}' criado (papel {role}, cwd {}). node {node}",
            cwd_real
                .as_deref()
                .map_or_else(|| "herdado".to_string(), |p| p.display().to_string())
        );
        Ok(node)
    }

    /// **SEAM-1 (M3) — executa um spawn aprovado pelo funil único, idempotente por `spawn_id`.**
    /// Admite o terminal com `requested_by` carimbado (auditoria + anti-fork-bomb) e apenda
    /// `SpawnAdmitted{spawn_id, node}` (a chave de DEDUPE DURÁVEL). Se o spawn JÁ foi admitido
    /// (replay/re-drain/banner re-aprovado), devolve o nó existente SEM criar um 2º terminal. O
    /// binding de cascata (M4 `seed_delivered_root`) e o 1º prompt são fiados pela `MailboxPump`
    /// (dona do router/mailbox) ao OBSERVAR o `SpawnAdmitted` — desacopla a criação (qualquer thread)
    /// da fiação do router (thread do pump).
    ///
    /// # Errors
    /// Propaga o `Err` do funil `admit_node` (nome/papel inválido, falha de PTY/persistência).
    pub fn execute_spawn(
        &self,
        spawn_id: &str,
        name: &str,
        role: &str,
        requested_by: NodeId,
    ) -> Result<NodeId, String> {
        // M3 — dedupe DURÁVEL: este spawn já virou terminal? (SpawnAdmitted no log) → no-op.
        if let Some(existing) = self.admitted_node_for_spawn(spawn_id) {
            return Ok(existing);
        }
        // #7 dogfooding r2: o colega spawnado nasce com o MOTOR default do workspace (paridade
        // ⌘N — `TerminalSpawned.cli` registra o CLI, nunca o nome do nó). Resolução injetável
        // (produção: discovery; testes: fixa).
        let engine = (self.spawn_engine)();
        let child = self.admit_node(NodeAdmission::spawned_agent(
            name,
            role,
            requested_by,
            engine,
        ))?;
        if let Err(e) = lock(&self.store).append(&DomainEvent::SpawnAdmitted {
            id: spawn_id.to_string(),
            node: child,
        }) {
            // O terminal já nasceu (irreversível); sem o marcador durável o dedupe degrada para o
            // tick — ALTO e VISÍVEL, nunca silencioso (inv #4).
            eprintln!(
                "lina-gpui: CRÍTICO — spawn {spawn_id} admitido (node {child}) mas SpawnAdmitted NÃO logado: {e}"
            );
        }
        Ok(child)
    }

    /// M3: o `NodeId` já admitido para `spawn_id` (varre `SpawnAdmitted` no log), se houver.
    fn admitted_node_for_spawn(&self, spawn_id: &str) -> Option<NodeId> {
        let recs = lock(&self.store).events().ok()?;
        recs.iter().rev().find_map(|r| {
            if r.kind != "SpawnAdmitted"
                || r.payload.get("id").and_then(serde_json::Value::as_str) != Some(spawn_id)
            {
                return None;
            }
            r.payload
                .get("node")
                .and_then(serde_json::Value::as_str)
                .and_then(|s| s.parse().ok())
        })
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
        lock(&self.cwds).remove(&node);

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

// ─────────────── R2b · fiação VIVA da detecção (metade-app — grids do APP) ───────────────
//
// O `PermissionWatch::scan` do core varre um `lina_core::PtyHost` — mas o APP roda
// `PtyManager`-direto (`wire_terminal`: reader thread + `Grid` próprios; zero `PtyHost`
// no app). Esta é a MESMA fiação com o DETECTOR CANÔNICO do core
// (`PermissionDetector::observe_grid` — padrões/re-arm/cleared/FP, fonte única) sobre o
// host que o app POSSUI: idle por silêncio de OUTPUT (o `GridDelta.bytes` do reader —
// sinal do produtor, idêntico ao `metrics.bytes_read` do core), mute pela MESMA
// projeção do log (via closure — o hub injeta), append no MESMO EventStore e Blocked
// pela MESMA porta canônica (`LifecycleEngine::transition`). DIVERGÊNCIA DE SEAM
// regisrada na entrega: quando o app migrar p/ `PtyHost` (ou o core generalizar o host
// num trait), isto troca 1-para-1 pelo `PermissionWatch::scan`.

/// O que UMA varredura emitiu — espelho da `lina_core::ScanOutcome` da metade-core
/// (asks novos + prompts respondidos no terminal), AMBOS já apendados no log pela
/// própria varredura (`PermissionAsked` e `PermissionPromptCleared`). O pump usa as
/// contagens p/ marcar `worked`; o hub reflete ao vivo no próximo `sync` (re-replay do
/// log). Alinhar a forma à do core torna o swap p/ `PermissionWatch::scan` (quando o
/// app migrar p/ `PtyHost`/trait) 1-para-1.
#[derive(Debug, Default, Clone)]
pub struct AppScanOutcome {
    pub asks: Vec<lina_core::PermissionAsk>,
    pub cleared: Vec<lina_core::ClearedPrompt>,
}

impl AppScanOutcome {
    /// Houve algo nesta varredura (asks novos OU prompts respondidos)?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.asks.is_empty() && self.cleared.is_empty()
    }
}

/// Metade-app do loop vivo de detecção (caller-driven, sem thread própria — padrão do
/// módulo do core): o pump chama [`Self::note_output`] a cada `GridDelta` e
/// [`Self::scan`] no tick (≤1s ⇒ latência ≤ `WATCH_IDLE_MS` + 1 tick < 3s).
pub struct AppPermissionWatch {
    det: lina_core::PermissionDetector,
    /// Por nó: `now_ms` da última MUDANÇA de output (alimentado pelo reader via pump).
    last_output: BTreeMap<NodeId, u64>,
    idle_after_ms: u64,
}

impl Default for AppPermissionWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl AppPermissionWatch {
    #[must_use]
    pub fn new() -> Self {
        Self {
            det: lina_core::PermissionDetector::new(),
            last_output: BTreeMap::new(),
            // Mesma calibração da metade-core (silêncio de output que conta como idle).
            idle_after_ms: lina_core::WATCH_IDLE_MS,
        }
    }

    /// O reader emitiu bytes p/ o nó AGORA (chamado no drain dos `GridDelta`).
    pub fn note_output(&mut self, node: NodeId, now_ms: u64) {
        self.last_output.insert(node, now_ms);
    }

    /// UMA varredura dos terminais do APP (espelha `PermissionWatch::scan` da metade-
    /// core): respeita o mute (projeção do log, injetada pelo hub), deriva idle por
    /// silêncio de output, roda o detector canônico e apenda no log, na MESMA varredura:
    /// (1) cada `PermissionAsked` + `Blocked{reason=permission_prompt}` pela porta
    /// canônica; (2) cada `PermissionPromptCleared{node_id, stable_id}` dos prompts
    /// respondidos DIRETO no terminal (R2b item 5 — contrato aprovado: remoção no fold,
    /// SEM decisão fabricada; ≠ dismiss/FP, ≠ resolve). Devolve [`AppScanOutcome`] p/ o
    /// hub refletir ao vivo (ambos já no log; o `sync` re-projeta). Identidade = NOME
    /// canônico do roster (ADR 0021 §5).
    pub fn scan(
        &mut self,
        grids: &BTreeMap<NodeId, Grid>,
        sup: &Supervisor,
        store: &Arc<Mutex<EventStore>>,
        lifecycle: &mut lina_core::lifecycle::LifecycleEngine,
        is_muted: &dyn Fn(&str) -> bool,
        now_ms: u64,
    ) -> AppScanOutcome {
        let mut out = AppScanOutcome::default();
        for (node, grid) in grids {
            // Nome CANÔNICO do roster — nó fora do roster (corrida de fechamento) pula.
            let Some(name) = sup.get(*node).map(|n| n.name) else {
                continue;
            };
            // Mute por nó: nem roda `observe_grid` (defesa em profundidade — a fila
            // aplica a mesma regra no fold dela).
            if is_muted(&name) {
                continue;
            }
            let last = self.last_output.entry(*node).or_insert(now_ms);
            let idle = now_ms.saturating_sub(*last) >= self.idle_after_ms;
            let ask = {
                let g = lock(grid);
                self.det.observe_grid(&name, &**g, idle, now_ms)
            };
            let Some(ask) = ask else { continue };
            // Fato antes do efeito: o pedido entra no log JÁ na varredura.
            if let Err(e) = lock(store).append(&ask.to_event()) {
                eprintln!("lina-gpui: [ATT] watch — append do PermissionAsked falhou: {e}");
                continue;
            }
            // Cabeçalho "Idle" parado numa pergunta é estado MENTIROSO (direção do
            // fundador): Blocked auditado pela porta canônica; falha não derruba a
            // varredura (o pedido já está no log).
            if let Err(e) = lifecycle.transition(
                sup,
                &mut lock(store),
                *node,
                lina_core::NodeStatus::Blocked,
                lina_core::permission_detect::BLOCKED_REASON_PROMPT,
            ) {
                eprintln!("lina-gpui: [ATT] watch — transição p/ Blocked falhou: {e}");
            }
            eprintln!(
                "lina-gpui: [ATT] watch — PermissionAsked emitido (node={name:?} kind={:?})",
                ask.kind
            );
            out.asks.push(ask);
        }
        // R2b item 5 (FECHADO): prompts respondidos DIRETO no terminal desde o último
        // scan → `PermissionPromptCleared` no log (o fold do core remove o item; o hub
        // reflete no `sync`). Apêndice na PRÓPRIA varredura (como a metade-core), não
        // mais no pump — forma idêntica ao `ScanOutcome`.
        for c in self.det.take_cleared() {
            if let Err(e) = lock(store).append(&DomainEvent::PermissionPromptCleared {
                node_id: c.node_id.clone(),
                stable_id: c.stable_id.clone(),
            }) {
                eprintln!("lina-gpui: [ATT] watch — append de PermissionPromptCleared falhou: {e}");
                continue;
            }
            eprintln!(
                "lina-gpui: [ATT] watch — prompt respondido no terminal → PermissionPromptCleared (node={:?} stable_id={})",
                c.node_id, c.stable_id
            );
            out.cleared.push(c);
        }
        out
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
#[allow(clippy::too_many_arguments)] // fiação única do pump: os fios do app entram aqui.
pub fn spawn_pump(
    mut bridge: GpuiBridgeHost,
    delta_rx: Receiver<GridDelta>,
    mut bus_rx: broadcast::Receiver<BusEvent>,
    store: Arc<Mutex<EventStore>>,
    idle_ms: u64,
    // FIX DE GATE F1-0 (fiação final): o fim-de-resposta precisa DEVOLVER o nó a Idle no
    // SUPERVISOR (transição event-sourced do lifecycle F1-0-3) — sem isto o router marca Busy
    // na entrega (F1-0-4) e nada o desfaz em produção: a 2ª msg ao mesmo alvo reteria até a DLQ.
    sup: Arc<Supervisor>,
    // R2b (fiação viva da detecção): os grids do app (a varredura lê o VT parseado) e a
    // projeção de mute (hub). `live_detection=false` desliga o loop vivo — modo DEMO
    // (`LINA_ATTENTION_DEMO`) é teatro determinístico: demo OU real, nunca os dois.
    grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    attention: Arc<AttentionHub>,
    live_detection: bool,
) -> std::io::Result<Pump> {
    let stop = Arc::new(AtomicBool::new(false));
    let join = {
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("lina-bridge-pump".into())
            .spawn(move || {
                let mut meter = crate::cost::CostMeter::new();
                // Engine do lifecycle (F1-0-3): transição Idle event-sourced no fim-de-resposta.
                // Engine local do pump: `transition` lê o roster vivo e apenda no store — o estado
                // interno (stall) não é usado neste caminho.
                let mut lifecycle = lina_core::lifecycle::LifecycleEngine::new();
                // W4-6 gap2: nós atualmente em turno (produzindo). Dirige Busy↔Idle na ÁRVORE a11y SÓ na
                // transição (não a cada delta → sem busy-loop/churn de render). Idle = "resposta pronta".
                let mut active: std::collections::HashSet<NodeId> =
                    std::collections::HashSet::new();
                // R2b: metade-app do loop vivo de detecção (detector canônico do core
                // sobre os grids do app; tick ≤1s ⇒ claude real bloqueado vira
                // PermissionAsked no log em <3s — o hub re-projeta e o toast aparece).
                let mut watch = AppPermissionWatch::new();
                let mut last_scan_ms = 0u64;
                // F1-1-8 (ADR 0021 §3): relógio do driver de auto-deny do SLA (10 min).
                let mut last_autodeny_ms = 0u64;
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
                        // R2b: output vivo re-arma o relógio de idle da detecção (sinal
                        // do PRODUTOR — o mesmo do `metrics.bytes_read` da metade-core).
                        watch.note_output(delta.node, now);
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
                    // R2b: varredura viva a cada ~500ms (≤1s da receita): detector
                    // canônico + mute do hub (projeção do log) + Blocked auditado.
                    if live_detection && now.saturating_sub(last_scan_ms) >= 500 {
                        last_scan_ms = now;
                        // Snapshot raso dos handles (lock do mapa curto; cada grid trava
                        // individualmente DENTRO do scan — mesma ordem do render).
                        let snapshot: BTreeMap<NodeId, Grid> = lock(&grids)
                            .iter()
                            .map(|(k, v)| (*k, Arc::clone(v)))
                            .collect();
                        // A varredura apenda asks E cleared no log (forma do
                        // `ScanOutcome` da metade-core); o hub reflete ao vivo no
                        // próximo `sync` (re-replay). Marca `worked` se algo aconteceu.
                        let outcome = watch.scan(
                            &snapshot,
                            &sup,
                            &store,
                            &mut lifecycle,
                            &|name| attention.is_node_muted(name),
                            now,
                        );
                        if !outcome.is_empty() {
                            worked = true;
                        }
                    }
                    // F1-1-8 / ADR 0021 §3: driver de auto-deny do SLA. Throttle ~15s (folga
                    // sobejante p/ o threshold de 10 min). Drena cada pendência vencida pelo MESMO
                    // pipeline validado do executor com Deny/Timeout (NUNCA approve) — a fila não
                    // tem porta (AC-0021.6); o write valida a tela e aborta se ela divergiu.
                    if now.saturating_sub(last_autodeny_ms) >= 15_000 {
                        last_autodeny_ms = now;
                        attention.drive_auto_deny(now);
                    }
                    // Fim-de-turno por idle → apenda TokenUsageReported (estimado) no store autoritativo.
                    for (node, tokens) in meter.poll_finished_turns(now, idle_ms) {
                        if let Err(e) = lock(&store).append(&DomainEvent::TokenUsageReported {
                            node: node.to_string(),
                            tokens,
                        }) {
                            eprintln!("lina-gpui: falha ao apendar TokenUsageReported: {e}");
                        }
                        // FIX DE GATE F1-0: fim-de-resposta REAL (o MESMO sinal do medidor W0-10 —
                        // idle do perfil fecha o turno; nunca um flush qualquer) → transição
                        // event-sourced p/ Idle no SUPERVISOR (`NodeStatusChanged{end_of_response}`
                        // no log + roster). É o que destrava a retenção F1-0-4: a próxima msg ao
                        // mesmo alvo ENTREGA em vez de reter até a DLQ. Best-effort com erro ALTO
                        // (nó morto entre o turno e o tick é corrida legítima — Dead é terminal).
                        if let Err(e) = lifecycle.on_end_of_response(&sup, &mut lock(&store), node)
                        {
                            eprintln!(
                                "lina-gpui: fim-de-resposta de {node} não virou Idle no roster: {e}"
                            );
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
    use std::cell::Cell;

    thread_local! {
        /// **Fault-injection da janela pós-`register` de [`wire_terminal`]** (red-team 360, inv 4).
        /// `clone_reader`/spawn-da-thread só falham por exaustão de FD/threads do SO — não
        /// forçável de forma determinística pela API pública. Este seam (pattern do projeto:
        /// "injetável p/ teste determinístico") força a falha logo após o `register`, exercitando
        /// o caminho REAL de COMPENSAÇÃO em `wire_terminal` (não um substituto). Consumido na 1ª
        /// checada (replace→false): nunca vaza entre testes na mesma thread.
        static WIRE_FAULT: Cell<bool> = const { Cell::new(false) };
    }

    /// Arma a próxima [`wire_terminal`] para falhar logo após o `register` (consumido 1×).
    pub(super) fn arm_wire_fault() {
        WIRE_FAULT.with(|f| f.set(true));
    }

    /// `wire_terminal` consulta isto pós-`register` (sob `#[cfg(test)]`): toma-e-limpa a flag.
    pub(super) fn take_wire_fault() -> bool {
        WIRE_FAULT.with(|f| f.replace(false))
    }

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

    /// R2c (bug de tela 21:40 do fundador): a função PURA classifica QUAIS nomes de env
    /// são de orquestradores estrangeiros — `MAESTRI_CLI` e todo `MAESTRI_*` (prefixo,
    /// extensível); e NUNCA classifica PATH/HOME/TERM/SHELL nem as vars de auth do
    /// claude (ANTHROPIC_*/CLAUDE_*) — esses precisam sobreviver no PTY do agente.
    #[test]
    fn foreign_orchestrator_var_matches_maestri_prefix_only() {
        // Estrangeiras (vazam a skill do Maestri p/ o agente).
        for k in [
            "MAESTRI_CLI",
            "MAESTRI_SESSION",
            "MAESTRI_CANVAS_ID",
            "MAESTRI_",
        ] {
            assert!(
                is_foreign_orchestrator_var(k),
                "{k} deveria ser estrangeira"
            );
        }
        // Intocáveis: básicas do shell + auth do claude + as do próprio Lina.
        for k in [
            "PATH",
            "HOME",
            "TERM",
            "SHELL",
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_ENTRYPOINT",
            "LINA_HOME",
            "VIBE_ROLE",
            "MAESTRO", // nome de papel do Lina, NÃO uma var MAESTRI_*
        ] {
            assert!(!is_foreign_orchestrator_var(k), "{k} NÃO pode ser scrubada");
        }
    }

    /// A seleção PURA sobre um conjunto de nomes devolve só as estrangeiras — base do
    /// scrub real, testável sem tocar o env global do processo (sem corrida entre testes).
    #[test]
    fn foreign_orchestrator_keys_selects_only_foreign() {
        let names = [
            "MAESTRI_CLI",
            "PATH",
            "MAESTRI_SESSION",
            "HOME",
            "ANTHROPIC_API_KEY",
        ];
        let mut got = foreign_orchestrator_keys(names.iter().copied());
        got.sort();
        assert_eq!(got, vec!["MAESTRI_CLI", "MAESTRI_SESSION"]);
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

    /// R2c (fechamento do bug): `scrub_foreign_orchestrator_env` remove do env do
    /// processo do app a var de orquestrador estrangeiro (que TODO PTY filho herdaria),
    /// e PRESERVA as básicas do shell + auth do claude. Vars com nomes ÚNICOS (sufixo
    /// pid) p/ não colidir com outros testes em paralelo que mutam o env global.
    #[test]
    fn scrub_removes_foreign_orchestrator_env_keeps_essentials() {
        let pid = std::process::id();
        let foreign = format!("MAESTRI_CLI_{pid}");
        // `MAESTRI_CLI` "puro" também é coberto, mas a var com pid evita corrida.
        std::env::set_var(&foreign, "/Users/x/.maestri/cli");
        // Sentinelas que NÃO podem ser tocadas (auth do claude + shell).
        let auth = format!("ANTHROPIC_API_KEY_{pid}"); // prefixo intocável
        std::env::set_var(&auth, "sk-ant-xxx");
        assert!(std::env::var_os(&foreign).is_some());

        let removed = scrub_foreign_orchestrator_env();
        assert!(
            removed.contains(&foreign),
            "a var estrangeira deve constar dos removidos: {removed:?}"
        );
        assert!(
            std::env::var_os(&foreign).is_none(),
            "após o scrub, nenhum PTY filho herda MAESTRI_* — a skill estrangeira morre"
        );
        assert!(
            std::env::var_os(&auth).is_some(),
            "auth do claude (ANTHROPIC_*) SOBREVIVE — o agente ainda funciona"
        );
        std::env::remove_var(&auth);
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

        // F1-0-2 (costura coordenada): a entrega agora EXIGE prontidão real — timeout não
        // injeta mais, e a região de input de um grid VAZIO não casa o regex. O alvo é `cat`
        // (nunca imprime prompt) → semeia uma linha-prompt viva no grid de B, como um CLI real
        // mostraria (a detecção de prontidão em si é testada no core — gate F1-0-2).
        lock(&grid_b).advance(b"\r\n> ");

        // A2A A→B: route (publica Message) + deliver_a2a (injeta) + persist.
        let env = A2aEnvelope::new(node_a, Recipient::Node(node_b), Some("a2a".into()));
        let id = env.id.clone();
        let targets = sup.route(&env, RolePolicy::All);
        assert!(targets.contains(&node_b));
        // W5-5: o gate de render exercita a allow-list REAL — A→B do mesmo Espaço passa.
        let trust = WorkspaceTrust::from_members(&[node_a, node_b]);
        let out = deliver_a2a(
            &sup,
            node_b,
            node_a,
            "LINA_A2A_MARKER",
            &demo_profile(),
            &grid_b,
            trust.policy(),
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
        // F1-0-2 (costura coordenada): timeout de prontidão NÃO injeta mais; `cat` nunca
        // imprime prompt → semeia a linha-prompt viva nos grids (a prontidão real é do core).
        lock(&grid_a).advance(b"\r\n> ");
        lock(&grid_b).advance(b"\r\n> ");
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
            new_reinject_queue(), // FIX-A3: reinject não exercitado aqui — fila vazia
            Arc::clone(&model),
            Arc::clone(&brake),
            0,                                // teto de custo desligado neste teste (freio/pulso)
            demo_profile(), // perfil demo: os grids de teste já têm linha-prompt semeada
            Arc::new(ProfileRegistry::new()), // A2A: registry vazio ⇒ target_profile cai no fallback (= pré-fix)
            Autonomy::Assisted, // SEAM-1: testes de roteamento — autonomia default (não-spawn)
            test_nodes(Arc::clone(&store)), // SEAM-1: nodes não exercitado (sem SpawnApproved aqui)
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

        // F1-0-4 (CONTRATO NOVO — entrega ciente de estado): a entrega de "oi 1" marcou B
        // `Busy` (`NodeStatusChanged{a2a_delivery}`); em PRODUÇÃO o lifecycle devolve B a
        // `Idle` no fim-de-resposta (meter de idle — outra pump). Aqui B é um `cat`
        // sintético sem fim-de-turno e este teste só roda a `MailboxPump`, então simulamos
        // o término do turno de "oi 1" — senão "oi 2" seria corretamente RETIDA (alvo Busy)
        // em vez de entregue, e o teste do FREIO mediria o efeito errado. Mesmo padrão do
        // helper `turn_done()` dos testes do router (lina-core).
        let _ = sup.set_status(node_b, CoreStatus::Idle);

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

    // ════════════════════ F1-2-4 · RE-INJEÇÃO DE DOUTRINA (terminal vivo) ════════════════════

    /// O texto re-injetado nomeia o papel novo (é DADO, nunca autoridade) — base do critério 3.
    #[test]
    fn doctrine_reinjection_text_names_the_role() {
        let t = doctrine_reinjection_text("reviewer");
        assert!(
            t.contains("reviewer"),
            "a doutrina re-injetada cita o papel novo"
        );
        assert!(!t.trim().is_empty(), "doutrina não-vazia");
    }

    /// **Critério 4 (freio) + durabilidade — prova de função (não-vacuosa).** `drain_reinject_active`
    /// é o GATE do freio sobre a fila de re-injeção: PAUSADO devolve `[]` SEM tocar a fila (nada se
    /// injeta, nada se perde); SOLTO drena a MESMA mensagem (prova de que estava represada, não
    /// perdida).
    #[test]
    fn reinject_drain_respects_brake() {
        // Fila EM-PROCESSO (FIX-A3) — sem filesystem. O `target` é um NodeId qualquer: o GATE do
        // freio (`drain_reinject_active`) não toca o roster, só decide SE drena.
        let queue = new_reinject_queue();
        let target: NodeId = uuid::Uuid::now_v7();
        enqueue_doctrine_reinjection(&queue, target, "reviewer");

        let under_brake = drain_reinject_active(&queue, true);
        assert!(
            under_brake.is_empty(),
            "sob o freio a re-injeção NÃO drena (fica enfileirada em RAM)"
        );

        let resumed = drain_reinject_active(&queue, false);
        assert_eq!(
            resumed.len(),
            1,
            "ao soltar o freio, a fila represada drena"
        );
        assert_eq!(
            resumed[0].role, "reviewer",
            "drena exatamente a doutrina represada (papel preservado p/ regenerar o texto)"
        );
    }

    /// **FIX-A3 (ALTA — red-team do gate F1-1): a re-injeção NÃO tem mais superfície de FILESYSTEM
    /// que um agente alcance.** Vetor original: um agente malicioso (mesmo usuário de SO) escrevia
    /// direto em `<.lina>/reinject/<peer>/x.json` e o drain INJETAVA o payload arbitrário no PTY do
    /// peer (sem 2ª camada). Prova (não-vacuosa por controle positivo): um drop BEM-FORMADO no
    /// caminho FS legado é INERTE (o pump não lê mais o disco), enquanto a fila EM-PROCESSO legítima
    /// injeta — i.e. o pump funciona, mas só pela fila interna.
    #[test]
    fn reinject_has_no_filesystem_dropzone_fix_a3() {
        let dir = std::env::temp_dir().join(format!("lina-reinj-a3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
        let _ = sup.set_status(node_a, CoreStatus::Running);
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        lock(&grid_a).advance(b"\r\n> "); // semeia a linha-prompt viva (o `cat` não imprime prompt)
        lock(&grids).insert(node_a, grid_a);
        let store = Arc::new(Mutex::new(
            EventStore::open(dir.join("events")).expect("store"),
        ));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let brake = crate::wiring::new_brake(); // solto
        let mailbox_root = dir.join(".lina");
        let reinject_queue = new_reinject_queue();
        let mut pump = MailboxPump::new(
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&grids),
            Mailbox::new(&mailbox_root),
            Arc::clone(&reinject_queue),
            Arc::clone(&model),
            Arc::clone(&brake),
            0,
            demo_profile(),
            Arc::new(ProfileRegistry::new()), // A2A: registry vazio ⇒ target_profile cai no fallback (= pré-fix)
            Autonomy::Assisted, // SEAM-1: testes de roteamento — autonomia default (não-spawn)
            test_nodes(Arc::clone(&store)), // SEAM-1: nodes não exercitado (sem SpawnApproved aqui)
        );
        let bus_count = |s: &Arc<Mutex<EventStore>>| {
            lock(s)
                .events()
                .expect("ev")
                .into_iter()
                .filter(|r| r.kind == "BusMessageSent")
                .count()
        };

        // (NEGATIVO) ATAQUE A3: um agente deposita um drop BEM-FORMADO no caminho FS legado
        // (`<.lina>/reinject/Terminal A/`). Antes do fix, o drain o injetaria no PTY de A.
        let legacy_dropzone = Mailbox::new(mailbox_root.join("reinject"));
        let forged = MailMessage::new(
            "Terminal A",
            "Terminal A",
            "handoff",
            "PAYLOAD MALICIOSO ARBITRÁRIO",
        );
        // O `enqueue_as` é o MESMO mecanismo que o código vulnerável usava (write atômico no subdir
        // por-nó): o `.expect` Ok garante que o drop FOI gravado no disco, exatamente onde o drain
        // antigo o leria — controle negativo não-vacuoso (há um drop real a ser ignorado).
        legacy_dropzone
            .enqueue_as("Terminal A", &forged)
            .expect("atacante escreve no caminho FS legado");
        let before = bus_count(&store);
        pump.tick();
        assert_eq!(
            bus_count(&store),
            before,
            "FIX-A3: o drop no FILESYSTEM é INERTE — o pump não lê mais o disco (zero injeção)"
        );

        // (POSITIVO) a fila EM-PROCESSO legítima injeta — o pump FUNCIONA (teste não-vacuoso).
        enqueue_doctrine_reinjection(&reinject_queue, node_a, "reviewer");
        pump.tick();
        assert_eq!(
            bus_count(&store),
            before + 1,
            "controle positivo: a fila em-processo injeta (1 BusMessageSent) — o pump não está inerte"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 5 (kill -9 → replay):** o log com `NodeRenamed`+`NodeRoleAssigned` reconstrói nome e
    /// papel NOVOS na projeção (a fonte da verdade — inv #4), com o MESMO `NodeId` (sem respawn).
    #[test]
    fn replay_reconstructs_new_name_and_role() {
        let dir = std::env::temp_dir().join(format!("lina-reinj-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut pty = PtyManager::new();
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _drx) = std::sync::mpsc::channel::<GridDelta>();
        let (node, _g) = wire_terminal(
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
        .expect("wire");
        let mut store = EventStore::open(dir.join("events")).expect("store");
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("added");
        store
            .append(&DomainEvent::NodeRenamed {
                node,
                name: "Analista de Mercado".into(),
            })
            .expect("renamed");
        store
            .append(&DomainEvent::NodeRoleAssigned {
                node,
                role: "analyst".into(),
            })
            .expect("role");

        let proj = store.project().expect("project");
        let n = proj.nodes.get(&node).expect("nó na projeção");
        assert_eq!(
            n.name.as_deref(),
            Some("Analista de Mercado"),
            "nome NOVO reconstruído"
        );
        assert_eq!(
            n.role.as_deref(),
            Some("analyst"),
            "papel NOVO reconstruído"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 1 + 2 (via NodeManager real):** `rename_node` para nome VAZIO/só-espaços é rejeitado
    /// inline SEM apendar evento (critério 2); um rename válido apenda APENAS `NodeRenamed` — nenhum
    /// `TerminalSpawned`/`TerminalExited` → o PID é preservado (critério 1, sem respawn).
    #[test]
    fn rename_rejects_empty_and_does_not_respawn() {
        let (nm, store, _model) = test_manager("rename-guard", None);
        let node = *lock(&nm.model).order.first().expect("nó seed");

        // Critério 2: nome vazio/só-espaços → Err inline, ZERO eventos.
        let before = lock(&store).event_count().expect("count");
        assert!(
            nm.rename_node(node, "   ").is_err(),
            "nome só-espaços é rejeitado"
        );
        assert!(nm.rename_node(node, "").is_err(), "nome vazio é rejeitado");
        assert_eq!(
            lock(&store).event_count().expect("count"),
            before,
            "nome inválido NÃO apenda evento algum (nenhum NodeRenamed vazio no log)"
        );

        // Critério 1: rename válido apenda só NodeRenamed (sem respawn → PID preservado).
        nm.rename_node(node, "Renomeado").expect("rename válido");
        let new_kinds: Vec<String> = lock(&store)
            .events()
            .expect("ev")
            .into_iter()
            .skip(usize::try_from(before).expect("count cabe em usize"))
            .map(|r| r.kind)
            .collect();
        assert_eq!(
            new_kinds,
            vec!["NodeRenamed".to_string()],
            "só NodeRenamed; sem respawn"
        );
    }

    /// **Critério 3 (aceite → re-injeção enfileirada):** `assign_role` (chamado só na edição
    /// confirmada) persiste `NodeRoleAssigned` E enfileira a re-injeção na FILA EM-PROCESSO com o
    /// `target` = `NodeId` AUTENTICADO do roster (não um nome re-resolvido depois) e o papel novo.
    #[test]
    fn assign_role_enqueues_doctrine_reinjection() {
        let (nm, _store, _model) = test_manager("reinject-assign", None);
        let node = *lock(&nm.model).order.first().expect("nó seed");

        nm.assign_role(node, "reviewer").expect("assign role");

        let queue = nm.reinject_queue();
        let drained = drain_reinject_active(&queue, false);
        assert_eq!(drained.len(), 1, "assign_role enfileira UMA re-injeção");
        assert_eq!(
            drained[0].target, node,
            "target = NodeId AUTENTICADO do roster (não um nome/string re-resolvido depois)"
        );
        assert_eq!(
            drained[0].role, "reviewer",
            "a re-injeção carrega o papel novo (o texto é REGENERADO no consumidor)"
        );
    }

    /// **Critério 4 (END-TO-END no pump, log-verificável):** sob o FREIO a re-injeção NÃO injeta
    /// (nenhum `BusMessageSent` novo) e fica represada; ao RETOMAR, drena+injeta e LOGA exatamente um
    /// `BusMessageSent` (verificável no log). Idempotente: um tick extra não re-injeta (ack).
    #[test]
    fn reinject_doctrine_enqueues_under_brake_delivers_on_resume() {
        let dir = std::env::temp_dir().join(format!("lina-reinj-pump-{}", std::process::id()));
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
        let _ = sup.set_status(node_a, CoreStatus::Running);
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        lock(&grid_a).advance(b"\r\n> ");
        lock(&grids).insert(node_a, grid_a);
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let brake = crate::wiring::new_brake();
        let mailbox_root = dir.join(".lina");
        let reinject_queue = new_reinject_queue();
        let mut pump = MailboxPump::new(
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&grids),
            Mailbox::new(&mailbox_root),
            Arc::clone(&reinject_queue),
            Arc::clone(&model),
            Arc::clone(&brake),
            0,
            demo_profile(),
            Arc::new(ProfileRegistry::new()), // A2A: registry vazio ⇒ target_profile cai no fallback (= pré-fix)
            Autonomy::Assisted, // SEAM-1: testes de roteamento — autonomia default (não-spawn)
            test_nodes(Arc::clone(&store)), // SEAM-1: nodes não exercitado (sem SpawnApproved aqui)
        );
        let bus_count = |s: &Arc<Mutex<EventStore>>| {
            lock(s)
                .events()
                .expect("ev")
                .into_iter()
                .filter(|r| r.kind == "BusMessageSent")
                .count()
        };

        // (1) PAUSA o freio.
        lock(&brake).toggle_requested = true;
        pump.tick();
        assert!(lock(&brake).paused, "freio pausado");

        // (2) Enfileira a re-injeção SOB o freio (fila em-processo, target = NodeId autenticado).
        enqueue_doctrine_reinjection(&reinject_queue, node_a, "reviewer");
        let bus_before = bus_count(&store);

        // (3) Tick sob o freio: NÃO injeta → sem BusMessageSent novo (enfileirada até Retomar).
        pump.tick();
        assert_eq!(
            bus_count(&store),
            bus_before,
            "sob o freio a doutrina NÃO é injetada (nenhum BusMessageSent no log)"
        );

        // (4) SOLTA o freio → drena+injeta → BusMessageSent da doutrina aparece no log.
        lock(&brake).toggle_requested = true;
        pump.tick();
        assert!(!lock(&brake).paused, "freio solto");
        assert_eq!(
            bus_count(&store),
            bus_before + 1,
            "ao RETOMAR, a doutrina é injetada e LOGADA (BusMessageSent — log-verificável)"
        );

        // (5) Idempotência: a fila foi confirmada (ack); outro tick não re-injeta (write irreversível).
        pump.tick();
        assert_eq!(
            bus_count(&store),
            bus_before + 1,
            "re-injeção entregue UMA vez (ack_inflight); nenhuma duplicata"
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
            new_reinject_queue(), // FIX-A3: reinject não exercitado aqui — fila vazia
            Arc::clone(&model),
            Arc::clone(&brake),
            100,
            demo_profile(),
            Arc::new(ProfileRegistry::new()), // A2A: registry vazio ⇒ target_profile cai no fallback (= pré-fix)
            Autonomy::Assisted, // SEAM-1: testes de roteamento — autonomia default (não-spawn)
            test_nodes(Arc::clone(&store)), // SEAM-1: nodes não exercitado (sem SpawnApproved aqui)
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

        let mut nm = NodeManager::new(
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
        // #7: motor de spawn FIXO (None) — a discovery real leria o PATH do runner e um
        // spawn de teste LANÇARIA o CLI de verdade. `None` = fábrica `cat` (como sempre).
        nm.set_spawn_engine_factory(Arc::new(|| None));
        (nm, store, model)
    }

    /// SEAM-1: `Arc<NodeManager>` de teste compartilhando o `store` dado — para os testes do
    /// `AttentionHub` (permissão/custódia, que NÃO exercitam spawn): só satisfaz o tipo. Se um teste
    /// chamar `resolve_spawn`, o `execute_spawn` opera sobre o MESMO log do hub (store compartilhado).
    fn test_nodes(store: Arc<Mutex<EventStore>>) -> Arc<NodeManager> {
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _rx) = std::sync::mpsc::channel::<GridDelta>();
        let cmd_factory: CmdFactory = Arc::new(|_n: &str| PtyCommand::new("cat"));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let mut nm = NodeManager::new(
            pty,
            sup,
            store,
            model,
            grids,
            BTreeMap::new(),
            delta_tx,
            80,
            24,
            0,
            cmd_factory,
            None,
            std::env::temp_dir().join("lina-test-nodes"),
        );
        // #7: motor de spawn FIXO (None) — sem discovery real no runner (ver `test_manager`).
        nm.set_spawn_engine_factory(Arc::new(|| None));
        Arc::new(nm)
    }

    /// **Harness do funil de cwd ÚNICO (ADR 0022 §4 / fix do slot reciclável):** um
    /// `NodeManager` com `ws_root`, `store_dir` e `seq_start` EXPLÍCITOS — para simular DOIS
    /// boots (duas "sessões") sobre o MESMO workspace, ambos com `seq` reiniciado em 0 (como
    /// no `main`). Sem seeds (o teste só exercita a admissão), bootstrap SEMPRE ligado (a
    /// política `Managed` só cria/persiste o cwd quando há `BootstrapWriter`).
    fn managed_manager(
        ws_root: &std::path::Path,
        store_dir: &std::path::Path,
        seq_start: u32,
    ) -> (NodeManager, Arc<Mutex<EventStore>>) {
        let store = Arc::new(Mutex::new(EventStore::open(store_dir).expect("open store")));
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        let cmd_factory: CmdFactory = Arc::new(|_name: &str| PtyCommand::new("cat"));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let bw = BootstrapWriter::new(
            ws_root.to_path_buf(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let nm = NodeManager::new(
            pty,
            sup,
            Arc::clone(&store),
            model,
            grids,
            BTreeMap::new(),
            delta_tx,
            80,
            24,
            seq_start,
            cmd_factory,
            Some(bw),
            ws_root.join(".lina"),
        );
        (nm, store)
    }

    /// **GATE de ADD (headless, determinístico):** `add_node` sobe a contagem 2→3, dá um grid
    /// ao novo nó, persiste a sequência CANÔNICA (ADR 0022 §2: NodeAdded + TerminalSpawned +
    /// NodeRoleAssigned — "terminal" também é papel, fim da classe dos 31-sem-papel), e o
    /// event log RECONSTRÓI o novo nó COM papel (`project()`) — "todo nó é shell real + o
    /// log é a fonte da verdade".
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
            before + 4,
            "persiste a sequência canônica: NodeAdded + TerminalSpawned + NodeRenamed + NodeRoleAssigned"
        );
        let projected = lock(&store).project().expect("project");
        assert!(
            projected.nodes.contains_key(&id),
            "o event log reconstrói o novo nó (fonte da verdade)"
        );
        assert_eq!(
            projected.nodes.get(&id).and_then(|n| n.role.clone()),
            Some("terminal".to_string()),
            "⌘T também grava papel no log (ADR 0022 §2 — nunca mais nó-órfão de papel)"
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

    /// **FIX DE GATE F1-0 (fiação final do lifecycle)** — o ciclo que faltava em produção:
    /// entrega marca o alvo `Busy` (F1-0-4, no router) → o FIM-DE-RESPOSTA do terminal (o mesmo
    /// sinal do medidor W0-10: idle do perfil fecha o turno — nunca um flush qualquer) devolve o
    /// nó a `Idle` no SUPERVISOR via transição event-sourced (`NodeStatusChanged{end_of_response}`
    /// no log) → a 2ª mensagem ao MESMO alvo **ENTREGA** (não retém até a DLQ). Exercita o
    /// `spawn_pump` REAL (thread + meter), não um atalho: é a fiação que o achado apontou ausente.
    #[test]
    fn end_of_response_returns_node_to_idle_and_unblocks_second_delivery() {
        let dir = std::env::temp_dir().join(format!("lina-fix-idle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lina = dir.join(".lina");
        let store = Arc::new(Mutex::new(
            EventStore::open(lina.join("events")).expect("store"),
        ));
        let sup = Arc::new(Supervisor::new());
        let mut router = Router::with_config(
            Arc::clone(&sup),
            Mailbox::new(&lina),
            RouterConfig::default(),
        );

        let _node_a = sup.register("@A", Some("dev".into()), Box::new(std::io::sink()));
        let node_b = sup.register("@B", Some("dev".into()), Box::new(std::io::sink()));

        // O pump REAL (idle_ms curto p/ o teste): é nele que mora a fiação do fix.
        let (delta_tx, delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        let bus_rx = sup.subscribe();
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        // R2b: a varredura viva da detecção é IRRELEVANTE p/ este teste de lifecycle
        // (sem grids) → `live_detection=false` mantém o foco no fim-de-resposta.
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let desk_t: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let attention_t = Arc::new(AttentionHub::new(
            Arc::clone(&store),
            desk_t,
            Arc::clone(&model),
            ApprovalWiring::new(
                Arc::clone(&sup),
                Arc::clone(&grids),
                ApprovalKeys::default(),
            ),
            test_nodes(Arc::clone(&store)),
        ));
        let mut pump = spawn_pump(
            GpuiBridgeHost::new(Arc::clone(&model)),
            delta_rx,
            bus_rx,
            Arc::clone(&store),
            50, // idle do perfil: 50ms fecha o turno (sinal de fim-de-resposta do medidor)
            Arc::clone(&sup),
            grids,
            attention_t,
            false,
        )
        .expect("pump");

        let mut deliver = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        };

        // (1) 1ª entrega @A→@B → o ROUTER marca @B Busy (F1-0-4).
        let m1 = MailMessage::new("@A", "@B", "ask", "tarefa 1");
        let out = {
            let mut s = lock(&store);
            router.route_message(&m1, &mut s, 1_000, &mut deliver)
        };
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "1ª msg entrega: {out:?}"
        );
        assert_eq!(
            sup.get(node_b).expect("@B").status,
            CoreStatus::Busy,
            "entrega marcou o alvo Busy (F1-0-4)"
        );

        // (2) FIM-DE-RESPOSTA real: @B produz output (GridDelta) e fica quieto ≥ idle_ms — o
        //     medidor fecha o turno e a fiação NOVA devolve @B a Idle no supervisor.
        delta_tx
            .send(GridDelta {
                node: node_b,
                rows: vec![0],
                bytes: 64,
                seq: 1,
            })
            .expect("delta");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if sup.get(node_b).map(|i| i.status) == Some(CoreStatus::Idle) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fim-de-resposta não devolveu @B a Idle em 3s (status: {:?})",
                sup.get(node_b).map(|i| i.status)
            );
            thread::sleep(Duration::from_millis(20));
        }

        // (2b) A transição é EVENT-SOURCED: `NodeStatusChanged{Idle, end_of_response}` no log.
        let recs = lock(&store).events().expect("events");
        assert!(
            recs.iter().any(|r| r.kind == "NodeStatusChanged"
                && r.payload.get("status").and_then(|v| v.as_str()) == Some("Idle")
                && r.payload.get("reason").and_then(|v| v.as_str()) == Some("end_of_response")),
            "NodeStatusChanged(end_of_response → Idle) no log; kinds: {:?}",
            recs.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>()
        );

        // (3) 2ª mensagem ao MESMO alvo → ENTREGA (sem o fix: Retained até o teto → DLQ).
        let m2 = MailMessage::new("@A", "@B", "ask", "tarefa 2");
        let out = {
            let mut s = lock(&store);
            router.route_message(&m2, &mut s, 2_000, &mut deliver)
        };
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "2ª msg ao mesmo alvo ENTREGA após o fim-de-resposta (não retém): {out:?}"
        );

        pump.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **F1-0-2 critério 5 (fiação)** — o resolver de perfil carrega o 1º candidato VÁLIDO:
    /// TOML inválido ganha warning e cai pro próximo; ausente é pulado; o claude-code.toml real
    /// parseia com os campos do fix (`ready_timeout_ms`/`busy_markers`) vivos.
    #[test]
    fn injection_profile_resolves_first_valid_candidate() {
        let dir = std::env::temp_dir().join(format!(
            "lina-e3-prof-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let invalid = dir.join("quebrado.toml");
        std::fs::write(&invalid, "isto nao é um profile").expect("write");
        let missing = dir.join("nao-existe.toml");
        // O claude-code.toml REAL do repo (o mesmo que o bundle embarca).
        let real = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root")
            .join("profiles")
            .join("claude-code.toml");

        let (prof, from) = resolve_profile(&[missing.clone(), invalid.clone(), real.clone()])
            .expect("resolve no candidato válido");
        assert_eq!(from, real, "pulou ausente + inválido, achou o real");
        assert_eq!(prof.id, "claude-code");
        // Os campos do fix F1-0-2 estão VIVOS (não defaults): timeout calibrado + busy markers.
        assert_eq!(prof.ready_timeout_ms, Some(30_000));
        assert!(
            prof.busy_markers
                .iter()
                .any(|m| m.contains("esc to interrupt")),
            "busy_markers do fix presentes: {:?}",
            prof.busy_markers
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Fallback-com-warning (critério 2 da rodada)** — nenhum candidato carregável → o perfil
    /// DEMO embutido entra no lugar (id `lina-demo`), sem panic. O warning é o `eprintln` do
    /// caminho (inspecionável no boot).
    #[test]
    fn injection_profile_falls_back_to_demo_with_warning() {
        let dir = std::env::temp_dir().join(format!("lina-e3-fb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let invalid = dir.join("ruim.toml");
        std::fs::write(&invalid, "[broken").expect("write");
        let prof = injection_profile_from(&[dir.join("ausente.toml"), invalid]);
        assert_eq!(
            prof.id, "lina-demo",
            "fallback = demo embutido, nunca panic"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ════════════ A2A UNIVERSAL: a entrega usa o profile do ALVO, não um global único ════════════

    /// Registry dos profiles REAIS do repo (claude-code + codex + …) — base não-vacuosa dos testes.
    fn repo_profile_registry() -> ProfileRegistry {
        ProfileRegistry::load_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles"),
        )
        .expect("profiles/ do repo deve carregar")
    }

    /// O FIX, provado NÃO-VACUOSAMENTE contra os profiles REAIS: a resolução por-alvo entrega o
    /// `prompt_ready_regex`/`busy_markers` do ALVO — alvo Claude → regex do Claude; alvo Codex →
    /// regex do `›` do Codex; os dois DIFEREM (a raiz do bug era usar o do Claude p/ TODO alvo →
    /// a TUI do Codex nunca casava → timeout de 30s).
    #[test]
    fn a2a_resolves_target_profile_not_global() {
        let reg = repo_profile_registry();
        let claude = reg
            .get("claude-code")
            .expect("claude-code no registry")
            .clone();
        let codex = reg.get("codex").expect("codex no registry").clone();
        assert_ne!(
            claude.prompt_ready_regex, codex.prompt_ready_regex,
            "claude e codex têm prompt_ready distintos — a raiz do bug do timeout"
        );

        // O fallback global é o claude-code (exatamente o que o bug usava p/ TODO alvo).
        let fallback = claude.clone();

        // Alvo Claude → profile do Claude.
        assert_eq!(
            target_profile(Some("claude-code"), &reg, &fallback).prompt_ready_regex,
            claude.prompt_ready_regex
        );
        // Alvo CODEX → profile do CODEX (regex do `›`), NÃO o global do Claude. (O FIX.)
        let resolved_codex = target_profile(Some("codex"), &reg, &fallback);
        assert_eq!(resolved_codex.prompt_ready_regex, codex.prompt_ready_regex);
        assert!(
            resolved_codex.prompt_ready_regex.contains('\u{203a}'),
            "alvo codex resolve o prompt_ready do glifo `›`, não o do Claude"
        );
        // Roteamento per-alvo: 2 alvos distintos → 2 prompt_ready distintos (fim do global único).
        assert_ne!(
            target_profile(Some("claude-code"), &reg, &fallback).prompt_ready_regex,
            target_profile(Some("codex"), &reg, &fallback).prompt_ready_regex,
            "cada alvo usa o prompt_ready do SEU profile"
        );
    }

    /// A2A UNIVERSAL — a entrega NUNCA falha por falta de profile: nó sem profile_id (shell puro /
    /// log antigo) ou id desconhecido → fallback global (degrada p/ timings genéricos, jamais "não
    /// entrega"). Controle positivo: id conhecido → o profile DELE.
    #[test]
    fn a2a_target_profile_falls_back_safely() {
        let reg = repo_profile_registry();
        let fallback = reg.get("claude-code").expect("claude-code").clone();
        assert_eq!(
            target_profile(None, &reg, &fallback).id,
            "claude-code",
            "sem profile_id → fallback"
        );
        assert_eq!(
            target_profile(Some("cli-inexistente"), &reg, &fallback).id,
            "claude-code",
            "id desconhecido → fallback (nunca panic)"
        );
        assert_eq!(
            target_profile(Some("codex"), &reg, &fallback).id,
            "codex",
            "id conhecido → o profile dele (controle positivo)"
        );
    }

    /// A2A UNIVERSAL — a cadeia COMPLETA que o `deliver_fn` do pump usa: mapa `node→profile_id`
    /// (projetado do log, campo `cli` de `CliProfileSet`) → resolução per-alvo. Dois nós com
    /// profiles distintos resolvem CADA UM o seu; nó sem entrada (terminal puro) cai no fallback.
    #[test]
    fn a2a_cli_by_node_routes_each_target_to_its_own_profile() {
        let reg = repo_profile_registry();
        let fallback = reg.get("claude-code").expect("claude-code").clone();
        let na: NodeId = uuid::Uuid::now_v7(); // alvo Claude
        let nb: NodeId = uuid::Uuid::now_v7(); // alvo Codex
        let nc: NodeId = uuid::Uuid::now_v7(); // terminal puro (sem profile_id)
        let mut cli_by_node: std::collections::BTreeMap<NodeId, String> =
            std::collections::BTreeMap::new();
        cli_by_node.insert(na, "claude-code".into());
        cli_by_node.insert(nb, "codex".into());

        // EXATAMENTE o que o `deliver_fn` faz: cli_by_node.get(alvo) → registry → profile.
        let pa = target_profile(cli_by_node.get(&na).map(String::as_str), &reg, &fallback);
        let pb = target_profile(cli_by_node.get(&nb).map(String::as_str), &reg, &fallback);
        let pc = target_profile(cli_by_node.get(&nc).map(String::as_str), &reg, &fallback);
        assert_eq!(pa.id, "claude-code");
        assert_eq!(pb.id, "codex");
        assert_eq!(pc.id, "claude-code", "terminal sem profile_id → fallback");
        assert_ne!(
            pa.prompt_ready_regex, pb.prompt_ready_regex,
            "cada alvo usa o prompt_ready do SEU profile"
        );
    }

    /// A ordem dos candidatos honra a doutrina: override de env → dir do usuário (`<.lina>/
    /// profiles`, que o modal semeia) → bundle (ancestrais do exe) → repo (dev). O bundle vir
    /// ANTES do repo é o que torna o .app auto-contido PROVÁVEL sem esconder o repo.
    #[test]
    fn injection_candidates_order_user_dir_before_repo() {
        let home = std::path::Path::new("/tmp/fake-lina-home");
        let cands = injection_profile_candidates(Some(home));
        let user_pos = cands
            .iter()
            .position(|p| p.starts_with(home))
            .expect("dir do usuário está na lista");
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root")
            .join("profiles")
            .join("claude-code.toml");
        let repo_pos = cands
            .iter()
            .position(|p| *p == repo)
            .expect("repo está na lista (dev)");
        assert!(
            user_pos < repo_pos,
            "usuário ({user_pos}) vence o repo ({repo_pos}): {cands:?}"
        );
        // todo candidato termina no arquivo certo.
        assert!(cands.iter().all(|p| p.ends_with("claude-code.toml")));
    }

    /// **W4-2 (M2) — GATE headless de "Novo Agente":** `create_agent("Revisor")` cria um nó-terminal
    /// (CLI real spawned via `cat` no teste), DERIVA o papel `reviewer`, GRAVA `NodeRoleAssigned` no
    /// log (→ projeção `ProjectedNode.role` = reviewer; sobrevive a replay), e projeta no model com o
    /// nome. Nome inválido é recusado sem efeito.
    #[test]
    fn create_agent_derives_role_records_and_projects() {
        let (nm, store, model) = test_manager("m2", None);
        assert_eq!(lock(&model).order.len(), 2, "começa com 2 nós");

        let id = nm
            .create_agent_with("Revisor", None, None, None, false)
            .expect("M2/M6 cria o agente");
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
        assert!(
            nm.create_agent_with("  ", None, None, None, false).is_err(),
            "nome vazio recusado"
        );
        assert!(
            nm.create_agent_with("a{{b}}", None, None, None, false)
                .is_err(),
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

    /// **BUG C — coords do arraste estendem CONTÍNUO (sem desselecionar).** Fixa a âncora e arrasta
    /// linha a linha p/ baixo: cada passo é um SUPERCONJUNTO do anterior (a contagem de células
    /// membros só cresce) e o head acompanha o cursor 1:1. Isso prova que a LÓGICA de coords sempre
    /// esteve correta — a causa raiz do bug relatado ("não estende / desselecciona") era de RENDER:
    /// a view quiescente não repintava o gesto sem `cx.notify()` (corrigido no `on_mouse_move`). A
    /// confirmação VISUAL ao vivo num Claude = PENDENTE TELA DO FUNDADOR (gpui não roda headless).
    #[test]
    fn drag_extends_selection_continuously_in_viewport() {
        let cam = Camera::default();
        let card = (30.0_f32, 96.0_f32);
        let dims = (80usize, 24usize);
        let z = cam.zoom;
        let (sx, sy) = cam.world_to_screen(card);
        let (gx, gy) = (sx + GRID_OX, sy + GRID_OY_FIXED + CELL_H * z);
        let col_px = |c: f32| gx + (c + 0.5) * CELL_W * z;
        let row_px = |r: f32| gy + (r + 0.5) * CELL_H * z;
        // Âncora no MEIO de uma linha (col 10, row 2).
        let anchor = screen_to_cell(&cam, card, (col_px(10.0), row_px(2.0)), dims);
        assert_eq!(anchor, (10, 2));
        let mut prev_members = 0usize;
        for row in 2..=20u32 {
            let head = screen_to_cell(&cam, card, (col_px(10.0), row_px(row as f32)), dims);
            assert_eq!(
                head,
                (10, row as usize),
                "head acompanha o cursor linha a linha"
            );
            let (sc, sr, ec, er) = normalize_sel(anchor, head);
            let members = (0..dims.1)
                .flat_map(|r| (0..dims.0).map(move |c| (c, r)))
                .filter(|&(c, r)| cell_in_selection(c, r, sc, sr, ec, er))
                .count();
            assert!(
                members >= prev_members,
                "estender p/ baixo NUNCA desseleciona (row={row}, members={members}, prev={prev_members})"
            );
            prev_members = members;
        }
        assert!(
            prev_members > dims.0,
            "seleção multi-linha real (não presa a uma sub-região de 1 linha)"
        );
    }

    /// **BUG B — caminho(s) solto(s) → string CITADA p/ injeção no PTY** (aspas simples POSIX; aspa
    /// interna vira `'\''`; vários separados por espaço; termina com 1 espaço). Vazio → string vazia.
    #[test]
    fn dropped_paths_are_single_quoted_and_space_safe() {
        use std::path::PathBuf;
        assert_eq!(
            quote_dropped_paths(&[PathBuf::from("/Users/x/SOP CUSTOMER SERVICE FCL.pdf")]),
            "'/Users/x/SOP CUSTOMER SERVICE FCL.pdf' "
        );
        // Aspa simples interna → fecha-escapa-reabre `'\''`.
        assert_eq!(
            quote_dropped_paths(&[PathBuf::from("/tmp/it's a file.txt")]),
            "'/tmp/it'\\''s a file.txt' "
        );
        // Vários caminhos → cada um citado, separados por espaço.
        assert_eq!(
            quote_dropped_paths(&[PathBuf::from("/a/b.png"), PathBuf::from("/c d/e.mov")]),
            "'/a/b.png' '/c d/e.mov' "
        );
        // Vazio → o chamador NÃO escreve no PTY.
        assert!(quote_dropped_paths(&[]).is_empty());
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
        let (nm, store, model) = test_manager("boot", Some(bw));
        nm.rewrite_bootstrap(); // seed inicial: escreve A e B.

        let a_md = ws.join("A").join("CLAUDE.md");
        let a1 = std::fs::read_to_string(&a_md).expect("CLAUDE.md de A");
        // Doutrina canônica: 8 BLOCOS marcados `<!-- ===== BLOCO N · … -->` (1ª e última presentes).
        assert!(
            a1.contains("===== BLOCO 1 ·") && a1.contains("===== BLOCO 8 ·"),
            "8 blocos canônicos"
        );
        assert!(a1.contains("Terminal B"), "A lista B como colega");

        // ADD: novo AGENTE C → CLAUDE.md próprio (no cwd dele) e o de A REESCRITO com C. Pós
        // terminal-puro, o ⌘T (`add_node`) é PURO (HOME, sem kit); a escrita/reescrita de kit é
        // invariante de AGENTE gerenciado (⌘N sem pasta = `Managed`), então criamos C por aí.
        let c = nm
            .create_agent_with("Terminal C", None, None, None, false)
            .expect("add C (agente gerenciado)");
        // O cwd de C é um dir gerenciado ÚNICO (`<ws>/n-<uuid>`), não mais o slot reciclável
        // `t{seq}` — lido da projeção (costura canônica), nunca do nome de dir hardcoded.
        let c_cwd = lock(&store)
            .project()
            .expect("project")
            .nodes
            .get(&c)
            .and_then(|nd| nd.cwd.clone())
            .expect("cwd gerenciado de C persistido");
        assert!(
            std::path::Path::new(&c_cwd).join("CLAUDE.md").exists(),
            "C tem CLAUDE.md no seu cwd gerenciado"
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

    /// **Regressão (bug de ordem do vault):** o `vault_path` é resolvido no boot, ANTES de o onboarding
    /// confirmar os vaults — então ficava no fallback `<ws_root>/vault` (inexistente). Ao linkar um vault
    /// (gravando `.lina/vault.json`), a doutrina REGENERADA deve apontar para o vault REAL, não o fallback.
    #[test]
    fn bootstrap_picks_linked_vault_over_boot_fallback() {
        let ws = std::env::temp_dir().join(format!("lina-vaultfix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        // Boot: vault_path cai no FALLBACK (o que o main.rs resolve quando ainda não há vault.json).
        let fallback = ws.join("vault").display().to_string();
        let bw = BootstrapWriter::new(
            ws.clone(),
            fallback.clone(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        // ANTES de linkar: a doutrina usa o fallback.
        let real_vault = "/Users/teste/Documents/Meu Vault";
        bw.write_one("X", "Terminal X", &["Terminal X".to_string()]);
        let x_md = ws.join("X").join("CLAUDE.md");
        let before = std::fs::read_to_string(&x_md).expect("CLAUDE.md de X");
        assert!(
            before.contains(&fallback),
            "antes de linkar: usa o fallback"
        );
        assert!(
            !before.contains(real_vault),
            "antes de linkar: não conhece o vault real"
        );

        // Usuário linka o vault no onboarding → grava .lina/vault.json (escrita do obsidian.rs).
        let cfg = crate::obsidian::VaultConfig {
            primary: real_vault.to_string(),
            vaults: vec![crate::obsidian::VaultEntry {
                name: "Meu Vault".to_string(),
                path: real_vault.to_string(),
                writable: format!("{real_vault}/Lina"),
            }],
        };
        crate::obsidian::write_vault_config(&ws.join(".lina"), &cfg).expect("grava vault.json");

        // DEPOIS de linkar: a MESMA regeneração passa a apontar pro vault REAL (re-lê o vault.json).
        bw.write_one("X", "Terminal X", &["Terminal X".to_string()]);
        let after = std::fs::read_to_string(&x_md).expect("re-read X");
        assert!(
            after.contains(real_vault),
            "depois de linkar: usa o vault REAL"
        );
        assert!(
            !after.contains(&fallback),
            "depois de linkar: não usa mais o fallback inexistente"
        );

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

        // R2c (T3): a skill INSTALADA traz os gatilhos pt-br do fundador (vence a skill
        // estrangeira por matching de descrição) E o alerta anti-Maestri; o `alias
        // maestri` (reforço do bug) sumiu.
        assert!(
            body.contains("mande") && body.contains("peça") && body.contains("avise"),
            "descrição casa com a fala pt-br do fundador (mande/peça/avise)"
        );
        assert!(
            body.contains("Maestri") && body.contains("MAESTRI_CLI"),
            "a skill alerta contra o orquestrador estrangeiro"
        );
        assert!(
            !body.contains("alias `maestri`"),
            "o 'alias maestri' enganoso foi removido da skill"
        );

        let _ = std::fs::remove_dir_all(&cwd);
    }

    // ─────────────── F1-2-3 p2 (seam Dev 03): papéis custom do log ───────────────

    /// O decode devolve as linhas EM ORDEM DE LOG, SEM dedup (o last-wins é do modal —
    /// doutrina do evento no core), e payload PARCIAL (shape antigo) vira defaults, nunca
    /// quebra. Round-trip: o que `RoleTemplateSaved` grava é o que o decode lê.
    #[test]
    fn role_templates_decode_in_log_order_without_dedup() {
        let base = std::env::temp_dir().join(format!("lina-roletpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut store = EventStore::open(base.join("events")).expect("store");
        for (n, d) in [("Revisora", "v1"), ("Designer", "d1"), ("Revisora", "v2")] {
            store
                .append(&DomainEvent::RoleTemplateSaved {
                    name: n.into(),
                    description: d.into(),
                    doctrine: format!("doutrina {d}"),
                    ts: now_ms(),
                })
                .expect("append");
        }
        let got = role_templates_from_records(&store.events().expect("events"));
        assert_eq!(got.len(), 3, "SEM dedup — as 3 linhas, em ordem de log");
        assert_eq!(
            (got[0].name.as_str(), got[0].description.as_str()),
            ("Revisora", "v1")
        );
        assert_eq!(got[1].name.as_str(), "Designer");
        assert_eq!(
            (got[2].name.as_str(), got[2].description.as_str()),
            ("Revisora", "v2"),
            "a duplicata de name SOBREVIVE (last-wins é decisão do modal)"
        );
        assert!(got[2].doctrine.contains("v2"));
        let _ = std::fs::remove_dir_all(&base);
    }

    // ─────────────── F1-1-7 · AttentionHub (fiação core AttentionQueue ↔ app) ───────────────

    /// Hub de teste enraizado num store temp + desk próprio. Devolve também o caminho
    /// p/ limpeza.
    fn attention_hub(tag: &str) -> (AttentionHub, Desk, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("lina-atthub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        // F1-1-8: estes testes (F1-1-7) usam itens hook-only (sem Captura 1) → caminho
        // audit-only, que NÃO exercita a porta — wiring stub (sup/grids vazios) basta.
        let wiring = ApprovalWiring::new(
            Arc::new(Supervisor::new()),
            Arc::new(Mutex::new(BTreeMap::new())),
            ApprovalKeys::default(),
        );
        let nodes = test_nodes(Arc::clone(&store));
        let hub = AttentionHub::new(store, Arc::clone(&desk), model, wiring, nodes);
        (hub, desk, base)
    }

    /// SEAM-1 (M2): a tradução `Autonomy → AutonomyLevel` é 1:1 (o que fia o gate manual/cascata no
    /// `RouterConfig` de produção; o core já prova `RouterConfig{Manual} → SpawnBlocked`).
    #[test]
    fn autonomy_conversion_is_one_to_one() {
        assert_eq!(
            autonomy_to_level(Autonomy::Manual),
            lina_core::AutonomyLevel::Manual
        );
        assert_eq!(
            autonomy_to_level(Autonomy::Assisted),
            lina_core::AutonomyLevel::Assisted
        );
        assert_eq!(
            autonomy_to_level(Autonomy::Autonomous),
            lina_core::AutonomyLevel::Autonomous
        );
    }

    /// **SEAM-1 (banner + M3, app-side end-to-end):** um spawn CASCATA (`SpawnRequested`+`SpawnGated
    /// {cascade}` no log) vira banner pendente; APROVAR → 1 terminal NASCE (NodeAdded{requested_by}+
    /// SpawnAdmitted) pelo funil único; RE-APROVAR → dedupe DURÁVEL (M3) → SEM 2º terminal; um outro
    /// spawn RECUSADO → `SpawnDeclined`, nada nasce.
    #[test]
    fn attention_hub_spawn_banner_approves_once_and_declines() {
        let base = std::env::temp_dir().join(format!("lina-spawn-banner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let wiring = ApprovalWiring::new(
            Arc::new(Supervisor::new()),
            Arc::new(Mutex::new(BTreeMap::new())),
            ApprovalKeys::default(),
        );
        let nodes = test_nodes(Arc::clone(&store));
        let hub = AttentionHub::new(
            Arc::clone(&store),
            Arc::clone(&desk),
            Arc::clone(&model),
            wiring,
            Arc::clone(&nodes),
        );
        let spawner = uuid::Uuid::from_u128(1);
        let push = |id: &str, name: &str| {
            let mut s = lock(&store);
            s.append(&DomainEvent::SpawnRequested {
                id: id.into(),
                requested_by: spawner,
                name: name.into(),
                role: "qa".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "valide o checkout".into(),
            })
            .unwrap();
            s.append(&DomainEvent::SpawnGated {
                id: id.into(),
                requested_by: spawner,
                reason: "cascade".into(),
            })
            .unwrap();
        };
        let admitted = |id: &str| {
            lock(&store)
                .events()
                .unwrap()
                .into_iter()
                .filter(|r| r.kind == "SpawnAdmitted" && r.payload["id"].as_str() == Some(id))
                .count()
        };

        // Cascata → banner pendente (após sync).
        push("msg_a", "@QA");
        assert!(
            hub.sync(1000)
                .iter()
                .any(|i| i.kind == lina_core::AttentionKind::Spawn && i.stable_id == "msg_a"),
            "SpawnGated{{cascade}} vira banner pendente"
        );

        // APROVA → 1 terminal nasce (SpawnAdmitted + NodeAdded com requested_by).
        assert!(
            hub.resolve_spawn("msg_a", true),
            "aprovação executa o spawn"
        );
        assert_eq!(admitted("msg_a"), 1, "1 SpawnAdmitted (1 terminal)");
        let req_by_stamped = lock(&store).events().unwrap().into_iter().any(|r| {
            r.kind == "NodeAdded"
                && r.payload.get("requested_by").and_then(|v| v.as_str())
                    == Some(spawner.to_string().as_str())
        });
        assert!(
            req_by_stamped,
            "NodeAdded carimba requested_by = solicitante"
        );

        // RE-APROVA (queue ainda acha pendente sem re-sync) → M3 dedupe DURÁVEL: SEM 2º terminal.
        let _ = hub.resolve_spawn("msg_a", true);
        assert_eq!(admitted("msg_a"), 1, "M3: re-aprovar NÃO cria 2º terminal");

        // Outro spawn → RECUSA → SpawnDeclined, nada admitido.
        push("msg_b", "@Helper");
        hub.sync(2000);
        assert!(
            hub.resolve_spawn("msg_b", false),
            "recusa registra a decisão"
        );
        assert!(
            lock(&store)
                .events()
                .unwrap()
                .into_iter()
                .any(|r| r.kind == "SpawnDeclined" && r.payload["id"].as_str() == Some("msg_b")),
            "recusa apenda SpawnDeclined"
        );
        assert_eq!(admitted("msg_b"), 0, "recusado → nada nasce");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn ask_event(sid: &str, node: &str, detail: &str) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node.into(),
            tool: Some("Bash".into()),
            detail: Some(detail.into()),
            evidence: lina_core::PermissionEvidence::Hook,
            stable_id: sid.into(),
            // R2b: shape novo do contrato — y/n aprovável; sem Captura 1 (origem hook).
            prompt_kind: lina_core::attention::PromptKind::Yn,
            vt_snapshot_hash: None,
        }
    }

    /// O gesto APROVAR/RECUSAR registra `PermissionResolved` no log (audit — critério 3
    /// da story), remove a pendência, e o CLIQUE DUPLO não gera segundo evento
    /// (idempotência da fila). NENHUM write no PTY existe neste caminho (ADR 0021 §6 —
    /// auditável: o hub não tem handle de PTY; ver AC-0021.6).
    #[test]
    fn attention_hub_resolve_appends_once_and_removes_pending() {
        let (hub, _desk, base) = attention_hub("resolve");
        hub.store_append_for_test(&ask_event("s1", "Terminal B", "git push origin/master"));
        let items = hub.sync(1_000);
        assert_eq!(items.len(), 1, "pendência visível após o sync");
        assert_eq!(items[0].stable_id, "s1");

        assert!(hub.resolve(
            "s1",
            lina_core::ApprovalDecision::Approve,
            lina_core::ResolutionVia::Human
        ));
        assert!(hub.sync(2_000).is_empty(), "decisão remove o toast/fila");
        // Clique duplo: idempotente — NÃO apenda um 2º Resolved.
        assert!(!hub.resolve(
            "s1",
            lina_core::ApprovalDecision::Approve,
            lina_core::ResolutionVia::Human
        ));
        let resolved = hub
            .events_for_test()
            .into_iter()
            .filter(|r| r.kind == "PermissionResolved")
            .count();
        assert_eq!(resolved, 1, "exatamente 1 evento de decisão");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// «Não era um pedido» (mitigação de FP): registra `PermissionDismissed` (NÃO é
    /// deny) e remove a pendência.
    #[test]
    fn attention_hub_dismiss_records_false_positive() {
        let (hub, _desk, base) = attention_hub("dismiss");
        hub.store_append_for_test(&ask_event("s1", "Terminal A", "(y/n) fake"));
        assert_eq!(hub.sync(1_000).len(), 1);
        assert!(hub.dismiss("s1"));
        assert!(hub.sync(2_000).is_empty());
        assert!(!hub.dismiss("s1"), "já saiu — sem 2º evento");
        let dismissed = hub
            .events_for_test()
            .into_iter()
            .filter(|r| r.kind == "PermissionDismissed")
            .count();
        assert_eq!(dismissed, 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A custódia VIVA do `CustodyDesk` aparece na MESMA fila (unificação — critério 1),
    /// com `first_seen` ESTÁVEL entre syncs (a escalação dos 5 min não reseta) e
    /// PRECEDÊNCIA sobre permissão (ordenação do core). Decidir custódia pela fila é
    /// IMPOSSÍVEL por construção (`resolve` → None; o canal é o ⌘⏎ existente).
    #[test]
    fn attention_hub_mirrors_live_custody_with_stable_first_seen() {
        let (hub, desk, base) = attention_hub("custody");
        hub.store_append_for_test(&ask_event("p1", "Terminal A", "git push"));
        lock(&desk).queue.push_back(PendingGate::Custody {
            id: "msg_c1".into(),
            action: "deploy".into(),
            secret_key: "prod".into(),
            requester: "Terminal B".into(),
            display: "🔒 CUSTODIA: deploy".into(),
        });

        let items = hub.sync(5_000);
        assert_eq!(items.len(), 2, "custódia + permissão na MESMA fila");
        assert_eq!(
            items[0].kind,
            lina_core::AttentionKind::Custody,
            "custódia na frente (precedência do core)"
        );
        assert_eq!(items[0].created_ts, 5_000, "first_seen = 1º sync");
        // Re-sync mais tarde: o first_seen NÃO anda (escalação estável).
        let items = hub.sync(9_000);
        assert_eq!(items[0].created_ts, 5_000, "first_seen estável entre syncs");
        // Decidir custódia pela fila: bloqueado por construção (zero regressão ⌘⏎).
        assert!(!hub.resolve(
            "msg_c1",
            lina_core::ApprovalDecision::Approve,
            lina_core::ResolutionVia::Human
        ));
        // A pump resolveu o gate (⌘⏎ atual) → some da fila no próximo sync.
        lock(&desk).queue.pop_front();
        let items = hub.sync(10_000);
        assert_eq!(items.len(), 1, "custódia saiu; a permissão fica");
        assert_eq!(items[0].stable_id, "p1");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Crash com pendência → REABRIR reconstrói a fila do log (critério 5 da story):
    /// um hub NOVO sobre o mesmo store re-projeta o pendente.
    #[test]
    fn attention_hub_rebuilds_pending_from_log_after_restart() {
        let (hub, _desk, base) = attention_hub("replay");
        hub.store_append_for_test(&ask_event("s1", "Terminal B", "npm publish"));
        assert_eq!(hub.sync(1_000).len(), 1);
        drop(hub);
        // "Reabre o app": hub novo, mesmo log no disco.
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("reabrir store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let wiring = ApprovalWiring::new(
            Arc::new(Supervisor::new()),
            Arc::new(Mutex::new(BTreeMap::new())),
            ApprovalKeys::default(),
        );
        let nodes = test_nodes(Arc::clone(&store));
        let hub2 = AttentionHub::new(store, desk, model, wiring, nodes);
        let items = hub2.sync(2_000);
        assert_eq!(items.len(), 1, "pendência reconstruída do log");
        assert_eq!(items[0].stable_id, "s1");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// «Silenciar detecção deste terminal»: persiste `NodeDetectionMuted` (reversível,
    /// último vence) e o estado é legível p/ o rótulo do botão.
    #[test]
    fn attention_hub_node_mute_persists_and_reads_back() {
        let (hub, _desk, base) = attention_hub("mute");
        assert!(!hub.is_node_muted("Terminal A"));
        assert!(hub.set_node_muted("Terminal A", true));
        assert!(hub.is_node_muted("Terminal A"));
        assert!(hub.set_node_muted("Terminal A", false));
        assert!(!hub.is_node_muted("Terminal A"));
        let muted_events = hub
            .events_for_test()
            .into_iter()
            .filter(|r| r.kind == "NodeDetectionMuted")
            .count();
        assert_eq!(muted_events, 2, "liga + desliga persistidos no log");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ─────────── F1-1-8 / ADR 0024 · porta de escrita do hub (gesto humano + auto-deny) ───────────

    /// Sink observável (escreve num `Vec` compartilhado) — o "log de writes do master" do nó.
    struct VecSink(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for VecSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Hub WIRED (F1-1-8): `Supervisor` real + 1 nó registrado (sink observável) + grid no mapa —
    /// o caminho completo do app. Devolve o buffer de writes do master, a Captura 1 (hash da tela
    /// atual) e o grid (para mutar nos testes de tela-divergente).
    fn wired_hub(
        tag: &str,
        node_name: &str,
        screen: &[u8],
    ) -> (
        AttentionHub,
        Arc<Mutex<Vec<u8>>>,
        String,
        Grid,
        std::path::PathBuf,
    ) {
        let base =
            std::env::temp_dir().join(format!("lina-atthub-wired-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));

        let sup = Arc::new(Supervisor::new());
        let buf = Arc::new(Mutex::new(Vec::new()));
        let node = sup.register(node_name, None, Box::new(VecSink(Arc::clone(&buf))));

        let grid: Grid = {
            let mut vt = AlacrittyBackend::new(80, 24);
            vt.advance(screen);
            Arc::new(Mutex::new(Box::new(vt)))
        };
        let captura1 = lina_core::prompt_snapshot_hash(&**lock(&grid), PROMPT_REGION_ROWS);
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        lock(&grids).insert(node, Arc::clone(&grid));

        let wiring = ApprovalWiring::new(Arc::clone(&sup), grids, ApprovalKeys::default());
        let nodes = test_nodes(Arc::clone(&store));
        let hub = AttentionHub::new(store, desk, model, wiring, nodes);
        (hub, buf, captura1, grid, base)
    }

    /// `PermissionAsked` Yn de GRID com a Captura 1 (`hash`) — o que o detector da F1-1-6 anexaria.
    fn yn_ask(node: &str, sid: &str, hash: &str) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node.into(),
            tool: Some("Bash".into()),
            detail: Some("git push origin/master".into()),
            evidence: lina_core::PermissionEvidence::Grid,
            stable_id: sid.into(),
            vt_snapshot_hash: Some(hash.into()),
            prompt_kind: lina_core::attention::PromptKind::Yn,
        }
    }

    /// **Gesto humano APROVAR com Captura 1 → porta única → write `y\r` validado contra a tela.**
    /// Binding e `expected_hash` vêm do LOG; o clique só carrega o `stable_id`. Clique duplo é no-op
    /// (idempotência da fila) — não re-escreve. Audit completo: Resolved + ApprovalInjected.
    #[test]
    fn attention_hub_resolve_with_capture_writes_approve_keys() {
        let (hub, buf, h, _grid, base) =
            wired_hub("approve", "Terminal A", b"Deploy to prod? (y/n) ");
        hub.store_append_for_test(&yn_ask("Terminal A", "s1", &h));
        assert_eq!(hub.sync(1_000).len(), 1, "pendência visível");

        assert!(hub.resolve("s1", ApprovalDecision::Approve, ResolutionVia::Human));
        assert_eq!(
            *lock(&buf),
            b"y\r".to_vec(),
            "approve digita as approve_keys no master"
        );
        assert!(hub.sync(2_000).is_empty(), "item resolvido sai da fila");

        // Clique duplo: no-op (item já saiu) — não re-escreve.
        assert!(!hub.resolve("s1", ApprovalDecision::Approve, ResolutionVia::Human));
        assert_eq!(*lock(&buf), b"y\r".to_vec(), "clique duplo NÃO re-escreve");

        let recs = hub.events_for_test();
        assert_eq!(
            recs.iter()
                .filter(|r| r.kind == "PermissionResolved")
                .count(),
            1
        );
        assert_eq!(
            recs.iter().filter(|r| r.kind == "ApprovalInjected").count(),
            1
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Tela DIVERGIU entre a Captura 1 e o gesto (caminho completo do app):** resolve → executor →
    /// porta → `deliver_approval_with_grid` re-snapshota o grid mudado ⇒ `ScreenChanged`, ZERO bytes
    /// ao master; audit = Resolved + ApprovalAborted (SEM Injected).
    #[test]
    fn attention_hub_resolve_aborts_with_zero_bytes_when_screen_changed() {
        let (hub, buf, h, grid, base) =
            wired_hub("changed", "Terminal A", b"Deploy to prod? (y/n) ");
        hub.store_append_for_test(&yn_ask("Terminal A", "s1", &h));
        assert_eq!(hub.sync(1_000).len(), 1);

        // A tela muda ANTES do clique (output novo do CLI).
        lock(&grid).advance(b"\r\nWARNING: lock changed\r\nDeploy to prod? (y/n) ");

        assert!(hub.resolve("s1", ApprovalDecision::Approve, ResolutionVia::Human));
        assert!(
            lock(&buf).is_empty(),
            "tela divergiu ⇒ ZERO bytes ao master"
        );
        let recs = hub.events_for_test();
        assert_eq!(
            recs.iter()
                .filter(|r| r.kind == "PermissionResolved")
                .count(),
            1
        );
        assert_eq!(
            recs.iter().filter(|r| r.kind == "ApprovalInjected").count(),
            0,
            "sem Injected"
        );
        assert_eq!(
            recs.iter().filter(|r| r.kind == "ApprovalAborted").count(),
            1,
            "auditado como ApprovalAborted"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **AUTO-DENY do SLA (10 min) end-to-end:** a pendência Yn vence → `drive_auto_deny` drena pela
    /// MESMA porta com `Deny/Timeout` → digita as DENY keys (`n\r`), NUNCA approve. Antes dos 10 min,
    /// zero bytes (controle). Idempotente: a 2ª passada não re-escreve (item já resolvido). É o teste
    /// 5c do seam — prova o caminho do app, complementar ao AC-0021.5 do gate (executor isolado).
    #[test]
    fn attention_hub_auto_deny_writes_deny_keys_after_ten_minutes_never_approve() {
        let (hub, buf, h, _grid, base) = wired_hub(
            "autodeny",
            "Terminal A",
            b"$ deploy\r\nDeploy to prod? (y/n) ",
        );
        hub.store_append_for_test(&yn_ask("Terminal A", "s1", &h));
        let ts = hub.events_for_test().last().expect("ask no log").ts;
        let auto = lina_core::attention::AUTO_DENY_AFTER_MS;

        // Controle: antes dos 10 min nada vence → zero bytes.
        hub.drive_auto_deny(ts + auto - 1);
        assert!(lock(&buf).is_empty(), "antes de 10 min: zero bytes");

        // Aos 10 min: auto-deny digita as DENY keys (n\r) — NUNCA approve (seria y\r).
        hub.drive_auto_deny(ts + auto);
        assert_eq!(
            *lock(&buf),
            b"n\r".to_vec(),
            "auto-deny digita DENY (n\\r), nunca approve (y\\r)"
        );

        let recs = hub.events_for_test();
        assert_eq!(
            recs.iter()
                .filter(|r| r.kind == "PermissionResolved")
                .count(),
            1
        );
        assert_eq!(
            recs.iter().filter(|r| r.kind == "ApprovalInjected").count(),
            1
        );
        // Item resolvido sai da fila; 2ª passada não re-escreve (idempotência via fila).
        assert!(hub.sync(ts + auto + 1).is_empty());
        hub.drive_auto_deny(ts + auto + 2);
        assert_eq!(
            *lock(&buf),
            b"n\r".to_vec(),
            "2ª passada não re-escreve (já resolvido)"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // ─────────────── R2b · AppPermissionWatch (metade-app da fiação viva) ───────────────

    /// Grid com a CAIXA DE ESCOLHA real da tela do fundador (20:30) + roster com o nó.
    fn choice_grid_setup(
        tag: &str,
    ) -> (
        Arc<Supervisor>,
        NodeId,
        Grid,
        Arc<Mutex<EventStore>>,
        PathBuf,
    ) {
        let base = std::env::temp_dir().join(format!("lina-appwatch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let store = Arc::new(Mutex::new(
            EventStore::open(base.join("events")).expect("store"),
        ));
        let sup = Arc::new(Supervisor::new());
        let node = sup.register("Terminal A", None, Box::new(std::io::sink()));
        let grid: Grid = Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(80, 24))));
        lock(&grid).advance(
            b"Isto e um teste de multipla escolha. Qual opcao voce quer selecionar?\r\n\
              \xE2\x9D\xAF 1. Opcao A\r\n  2. Opcao B\r\n  3. Opcao C\r\n\
              Enter to select \xC2\xB7 \xE2\x86\x91\xE2\x86\x93 to navigate \xC2\xB7 Esc to cancel\r\n",
        );
        (sup, node, grid, store, base)
    }

    /// **Metade-app da fiação viva (R2b):** com a caixa de escolha REAL parada no grid
    /// e silêncio de output ≥1,5s, UM scan apenda `PermissionAsked{prompt_kind:choice}`
    /// no log com o NOME do roster (ADR 0021 §5) e transiciona o nó p/ Blocked
    /// (`reason=permission_prompt`); scans seguintes NÃO duplicam (re-arm do detector).
    /// Output vivo (sem idle) nunca emite.
    #[test]
    fn app_watch_appends_choice_ask_blocks_node_without_duplicates() {
        let (sup, node, grid, store, base) = choice_grid_setup("choice");
        let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
        grids.insert(node, Arc::clone(&grid));
        let mut lifecycle = lina_core::lifecycle::LifecycleEngine::new();
        let mut watch = AppPermissionWatch::new();
        let t0 = 1_000_000_u64;
        watch.note_output(node, t0);

        // Ainda "vivo" (sem 1,5s de silêncio): nada emite.
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 200);
        assert!(out.is_empty(), "output vivo nunca emite");
        // Silêncio ≥ idle: emite UMA vez, com kind=choice e nome do roster.
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 1_600);
        assert_eq!(out.asks.len(), 1, "prompt parado + idle ⇒ 1 pedido");
        let recs = lock(&store).events().expect("events");
        let ask = recs
            .iter()
            .find(|r| r.kind == "PermissionAsked")
            .expect("PermissionAsked no log");
        assert_eq!(ask.payload["prompt_kind"], "choice");
        assert_eq!(
            ask.payload["node_id"], "Terminal A",
            "nome do roster, não uuid"
        );
        assert!(
            ask.payload["vt_snapshot_hash"].as_str().is_some(),
            "Captura 1 presente (origem grid)"
        );
        assert!(
            recs.iter().any(
                |r| r.kind == "NodeStatusChanged" && r.payload["reason"] == "permission_prompt"
            ),
            "nó Blocked auditado: {recs:?}"
        );
        // Re-scan: o re-arm do detector NÃO duplica o mesmo prompt.
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 3_000);
        assert!(out.is_empty(), "mesmo prompt não re-emite");
        let asks = lock(&store)
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "PermissionAsked")
            .count();
        assert_eq!(asks, 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **R2b item 5 (contrato aprovado) — ciclo cleared FIM-A-FIM:** Watch detecta →
    /// item na fila do hub; respondido NO TERMINAL → `PermissionPromptCleared` apendado
    /// → o item SOME de `items()` SEM evento de decisão (nem Resolved nem Dismissed).
    /// Distinto do dismiss (FP): a telemetria de FP NÃO é tocada.
    #[test]
    fn cleared_event_removes_item_from_queue_without_decision() {
        let (sup, node, grid, store, base) = choice_grid_setup("e2e-cleared");
        let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
        grids.insert(node, Arc::clone(&grid));
        let mut lifecycle = lina_core::lifecycle::LifecycleEngine::new();
        let mut watch = AppPermissionWatch::new();
        // Hub sobre o MESMO store (re-projeta o log a cada sync).
        let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let wiring = ApprovalWiring::new(
            Arc::new(Supervisor::new()),
            Arc::new(Mutex::new(BTreeMap::new())),
            ApprovalKeys::default(),
        );
        let hub = AttentionHub::new(
            Arc::clone(&store),
            desk,
            model,
            wiring,
            test_nodes(Arc::clone(&store)),
        );
        let t0 = 3_000_000_u64;
        watch.note_output(node, t0);

        // Detecta o prompt de escolha → o hub vê 1 item.
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 1_600);
        assert_eq!(out.asks.len(), 1);
        let items = hub.sync(t0 + 1_700);
        assert_eq!(items.len(), 1, "pergunta na fila");
        assert_eq!(
            items[0].prompt_kind,
            lina_core::attention::PromptKind::Choice
        );

        // Humano responde NO TERMINAL (caixa some do grid) → a PRÓPRIA varredura apenda
        // PermissionPromptCleared (forma do ScanOutcome — sem etapa extra no caller).
        lock(&grid).advance(b"\x1b[2J\x1b[H$ rodando o que escolhi...\r\n");
        watch.note_output(node, t0 + 2_000);
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 4_000);
        assert_eq!(out.cleared.len(), 1, "1 prompt respondido no terminal");
        // O item SOME da fila — sem Resolved nem Dismissed no log.
        let items = hub.sync(t0 + 4_100);
        assert!(
            items.is_empty(),
            "prompt respondido no terminal sai da fila"
        );
        let recs = lock(&store).events().expect("events");
        assert!(
            recs.iter().any(|r| r.kind == "PermissionPromptCleared"),
            "o fato 'respondido no terminal' está no log"
        );
        assert!(
            !recs
                .iter()
                .any(|r| r.kind == "PermissionResolved" || r.kind == "PermissionDismissed"),
            "NENHUMA decisão fabricada (cleared ≠ resolve/dismiss)"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Mute por nó (projeção injetada — a mesma do hub) DESLIGA a varredura do nó; e o
    /// prompt RESPONDIDO no terminal (grid mudou) volta em `out.cleared` com o stable_id
    /// original (apendado como `PermissionPromptCleared` pela própria varredura).
    #[test]
    fn app_watch_respects_mute_and_reports_cleared() {
        let (sup, node, grid, store, base) = choice_grid_setup("mute");
        let mut grids: BTreeMap<NodeId, Grid> = BTreeMap::new();
        grids.insert(node, Arc::clone(&grid));
        let mut lifecycle = lina_core::lifecycle::LifecycleEngine::new();
        let mut watch = AppPermissionWatch::new();
        let t0 = 2_000_000_u64;
        watch.note_output(node, t0);

        // Nó mutado: nem roda o detector (defesa em profundidade com a fila).
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| true, t0 + 1_600);
        assert!(out.is_empty(), "mutado nunca emite");

        // Desmutado: emite; depois o humano responde NO TERMINAL (grid muda) → cleared.
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 1_700);
        assert_eq!(out.asks.len(), 1);
        let sid = out.asks[0].stable_id.clone();
        // Resposta no terminal: a caixa sai da tela.
        lock(&grid).advance(b"\x1b[2J\x1b[H$ comando rodando...\r\n");
        watch.note_output(node, t0 + 2_000);
        let out = watch.scan(&grids, &sup, &store, &mut lifecycle, &|_| false, t0 + 4_000);
        assert!(
            out.cleared.iter().any(|c| c.stable_id == sid),
            "prompt respondido no terminal volta em out.cleared: {:?}",
            out.cleared
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **ARBITRAGEM do Maestro (ux-flows vence): Esc NUNCA decide.** O caminho do Esc
    /// no `handle_key` é exatamente `ToastMachine::snooze` — aqui provamos, na camada
    /// testável, que esse caminho recolhe o toast SEM emitir NENHUM evento de decisão
    /// (`PermissionResolved`/`PermissionDismissed`) e o item segue pendente (fila/
    /// badge). NÃO-VACUOSO por controle positivo: o mesmo contador captura o evento
    /// quando uma decisão EXPLÍCITA (clique → `resolve`) acontece em seguida.
    #[test]
    fn esc_snooze_never_emits_decision_event() {
        let (hub, _desk, base) = attention_hub("esc");
        hub.store_append_for_test(&ask_event("s1", "Terminal B", "git push origin/master"));
        let items = hub.sync(1_000);
        let mut machine = crate::attention_ui::ToastMachine::default();
        machine.tick(&items, 1_000);
        assert!(
            matches!(
                machine.visible(&items, 1_000),
                Some(crate::attention_ui::ToastView::Single(0))
            ),
            "toast single de permissão no ar"
        );

        machine.snooze(&items); // ← o que o Esc faz agora (e NADA mais)
        machine.tick(&items, 1_100);
        assert_eq!(machine.visible(&items, 1_100), None, "Esc recolheu o toast");
        let items_after = hub.sync(1_200);
        assert_eq!(items_after.len(), 1, "o item SEGUE pendente (fila/badge)");
        let decided = |hub: &AttentionHub| {
            hub.events_for_test()
                .into_iter()
                .filter(|r| r.kind == "PermissionResolved" || r.kind == "PermissionDismissed")
                .count()
        };
        assert_eq!(decided(&hub), 0, "Esc não emitiu evento de decisão");

        // Controle positivo (não-vacuidade): a decisão EXPLÍCITA emite — e o mesmo
        // contador a captura.
        assert!(hub.resolve(
            "s1",
            lina_core::ApprovalDecision::Deny,
            lina_core::ResolutionVia::Human
        ));
        assert_eq!(decided(&hub), 1, "o contador captura decisões reais");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Payload parcial/estranho: campos ausentes → `""` (espírito `serde(default)`);
    /// kinds alheios são ignorados — replay de log antigo nunca quebra o modal.
    #[test]
    fn role_templates_tolerate_partial_payload_and_skip_other_kinds() {
        let rec = |seq: u64, kind: &str, payload: serde_json::Value| EventRecord {
            seq,
            ts: seq,
            kind: kind.into(),
            version: 1,
            payload,
        };
        let records = vec![
            rec(
                1,
                "RoleTemplateSaved",
                serde_json::json!({"name":"Só Nome"}),
            ),
            rec(2, "NodeAdded", serde_json::json!({"node":"x"})),
            rec(3, "RoleTemplateSaved", serde_json::json!({})),
        ];
        let got = role_templates_from_records(&records);
        assert_eq!(got.len(), 2, "só os RoleTemplateSaved");
        assert_eq!(got[0].name, "Só Nome");
        assert_eq!(got[0].description, "", "ausente → default vazio");
        assert_eq!(got[1].name, "", "payload vazio não quebra");
    }

    // ─────────────── Rodada 360 (ADR 0022) · admissão canônica de nó ───────────────

    /// Os KINDS dos eventos apendados no store DEPOIS de `mark` (na ordem do log).
    fn kinds_since(store: &Arc<Mutex<EventStore>>, mark: usize) -> Vec<String> {
        lock(store)
            .events()
            .expect("eventos")
            .into_iter()
            .skip(mark)
            .map(|r| r.kind)
            .collect()
    }

    /// **TESTE DE PARIDADE (ADR 0022 §2):** as TRÊS portas de admissão — ⌘T (`add_node`),
    /// ⌘N (`create_agent_with`) e seed/demo (`admit_node(seeded_terminal)`) — produzem a
    /// MESMA sequência canônica de eventos módulo parâmetros: `NodeAdded` +
    /// `TerminalSpawned{cwd REAL}` + `NodeRoleAssigned`; `CliProfileSet` entra QUANDO o
    /// profile é conhecido. Fim das três portarias com fichas diferentes. (Pós terminal-puro,
    /// a SEQUÊNCIA continua idêntica; o VALOR do cwd diverge: ⌘T puro = HOME `UserHome`,
    /// agentes = `Managed` `<ws>/n-<uuid>` — verificado no bloco de cwd ao final.)
    #[test]
    fn admissao_paridade_tres_portas_mesma_sequencia() {
        let ws = std::env::temp_dir().join(format!("lina-paridade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, store, _model) = test_manager("paridade", Some(bw));
        let canonical = [
            "NodeAdded",
            "TerminalSpawned",
            "NodeRenamed",
            "NodeRoleAssigned",
        ];

        // Porta 1 — ⌘T / botão +.
        let mark = lock(&store).events().expect("eventos").len();
        let t = nm.add_node().expect("⌘T");
        let seq_t = kinds_since(&store, mark);
        assert_eq!(seq_t, canonical, "⌘T emite a sequência canônica");

        // Porta 2 — ⌘N sem motor (M2 legado).
        let mark = lock(&store).events().expect("eventos").len();
        let n = nm
            .create_agent_with("Revisor", None, None, None, false)
            .expect("⌘N");
        assert_eq!(
            kinds_since(&store, mark),
            seq_t,
            "⌘N (sem profile) = MESMA sequência do ⌘T, módulo parâmetros"
        );

        // Porta 3 — seed/demo (posição fixa do roteiro).
        let mark = lock(&store).events().expect("eventos").len();
        let s = nm
            .admit_node(NodeAdmission::seeded_terminal("Terminal Demo", 30.0, 96.0))
            .expect("seed");
        assert_eq!(
            kinds_since(&store, mark),
            seq_t,
            "seed/demo = MESMA sequência, módulo parâmetros"
        );
        // Payloads do seed (não só os kinds): papel "terminal" e a posição FIXA do roteiro.
        let seed_events: Vec<_> = lock(&store)
            .events()
            .expect("eventos")
            .into_iter()
            .skip(mark)
            .collect();
        assert!(
            seed_events
                .iter()
                .any(|r| r.kind == "NodeRoleAssigned" && r.payload["role"] == "terminal"),
            "seed grava o papel 'terminal' (também é papel — ADR 0022 §2)"
        );
        assert!(
            seed_events
                .iter()
                .any(|r| r.kind == "NodeAdded" && r.payload["x"] == 30.0 && r.payload["y"] == 96.0),
            "seed preserva a posição fixa do roteiro"
        );

        // ⌘N com MOTOR de profile conhecido → a sequência ganha `CliProfileSet` no fim.
        let mark = lock(&store).events().expect("eventos").len();
        let engine = AgentEngine {
            program: "cat".to_string(),
            args: vec![],
            profile_id: Some("claude-code".to_string()),
            label: "Claude Code".to_string(),
        };
        let e = nm
            .create_agent_with("Motorizado", Some(&engine), None, None, false)
            .expect("⌘N com motor");
        assert_eq!(
            kinds_since(&store, mark),
            [
                "NodeAdded",
                "TerminalSpawned",
                "NodeRenamed",
                "NodeRoleAssigned",
                "CliProfileSet"
            ],
            "profile conhecido → CliProfileSet SEMPRE que houver"
        );
        // Payloads do motor: o RÓTULO no TerminalSpawned.cli e o id no CliProfileSet.
        let engine_events: Vec<_> = lock(&store)
            .events()
            .expect("eventos")
            .into_iter()
            .skip(mark)
            .collect();
        assert!(
            engine_events
                .iter()
                .any(|r| r.kind == "TerminalSpawned" && r.payload["cli"] == "Claude Code"),
            "motor escolhido → TerminalSpawned.cli leva o RÓTULO"
        );
        assert!(
            engine_events
                .iter()
                .any(|r| r.kind == "CliProfileSet" && r.payload["profile"] == "claude-code"),
            "CliProfileSet leva o id do profile"
        );

        // O binding node↔cwd (§3): TODA porta persistiu o cwd REAL do spawn na projeção.
        // Pós terminal-puro, há DUAS semânticas (a sequência de eventos é a MESMA — provada
        // acima — mas o cwd difere): o ⌘T puro nasce no HOME do usuário (`UserHome`, sem dir
        // gerenciado); as 3 portas de AGENTE (⌘N, seed, ⌘N+motor) seguem `Managed` num dir-casa
        // ÚNICO `<ws>/n-<uuid>` (nunca o slot reciclável `t{seq}`).
        let projected = lock(&store).project().expect("project");
        // ⌘T PURO → HOME do usuário, fora do ws_root.
        let t_cwd = projected
            .nodes
            .get(&t)
            .and_then(|nd| nd.cwd.clone())
            .expect("cwd REAL persistido p/ a porta ⌘T (HOME)");
        assert_eq!(
            t_cwd,
            user_home_dir().display().to_string(),
            "⌘T puro nasce no HOME do usuário (UserHome), não num dir gerenciado"
        );
        // As 3 portas de AGENTE → dir-casa gerenciado ÚNICO sob o ws_root.
        let mut managed: Vec<String> = Vec::new();
        for (id, porta) in [(n, "⌘N"), (s, "seed"), (e, "⌘N+motor")] {
            let cwd = projected
                .nodes
                .get(&id)
                .and_then(|nd| nd.cwd.clone())
                .unwrap_or_else(|| panic!("cwd REAL persistido p/ a porta {porta}"));
            let p = std::path::Path::new(&cwd);
            assert_eq!(
                p.parent(),
                Some(ws.as_path()),
                "o cwd gerenciado fica sob o ws_root ({porta})"
            );
            assert!(
                p.file_name()
                    .is_some_and(|f| f.to_string_lossy().starts_with("n-")),
                "dir-casa gerenciado é único por construção (n-<uuid>), não slot reciclável ({porta})"
            );
            managed.push(cwd);
        }
        managed.sort();
        managed.dedup();
        assert_eq!(
            managed.len(),
            3,
            "as 3 portas de AGENTE nascem em cwds gerenciados DISTINTOS (slot não-reciclável)"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// **COMPENSAÇÃO (ADR 0022 §5):** o NodeId nasce no `register` (antes do append), então
    /// uma FALHA de persistência compensa com `retire_pty` (unregister + kill) — o nó NUNCA
    /// fica no roster vivo (Supervisor → agents.json) nem no model sem existir no log.
    /// Falha forçada: o espelho `log.jsonl` vira DIRETÓRIO → todo append erra em I/O.
    /// (A atomicidade SQLite↔JSONL dentro do store é assunto do core, não deste funil.)
    #[test]
    fn falha_de_append_compensa_e_nada_fica_no_roster() {
        let dir = std::env::temp_dir().join(format!("lina-compensa-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(EventStore::open(&dir).expect("open store")));
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        let cmd_factory: CmdFactory = Arc::new(|_n: &str| PtyCommand::new("cat"));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let nm = NodeManager::new(
            Arc::clone(&pty),
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&model),
            Arc::clone(&grids),
            BTreeMap::new(),
            delta_tx,
            80,
            24,
            0,
            cmd_factory,
            None,
            dir.join(".lina"),
        );

        // Sabotagem determinística do espelho: `log.jsonl` vira diretório → append falha.
        let jsonl = dir.join("log.jsonl");
        let _ = std::fs::remove_file(&jsonl);
        std::fs::create_dir_all(&jsonl).expect("log.jsonl como dir");

        let before_roster = sup.list().len();
        let result = nm.add_node();

        assert!(result.is_err(), "append falhou → admissão devolve Err");
        assert_eq!(
            sup.list().len(),
            before_roster,
            "COMPENSAÇÃO: o nó foi DESREGISTRADO do roster vivo (nada no agents.json)"
        );
        assert!(lock(&model).order.is_empty(), "nada projetado no model");
        assert!(lock(&grids).is_empty(), "nenhum grid registrado");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **UserDir SEM consentimento (ADR 0022 §4):** o agente nasce na pasta do usuário SEM
    /// nenhum arquivo escrito nela (a Lina não toca a pasta sem o toque no modal), com a
    /// degradação VISÍVEL (`kit_missing` → badge no card) — e SEM o dir-fantasma
    /// `<ws_root>/t{seq}` órfão (nem na admissão, nem no rewrite de roster).
    #[test]
    fn userdir_sem_consentimento_degrada_visivel_sem_dir_fantasma() {
        let ws = std::env::temp_dir().join(format!("lina-noconsent-{}", std::process::id()));
        let user = std::env::temp_dir().join(format!("lina-userdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&user);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, store, model) = test_manager("noconsent", Some(bw));

        let id = nm
            .create_agent_with("Free Lancer", None, Some(&user), None, false)
            .expect("⌘N pasta própria sem consentimento");

        assert!(
            !user.join("CLAUDE.md").exists() && !user.join(".claude").exists(),
            "SEM consentimento a Lina não escreve NADA na pasta do usuário"
        );
        assert!(
            lock(&model).nodes.get(&id).is_some_and(|v| v.kit_missing),
            "degradação VISÍVEL: kit_missing liga o badge no card"
        );
        // Fim do dir-fantasma: a admissão de um UserDir não-consentido NÃO cunha dir-casa
        // gerenciado algum. Pós-fix, o slot gerenciado é `<ws>/n-<uuid>` (único, não mais o
        // reciclável `t{seq}`) — então a prova é "nenhum dir `n-*` órfão sob o ws" (só os
        // seeds Managed A/B existem ali).
        let has_managed_phantom = || {
            std::fs::read_dir(&ws)
                .into_iter()
                .flatten()
                .flatten()
                .any(|ent| ent.file_name().to_string_lossy().starts_with("n-"))
        };
        assert!(
            !has_managed_phantom(),
            "nenhum dir-casa gerenciado (n-<uuid>) órfão criado na admissão UserDir"
        );
        nm.rewrite_bootstrap();
        assert!(
            !has_managed_phantom(),
            "o rewrite de roster também NÃO cria o dir-fantasma gerenciado"
        );
        // O binding node↔cwd persiste a pasta REAL (correlação custo/sessão futura).
        let projected = lock(&store).project().expect("project");
        assert_eq!(
            projected.nodes.get(&id).and_then(|n| n.cwd.clone()),
            Some(user.display().to_string()),
            "TerminalSpawned.cwd = pasta REAL do usuário"
        );
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&user);
    }

    /// **ROOT CAUSE do bug do fundador (slot reciclável poluído):** o dir-casa de um agente
    /// `Managed` é ÚNICO e VIRGEM por construção — nunca um slot `t{seq}` reusado entre boots.
    /// O `seq` REINICIA em 0 a cada abertura do app, mas os dirs `t{seq}` PERSISTEM com o
    /// conteúdo da sessão anterior; o CLI usa o cwd como CHAVE de identidade/histórico de
    /// projeto, então um agente novo no slot reciclado CONTINUAVA o projeto errado. Prova:
    /// planta o projeto de uma sessão passada no slot que a key reciclável (`t0`, seq_start=0)
    /// reusaria; admite um agente novo no MESMO workspace; (a) o cwd persistido NÃO é o slot
    /// poluído e (b) o dir do agente novo é VIRGEM (sem o `projeto-antigo.md`).
    ///
    /// NÃO-VACUOSO: no código com `key = format!("t{seq}")` o cwd resolve para `<ws>/t0` →
    /// (a) falha (igual ao slot) e (b) falha (herda `projeto-antigo.md`). Controle positivo.
    #[test]
    fn agente_novo_nasce_em_cwd_virgem_nao_herda_projeto_de_sessao_passada() {
        let ws = std::env::temp_dir().join(format!("lina-cwdunico-virgem-{}", std::process::id()));
        let store_dir =
            std::env::temp_dir().join(format!("lina-cwdunico-virgem-st-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&store_dir);

        // SESSÃO PASSADA: o slot que a key reciclável `t{seq}` (seq_start=0 → 1º admitido = t0)
        // reusaria ENTRE boots — já POLUÍDO com o projeto de outra sessão (estado real do disco).
        let slot_reciclavel = ws.join("t0");
        std::fs::create_dir_all(&slot_reciclavel).expect("slot da sessão passada");
        std::fs::write(
            slot_reciclavel.join("projeto-antigo.md"),
            "jogo-da-velha da sessão anterior",
        )
        .expect("plantar projeto antigo");

        // SESSÃO ATUAL: novo boot (seq REINICIA em 0), MESMO ws_root.
        // Pós terminal-puro: o ⌘T (`add_node`) é PURO (nasce no HOME, sem dir gerenciado). O
        // root cause do slot reciclável só atinge AGENTES gerenciados (⌘N sem pasta própria =
        // `CwdPolicy::Managed`) — é essa porta que herdaria o `t{seq}` poluído. Exercitamos ela.
        let (nm, store) = managed_manager(&ws, &store_dir, 0);
        let id = nm
            .create_agent_with("Agente Novo", None, None, None, false)
            .expect("⌘N admite um AGENTE gerenciado");

        let cwd = {
            let projected = lock(&store).project().expect("project");
            projected
                .nodes
                .get(&id)
                .and_then(|n| n.cwd.clone())
                .expect("cwd gerenciado persistido no TerminalSpawned")
        };

        // (a) IDENTIDADE FRESCA: o cwd gerenciado NÃO é o slot reciclável poluído.
        assert_ne!(
            cwd,
            slot_reciclavel.display().to_string(),
            "o agente novo NÃO nasce no slot t0 reciclado da sessão passada"
        );
        // (b) VIRGEM: o dir REAL do agente novo não contém o projeto da sessão anterior.
        assert!(
            !std::path::Path::new(&cwd)
                .join("projeto-antigo.md")
                .exists(),
            "o cwd do agente novo é VIRGEM — sem o projeto-antigo.md da sessão passada"
        );

        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&store_dir);
    }

    /// **Reforço do root cause (colisão entre boots):** DUAS sessões = dois boots do app sobre
    /// o MESMO workspace, AMBOS com `seq` reiniciado em 0 (`main.rs` `seq_start=0`). O 1º
    /// agente gerenciado de cada boot NÃO pode resolver para o MESMO dir — senão o segundo
    /// herda o histórico/projeto do primeiro pelo cwd reusado.
    ///
    /// NÃO-VACUOSO: com `key = format!("t{seq}")` ambos os cwds são `<ws>/t0` → `assert_ne`
    /// falha (colisão). Controle positivo: reverter o fix faz este teste falhar aqui.
    #[test]
    fn dois_boots_seq_zero_admitem_em_cwds_distintos() {
        let ws = std::env::temp_dir().join(format!("lina-cwdunico-2boots-{}", std::process::id()));
        let st1 =
            std::env::temp_dir().join(format!("lina-cwdunico-2boots-st1-{}", std::process::id()));
        let st2 =
            std::env::temp_dir().join(format!("lina-cwdunico-2boots-st2-{}", std::process::id()));
        for d in [&ws, &st1, &st2] {
            let _ = std::fs::remove_dir_all(d);
        }

        let read_cwd = |store: &Arc<Mutex<EventStore>>, id: NodeId| -> String {
            lock(store)
                .project()
                .expect("project")
                .nodes
                .get(&id)
                .and_then(|n| n.cwd.clone())
                .expect("cwd persistido")
        };

        // Pós terminal-puro: o ⌘T é PURO (HOME, dir não-gerenciado). O slot reciclável só
        // ameaça AGENTES gerenciados (⌘N sem pasta = `Managed`) — admitimos essa porta nos 2 boots.
        // Boot 1 (seq=0).
        let (nm1, store1) = managed_manager(&ws, &st1, 0);
        let id1 = nm1
            .create_agent_with("Agente", None, None, None, false)
            .expect("boot 1 admite agente gerenciado");
        let cwd1 = read_cwd(&store1, id1);

        // Boot 2 (seq REINICIA em 0), MESMO ws_root.
        let (nm2, store2) = managed_manager(&ws, &st2, 0);
        let id2 = nm2
            .create_agent_with("Agente", None, None, None, false)
            .expect("boot 2 admite agente gerenciado");
        let cwd2 = read_cwd(&store2, id2);

        assert_ne!(
            cwd1, cwd2,
            "dois boots (seq=0) admitem em cwds DISTINTOS — o slot gerenciado não é reciclável"
        );

        for d in [&ws, &st1, &st2] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// **UserDir COM consentimento + merge-safe (ADR 0022 §4):** (a) numa pasta com
    /// `CLAUDE.md` PRÓPRIO do usuário, o kit entra SEM tocá-lo (recusa com aviso → badge),
    /// mas settings/skill (aditivos) entram; (b) numa pasta limpa, o kit COMPLETO entra
    /// (doutrina marcada + settings com hook da Lina) e o nó nasce SEM badge; o rewrite
    /// de roster REESCREVE o nosso (marcado) sem virar recusa.
    #[test]
    fn userdir_consentido_kit_merge_safe_jamais_sobrescreve_usuario() {
        let ws = std::env::temp_dir().join(format!("lina-consent-{}", std::process::id()));
        let user_a = std::env::temp_dir().join(format!("lina-consent-a-{}", std::process::id()));
        let user_b = std::env::temp_dir().join(format!("lina-consent-b-{}", std::process::id()));
        for d in [&ws, &user_a, &user_b] {
            let _ = std::fs::remove_dir_all(d);
        }
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, _store, model) = test_manager("consent", Some(bw));

        // (a) CLAUDE.md PRÉ-EXISTENTE do usuário — jamais sobrescrito.
        std::fs::create_dir_all(&user_a).expect("user_a");
        // O arquivo do usuário até CITA a doutrina da Lina no corpo (red-team da rodada):
        // citação NÃO transfere posse — só o marcador de máquina na 1ª linha transferiria.
        let user_doctrine =
            "# Meu projeto\n\nRegras minhas.\n\n> citando: # Você está no Lina Space\n";
        std::fs::write(user_a.join("CLAUDE.md"), user_doctrine).expect("CLAUDE.md do usuário");
        let a = nm
            .create_agent_with("Convidado", None, Some(&user_a), None, true)
            .expect("⌘N consentido");
        assert_eq!(
            std::fs::read_to_string(user_a.join("CLAUDE.md")).expect("re-read"),
            user_doctrine,
            "o CLAUDE.md do usuário fica BYTE-IDÊNTICO (recusa, nunca overwrite)"
        );
        assert!(
            user_a.join(".claude").join("settings.json").exists(),
            "settings (aditivo, ausente antes) entra mesmo com a doutrina recusada"
        );
        assert!(
            lock(&model).nodes.get(&a).is_some_and(|v| v.kit_missing),
            "kit PARCIAL → badge avisa (nunca silencioso)"
        );

        // (b) pasta LIMPA → kit completo, sem badge; rewrite re-escreve o NOSSO arquivo.
        let b = nm
            .create_agent_with("Anfitrião", None, Some(&user_b), None, true)
            .expect("⌘N consentido, pasta limpa");
        let written = std::fs::read_to_string(user_b.join("CLAUDE.md")).expect("kit escrito");
        assert!(
            written.starts_with("<!-- lina-space:gerado"),
            "doutrina NOSSA nasce CARIMBADA com o marcador de máquina na 1ª linha"
        );
        assert!(
            written.contains("# Você está no Lina Space"),
            "o corpo é a doutrina canônica do template"
        );
        let settings =
            std::fs::read_to_string(user_b.join(".claude").join("settings.json")).expect("hooks");
        assert!(
            settings.contains("whoami --bootstrap"),
            "settings com o hook da Lina (observabilidade ligada)"
        );
        assert!(
            settings_is_ours(&settings),
            "settings NOSSO carrega a chave estruturada de posse _lina_managed (inv 1)"
        );
        assert!(
            lock(&model).nodes.get(&b).is_some_and(|v| !v.kit_missing),
            "kit completo → sem badge"
        );
        // Rewrite de roster: o NOSSO CLAUDE.md (marcado) é re-escrito (roster muda), não recusado.
        nm.rename_node(b, "Anfitrião Renomeado").expect("rename");
        let rewritten = std::fs::read_to_string(user_b.join("CLAUDE.md")).expect("re-read");
        assert!(
            rewritten.contains("Anfitrião Renomeado"),
            "merge-safe re-escreve o arquivo NOSSO a cada mudança de roster"
        );
        for d in [&ws, &user_a, &user_b] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// **Fix 1 (red-team 360, inv 4 — ATOMICIDADE):** a janela de `wire_terminal` entre o
    /// `sup.register` e o `Ok` (`clone_reader`/spawn-da-thread) é COMPENSADA — falha pós-register
    /// não deixa o nó no roster vivo (agents.json/`lina list`) nem o PTY meio-criado vazado.
    /// NÃO-VACUOSO: sabota a janela pós-register (via `arm_wire_fault`, não o append); se o
    /// `compensate_wire` fosse removido do caminho de erro, `sup.list()`/`pty` reteriam o fantasma.
    #[test]
    fn wire_terminal_failure_after_register_compensates_roster() {
        let dir = std::env::temp_dir().join(format!("lina-wirefault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(EventStore::open(&dir).expect("open store")));
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        let cmd_factory: CmdFactory = Arc::new(|_n: &str| PtyCommand::new("cat"));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let nm = NodeManager::new(
            Arc::clone(&pty),
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&model),
            Arc::clone(&grids),
            BTreeMap::new(),
            delta_tx,
            80,
            24,
            0,
            cmd_factory,
            None,
            dir.join(".lina"),
        );

        let before_roster = sup.list().len();
        let before_events = lock(&store).events().expect("eventos").len();

        // Arma a falha da janela pós-register (o append NÃO é tocado — store íntegro).
        arm_wire_fault();
        let result = nm.add_node();

        assert!(
            result.is_err(),
            "falha pós-register em wire_terminal → admissão devolve Err"
        );
        assert_eq!(
            sup.list().len(),
            before_roster,
            "COMPENSAÇÃO: nó DESREGISTRADO do roster vivo (nada no agents.json/lina list)"
        );
        assert!(
            lock(&pty).is_empty(),
            "COMPENSAÇÃO: PTY meio-criado foi morto+removido (sem vazamento de handle)"
        );
        assert!(lock(&model).order.is_empty(), "nada projetado no model");
        assert!(lock(&grids).is_empty(), "nenhum grid registrado");
        assert_eq!(
            lock(&store).events().expect("eventos").len(),
            before_events,
            "a falha foi ANTES do append: zero eventos no log (nem NodeAdded órfão)"
        );
        // O seam não vaza: a próxima admissão (sem armar) sobe normalmente.
        let ok = nm
            .add_node()
            .expect("admissão seguinte sobe normal (flag consumida)");
        assert!(sup.list().iter().any(|n| n.id == ok));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Fix 2 (red-team 360, inv 1 — PRESERVAÇÃO):** um `settings.json` do USUÁRIO que apenas
    /// CONTÉM a substring `whoami --bootstrap` (mas sem a chave estruturada `_lina_managed`) é
    /// preservado BYTE-IDÊNTICO. NÃO-VACUOSO: regrediria se a posse voltasse a ser `contains`
    /// do comando do hook (o anti-padrão que o inv 1 proíbe).
    #[test]
    fn settings_do_usuario_com_substring_do_hook_e_preservado() {
        let ws = std::env::temp_dir().join(format!("lina-setsub-ws-{}", std::process::id()));
        let user = std::env::temp_dir().join(format!("lina-setsub-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&user);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, _store, model) = test_manager("setsub", Some(bw));

        // settings do usuário: contém a substring marcadora antiga, mas é DELE (sem a chave).
        std::fs::create_dir_all(user.join(".claude")).expect("user .claude");
        let user_settings =
            "{\n  \"hooks\": {},\n  \"_nota\": \"rodo `lina whoami --bootstrap` no meu shell\"\n}\n";
        std::fs::write(user.join(".claude").join("settings.json"), user_settings)
            .expect("settings do usuário");

        let id = nm
            .create_agent_with("Convidado", None, Some(&user), None, true)
            .expect("⌘N consentido");

        assert_eq!(
            std::fs::read_to_string(user.join(".claude").join("settings.json")).expect("re-read"),
            user_settings,
            "settings do usuário com a substring do hook fica BYTE-IDÊNTICO (posse por chave, não substring)"
        );
        assert!(
            !settings_is_ours(user_settings),
            "sem a chave _lina_managed, o settings é do USUÁRIO"
        );
        assert!(
            lock(&model).nodes.get(&id).is_some_and(|v| v.kit_missing),
            "settings recusado → kit parcial → badge (nunca silencioso)"
        );
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&user);
    }

    /// **Fix 3 (red-team 360, inv 6 — DEGRADAÇÃO VISÍVEL):** admissão `Managed` num boot
    /// degradado (`self.bootstrap == None` → `ensure_cwd == None`) ainda sobe o nó, mas LIGA o
    /// badge (`kit_missing`). NÃO-VACUOSO: sem o `kit_missing = true` no ramo `None`, o nó
    /// nasceria sem kit E sem aviso — o fundador não veria o Espaço degradado.
    #[test]
    fn managed_sem_bootstrap_liga_badge_de_degradacao() {
        let dir = std::env::temp_dir().join(format!("lina-managed-nb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(EventStore::open(&dir).expect("open store")));
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        let cmd_factory: CmdFactory = Arc::new(|_n: &str| PtyCommand::new("cat"));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        // bootstrap = None ⇒ boot degradado (BootstrapWriter::new falhou no main, `.ok()`).
        let nm = NodeManager::new(
            Arc::clone(&pty),
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&model),
            Arc::clone(&grids),
            BTreeMap::new(),
            delta_tx,
            80,
            24,
            0,
            cmd_factory,
            None,
            dir.join(".lina"),
        );

        // Porta de AGENTE gerenciado (⌘N sem pasta própria → `CwdPolicy::Managed`). Pós
        // terminal-puro o ⌘T é PURO (sem kit, sem badge por design); a degradação VISÍVEL é
        // invariante dos AGENTES, então exercitamos a porta que pede `Managed`. A admissão SOBE
        // (degradação não impede o nó), mas com o badge.
        let id = nm
            .create_agent_with("Agente", None, None, None, false)
            .expect("Managed degradado ainda sobe o nó");
        assert!(
            lock(&model).nodes.get(&id).is_some_and(|v| v.kit_missing),
            "inv#6: Managed + bootstrap=None liga o badge (degradação VISÍVEL)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Terminal default = PURO, abre no HOME (pedido do fundador):** a porta ⌘T / `+ Terminal`
    /// (`default_terminal` → `add_node`) usa `CwdPolicy::UserHome` e o nó nasce com o cwd = HOME
    /// do usuário, SEM badge de degradação (não é agente meio-cidadão — é shell limpo por
    /// design). NÃO-VACUOSO: no código com `default_terminal` = `Managed`, a porta pediria um dir
    /// gerenciado (≠ HOME) e a 1ª asserção falha.
    #[test]
    fn terminal_default_e_puro_abre_no_home_sem_badge() {
        // (a) A INTENÇÃO da porta ⌘T: terminal PURO no HOME do usuário (UserHome), não Managed.
        assert_eq!(
            NodeAdmission::default_terminal().cwd,
            CwdPolicy::UserHome {
                path: user_home_dir()
            },
            "⌘T / + Terminal é PURO: cwd = HOME do usuário (UserHome), não dir gerenciado"
        );
        // (b) End-to-end: admitir pelo ⌘T persiste o cwd = HOME e NÃO liga o badge de degradação.
        let (nm, store, model) = test_manager("puro-home", None);
        let id = nm.add_node().expect("⌘T admite o terminal puro");
        let cwd = lock(&store)
            .project()
            .expect("project")
            .nodes
            .get(&id)
            .and_then(|n| n.cwd.clone())
            .expect("cwd persistido do terminal puro (TerminalSpawned.cwd = HOME)");
        assert_eq!(
            cwd,
            user_home_dir().display().to_string(),
            "o terminal puro nasce no HOME do usuário"
        );
        assert!(
            lock(&model).nodes.get(&id).is_some_and(|v| !v.kit_missing),
            "terminal puro NÃO é agente degradado: sem badge 'sem doutrina/observabilidade'"
        );
    }

    /// **Terminal PURO sem a linha de intro (pedido do fundador):** o `+ Terminal` deve ser
    /// como o Terminal.app — dá `exec` direto no shell de login do usuário, SEM imprimir a
    /// antiga linha "… — shell interativo (digite comandos; …)". O env útil (TERM + PATH com o
    /// `lina`) é PRESERVADO (invisível e necessário p/ A2A se o user rodar um agente ali).
    /// NÃO-VACUOSO: no código com o `printf` de intro, as duas primeiras asserções falham.
    #[cfg(not(windows))]
    #[test]
    fn shell_puro_da_exec_sem_intro_preservando_env() {
        let dbg = format!("{:?}", shell_cmd("Terminal X"));
        assert!(
            !dbg.contains("shell interativo"),
            "terminal puro não imprime a linha de intro (sem 'shell interativo'): {dbg}"
        );
        assert!(
            !dbg.contains("printf"),
            "sem o printf da intro — só o exec do shell: {dbg}"
        );
        assert!(
            dbg.contains("exec"),
            "dá exec no shell de login do usuário ($SHELL, fallback /bin/sh): {dbg}"
        );
        assert!(
            dbg.contains("TERM") && dbg.contains("xterm-256color"),
            "env TERM preservado (não regrediu junto com a intro): {dbg}"
        );
    }

    /// **ADR 0026 (BUG-1 dogfood r1) — TODO nó nasce com a identidade no env do PTY:** o
    /// funil único (`admit_node`) carimba `LINA_NODE_NAME` (nome do nó) e `LINA_NODE_ID`
    /// (a key durável `n-<uuidv7>`) via [`node_identity_env`] — junto do `VIBE_ROLE`
    /// histórico. É o que impede a colisão de identidade quando N terminais compartilham o
    /// MESMO cwd (a ficha `.lina/bootstrap.json` é último-escritor-vence; o env, não).
    /// NÃO-VACUOSO: remover a injeção de qualquer uma das 4 vars falha o assert dela.
    #[test]
    fn admitted_pty_env_carries_node_identity() {
        // Autonomia NÃO-default (`Manual`) prova que a escolha do plano FLUI ao env (FIX-3) —
        // distinta do `Assisted` default; `label()` casa com `lina_core::guard::parse_autonomy`.
        let cmd = node_identity_env(
            PtyCommand::new("cat"),
            "Bug Finder",
            "BUG_FIXER",
            "n-0197-deadbeef",
            Autonomy::Manual,
        );
        let dbg = format!("{cmd:?}");
        for (var, val) in [
            ("VIBE_ROLE", "BUG_FIXER"),
            ("LINA_NODE_NAME", "Bug Finder"),
            ("LINA_NODE_ID", "n-0197-deadbeef"),
            ("LINA_AUTONOMY", "manual"),
        ] {
            assert!(
                dbg.contains(var) && dbg.contains(val),
                "env de identidade ausente no comando do PTY: {var}={val} em {dbg}"
            );
        }
    }

    /// **FIX-1 (dogfooding 2026-06-10) — admissões no MESMO cwd de TRABALHO são DETECTADAS e
    /// sinalizadas (badge "vários agentes nesta pasta"):** o caso de uso central do produto
    /// (um time inteiro no projeto do usuário; default F1-4-1) põe N agentes no MESMO `UserDir`.
    /// A identidade A2A já é isolada pelo env (ADR 0026: `admitted_pty_env_carries_node_identity`
    /// + `identity_env_cli.rs`); ESTE teste prova a DEFESA ESTRUTURAL — o funil detecta que o
    ///   cwd já pertence a outro nó vivo e marca `cwd_shared` em TODOS os que dividem a pasta (o
    ///   card avisa; nunca surpresa silenciosa). Controle: nó em pasta DISTINTA não é marcado.
    ///   NÃO-VACUOSO: sem a detecção, `cwd_shared` fica `false` e o assert do 2º (+ o re-marque
    ///   do 1º) falha.
    #[test]
    fn admissoes_no_mesmo_user_dir_detectam_e_marcam_cwd_compartilhado() {
        let ws = std::env::temp_dir().join(format!("lina-cwdshared-ws-{}", std::process::id()));
        let shared =
            std::env::temp_dir().join(format!("lina-cwdshared-proj-{}", std::process::id()));
        let other =
            std::env::temp_dir().join(format!("lina-cwdshared-other-{}", std::process::id()));
        for d in [&ws, &shared, &other] {
            let _ = std::fs::remove_dir_all(d);
        }
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, _store, model) = test_manager("cwdshared", Some(bw));

        // 1º agente na pasta compartilhada (consentido) — SOZINHO: ainda não compartilha.
        let a = nm
            .create_agent_with("Arquiteto", None, Some(&shared), None, true)
            .expect("admite o 1º na pasta");
        assert!(
            !lock(&model).nodes.get(&a).expect("nó a").cwd_shared,
            "1º sozinho na pasta: ainda NÃO compartilha"
        );

        // 2º agente na MESMA pasta → colisão DETECTADA: AMBOS marcados.
        let b = nm
            .create_agent_with("Backend", None, Some(&shared), None, true)
            .expect("admite o 2º na MESMA pasta");
        {
            let m = lock(&model);
            assert!(
                m.nodes.get(&b).expect("nó b").cwd_shared,
                "2º no mesmo cwd: marcado compartilhado"
            );
            assert!(
                m.nodes.get(&a).expect("nó a").cwd_shared,
                "o 1º é RE-marcado ao ganhar companhia na pasta (badge honesto)"
            );
        }

        // Controle: 3º agente em pasta DISTINTA → NÃO marcado.
        let c = nm
            .create_agent_with("QA", None, Some(&other), None, true)
            .expect("admite o 3º em pasta distinta");
        assert!(
            !lock(&model).nodes.get(&c).expect("nó c").cwd_shared,
            "pasta própria distinta: não compartilha"
        );

        for d in [&ws, &shared, &other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// **FIX-1 — duas admissões no MESMO cwd nascem com IDENTIDADE DISTINTA (a base da
    /// isolação A2A via env, ADR 0026):** a `key` durável (`n-<uuid>` → `LINA_NODE_ID`) difere
    /// por nó MESMO compartilhando a pasta — é o que o `node_identity_env` carimba
    /// (`admitted_pty_env_carries_node_identity`) e o que faz o outbox por-nó do `lina ask` não
    /// colidir (`identity_env_cli::ask_outbox_lands_in_env_named_subdir`, lina-bootstrap). O
    /// assert do MESMO cwd prova que a isolação vem da IDENTIDADE, não de separar a pasta (raiz
    /// comum preservada — o caso de uso "time no projeto do usuário"). NÃO-VACUOSO: keys
    /// recicláveis (o bug do slot `t{seq}`, já corrigido) fariam as keys coincidirem.
    #[test]
    fn duas_admissoes_no_mesmo_cwd_tem_identidades_distintas() {
        let ws = std::env::temp_dir().join(format!("lina-ident2-ws-{}", std::process::id()));
        let shared = std::env::temp_dir().join(format!("lina-ident2-proj-{}", std::process::id()));
        for d in [&ws, &shared] {
            let _ = std::fs::remove_dir_all(d);
        }
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, _store, _model) = test_manager("ident-mesmo-cwd", Some(bw));

        let a = nm
            .create_agent_with("Arquiteto", None, Some(&shared), None, true)
            .expect("1º no cwd compartilhado");
        let b = nm
            .create_agent_with("Backend", None, Some(&shared), None, true)
            .expect("2º no MESMO cwd");

        // `LINA_NODE_ID` = a key durável; distinta por construção (`n-<uuid v7>`, nunca reciclada).
        {
            let keys = lock(&nm.keys);
            let ka = keys.get(&a).expect("key a");
            let kb = keys.get(&b).expect("key b");
            assert_ne!(
                ka, kb,
                "LINA_NODE_ID (key) distinto por nó, mesmo no MESMO cwd"
            );
            assert!(
                ka.starts_with("n-") && kb.starts_with("n-"),
                "as keys são `n-<uuid>` (virgens, não-recicláveis): {ka} / {kb}"
            );
        }
        // E os dois apontam para o MESMO cwd de trabalho: a isolação é por IDENTIDADE, não por pasta.
        {
            let cwds = lock(&nm.cwds);
            assert_eq!(
                shared_cwd_path(cwds.get(&a).expect("cwd a")),
                shared_cwd_path(cwds.get(&b).expect("cwd b")),
                "raiz comum: ambos no MESMO cwd (isolação por identidade, não por subpasta)"
            );
        }

        for d in [&ws, &shared] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// **FIX-1 — o estado COLIDIDO de produção (N nós no mesmo cwd, dogfooding 2026-06-10)
    /// reabre sem panic:** o event log já tem `TerminalSpawned.cwd` idêntico em vários nós; o
    /// `project()` (replay = fonte da verdade, inv#4) reconstrói TODOS sem panic, com o mesmo
    /// cwd fiel. `cwd_shared` é estado de UI por sessão (nasce `false` no replay — documentado);
    /// o ponto é a ROBUSTEZ: reabrir o app no estado atual nunca quebra.
    #[test]
    fn replay_de_estado_com_nos_no_mesmo_cwd_reconstroi_sem_panic() {
        let ws = std::env::temp_dir().join(format!("lina-replay-ws-{}", std::process::id()));
        let shared = std::env::temp_dir().join(format!("lina-replay-proj-{}", std::process::id()));
        for d in [&ws, &shared] {
            let _ = std::fs::remove_dir_all(d);
        }
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, store, _model) = test_manager("replay-colidido", Some(bw));

        // Estado colidido: 2 nós no MESMO cwd (exatamente o que o log de produção tem hoje).
        nm.create_agent_with("Arquiteto", None, Some(&shared), None, true)
            .expect("1º");
        nm.create_agent_with("Backend", None, Some(&shared), None, true)
            .expect("2º");

        // Replay do log → reconstrói SEM panic; os 2 nós voltam, ambos com o MESMO cwd.
        let projected = lock(&store)
            .project()
            .expect("replay do estado colidido NÃO paniqueia");
        assert_eq!(
            projected.nodes.len(),
            2,
            "os 2 nós do estado colidido reconstroem pelo event log"
        );
        let cwds: std::collections::BTreeSet<&str> = projected
            .nodes
            .values()
            .filter_map(|n| n.cwd.as_deref())
            .collect();
        assert_eq!(
            cwds.len(),
            1,
            "ambos reconstroem com o MESMO cwd (estado colidido fiel ao TerminalSpawned.cwd)"
        );

        for d in [&ws, &shared] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// **FIX-3 (costura per-Agente) — a autonomia ESCOLHIDA no ⌘N chega ao nó (badge + guard):**
    /// `create_agent_with_autonomy(Manual)` projeta `NodeView.autonomy == Manual` (a FONTE do
    /// badge do card, per-Agente) e o funil carimba `LINA_AUTONOMY` no env (o guard honra —
    /// provado em `admitted_pty_env_carries_node_identity`). Controle: o wrapper
    /// `create_agent_with` mantém o default do produto (`Assisted`). NÃO-VACUOSO: sem a
    /// propagação `view.autonomy = plan.autonomy`, ambos ficariam no default do `new`.
    #[test]
    fn create_agent_propaga_autonomia_escolhida_pro_no() {
        let (nm, _store, model) = test_manager("autonomy-prop", None);

        let manual = nm
            .create_agent_with_autonomy("Cauteloso", None, None, None, false, Autonomy::Manual)
            .expect("⌘N com autonomia explícita");
        assert_eq!(
            lock(&model).nodes.get(&manual).expect("nó manual").autonomy,
            Autonomy::Manual,
            "a autonomia escolhida no modal projeta no NodeView (badge per-Agente)"
        );

        // Controle: o wrapper sem autonomia explícita mantém o default do produto (Assisted).
        let default = nm
            .create_agent_with("Padrao", None, None, None, false)
            .expect("⌘N default");
        assert_eq!(
            lock(&model)
                .nodes
                .get(&default)
                .expect("nó default")
                .autonomy,
            Autonomy::Assisted,
            "default do produto intocado (Assisted)"
        );
    }

    /// **#6 dogfooding r2 — o sigil `@` é ENDEREÇAMENTO, não nome:** o `lina spawn` chega com
    /// o alvo "@Nome" e o nó nascia com o `@` embutido (seq 20326: "@Especialista em
    /// Licenças"; superfícies exibiam "@@"). O funil normaliza: spawn com "@X" e com "X"
    /// produz nó SEMPRE "X" — e o casamento tolerante do roster (`node_by_name`) segue
    /// resolvendo o endereço "@X" para ele. NÃO-VACUOSO: sem o strip no funil, o 1º assert
    /// recebe "@Especialista em Licenças" e falha.
    #[test]
    fn spawn_strips_addressing_sigil_from_node_name() {
        let (nm, _store, model) = test_manager("sigil6", None);
        let spawner = nm
            .admit_node(NodeAdmission::default_terminal())
            .expect("spawner");

        let with_sigil = nm
            .execute_spawn("msg_sig1", "@Especialista em Licenças", "terminal", spawner)
            .expect("spawn com @ no alvo");
        let without = nm
            .execute_spawn("msg_sig2", "Especialista em Dados", "terminal", spawner)
            .expect("spawn sem @");
        {
            let m = lock(&model);
            assert_eq!(
                m.nodes.get(&with_sigil).expect("nó @").name,
                "Especialista em Licenças",
                "o @ de endereçamento NÃO entra no nome do nó"
            );
            assert_eq!(
                m.nodes.get(&without).expect("nó").name,
                "Especialista em Dados",
                "nome sem sigil passa intacto (controle)"
            );
        }
        // "Endereçar @X depois funciona": o roster casa o endereço com o nome limpo (a
        // tolerância de `node_by_name` — sem o "@@" que o nome-com-sigil produzia).
        let sup = Supervisor::new();
        let id = sup.register("Especialista em Licenças", None, Box::new(std::io::sink()));
        assert_eq!(
            sup.node_by_name("@Especialista em Licenças"),
            Some(id),
            "o endereço @X resolve o nó X"
        );
        // E "@"/"@@" puros não viram nó sem nome: recusa visível do funil.
        assert!(
            nm.execute_spawn("msg_sig3", "@", "terminal", spawner)
                .is_err(),
            "@ puro é endereço vazio — nome inválido"
        );
    }

    /// **#7 dogfooding r2 — `TerminalSpawned.cli` do spawn registra o CLI, não o nome do nó
    /// (paridade com o ⌘N):** com o MESMO motor, a porta spawn e a porta ⌘N gravam o MESMO
    /// `cli` ("Claude Code"). Antes, o spawn nascia com engine `None` e o campo caía no
    /// fallback legado = NOME do nó (seq 20326 vs 20134) — poluindo as projeções por-CLI
    /// (detecção/perfis/custo). NÃO-VACUOSO: voltar o engine do spawn a `None` faz o `cli`
    /// gravar o nome de novo e o assert anti-nome falha.
    #[test]
    fn spawn_terminal_spawned_cli_matches_modal_path() {
        let (mut nm, store, _model) = test_manager("cli7", None);
        let engine = AgentEngine {
            program: "cat".into(), // determinístico no PTY de teste; o label é o que se grava
            args: Vec::new(),
            profile_id: None,
            label: "Claude Code".into(),
        };
        let fixed = engine.clone();
        nm.set_spawn_engine_factory(Arc::new(move || Some(fixed.clone())));
        let spawner = nm
            .admit_node(NodeAdmission::default_terminal())
            .expect("spawner");

        nm.execute_spawn("msg_cli7", "Especialista em Licenças", "terminal", spawner)
            .expect("spawn agente-pede");
        nm.create_agent_with("Colega Modal", Some(&engine), None, None, true)
            .expect("porta ⌘N");

        let clis: Vec<String> = lock(&store)
            .events()
            .expect("events")
            .iter()
            .filter(|r| r.kind == "TerminalSpawned")
            .filter_map(|r| Some(r.payload.get("cli")?.as_str()?.to_string()))
            .collect();
        assert_eq!(
            clis.iter().filter(|c| *c == "Claude Code").count(),
            2,
            "spawn e ⌘N gravam o MESMO cli (paridade): {clis:?}"
        );
        assert!(
            !clis
                .iter()
                .any(|c| c.contains("Especialista") || c.contains("Colega")),
            "NENHUM TerminalSpawned grava nome de nó no campo cli: {clis:?}"
        );
    }

    /// **#7 — o motor default do spawn é o que o modal pré-seleciona:** `claude` primeiro,
    /// senão o 1º descoberto (com o `program` = binário REAL do PATH); máquina sem CLI →
    /// `None` (degrada à fábrica do workspace). Função PURA — sem tocar o PATH do runner.
    #[test]
    fn default_spawn_engine_prefers_claude_then_first_then_none() {
        let codex = DiscoveredCli {
            id: "codex".into(),
            version: None,
            path: "/u/bin/codex".into(),
        };
        let claude = DiscoveredCli {
            id: "claude".into(),
            version: Some("2.1".into()),
            path: "/u/bin/claude".into(),
        };
        let e = default_spawn_engine(&[codex.clone(), claude]).expect("acha claude");
        assert_eq!(e.label, "Claude Code");
        assert_eq!(
            e.program, "/u/bin/claude",
            "o program é o binário DESCOBERTO (não um chute)"
        );
        let e = default_spawn_engine(&[codex]).expect("sem claude → 1º descoberto");
        assert_eq!(e.label, "Codex");
        assert!(
            default_spawn_engine(&[]).is_none(),
            "máquina sem CLI → None (fábrica do workspace, comportamento histórico)"
        );
    }

    /// **#7 — trava de espelho:** `spawn_cli_label` (funil) ≡ `agent_modal::engine_label`
    /// (modal) para TODOS os CLIs conhecidos + o fallback Title-case. São duas cópias de
    /// propósito (o modal importa DESTE módulo; o import inverso criaria ciclo) — divergir
    /// é teste vermelho, não bug silencioso de rótulo.
    #[test]
    fn spawn_cli_label_matches_modal_engine_label() {
        for id in lina_core::KNOWN_CLIS.iter().copied().chain(["foo-cli"]) {
            assert_eq!(
                spawn_cli_label(id),
                crate::agent_modal::engine_label(id),
                "rótulo divergente entre funil e modal p/ {id:?}"
            );
        }
    }

    /// **#9 dogfooding r2 — a doutrina do nó gerenciado CITA o cwd ABSOLUTO (idempotente):**
    /// o spawnado nasce numa pasta NOVA que não conhece — um caminho relativo do orquestrador
    /// quebrava silencioso (correção manual na sessão). O kit (`write_one`, o gerador que o
    /// `admit_node` usa) anota a localização nos 3 arquivos de doutrina; o re-render por
    /// mudança de roster NÃO duplica a nota. NÃO-VACUOSO: sem o append, o 1º assert falha.
    #[test]
    fn write_one_doctrine_cites_absolute_workdir_idempotently() {
        let ws = std::env::temp_dir().join(format!("lina-wd9-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let roster = vec!["Spawnado".to_string(), "Maestro".to_string()];
        bw.write_one("n-teste9", "Spawnado", &roster);

        let dir = bw.dir_for("n-teste9");
        for f in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
            let body = std::fs::read_to_string(dir.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert!(
                body.contains(WORKDIR_NOTE_MARK),
                "{f} carrega a nota de localização"
            );
            assert!(
                body.contains(&dir.display().to_string()),
                "{f} cita o caminho ABSOLUTO da pasta do nó"
            );
        }
        // Re-render (mudança de roster reescreve a doutrina) → nota presente e ÚNICA.
        bw.write_one("n-teste9", "Spawnado", &roster);
        let body = std::fs::read_to_string(dir.join("CLAUDE.md")).expect("re-render");
        assert_eq!(
            body.matches(WORKDIR_NOTE_MARK).count(),
            1,
            "a nota não duplica no re-render (idempotente)"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// **Terminal PURO não escreve NADA no HOME do usuário (invariante crítico):** mesmo com o
    /// bootstrap LIGADO (que escreve o kit para os nós `Managed`), admitir um terminal puro
    /// (`UserHome`) deixa o HOME intocado — nenhum `CLAUDE.md`/`.claude`/`.lina`/`settings.json`,
    /// nem na admissão nem no rewrite de roster. O cwd persiste = HOME (binding §3) e o nó NÃO
    /// liga o badge. Usa um HOME TEMPORÁRIO (jamais toca o `~` real do teste-runner). NÃO-VACUOSO:
    /// se o ramo `UserHome` escrevesse kit, o dir não ficaria vazio.
    #[test]
    fn terminal_puro_nao_escreve_nada_no_home() {
        let ws = std::env::temp_dir().join(format!("lina-puro-ws-{}", std::process::id()));
        let home = std::env::temp_dir().join(format!("lina-puro-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("HOME temporário (já existe na vida real)");
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, store, model) = test_manager("puro-nowrite", Some(bw));

        // Mesmo plano do ⌘T (`default_terminal`), com o HOME apontado p/ um dir temporário VAZIO.
        let id = nm
            .admit_node(NodeAdmission {
                name: None,
                role: "terminal".into(),
                engine: None,
                cwd: CwdPolicy::UserHome { path: home.clone() },
                position: None,
                requested_by: None,
                autonomy: Autonomy::Assisted,
            })
            .expect("admite o terminal puro");

        // Mudança de roster também NÃO pode tocar o home (o no-op do rewrite p/ UserHome).
        nm.rewrite_bootstrap();

        // ZERO arquivos do Lina no home: o dir continua VAZIO após admissão + rewrite.
        assert!(!home.join("CLAUDE.md").exists(), "nenhum CLAUDE.md no home");
        assert!(!home.join(".claude").exists(), "nenhum .claude no home");
        assert!(!home.join(".lina").exists(), "nenhum .lina no home");
        assert_eq!(
            std::fs::read_dir(&home).expect("ler home").count(),
            0,
            "o HOME do usuário fica intocado (zero arquivos do Lina escritos)"
        );

        // O cwd REAL persiste = HOME (binding §3) e o nó é shell limpo (sem badge).
        let cwd = lock(&store)
            .project()
            .expect("project")
            .nodes
            .get(&id)
            .and_then(|n| n.cwd.clone())
            .expect("cwd persistido");
        assert_eq!(
            cwd,
            home.display().to_string(),
            "TerminalSpawned.cwd = HOME do usuário"
        );
        assert!(
            lock(&model).nodes.get(&id).is_some_and(|v| !v.kit_missing),
            "terminal puro NÃO liga o badge 'sem doutrina/observabilidade'"
        );

        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **Regressão (acceptance do fundador): AGENTE segue `Managed` + kit + dir `n-<uuid>`.**
    /// Criar um "Novo Agente" (motor escolhido no M6, engine `Some`) NÃO é afetado pelo fix do
    /// terminal-puro: nasce num dir-casa gerenciado `<ws>/n-<uuid>` COM o kit (`CLAUDE.md`) — o
    /// isolamento de contexto do ADR 0022 fica intacto onde importa (agentes). NÃO-VACUOSO: se o
    /// agente virasse `UserHome`, o cwd seria o HOME (sem `n-` sob o ws) e sem `CLAUDE.md`.
    #[test]
    fn agente_com_motor_nasce_managed_com_kit() {
        let ws = std::env::temp_dir().join(format!("lina-agente-managed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");
        let (nm, store, model) = test_manager("agente-managed", Some(bw));

        let engine = AgentEngine {
            program: "cat".to_string(),
            args: vec![],
            profile_id: Some("claude-code".to_string()),
            label: "Claude Code".to_string(),
        };
        // "Novo Agente" com motor, sem pasta própria → `CwdPolicy::Managed` (dir gerenciado).
        let id = nm
            .create_agent_with("Dev", Some(&engine), None, None, false)
            .expect("Novo Agente (motor)");

        let cwd = lock(&store)
            .project()
            .expect("project")
            .nodes
            .get(&id)
            .and_then(|n| n.cwd.clone())
            .expect("cwd gerenciado do agente");
        let p = std::path::Path::new(&cwd);
        assert_eq!(
            p.parent(),
            Some(ws.as_path()),
            "agente nasce sob o ws_root (Managed), não no HOME"
        );
        assert!(
            p.file_name()
                .is_some_and(|f| f.to_string_lossy().starts_with("n-")),
            "dir-casa gerenciado único (n-<uuid>)"
        );
        assert!(
            p.join("CLAUDE.md").exists(),
            "agente recebe o kit (CLAUDE.md no dir gerenciado) — isolamento intacto"
        );
        assert!(
            lock(&model).nodes.get(&id).is_some_and(|v| !v.kit_missing),
            "kit completo → sem badge de degradação"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }

    // ───────────────────────── F1-4-3 · restore de terminais vivos ─────────────────────────

    /// Semeia no log a sequência canônica de um terminal (ADR 0022 §2) — helper dos testes de
    /// `plan_restore`. `profile` `Some` apenda `CliProfileSet` (o `cli` da projeção vira o id).
    #[allow(clippy::too_many_arguments)] // espelha os campos da sequência canônica de admissão
    fn seed_terminal(
        store: &mut EventStore,
        node: NodeId,
        x: f64,
        y: f64,
        name: &str,
        role: &str,
        cli_label: &str,
        profile: Option<&str>,
    ) {
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x,
                y,
                requested_by: None,
            })
            .expect("NodeAdded");
        store
            .append(&DomainEvent::TerminalSpawned {
                node,
                cli: cli_label.into(),
                cwd: Some("/tmp/proj".into()),
            })
            .expect("TerminalSpawned");
        store
            .append(&DomainEvent::NodeRenamed {
                node,
                name: name.into(),
            })
            .expect("NodeRenamed");
        store
            .append(&DomainEvent::NodeRoleAssigned {
                node,
                role: role.into(),
            })
            .expect("NodeRoleAssigned");
        if let Some(p) = profile {
            store
                .append(&DomainEvent::CliProfileSet {
                    node,
                    profile: p.into(),
                })
                .expect("CliProfileSet");
        }
    }

    fn claude_resume_profile() -> CliProfile {
        CliProfile::from_toml_str(
            r#"
                id = "claude-code"
                program = "claude"
                args = ["--output-format", "stream-json"]
                resume_args = ["--resume"]
                delivery = "session_resume"
                prompt_ready_regex = "> "
                session_dir_pattern = "~/.claude/projects/*/*.jsonl"
                [end_signal]
                kind = "idle"
            "#,
            "<claude>",
        )
        .expect("claude profile parseia")
    }

    /// **F1-4-3 critério 1+2+4:** quit limpo com 3 nós (motor com resume, shell puro, motor SEM
    /// resume) → o PLANO de restore, derivado SÓ do log + store, recupera posições/nomes/papéis,
    /// re-hidrata o scrollback BYTE-IDÊNTICO à janela viva, e dá o badge honesto. E é derivável do
    /// log: re-projetar do mesmo store produz o MESMO plano.
    #[test]
    fn plan_restore_recovers_layout_and_rehydrates_scrollback() {
        let base = std::env::temp_dir().join(format!("lina-restore-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let mut reg = ProfileRegistry::new();
        reg.insert(claude_resume_profile());
        reg.insert(
            CliProfile::from_toml_str(
                r#"
                    id = "gemini"
                    program = "gemini"
                    delivery = "pty_inject"
                    prompt_ready_regex = "> "
                    [end_signal]
                    kind = "idle"
                "#,
                "<gemini>",
            )
            .expect("gemini profile"),
        );

        let n_resume = NodeId::from_u128(1);
        let n_shell = NodeId::from_u128(2);
        let n_plain = NodeId::from_u128(3);

        let mut store = EventStore::open(base.join("events")).expect("event store");
        seed_terminal(
            &mut store,
            n_resume,
            10.0,
            20.0,
            "Especialista",
            "backend",
            "Claude Code",
            Some("claude-code"),
        );
        seed_terminal(
            &mut store,
            n_shell,
            30.0,
            40.0,
            "Terminal Puro",
            "terminal",
            "Shell",
            None,
        );
        seed_terminal(
            &mut store,
            n_plain,
            50.0,
            60.0,
            "Designer",
            "designer",
            "Gemini",
            Some("gemini"),
        );

        // Scrollback persistido (janela viva) — só os que tiveram conversa.
        let sb = Arc::new(Mutex::new(
            ScrollbackStore::open_default(&base).expect("scrollback store"),
        ));
        let resume_lines = vec![
            "linha 1 do claude".to_string(),
            "linha 2 — com acento çã".to_string(),
        ];
        let plain_lines = vec!["saída do gemini".to_string()];
        {
            let mut s = lock(&sb);
            for l in &resume_lines {
                s.push_line(&n_resume.to_string(), l.clone())
                    .expect("push resume");
            }
            for l in &plain_lines {
                s.push_line(&n_plain.to_string(), l.clone())
                    .expect("push plain");
            }
        }

        let proj = store.project().expect("project");
        let plan = plan_restore(&proj, &reg, Some(&sb));
        assert_eq!(plan.len(), 3, "3 terminais re-erguem");
        let by = |n: NodeId| plan.iter().find(|r| r.node == n).expect("nó no plano");

        let r = by(n_resume);
        assert_eq!((r.x, r.y), (10.0, 20.0), "posição vinda do log");
        assert_eq!(r.name, "Especialista");
        assert_eq!(r.role.as_deref(), Some("backend"));
        assert_eq!(
            r.scrollback_tail, resume_lines,
            "scrollback re-hidratado byte-idêntico"
        );
        assert_eq!(
            r.badge,
            RestoreBadge::Resumed,
            "resume declarado + sessão anterior observada"
        );
        assert_eq!(
            r.command,
            vec!["claude", "--output-format", "stream-json", "--resume"],
            "comando do restore carrega o verbo de resume do TOML"
        );

        let sh = by(n_shell);
        assert_eq!((sh.x, sh.y), (30.0, 40.0));
        assert_eq!(sh.name, "Terminal Puro");
        assert!(sh.scrollback_tail.is_empty(), "shell puro sem histórico");
        assert_eq!(
            sh.badge,
            RestoreBadge::FreshStart,
            "shell puro → novo começo"
        );
        assert!(
            sh.command.is_empty(),
            "shell sem perfil → sem comando montado"
        );
        assert_eq!(sh.profile_id, None);

        let pl = by(n_plain);
        assert_eq!(pl.name, "Designer");
        assert_eq!(
            pl.scrollback_tail, plain_lines,
            "gemini re-hidrata o histórico"
        );
        assert_eq!(
            pl.badge,
            RestoreBadge::FreshStart,
            "motor SEM resume → novo começo MESMO com histórico (badge segue capacidade, não dado)"
        );
        assert_eq!(
            pl.command,
            vec!["gemini"],
            "sem resume_args → comando sem retomada"
        );
        assert_eq!(pl.profile_id.as_deref(), Some("gemini"));

        // Critério 4: derivável do log — re-projetar do MESMO store dá o MESMO plano.
        let proj2 = store.project().expect("re-project");
        assert_eq!(
            plan_restore(&proj2, &reg, Some(&sb)),
            plan,
            "limpar projeções → replay → mesmo restore"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **F1-4-3 (risco #5, copy-f1-4 §4):** o badge segue o OBSERVADO, nunca o declarado. Um motor
    /// com resume declarado mas SEM sessão anterior cai para «Novo começo» e o comando NÃO ganha o
    /// verbo de resume — só se retoma o que existe de fato. Ao surgir a sessão, vira «Sessão retomada».
    #[test]
    fn restore_badge_follows_observed_not_declared() {
        let base =
            std::env::temp_dir().join(format!("lina-restore-observed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let mut reg = ProfileRegistry::new();
        reg.insert(claude_resume_profile());

        let node = NodeId::from_u128(7);
        let mut store = EventStore::open(base.join("events")).expect("store");
        seed_terminal(
            &mut store,
            node,
            0.0,
            0.0,
            "Especialista",
            "backend",
            "Claude Code",
            Some("claude-code"),
        );
        let proj = store.project().expect("project");

        // SEM store de scrollback → nada a retomar.
        let plan = plan_restore(&proj, &reg, None);
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].badge,
            RestoreBadge::FreshStart,
            "resume declarado, mas nada observado a retomar → novo começo"
        );
        assert_eq!(
            plan[0].command,
            vec!["claude", "--output-format", "stream-json"],
            "sem retomada observada → comando SEM --resume"
        );

        // COM store mas painel VAZIO → idem (o observado é vazio).
        let sb = Arc::new(Mutex::new(
            ScrollbackStore::open_default(&base).expect("scrollback"),
        ));
        let plan2 = plan_restore(&proj, &reg, Some(&sb));
        assert_eq!(
            plan2[0].badge,
            RestoreBadge::FreshStart,
            "painel vazio → novo começo"
        );

        // Agora COM sessão anterior → «Sessão retomada» + o verbo de resume entra.
        {
            let mut s = lock(&sb);
            s.push_line(&node.to_string(), "oi de ontem".to_string())
                .expect("push");
        }
        let plan3 = plan_restore(&proj, &reg, Some(&sb));
        assert_eq!(
            plan3[0].badge,
            RestoreBadge::Resumed,
            "sessão anterior observada → retomada de verdade"
        );
        assert_eq!(
            plan3[0].command,
            vec!["claude", "--output-format", "stream-json", "--resume"],
            "agora o verbo de resume entra"
        );

        // Strings CONGELADAS do badge + hover (copy-f1-4 §4 — fonte autoritativa).
        assert_eq!(RestoreBadge::Resumed.label(), "Sessão retomada");
        assert_eq!(
            RestoreBadge::FreshStart.label(),
            "Novo começo — o Agente não lembra da conversa anterior"
        );
        assert_eq!(
            RestoreBadge::Resumed.hover(),
            "O Agente continua de onde vocês pararam."
        );
        assert_eq!(
            RestoreBadge::FreshStart.hover(),
            "A conversa de antes continua guardada aqui na tela — é só rolar para cima."
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **F1-4-3 — a metade EXECUTORA (costura de boot): `restore_terminals` re-ergue o plano.**
    /// NÃO-VACUOSO: prova que cada `RestoredTerminal` vira nó VIVO pelo funil (nome/posição na
    /// projeção), que a janela viva re-hidratada APARECE no grid (a conversa "continua na
    /// tela") e que o badge honesto entra em `SharedModel::restore_badges` sob o nó NOVO.
    /// Remover qualquer uma das três pernas do executor derruba um assert.
    #[test]
    fn restore_terminals_executes_plan_rehydrates_grid_and_badges() {
        let dir = std::env::temp_dir().join(format!("lina-restore-exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(
            EventStore::open(dir.join("events")).expect("store"),
        ));
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let sup = Arc::new(Supervisor::new());
        let (delta_tx, _delta_rx) = std::sync::mpsc::channel::<GridDelta>();
        let cmd_factory: CmdFactory = Arc::new(|_n: &str| PtyCommand::new("cat"));
        let model: Model = Arc::new(Mutex::new(SharedModel::default()));
        let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let nm = NodeManager::new(
            Arc::clone(&pty),
            Arc::clone(&sup),
            Arc::clone(&store),
            Arc::clone(&model),
            Arc::clone(&grids),
            BTreeMap::new(),
            delta_tx,
            80,
            24,
            0,
            cmd_factory,
            None,
            dir.join(".lina"),
        );

        let plans = vec![
            RestoredTerminal {
                node: NodeId::from_u128(1), // id ANTIGO (do log da sessão passada)
                name: "Maestro".into(),
                role: Some("maestro".into()),
                x: 30.0,
                y: 96.0,
                cwd: None,
                profile_id: None,
                command: Vec::new(), // shell puro (fábrica do workspace)
                badge: RestoreBadge::FreshStart,
                scrollback_tail: vec!["plano da rodada 3 fechado".into(), "até amanhã ✓".into()],
                shadows: Vec::new(),
            },
            RestoredTerminal {
                node: NodeId::from_u128(2),
                name: "Revisor".into(),
                role: Some("revisor".into()),
                x: 740.0,
                y: 96.0,
                cwd: None,
                profile_id: None,
                command: Vec::new(),
                badge: RestoreBadge::FreshStart,
                scrollback_tail: Vec::new(), // sem histórico — nada a re-hidratar
                shadows: Vec::new(),
            },
        ];
        let up = nm.restore_terminals(&plans);
        assert_eq!(up, 2, "os dois terminais do plano re-ergueram");

        // Roster vivo: nomes/papéis re-erguidos pelo funil (NodeIds NOVOS desta geração).
        let roster = sup.list();
        let names: Vec<&str> = roster.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Maestro") && names.contains(&"Revisor"));

        // Posições do log na projeção (o canvas volta IGUAL).
        let m = lock(&model);
        let maestro = m
            .nodes
            .iter()
            .find(|(_, v)| v.name == "Maestro")
            .map(|(id, v)| (*id, v.x, v.y))
            .expect("Maestro na projeção");
        assert_eq!((maestro.1, maestro.2), (30.0, 96.0));

        // Badge honesto na projeção, sob o nó NOVO.
        assert_eq!(
            m.restore_badges.get(&maestro.0),
            Some(&RestoreBadge::FreshStart),
            "badge do restore registrado para o render"
        );
        drop(m);

        // A janela viva re-hidratada APARECE no grid (antes de output novo do shell).
        let grid = lock(&grids)
            .get(&maestro.0)
            .cloned()
            .expect("grid do Maestro");
        let screen = screen_text(&grid);
        assert!(
            screen.contains("plano da rodada 3 fechado") && screen.contains("até amanhã ✓"),
            "a conversa de antes está NA TELA (é só rolar para cima); screen:\n{screen}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────── F1-4-3 · fixes do restore (3 bugs da tela do fundador, 2026-06-11) ─────────

    /// **Bug "nomes resetados":** o nome dado na ADMISSÃO precisa sobreviver ao quit DENTRO do
    /// log — o restore só enxerga a projeção, e `ProjectedNode.name` só nasce de `NodeRenamed`.
    /// Caminho de PRODUÇÃO (funil `admit_node`, não o helper de seed): admite → projeta → plano.
    #[test]
    fn admission_persists_name_and_role_into_the_log_for_restore() {
        let (nm, store, _model) = test_manager("nome-persiste", None);
        nm.admit_node(NodeAdmission {
            name: Some("Maestro da Obra".into()),
            role: "maestro".into(),
            engine: None,
            cwd: CwdPolicy::Managed,
            position: Some((10.0, 20.0)),
            requested_by: None,
            autonomy: Autonomy::Assisted,
        })
        .expect("admissão");
        let proj = lock(&store).project().expect("project");
        let reg = ProfileRegistry::new();
        let plans = plan_restore(&proj, &reg, None);
        let plan = plans
            .iter()
            .find(|p| (p.x, p.y) == (10.0, 20.0))
            .expect("nó admitido entra no plano de restore");
        assert_eq!(
            plan.name, "Maestro da Obra",
            "o nome da admissão sobrevive no log — sem isto o restore renomeia tudo para 'Terminal N'"
        );
        assert_eq!(plan.role.as_deref(), Some("maestro"));
    }

    /// **Bug "nomes resetados" (corolário):** o nome AUTOMÁTICO (`⌘T`) não pode colidir com um
    /// nome já vivo no roster — o seq reinicia a cada boot, mas o restore re-ergue nós que JÁ
    /// se chamam "Terminal C/D/…" (colisão = outbox/A2A ambíguos). Reproduz: um nó explícito
    /// ocupa o PRÓXIMO rótulo do contador; o automático seguinte tem de pular para o D.
    #[test]
    fn auto_names_skip_names_already_taken_in_the_roster() {
        let (nm, _store, _model) = test_manager("nome-colisao", None);
        // Ocupa "Terminal D" — o rótulo que o contador dará ao PRÓXIMO ⌘T (esta admissão
        // explícita consome o seq do C; deriva real do restore: o nome auto-dado num boot
        // anterior volta persistido num seq diferente do que o contador novo vai alcançar).
        nm.admit_node(NodeAdmission {
            name: Some("Terminal D".into()),
            role: "terminal".into(),
            engine: None,
            cwd: CwdPolicy::Managed,
            position: None,
            requested_by: None,
            autonomy: Autonomy::Assisted,
        })
        .expect("nó restaurado com nome 'Terminal D'");
        let node = nm.add_node().expect("⌘T");
        let name = lock(&nm.model)
            .nodes
            .get(&node)
            .map(|v| v.name.clone())
            .expect("nó na projeção");
        assert_eq!(
            name, "Terminal E",
            "nome automático pula os já tomados ('Terminal D' está vivo no roster)"
        );
    }

    /// **Bug "terminais empilhados" (metade 1):** gerações já FECHADAS no log (`Dead`, póstumo do
    /// `close_previous_generation`) não re-erguem — só quem estava vivo no último quit volta.
    #[test]
    fn plan_restore_skips_dead_generations() {
        let base = std::env::temp_dir().join(format!("lina-restore-dead-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut store = EventStore::open(base.join("events")).expect("store");
        // Geração FECHADA de verdade: o Dead tem SUCESSOR VIVO de mesmo nome (o supersede
        // normal ainda removeria o antigo; aqui modelamos o pior caso — ele sobrou no log).
        let morto = NodeId::from_u128(1);
        let vivo = NodeId::from_u128(2);
        seed_terminal(
            &mut store, morto, 30.0, 40.0, "Maestro", "maestro", "Shell", None,
        );
        seed_terminal(
            &mut store, vivo, 10.0, 20.0, "Maestro", "maestro", "Shell", None,
        );
        store
            .append(&DomainEvent::NodeStatusChanged {
                node: morto,
                status: lina_core::NodeStatus::Dead.as_str().to_string(),
                from: "Idle".to_string(),
                reason: "app_reopened".to_string(),
            })
            .expect("morte póstuma");
        let proj = store.project().expect("project");
        let plans = plan_restore(&proj, &ProfileRegistry::new(), None);
        assert_eq!(
            plans.len(),
            1,
            "só o vivo re-ergue; o Dead COM sucessor vivo fica no passado (sem empilhar)"
        );
        assert_eq!(plans[0].node, vivo);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Bug da tela 2026-06-11 (2ª reabertura, metade "sumiço"):** `Dead` NOMEADO sem sucessor
    /// vivo de mesmo nome = ÓRFÃO (a admissão dele falhou num boot anterior; ninguém o re-ergueu
    /// nem o aposentou). Pular todo `Dead` o apagava do Espaço PARA SEMPRE — 23 terminais do
    /// fundador. O plano resgata a geração MAIS NOVA de cada nome órfão; `Dead` sem nome
    /// (pré-ADR-0022) segue fora; `Dead` com sucessor vivo segue fora (anti-empilhamento).
    #[test]
    fn plan_restore_rescues_named_dead_orphans() {
        let base = std::env::temp_dir().join(format!("lina-restore-orfao-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut store = EventStore::open(base.join("events")).expect("store");
        let mark_dead = |store: &mut EventStore, node: NodeId| {
            store
                .append(&DomainEvent::NodeStatusChanged {
                    node,
                    status: lina_core::NodeStatus::Dead.as_str().to_string(),
                    from: "Idle".to_string(),
                    reason: "app_reopened".to_string(),
                })
                .expect("morte póstuma");
        };
        // "Terminal J": DUAS gerações órfãs (5 e 9) — só a mais nova (9) volta.
        let j_velho = NodeId::from_u128(5);
        let j_novo = NodeId::from_u128(9);
        seed_terminal(
            &mut store,
            j_velho,
            30.0,
            96.0,
            "Terminal J",
            "terminal",
            "claude-code",
            Some("claude-code"),
        );
        seed_terminal(
            &mut store,
            j_novo,
            30.0,
            96.0,
            "Terminal J",
            "terminal",
            "claude-code",
            Some("claude-code"),
        );
        mark_dead(&mut store, j_velho);
        mark_dead(&mut store, j_novo);
        // "Terminal A": Dead antigo + sucessor VIVO → só o vivo (sem empilhar).
        let a_morto = NodeId::from_u128(3);
        let a_vivo = NodeId::from_u128(4);
        seed_terminal(
            &mut store,
            a_morto,
            740.0,
            96.0,
            "Terminal A",
            "terminal",
            "Shell",
            None,
        );
        seed_terminal(
            &mut store,
            a_vivo,
            740.0,
            96.0,
            "Terminal A",
            "terminal",
            "Shell",
            None,
        );
        mark_dead(&mut store, a_morto);
        // Sem nome (geração pré-ADR-0022: NodeRenamed nunca foi ao log) → sem identidade, fora.
        let sem_nome = NodeId::from_u128(6);
        store
            .append(&DomainEvent::NodeAdded {
                node: sem_nome,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("NodeAdded");
        store
            .append(&DomainEvent::TerminalSpawned {
                node: sem_nome,
                cli: "Shell".into(),
                cwd: None,
            })
            .expect("TerminalSpawned");
        mark_dead(&mut store, sem_nome);

        let proj = store.project().expect("project");
        let plans = plan_restore(&proj, &ProfileRegistry::new(), None);
        let nodes: std::collections::BTreeSet<NodeId> = plans.iter().map(|p| p.node).collect();
        assert_eq!(
            nodes,
            [a_vivo, j_novo].into_iter().collect(),
            "vivo entra; órfão MAIS NOVO volta; órfão velho, Dead-com-sucessor e sem-nome ficam"
        );
        // Sombras: a geração VELHA do J se aposenta junto com o resgate do novo — fechar o
        // card depois não ressuscita a próxima da fila (cadeia de zumbis morta na raiz).
        let plano_j = plans.iter().find(|p| p.node == j_novo).expect("plano do J");
        assert_eq!(plano_j.shadows, vec![j_velho]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Bug da tela 2026-06-11 (2ª reabertura):** com o PATH degradado (hidratação do shell de
    /// login estourou o timeout), o exec do MOTOR falha e o restore DESCARTAVA o nó em silêncio —
    /// o terminal sumia do Espaço para sempre (Dead sem sucessor). Agora: re-ergue como shell
    /// preservando nome/papel/posição, re-apenda a LINHAGEM do CLI no log (o próximo boot
    /// re-tenta o motor) e o badge conta ao leigo o que houve.
    #[test]
    fn restore_with_missing_engine_degrades_to_shell_preserving_lineage() {
        let (nm, store, model) = test_manager("restore-engine-missing", None);
        let antigo = NodeId::from_u128(88);
        seed_terminal(
            &mut lock(&store),
            antigo,
            30.0,
            96.0,
            "Terminal J",
            "terminal",
            "claude-code",
            Some("claude-code"),
        );

        // Perfil cujo binário NÃO existe nesta máquina — o exec falha exatamente como sob o
        // PATH mínimo do launchd (`claude` por nome puro, invisível sem hidratação).
        let mut reg = ProfileRegistry::new();
        reg.insert(
            CliProfile::from_toml_str(
                r#"
                    id = "claude-code"
                    program = "lina-motor-inexistente-9f3x"
                    delivery = "pty_inject"
                    prompt_ready_regex = "> "
                    [end_signal]
                    kind = "idle"
                "#,
                "<engine-missing>",
            )
            .expect("profile parseia"),
        );

        let proj = lock(&store).project().expect("project");
        let plans = plan_restore(&proj, &reg, None);
        assert_eq!(plans.len(), 1, "o nó com motor entra no plano");
        assert_eq!(
            plans[0].command.first().map(String::as_str),
            Some("lina-motor-inexistente-9f3x"),
            "o plano tenta o MOTOR primeiro"
        );

        assert_eq!(
            nm.restore_terminals(&plans),
            1,
            "o nó re-ergue mesmo com o motor indisponível (nunca descartado)"
        );

        // Projeção nova: o nome sobreviveu E a linhagem do CLI sobreviveu (CliProfileSet
        // re-apendado) — o próximo plan_restore volta a tentar o claude.
        let proj2 = lock(&store).project().expect("re-project");
        let (novo, info) = proj2
            .nodes
            .iter()
            .find(|(_, v)| v.name.as_deref() == Some("Terminal J"))
            .expect("geração nova existe com o mesmo nome");
        assert_eq!(
            info.cli.as_deref(),
            Some("claude-code"),
            "linhagem do CLI preservada no log do nó degradado"
        );
        // Badge honesto: o leigo vê que a IA não religou (e que nada se perdeu).
        assert_eq!(
            lock(&model).restore_badges.get(novo).copied(),
            Some(RestoreBadge::EngineMissing)
        );
    }

    /// **Bug da tela 2026-06-11 ("um terminal em cima do outro"):** o log herdou da era pré-fix
    /// coordenadas EMPILHADAS (até 10 cards no mesmo ponto) e o restore as reproduzia fielmente
    /// para sempre. Agora o primeiro ocupante mantém a coordenada; os demais re-alocam via
    /// `next_free_slot` e a posição nova PERSISTE (`NodeAdded`) — correção durável.
    #[test]
    fn restore_relayouts_colliding_positions() {
        let (nm, store, model) = test_manager("restore-colisao", None);
        for (id, name) in [(11_u128, "Card 1"), (12, "Card 2"), (13, "Card 3")] {
            seed_terminal(
                &mut lock(&store),
                NodeId::from_u128(id),
                30.0,
                96.0, // TODOS na mesma coordenada (herança da era pré-fix)
                name,
                "terminal",
                "Shell",
                None,
            );
        }
        let proj = lock(&store).project().expect("project");
        let plans = plan_restore(&proj, &ProfileRegistry::new(), None);
        assert_eq!(plans.len(), 3);
        assert_eq!(nm.restore_terminals(&plans), 3);

        let m = lock(&model);
        let mut posicoes: Vec<(i64, i64)> = m
            .nodes
            .values()
            .filter(|v| v.name.starts_with("Card "))
            .map(|v| (f64::from(v.x).round() as i64, f64::from(v.y).round() as i64))
            .collect();
        posicoes.sort_unstable();
        posicoes.dedup();
        assert_eq!(
            posicoes.len(),
            3,
            "três cards, três coordenadas distintas — nada empilhado: {posicoes:?}"
        );
        assert!(
            posicoes.contains(&(30, 96)),
            "o primeiro ocupante mantém a coordenada original"
        );
    }

    /// **Bug "terminais empilhados" (metade 2 — supersede):** o restore re-admite como nó NOVO;
    /// o nó ANTIGO precisa ser APOSENTADO no log (`NodeRemoved`) — senão cada reabertura dobra o
    /// canvas (23 cards na MESMA posição na tela do fundador). Auditável: o remove fica no log.
    #[test]
    fn restore_terminals_retires_the_superseded_node_in_the_log() {
        let (nm, store, _model) = test_manager("restore-supersede", None);
        let antigo = NodeId::from_u128(77);
        seed_terminal(
            &mut lock(&store),
            antigo,
            30.0,
            96.0,
            "Maestro",
            "maestro",
            "Shell",
            None,
        );
        let proj = lock(&store).project().expect("project");
        let plans = plan_restore(&proj, &ProfileRegistry::new(), None);
        assert_eq!(plans.len(), 1, "o antigo entra no plano");
        assert_eq!(nm.restore_terminals(&plans), 1, "re-ergueu");

        let proj2 = lock(&store).project().expect("re-project");
        assert!(
            !proj2.nodes.contains_key(&antigo),
            "o nó da geração anterior foi aposentado (NodeRemoved) — não re-ergue de novo no próximo boot"
        );
        let novo = proj2
            .nodes
            .iter()
            .find(|(_, v)| v.name.as_deref() == Some("Maestro"));
        assert!(
            novo.is_some(),
            "a geração nova existe na projeção com o MESMO nome"
        );
    }

    /// **Bug "shell puro por baixo":** nó de geração antiga sem `CliProfileSet` guarda só o
    /// RÓTULO do motor ("Claude Code") no `TerminalSpawned.cli`. O plano resolve o perfil pelo
    /// slug do rótulo em vez de degradar para shell puro — o claude volta como claude.
    #[test]
    fn plan_restore_resolves_profile_from_engine_label_when_id_missing() {
        let base = std::env::temp_dir().join(format!("lina-restore-label-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut store = EventStore::open(base.join("events")).expect("store");
        let node = NodeId::from_u128(9);
        // Geração antiga: TerminalSpawned com o RÓTULO, sem CliProfileSet (profile=None).
        seed_terminal(
            &mut store,
            node,
            0.0,
            0.0,
            "Especialista",
            "backend",
            "Claude Code",
            None,
        );
        let mut reg = ProfileRegistry::new();
        reg.insert(claude_resume_profile());
        let proj = store.project().expect("project");
        let plans = plan_restore(&proj, &reg, None);
        assert_eq!(plans.len(), 1);
        assert_eq!(
            plans[0].profile_id.as_deref(),
            Some("claude-code"),
            "rótulo 'Claude Code' resolve para o perfil 'claude-code' (slug)"
        );
        assert_eq!(
            plans[0].command,
            vec!["claude", "--output-format", "stream-json"],
            "o claude volta como claude — nunca shell puro silencioso"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Bug "porta morta nos hooks" (ECONNREFUSED):** o kit que a LINA escreveu nos dirs
    /// GERENCIADOS (`<ws_root>/n-…`) é nosso POR CONSTRUÇÃO — mesmo sem a marca (gerações
    /// antigas, pré-carimbo). O `write_user_dir` do restore/rewrite REESCREVE o kit nesses dirs
    /// (a porta efêmera dos hooks muda a cada boot); pasta REAL do usuário segue merge-safe.
    #[test]
    fn write_user_dir_refreshes_lina_kit_in_managed_namespace_dirs() {
        let ws = std::env::temp_dir().join(format!("lina-managed-ns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let bw = BootstrapWriter::new(
            ws.clone(),
            "/v/vault".to_string(),
            Autonomy::Assisted,
            "lina".to_string(),
        )
        .expect("bootstrap writer");

        // Dir gerenciado de GERAÇÃO ANTIGA: kit sem carimbo (como o app escrevia até hoje).
        let managed = ws.join("n-0192aaaa-bbbb-7ccc-8ddd-eeeeffff0001");
        std::fs::create_dir_all(managed.join(".claude")).expect("mkdir");
        std::fs::write(
            managed.join(".claude/settings.json"),
            r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"http://127.0.0.1:57545/morto"}]}]}}"#,
        )
        .expect("settings velho");
        std::fs::write(managed.join("CLAUDE.md"), "# doutrina velha sem marcador\n").expect("md");

        let roster = vec!["Maestro".to_string()];
        let outcome = bw.write_user_dir(&managed, "Maestro", &roster);
        assert_eq!(
            outcome,
            KitOutcome::Full,
            "kit em dir gerenciado é nosso por construção — reescreve, nunca recusa"
        );
        let settings =
            std::fs::read_to_string(managed.join(".claude/settings.json")).expect("settings novo");
        assert!(
            !settings.contains("57545/morto"),
            "a porta morta foi embora do settings"
        );
        assert!(
            settings.contains("whoami --bootstrap"),
            "o settings novo tem os hooks atuais"
        );
        assert!(
            settings_is_ours(&settings),
            "o settings reescrito sai CARIMBADO (próximo boot não depende do namespace)"
        );
        let md = std::fs::read_to_string(managed.join("CLAUDE.md")).expect("md novo");
        assert!(
            md.starts_with(LINA_MANAGED_MARKER),
            "a doutrina também é reescrita e carimbada"
        );

        // CONTROLE: pasta REAL do usuário (fora do namespace) continua merge-safe.
        let user_dir = std::env::temp_dir().join(format!("lina-userdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user_dir);
        std::fs::create_dir_all(user_dir.join(".claude")).expect("mkdir user");
        let user_settings = r#"{"meu_setting": true}"#;
        std::fs::write(user_dir.join(".claude/settings.json"), user_settings).expect("settings");
        let outcome2 = bw.write_user_dir(&user_dir, "Maestro", &roster);
        assert!(
            matches!(&outcome2, KitOutcome::Partial(refused) if refused.iter().any(|r| r.contains("settings.json"))),
            "settings do USUÁRIO sem a marca é preservado (recusa visível): {outcome2:?}"
        );
        assert_eq!(
            std::fs::read_to_string(user_dir.join(".claude/settings.json")).expect("re-read"),
            user_settings,
            "conteúdo do usuário intocado"
        );

        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&user_dir);
    }
}
