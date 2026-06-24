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

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lina_pty::PtyError;
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Re-exports da camada de PTY/VT para abrir terminais e ler o grid (gate E2E inclusive).
pub use lina_cli_profiles::{ApprovalKeys, CliProfile};
/// Re-exports do contrato de UI para os consumidores do core (e o host headless de teste).
pub use lina_host::{BusTarget, HeadlessUiHost, HostEvent, NodeId, NodeKind, UiHost};
pub use lina_pty::{PtyCommand, PtyManager};
pub use lina_vt::{AlacrittyBackend, VtBackend, VtCell, VtCursor, VtRgb, VtScreen};

/// Event Store (W0-5) + recuperação pós-crash visível (W0-6).
mod events;
pub use events::{
    apply, AcceptanceCriterion, ApprovalDecision, AwaitReason, BlockReason, CheckKind, DomainEvent,
    Effort, EffortOrigin, EventRecord, EventStore, FlushState, PermissionEvidence, ProjectedNode,
    ProjectedState, PromptKind, ResolutionVia, StoreError, Verdict,
};

/// F2-3-7: store de SESSÃO LOCAL (câmera pan/zoom) — FORA do event log (ADR 0029 §3).
mod session;
pub use session::{session_dir, CameraSnapshot, SessionError, SessionStore};

/// W4-1: CliDiscovery — varredura do `PATH` por CLIs de IA (substrato do check-up de onboarding).
mod cli_discovery;
pub use cli_discovery::{
    discover_clis, discover_clis_in, find_in_path, query_version, DiscoveredCli, KNOWN_CLIS,
};

/// Entrega A2A faseada (W0-9) + contrato de fim-de-resposta (W0-10).
mod a2a;
pub use a2a::{
    build_paste, build_reflection_prompt, deliver_a2a, dispatch_reflection, judge_reflection,
    parse_belief_sentinel, parse_match_marker, sanitize_paste, A2aError, DeliveryOutcome,
    EndDetector, EndOutcome, EndResult, GridSense, InjectPolicy, ReflectionRequest, ReflectorQueue,
    WorkspaceTrust, INTERACTIVE_READY_BUDGET_MS,
};

/// W3-4: mailbox de arquivo (`.lina/`) — contrato `lina/msg@1` CLI↔supervisor.
mod mailbox;
pub use mailbox::{
    now_ms, parse_target, render_message_block, render_message_block_full, render_message_block_v2,
    validate_envelope_v2, AgentPresence, EnvelopeViolation, HandoffContract, MailMessage, Mailbox,
    TargetSpec, CANONICAL_INTENTS_V2, MAIL_SCHEMA_V1, MAIL_SCHEMA_V2,
};

/// W3-4: roteador do supervisor (pipeline `route_message` com guardrails 0-4).
mod router;
pub use router::{
    is_reserved_role, reserved_role_admission_ok, AutonomyLevel, CostLedger, PlanOpError,
    RouteOutcome, Router, RouterConfig, DEDUPE_WINDOW_MS, DELEGATION_BUDGET, FANOUT_GATE,
    HUMAN_GESTURE, MAX_DEPTH, REFLECTOR_ROLE, REFLECTOR_SYSTEM, REFLECTOR_TRIGGER,
};

/// F3-0-1 ([12] Parâmetros): casa versionada dos ~15 números de orquestração — quatro camadas
/// (global ⊕ workspace ⊕ preset ⊕ terminal) resolvidas num [`RouterConfig`] denso. Ver spec 51.
mod params;
pub use params::{
    param_change_action_class, resolve_from_store, validate_range, ParamScope, ParamWarning,
    ParamsLedger, ResolvedParams, SystemParams,
};

/// W3-5: plano compartilhado (`.lina/plan.md`) — projeção event-sourced, escritor único supervisor.
mod plan;
pub use plan::{ItemState, Plan, PlanError, PlanItem, PLAN_SCHEMA_V1};

/// F3-1 ([3] Objetivos): a Goal como entidade event-sourced — projeção reconstruída por replay
/// varrendo o log por `goal_id`. Contrato (Rα-0); lógica de projeção na fatia CORE-Goal seguinte.
mod goal;
pub use goal::{project_goals, Goal, GoalPhase};

/// F1-4-1: multi-Espaço — um store por Espaço + registry-ponteiro + seam único de cwd.
mod workspace;
pub use workspace::{
    can_create_workspace, default_registry_path, resolve_spawn_cwd, workspace_limit, LicenseTier,
    ResolvedCwd, Workspace, WorkspaceEntry, WorkspaceError, WorkspaceRegistry,
};

/// W3-6: núcleo determinístico do gate de execução (classe de ação × autonomia → decisão).
mod guard;
pub use guard::{
    check_action, classify, decide, parse_autonomy, ActionClass, Decision, GateDecision, GuardError,
};

/// W3-6c (ADR 0004): broker de ação custodiada (`lina do`) — custódia de segredo é o gate duro real.
mod broker;
pub use broker::{
    lookup_action, run_custody, BrokerError, BrokerOutcome, BrokerRequest, CustodyAction,
    CLASS_GATED_HARD_EXTERNAL, CUSTODY_ACTIONS,
};

// F1-0-3: state-machine de lifecycle + heartbeat com progresso (ADR 0019).
pub mod lifecycle;
pub use lifecycle::{LifecycleEngine, LifecycleError, SampleOutcome};

/// F1-1-6: detecção de permissão bloqueante multi-camada (hook Notification correlacionado
/// ao `PreToolUse` pendente + fallback por regex no grid via `VtBackend`, FP medido).
pub mod permission_detect;
pub use permission_detect::{
    ClearedPrompt, DetectionTelemetry, LifecycleWiring, PermissionAsk, PermissionDetector,
    PermissionWatch, ScanOutcome, WATCH_IDLE_MS,
};

/// F1-1-7: fila de atenção UNIFICADA (custódia + permissão) — projeção do event log com
/// precedência custódia > permissão, round-robin por nó e escalação visual aos 5 min
/// (ADR 0021 §3/§6 — esta projeção nunca escreve no PTY; o write é F1-1-8).
pub mod attention;
pub use attention::{
    AttentionEvidence, AttentionItem, AttentionKind, AttentionQueue, AttentionState,
    ESCALATE_AFTER_MS, LAYER_MERGE_WINDOW_MS,
};

/// F1-1-8 (ADR 0021): confirmação de aprovação — snapshot da região do prompt (Captura 1),
/// porta única de escrita validada contra a tela (Captura 2) e idempotência por projeção.
pub mod approval;
pub use approval::{
    check_screen, prompt_snapshot_hash, AbortReason, ApprovalExecutor, ApprovalGesture,
    ApprovalLedger, ApprovalOutcome, ApprovalOutcomeKind, ApprovalPort, PortError, PortOutcome,
    ScreenCheck, PROMPT_REGION_ROWS,
};

impl ApprovalExecutor {
    /// F1-1-8 (ADR 0021 §2 / ADR 0024): reconstrói a projeção do executor varrendo o log —
    /// espelha [`AttentionQueue::replay`]/`CostLedger::replay`. O decode `EventRecord → DomainEvent`
    /// (`DomainEvent::from_record`) é **interno ao core**, então este construtor mora aqui (não na
    /// app, que só conhece `EventRecord`). Registros não-decodificáveis (kind de versão futura) são
    /// pulados — a projeção é derivada, não validadora do log. NUNCA escreve no PTY (sem porta neste
    /// caminho): replay/boot só reconstrói estado (trava por construção, ADR 0021 §2).
    #[must_use]
    pub fn replay(records: &[events::EventRecord]) -> Self {
        let mut exec = Self::new();
        for rec in records {
            if let Ok(ev) =
                events::DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone())
            {
                exec.observe(&ev);
            }
        }
        exec
    }
}

pub mod bench;

/// W5-2: scrollback com cap por painel + paginação em disco (SQLite WAL) — janela viva em RAM,
/// histórico além do cap hidratado do disco sob demanda (anti-leak `#painéis × scrollback`).
pub mod scrollback;

/// F1-5-8: consulta PAGINADA do histórico (tail/search/export com limite duro) sobre o
/// scrollback — leitura cross-terminal auditada pela fronteira de pertencimento (ADR 0006).
pub mod history;

/// F3-2-6: briefing em CAMADAS do worker (stable/context/volatile + verify-loop), gated por papel
/// — montagem PURA de texto acima do CLI (inv #1), projeção do log (inv #4). Hermes §6.1/§6.2.
pub mod briefing;

/// F3-3 Mentality (spec 35): projeção `Mentality(papel)` por REPLAY + política de promoção
/// DETERMINÍSTICA (sem LLM, inv #1) sobre o ciclo de vida da crença (`events.rs` §3). Padrão
/// `CostLedger`/`intelligence_adoption` — fonte de leitura para o injetor e o painel. Crença é
/// DADO comportamental, nunca autoridade (§6.1). Implementado em F3-3 (M-PROMO).
pub mod mentality;
// Tipos do casamento correção→crença (ADR 0048) usados na costura do Refletor (`judge_reflection`):
// re-exportados no root p/ o caller importar o conjunto do Refletor de um só lugar.
pub use mentality::{MatchMarker, PresentedBelief};

/// F3-4-5 (spec 36 §3): coordenação de código — projeção `branches_nao_integradas` + gatilho
/// determinístico do DevOps integrador (molde `CostLedger`/`Mentality`, ZERO LLM). O core DECIDE;
/// o disparo é do caminho de spawn governado. `branch`/`paths` são DADO, jamais autoridade.
pub mod code;
pub use code::{CodeIntegration, IntegratorDispatch, INTEGRATOR_ROLE, INTEGRATOR_SETTLE_MS};

/// F1-5-5: suspensão real de ociosos — máquina `Active → Idle → Suspended` (overlay ortogonal ao
/// [`NodeStatus`]; só o render pausa, a drenagem nunca para).
pub mod suspend;
pub use suspend::{
    SuspendConfig, SuspendController, SuspendGuards, SuspendState, SuspendTransition,
};

// ── F3-5 (Auto-aprimoramento, Sessões & Contexto): módulos da onda ──
// Stubs criados na LARGADA — lib.rs é dono único do Maestro (parecer de topologia do
// Arquiteto: 7 frentes criariam `pub mod` aqui = multi-editor). Cada frente PREENCHE só o
// seu arquivo e NÃO edita lib.rs. Re-exports (`pub use`) ficam comigo na integração; o
// acesso provisório é via caminho completo `lina_core::<mod>::<Tipo>`.
pub mod buffer_registry; // F3-5-7 · BUFFERS (K)
pub mod clue; // F3-5-6 · CONTEXTO (I)
pub mod disk_budget; // F3-5-8 · DISCO (M)
pub mod resume_session; // F3-5-1 · SESSÕES (B)
pub mod skill_factory; // F3-5-5 · SKILLS (J)
pub mod skill_index; // F3-5-4 · SKILLS (J)
pub mod workspace_template; // F3-5-10 · TEMPLATES (G)

// ── F4-0 (Substrato de canais & credenciais): módulos da onda ──
// Mesmo padrão da largada F3-5 — lib.rs é dono único do Maestro; cada frente preenche só o seu
// arquivo. Eventos do contrato já congelados em `events.rs` (largada). `broker.rs` (custódia) e
// `lina-secrets` (cofre) JÁ existem — F4-0-3/F4-0-2 os ESTENDEM, não criam módulo.
pub mod channel; // F4-0-1 · CANAIS — trait Channel + ChannelRegistry + manifesto (B)
pub mod credential; // F4-0-2 · CREDENCIAIS — binding cofre↔event-log (I; emerg. mid-onda)
pub mod tool_scope; // F4-0-4 · CONTEXTO — pré-config de ferramentas/grupos por projeto (M)

/// Origem de uma entrega A2A — QUEM a disparou, derivada server-side (inforjável). ADR 0007/0035.
///
/// O Router conhece **humano** (`hops == 0`, sender sem binding) e **colega** (cascata, `hops >= 1`
/// via `delivered_root[sender]`) — ambos derivados de binding interno, JAMAIS de campo de agente. A
/// F4-WA (webhook ativo) adiciona a TERCEIRA origem, `System`, carimbada pela engine `lina-webhooks`
/// no ato do despacho autenticado por HMAC — **nunca lida de `msg.from`/payload** (um agente que
/// escreva `from:"system:<id>"` cai em `node_by_name`/`UnknownSender`, jamais herda `System`).
///
/// ADITIVA: variante nova não quebra replay; `Default` = `Peer` (conservador) para envelope antigo.
/// Desenhada para ganhar novas origens-servidor (e-mail/fila/MQTT) sem reescrever o Router.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MsgOrigin {
    /// Agente a mando do usuário (`hops == 0`, sender sem binding) — ADR 0007.
    Human,
    /// Cascata A2A (`hops >= 1`, binding `delivered_root[sender]`) — ADR 0007.
    #[default]
    Peer,
    /// O SERVIDOR que recebeu um webhook autenticado por HMAC. Carimbada pela engine `lina-webhooks`
    /// no ato do despacho — NUNCA de `msg.from`/payload (inforjável; herda binding do ADR 0007/0035).
    System { webhook_id: String },
}

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
    /// F4-WA (ADITIVO, ADR 0035): origem da entrega, carimbada server-side. `Default` = `Peer`; só a
    /// engine de webhook chama `.with_origin(System{..})`. Os 3 call-sites via `::new` não quebram.
    pub origin: MsgOrigin,
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
            origin: MsgOrigin::Peer,
        }
    }

    /// Marca o envelope como bloqueante (espera `reply` correlacionado).
    #[must_use]
    pub fn awaiting(mut self) -> Self {
        self.await_reply = true;
        self
    }

    /// F4-WA (ADR 0035): carimba a origem da entrega. SÓ a engine de webhook a usa para setar
    /// `System{webhook_id}` — server-side, no despacho autenticado por HMAC. O caminho A2A normal
    /// (humano/colega) deixa o default `Peer` (a origem real é derivada por `derive_root_hops`).
    #[must_use]
    pub fn with_origin(mut self, origin: MsgOrigin) -> Self {
        self.origin = origin;
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

/// W5-2 FIAÇÃO: destino do cabo `append-on-scroll` de um terminal. Cada linha que o motor VT
/// solta do viewport (`VtBackend::take_scrollback`) é persistida no `ScrollbackStore` do workspace
/// sob a chave `panel` (o `NodeId` do terminal). O Store é compartilhado entre os terminais
/// (conexão SQLite single-thread, serializada pelo `Mutex`).
struct ScrollbackSink {
    store: Arc<Mutex<scrollback::ScrollbackStore>>,
    panel: String,
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
    /// W5-2 FIAÇÃO: cabo do scrollback (None = sem persistência; caminho legado intacto).
    scrollback: Option<ScrollbackSink>,
    /// F2-0-6 (higiene pós-QA): sinal "sync abriu" — a vigia BLOQUEIA aqui sem timeout
    /// enquanto não há sync pendente (**zero wakeup ocioso** — mesma doutrina anti-desperdício
    /// do scrollback/41GB); o `flush` do reader notifica quando `sync_deadline()` vira `Some`.
    /// O bool dentro do Mutex evita lost-wakeup (sinal antes do wait). Saída no shutdown:
    /// wait com timeout de segurança (5s) re-checa `stop`/estado — thread parada nunca vaza
    /// além disso após um kill.
    sync_wake: (Mutex<bool>, Condvar),
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
    /// W5-2 FIAÇÃO: Store de scrollback do workspace. Quando presente, todo terminal aberto a
    /// partir daí nasce **capturando** (cap = `store.cap()`) e tem o cabo `append-on-scroll` ligado.
    scrollback: Option<Arc<Mutex<scrollback::ScrollbackStore>>>,
    /// F1-5-6: o job ÚNICO de durabilidade (idle-drain + sinais + retenção diária F1-5-9).
    /// Vive aqui porque o `PtyHost` é o dono natural do ciclo de vida do scrollback
    /// (decisão da story: "idle-drain como job único no PtyHost"). Para no Drop.
    flush_guard: Option<scrollback::FlushGuard>,
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
            scrollback: None,
            flush_guard: None,
        }
    }

    /// W5-2 FIAÇÃO: liga o `ScrollbackStore` do workspace. Terminais abertos **a partir daqui**
    /// nascem capturando o scrollback (cabo `append-on-scroll`): cada linha que sai do viewport é
    /// persistida no Store sob a chave do `NodeId`, com o ring do motor VT fixado no `store.cap()`
    /// — fecha o anti-leak da W5-1 (RAM no cap, histórico completo no disco). Idempotente: chame
    /// antes de `spawn`.
    pub fn set_scrollback_store(&mut self, store: Arc<Mutex<scrollback::ScrollbackStore>>) {
        self.scrollback = Some(store);
    }

    /// F1-5-6: sobe o **job único de durabilidade** sobre o store ligado em
    /// [`PtyHost::set_scrollback_store`] — idle-drain (painel ocioso 1-2s → flush),
    /// handlers de sinal (`SIGTERM`/`SIGINT`/`SIGHUP` → `flush_all` antes de morrer) e o
    /// job diário de retenção (F1-5-9), tudo numa ÚNICA thread (a nota load-bearing do
    /// 13.16 — nunca uma thread por painel sobre o Mutex global). `Ok(false)` = sem store
    /// ligado (nada a proteger). Idempotente: re-chamar substitui o guard anterior
    /// (o velho para no drop).
    pub fn start_flush_guard(
        &mut self,
        cfg: scrollback::FlushGuardConfig,
    ) -> Result<bool, scrollback::ScrollbackError> {
        match &self.scrollback {
            Some(store) => {
                self.flush_guard = Some(scrollback::FlushGuard::start(Arc::clone(store), cfg)?);
                Ok(true)
            }
            None => Ok(false),
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

        // W5-2 FIAÇÃO: com Store ligado, o backend nasce CAPTURANDO (cap = cap do Store, fonte
        // única do teto da janela viva) e ganha o cabo `append-on-scroll`; senão, caminho legado.
        let (backend, sink): (Box<dyn VtBackend>, Option<ScrollbackSink>) = match &self.scrollback {
            Some(store) => {
                let cap = lock(store).cap();
                (
                    Box::new(AlacrittyBackend::with_scrollback_capture(cols, rows, cap)),
                    Some(ScrollbackSink {
                        store: Arc::clone(store),
                        panel: node.to_string(),
                    }),
                )
            }
            None => (Box::new(AlacrittyBackend::new(cols, rows)), None),
        };
        let shared = Arc::new(TermShared {
            vt: Mutex::new(backend),
            inflight: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            panic_inject: AtomicBool::new(false),
            state: Mutex::new(TerminalState::Running),
            metrics: MetricsInner::default(),
            scrollback: sink,
            sync_wake: (Mutex::new(false), Condvar::new()),
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
        // F2-0-6: vigia-irmã do teto do synchronized output (detached — encerra sozinha por
        // `stop`/estado ≤100ms após o terminal sair; o reader segue 100% intocado).
        {
            let shared = Arc::clone(&shared);
            let delta_tx = self.delta_tx.clone();
            let seq = Arc::clone(&self.seq);
            thread::Builder::new()
                .name(format!("lina-sync-watchdog-{node}"))
                .spawn(move || run_sync_watchdog(shared, delta_tx, node, seq))
                .map_err(|e| PtyHostError::Io(node, e.to_string()))?;
        }

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

    /// R2b: os terminais registrados neste host (ordem indefinida) — base da varredura
    /// viva da detecção de permissão ([`permission_detect::PermissionWatch::scan`]).
    #[must_use]
    pub fn nodes(&self) -> Vec<NodeId> {
        self.terminals.keys().copied().collect()
    }

    /// F1-1-8 (ADR 0021 §1, Captura 2): a **porta única** de entrega de aprovação —
    /// re-snapshot da região do prompt ([`approval::prompt_snapshot_hash`]), comparação
    /// com `expected_hash` e write de `keys`, tudo SOB o mesmo lock do `VtBackend` que
    /// serializa o `advance` do `flush()`: nenhum byte do PTY é aplicado ao grid entre o
    /// check e o write — a atomicidade local do "mesmo turno do loop". Tela divergente ⇒
    /// NENHUM byte é escrito (fail-safe). `region_rows` = o `K` da Captura 1 (mesmo valor,
    /// senão nunca casa). Output do filho ainda no buffer do kernel é a janela irredutível
    /// documentada (ADR §4 R2) — reduzida, não eliminada.
    pub fn deliver_approval(
        &self,
        node: NodeId,
        expected_hash: &str,
        keys: &[u8],
        region_rows: usize,
    ) -> Result<approval::PortOutcome, PtyHostError> {
        let term = self
            .terminals
            .get(&node)
            .ok_or(PtyHostError::NotFound(node))?;
        let vt = lock(&term.shared.vt);
        match approval::check_screen(&**vt, expected_hash, region_rows) {
            approval::ScreenCheck::Changed { current_hash } => {
                Ok(approval::PortOutcome::ScreenChanged { current_hash })
            }
            approval::ScreenCheck::Match { vt_snapshot_hash } => {
                // Write com o guard do VT ainda vivo: o reader-loop espera o lock para
                // aplicar o próximo lote — check↔write sem `advance` intercalado.
                let mut writer = lock(&term.writer);
                writer
                    .write_all(keys)
                    .map_err(|e| PtyHostError::Io(node, e.to_string()))?;
                writer
                    .flush()
                    .map_err(|e| PtyHostError::Io(node, e.to_string()))?;
                Ok(approval::PortOutcome::Written { vt_snapshot_hash })
            }
        }
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

/// F2-0-6 (costura do teto do synchronized output — spec §4 do RESULTADO): se há um batch
/// retido cujo deadline venceu, libera-o (apresentação atômica) e emite o `GridDelta`.
/// Devolve o PRÓXIMO deadline a observar (`Some` = sync ainda pendente; `None` = nenhum).
/// `bytes: 0` é intencional: o batch já foi contado no `inflight` quando entrou via
/// `advance` — o teto não lê bytes novos do PTY.
fn service_sync_ceiling(
    shared: &TermShared,
    delta_tx: &Sender<GridDelta>,
    node: NodeId,
    seq: &AtomicU64,
) -> Option<Instant> {
    let (next_deadline, flushed, rows, scrolled) = {
        let mut vt = lock(&shared.vt);
        match vt.sync_deadline() {
            None => return None,
            Some(dl) if Instant::now() < dl => return Some(dl),
            Some(_) => {
                let flushed = vt.flush_sync();
                let rows = vt.damaged_rows();
                vt.reset_damage();
                let scrolled = vt.take_scrollback();
                (vt.sync_deadline(), flushed, rows, scrolled)
            }
        }
    };
    // Persiste o scrollback colhido FORA do lock do vt (mesmo padrão do `flush` — não aninha
    // vt↔store; em erro de disco a linha fica no cache da cauda e o próximo push re-tenta).
    if let Some(sink) = &shared.scrollback {
        if !scrolled.is_empty() {
            let mut store = lock(&sink.store);
            for line in scrolled {
                if let Err(e) = store.push_line(&sink.panel, line) {
                    tracing::warn!(panel = %sink.panel, error = %e,
                        "scrollback push (teto sync) falhou; linha fica no cache, re-tenta");
                }
            }
        }
    }
    if flushed {
        let s = seq.fetch_add(1, Ordering::Relaxed);
        let _ = delta_tx.send(GridDelta {
            node,
            rows,
            bytes: 0,
            seq: s,
        });
    }
    next_deadline
}

/// F2-0-6: vigia do teto do synchronized output — DECISÃO DE COSTURA (desvio documentado da
/// spec §4b): em vez de des-bloquear o `read` do PTY (exigiria plumbar o fd cru por 3 crates),
/// uma thread-irmã por terminal serve o teto. O leitor fica INTOCADO (zero risco ao hot path).
/// Cadência: dorme até o deadline exato quando há sync pendente; cochilo grosso (100ms) quando
/// não há. Custo: 1 lock + 1 match por wake. Consequência honesta: um sync aberto pode ser
/// detectado até ~100ms tarde → teto efetivo ≤ ~250ms no pior caso — aceitável: o teto é
/// anti-travamento (CLI que esqueceu o ESU), não precisão de apresentação.
fn run_sync_watchdog(
    shared: Arc<TermShared>,
    delta_tx: Sender<GridDelta>,
    node: NodeId,
    seq: Arc<AtomicU64>,
) {
    // Timeout de SEGURANÇA do wait (não é cadência): só para re-checar stop/estado de vez em
    // quando — um kill com a vigia parada nunca vaza a thread além disso.
    const STOP_RECHECK: Duration = Duration::from_secs(5);
    loop {
        // Fase 1 — OCIOSA DE VERDADE: bloqueia na condvar até o flush sinalizar "sync abriu"
        // (zero wakeup enquanto nada acontece — invariante de QA da F2-0-6).
        {
            let (m, cv) = &shared.sync_wake;
            let mut pending = lock(m);
            loop {
                if shared.stop.load(Ordering::Relaxed)
                    || !matches!(*lock(&shared.state), TerminalState::Running)
                {
                    return;
                }
                if *pending {
                    *pending = false;
                    break;
                }
                pending = cv
                    .wait_timeout(pending, STOP_RECHECK)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
        }
        // Fase 2 — há (ou houve) um sync pendente: serve até o deadline zerar.
        while let Some(dl) = service_sync_ceiling(&shared, &delta_tx, node, &seq) {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(
                dl.saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            );
        }
    }
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

    let (rows, scrolled, sync_opened) = {
        let mut vt = lock(&shared.vt);
        vt.advance(batch.as_slice());
        let dirty = vt.damaged_rows();
        vt.reset_damage();
        // W5-2 FIAÇÃO: drena as linhas que saíram do viewport (vazio se o backend não captura).
        // Sempre drena para não deixar o buffer interno crescer.
        let scrolled = vt.take_scrollback();
        // F2-0-6: o advance pode ter ABERTO um sync (BSU sem ESU neste batch) — acorda a vigia.
        (dirty, scrolled, vt.sync_deadline().is_some())
    };
    if sync_opened {
        let (m, cv) = &shared.sync_wake;
        *lock(m) = true;
        cv.notify_one();
    }
    batch.clear();

    // W5-2 FIAÇÃO: persiste o scrollback colhido FORA do lock do vt (não aninha vt↔store). Cada
    // linha é gravada uma vez; o Store cuida do cache da cauda + paginação em disco. Em erro de
    // disco a linha permanece no cache da cauda do Store e o próximo push re-tenta (nada some).
    if let Some(sink) = &shared.scrollback {
        if !scrolled.is_empty() {
            let mut store = lock(&sink.store);
            for line in scrolled {
                if let Err(e) = store.push_line(&sink.panel, line) {
                    tracing::warn!(
                        panel = %sink.panel,
                        error = %e,
                        "scrollback push falhou; linha fica no cache da cauda, re-tenta no próximo flush"
                    );
                }
            }
        }
    }

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
///
/// F1-0-3: os estados CANÔNICOS da state-machine de lifecycle (ADR 0019) são
/// `Ready/Busy/Idle/Blocked/Dead`; `Starting`/`Running` permanecem como estados legados de
/// boot/genérico (Fase 0) — a doutrina de transição vive em [`lifecycle::LifecycleEngine`].
/// ⚠️ Fiação pendente COORDENADA VIA MAESTRO: `app/lina-gpui/src/bridge.rs` mapeia este enum
/// exaustivamente (linha ~1155) e precisa do arm `CoreStatus::Ready => …` (sugestão: tratar
/// como `Idle` na UI até F1-1-5 exibir estados ricos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Starting,
    Running,
    /// F1-0-3: spawn concluído, pronto para receber trabalho, antes do 1º output.
    Ready,
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
            NodeStatus::Idle | NodeStatus::Running | NodeStatus::Starting | NodeStatus::Ready
        )
    }
    /// F1-0-3: nome CANÔNICO persistido no log (`NodeStatusChanged.status/.from`) e lido pela
    /// projeção — estável por contrato (mudar uma string quebra replay de logs existentes).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            NodeStatus::Starting => "Starting",
            NodeStatus::Running => "Running",
            NodeStatus::Ready => "Ready",
            NodeStatus::Idle => "Idle",
            NodeStatus::Busy => "Busy",
            NodeStatus::Blocked => "Blocked",
            NodeStatus::Dead => "Dead",
        }
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
    /// Enter como keystroke separado (`0x0D`) — o que de fato submete (W0-9). `delay_ms` é o
    /// espaçamento AgentText→Enter (do CLI Profile, `submit_delay`): o `serial_writer` (thread
    /// por-terminal) dorme ANTES de escrever o `0x0D`. Antes, esse `sleep` rodava na thread que
    /// ENFILEIRA — que segura o lock do `EventStore` (`MailboxPump::pump`) — e congelava a UI por
    /// entrega. Movê-lo para cá tira o long-hold sem perder a temporização. `0` = imediato.
    Submit { from: Option<NodeId>, delay_ms: u64 },
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
        /// F3-CONF-2: motivo da transição (ex.: `"circuit_breaker"`) — DADO p/ a tela honesta
        /// (#21), nunca autoridade. `None` na maioria das transições; `Some(..)` quando há causa
        /// surfacável ao leigo. O canal vivo (Bus) é o que o render usa, então o reason viaja aqui.
        reason: Option<String>,
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
    /// F1-1-8 (ADR 0024): erro de I/O no write atômico da aprovação ([`Supervisor::deliver_approval_with_grid`]).
    /// O efeito NÃO foi confirmado — o executor audita como `ApprovalAborted{port_error}`, sem `Injected`.
    #[error("erro de I/O no write de aprovação do nó {0}: {1}")]
    WriteFailed(NodeId, String),
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

/// Grafo wait-for MULTI-ARESTA: `waiter -> {(ticket_id, target)}`. Um waiter pode estar bloqueado em
/// VÁRIOS `ask --await` ao mesmo tempo (single-targets concorrentes); cada aresta é uma pergunta
/// pendente, chaveada pelo `id` do ticket para que fechar UMA não derrube as outras (Round 5 #6/#7).
/// A checagem de deadlock considera TODAS as arestas (alcançabilidade no grafo).
#[derive(Default)]
struct WaitForGraph {
    awaiting: HashMap<NodeId, HashSet<(String, NodeId)>>,
}

impl WaitForGraph {
    /// `true` se `from` alcança `goal` seguindo as arestas de await (DFS com `visited`). O grafo é
    /// mantido acíclico por [`WaitForGraph::add`]; o `visited` garante terminação defensivamente.
    fn reaches(&self, from: NodeId, goal: NodeId) -> bool {
        let mut stack = vec![from];
        let mut visited = HashSet::new();
        while let Some(n) = stack.pop() {
            if n == goal {
                return true;
            }
            if !visited.insert(n) {
                continue;
            }
            if let Some(edges) = self.awaiting.get(&n) {
                for (_, t) in edges {
                    stack.push(*t);
                }
            }
        }
        false
    }

    /// `true` se abrir `waiter -> target` fecharia um ciclo (`target` já alcança `waiter`). PURO — não
    /// muta (Round 5 #6: o broadcast-await usa isto p/ não apagar arestas vivas).
    fn would_cycle(&self, waiter: NodeId, target: NodeId) -> bool {
        self.reaches(target, waiter)
    }

    /// Registra a aresta `waiter --(id)--> target`. `Err` se fecharia um ciclo (NÃO insere).
    /// Multi-aresta: NÃO sobrescreve as outras arestas do `waiter` (Round 5 #7).
    fn add(&mut self, waiter: NodeId, id: String, target: NodeId) -> Result<(), ()> {
        if self.would_cycle(waiter, target) {
            return Err(());
        }
        self.awaiting
            .entry(waiter)
            .or_default()
            .insert((id, target));
        Ok(())
    }

    /// Remove SÓ a aresta `(id, target)` de `waiter` (fechamento de UM ticket — Round 5 #7). Poda o
    /// waiter se ficou sem arestas.
    fn remove_edge(&mut self, waiter: NodeId, id: &str, target: NodeId) {
        if let Some(edges) = self.awaiting.get_mut(&waiter) {
            edges.retain(|(eid, t)| !(eid == id && *t == target));
            if edges.is_empty() {
                self.awaiting.remove(&waiter);
            }
        }
    }

    /// Remove TODAS as arestas de `waiter` (conveniência — fecha o await do nó como um todo).
    fn remove_all(&mut self, waiter: NodeId) {
        self.awaiting.remove(&waiter);
    }

    /// Remove `node` do grafo por completo: como waiter (suas arestas) E como alvo (arestas que
    /// apontam para ele) — não trava quem o esperava. Usado em `mark_dead`/`unregister`.
    fn drop_node(&mut self, node: NodeId) {
        self.awaiting.remove(&node);
        for edges in self.awaiting.values_mut() {
            edges.retain(|(_, t)| *t != node);
        }
        self.awaiting.retain(|_, edges| !edges.is_empty());
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
    /// F1-1-8 (ADR 0024): o writer do master, COMPARTILHADO entre a thread `serial_writer`
    /// (fila A2A/humano) e a porta atômica de aprovação
    /// ([`Supervisor::deliver_approval_with_grid`]). O `Mutex` serializa cada op — a ordem
    /// dos bytes A2A é preservada — e coexiste com a porta de aprovação, que tranca o MESMO
    /// writer SÓ depois de travar o grid (ordem de lock SEMPRE grid→writer, sem ciclo).
    sync_writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

/// Consumidor SERIAL da fila de escrita de um terminal: drena `rx` e aplica cada op ao
/// writer do master, uma de cada vez — bytes nunca interleavam. ADR 0024: o writer é
/// COMPARTILHADO (`Arc<Mutex<…>>`); trava-se o `Mutex` por op (escopo curto: write+flush),
/// o que preserva a ordem serial E coexiste com a porta atômica de aprovação (que tranca o
/// MESMO writer — a serialização é do `Mutex`, não da posse exclusiva).
fn serial_writer(
    rx: Receiver<WriteOp>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    applied: Arc<Mutex<Vec<WriteOp>>>,
    cap: usize,
) {
    while let Ok(op) = rx.recv() {
        // O espaçamento AgentText→Enter (`submit_delay` do CLI Profile) mora AQUI, na thread de
        // escrita — NUNCA na thread que enfileira (`deliver_a2a` roda sob o lock do `EventStore`;
        // bloquear lá congela a UI). A FIFO garante o par AgentText→Submit contíguo; o atraso só
        // adia o `0x0D` DESTE terminal, sem segurar lock algum compartilhado.
        if let WriteOp::Submit { delay_ms, .. } = &op {
            if *delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(*delay_ms));
            }
        }
        let bytes = op.bytes();
        {
            let mut w = lock(&writer);
            // Erro de escrita não derruba o supervisor — o terminal pode ter saído.
            if w.write_all(&bytes).is_err() {
                break;
            }
            let _ = w.flush();
        }
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

        // ADR 0024: o writer do master é COMPARTILHADO entre o consumidor serial (fila A2A/
        // humano) e a porta atômica de aprovação — um `Arc<Mutex<…>>` único; o consumidor
        // recebe um clone do MESMO handle.
        let sync_writer = Arc::new(Mutex::new(writer));
        let consumer = {
            let applied = Arc::clone(&applied);
            let writer = Arc::clone(&sync_writer);
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
                sync_writer,
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
        // Round 5 #7: limpa o nó do grafo multi-aresta — como waiter E como alvo (não trava quem
        // o esperava).
        lock(&self.waitfor).drop_node(node);
        self.publish(BusEvent::NodeDied { node });
        Ok(())
    }

    /// Remove um nó por completo (roster + fila). Publica `NodeDied`.
    pub fn unregister(&self, node: NodeId) -> Result<(), SupervisorError> {
        let existed = lock(&self.registry).remove(&node).is_some();
        lock(&self.order).retain(|n| *n != node);
        self.stop_terminal(node);
        // Round 5 #7: limpa o nó do grafo multi-aresta — como waiter E como alvo (não trava quem
        // o esperava).
        lock(&self.waitfor).drop_node(node);
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

    /// Atualiza o status de um nó e publica `NodeStatus` (sem reason surfacável).
    pub fn set_status(&self, node: NodeId, status: NodeStatus) -> Result<(), SupervisorError> {
        self.set_status_with_reason(node, status, None)
    }

    /// F3-CONF-2: atualiza o status e publica `NodeStatus` carregando um `reason` opcional
    /// para a tela honesta (#21 — ex.: `"circuit_breaker"`). O reason é DADO transportado ao
    /// render, nunca autoridade; quem decide o status é sempre o core, server-side.
    pub fn set_status_with_reason(
        &self,
        node: NodeId,
        status: NodeStatus,
        reason: Option<String>,
    ) -> Result<(), SupervisorError> {
        {
            let mut reg = lock(&self.registry);
            let info = reg
                .get_mut(&node)
                .ok_or(SupervisorError::NodeNotFound(node))?;
            info.status = status;
        }
        self.publish(BusEvent::NodeStatus {
            node,
            status,
            reason,
        });
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

    /// Roster completo, em ordem de registro (determinístico). Inclui nós de SISTEMA (role reservado,
    /// ex.: o sombra do Refletor) — é a fonte do GERENCIAMENTO (PTY/grid/unload). Para a lista de
    /// TIME (que o usuário vê / `agents.json` / `lina list`), use [`Supervisor::team_roster`].
    #[must_use]
    pub fn list(&self) -> Vec<NodeInfo> {
        let reg = lock(&self.registry);
        lock(&self.order)
            .iter()
            .filter_map(|id| reg.get(id).cloned())
            .collect()
    }

    /// **Roster de TIME (ADR 0047 add.1).** Como [`Supervisor::list`], mas OMITE nós com role
    /// reservado (`__*__`) — a projeção de presença das superfícies de TIME (`lina list`/`agents.json`).
    /// O sombra do Refletor segue gerenciável por `list` e auditável pelo `NodeAdded` no log (inv #4);
    /// só some das LISTAS que o time vê. O app escreve o `agents.json` a partir DESTE roster.
    #[must_use]
    pub fn team_roster(&self) -> Vec<NodeInfo> {
        self.list().into_iter().collect()
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
                        // Role reservado (`__*__`, ADR 0047 add.1: o sombra) NUNCA entra num fan-out
                        // de time — o sistema o endereça SÓ por entrega direta (`dispatch_reflection`).
                        Some(*id) != exclude
                            && reg.get(id).is_some_and(|n| {
                                n.status.is_alive()
                                    && !n
                                        .role
                                        .as_deref()
                                        .is_some_and(crate::router::is_reserved_role)
                            })
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
        // Caminho SIMPLES (sem espaçamento): Enter imediato. A entrega A2A faseada
        // (`deliver_a2a`) é quem carrega o `submit_delay` no `Submit`.
        self.enqueue_write(
            target,
            WriteOp::Submit {
                from: Some(from),
                delay_ms: 0,
            },
        )
    }

    /// F1-1-8 / ADR 0024 — a **porta atômica de aprovação** do app (porta única, Captura 2).
    /// Recebe o `grid` do app por referência (`&Mutex<Box<dyn VtBackend>>` — tipo de `lina-vt`,
    /// que já é dep do core; NENHUM tipo de toolkit/gpui entra aqui): trava o grid (o MESMO lock
    /// que o reader-loop do app segura antes de `advance` → bloqueia o advance), re-snapshota a
    /// região do prompt ([`approval::check_screen`]) e, **só em `Match`**, trava o `sync_writer`
    /// (compartilhado com a fila serial) e escreve `keys` — tudo com o grid-lock vivo. Tela
    /// divergente ⇒ NENHUM byte. `region_rows` = o `K` da Captura 1.
    ///
    /// **Invariante anti-deadlock (ADR 0024):** a ordem de locks é SEMPRE grid→writer. O reader
    /// segura só o grid; o `serial_writer` segura só o writer; este método segura grid e depois
    /// writer. Sem ciclo.
    pub fn deliver_approval_with_grid(
        &self,
        node: NodeId,
        grid: &Mutex<Box<dyn VtBackend>>,
        expected_hash: &str,
        keys: &[u8],
        region_rows: usize,
    ) -> Result<approval::PortOutcome, SupervisorError> {
        // Handle do writer compartilhado (lock breve do mapa, solto antes do grid).
        let sync_writer = lock(&self.terminals)
            .get(&node)
            .map(|t| Arc::clone(&t.sync_writer))
            .ok_or(SupervisorError::NodeNotFound(node))?;

        // Trava o grid (o MESMO lock que o reader-loop do app segura antes de `advance`): nenhum
        // byte do PTY é aplicado ao grid entre o check e o write — atomicidade local. O guard
        // permanece VIVO por todo o `match` (inclusive durante write+flush).
        let vt = lock(grid);
        match approval::check_screen(&**vt, expected_hash, region_rows) {
            approval::ScreenCheck::Changed { current_hash } => {
                Ok(approval::PortOutcome::ScreenChanged { current_hash })
            }
            approval::ScreenCheck::Match { vt_snapshot_hash } => {
                // Ordem grid→writer: o grid-lock segue seguro enquanto travamos o writer e
                // escrevemos — o reader-loop espera o grid para o próximo lote (sem advance
                // intercalado). Output do filho ainda no buffer do kernel é a janela irredutível
                // documentada (ADR 0021 §4 R2).
                let mut w = lock(&sync_writer);
                w.write_all(keys)
                    .map_err(|e| SupervisorError::WriteFailed(node, e.to_string()))?;
                w.flush()
                    .map_err(|e| SupervisorError::WriteFailed(node, e.to_string()))?;
                Ok(approval::PortOutcome::Written { vt_snapshot_hash })
            }
        }
    }

    /// Log das ops já aplicadas no terminal (observabilidade/teste).
    #[must_use]
    pub fn applied_ops(&self, node: NodeId) -> Vec<WriteOp> {
        lock(&self.terminals)
            .get(&node)
            .map(|t| lock(&t.applied).clone())
            .unwrap_or_default()
    }

    /// Registra que `waiter` espera o `reply` de `target` (ask bloqueante) — CONVENIÊNCIA (1 aresta
    /// por alvo, chaveada pelo próprio alvo). Retorna `CycleDetected` na hora (sem travar) se fecharia
    /// um ciclo. Para o lifecycle por-ticket do router (multi-aresta), use [`Supervisor::begin_await_ticket`].
    ///
    /// DISCIPLINA: pareie SEMPRE com [`Supervisor::end_await`] (fecha por waiter). NÃO misture com a
    /// API de ticket no MESMO `(waiter, target)` — as chaves de aresta diferem (alvo vs `id`), e fechar
    /// por uma não removeria a aresta aberta pela outra. (O router de produção usa SÓ a API de ticket;
    /// esta conveniência serve a testes/probes de deadlock.)
    pub fn begin_await(&self, waiter: NodeId, target: NodeId) -> Result<(), SupervisorError> {
        self.begin_await_ticket(waiter, &target.to_string(), target)
    }

    /// **Round 5 #7: abre a aresta de await chaveada pelo `id` do ticket** — multi-aresta por waiter
    /// (dois `ask --await` concorrentes coexistem; abrir o 2º NÃO apaga o 1º). `CycleDetected` (sem
    /// travar) se fecharia um ciclo no wait-for-graph.
    pub fn begin_await_ticket(
        &self,
        waiter: NodeId,
        id: &str,
        target: NodeId,
    ) -> Result<(), SupervisorError> {
        lock(&self.waitfor)
            .add(waiter, id.to_string(), target)
            .map_err(|()| SupervisorError::CycleDetected { waiter, target })
    }

    /// **Round 5 #6: checagem PURA de deadlock** — `true` se abrir `waiter --await--> target` fecharia
    /// um ciclo. NÃO muta o grafo (o broadcast-await usa isto p/ não apagar a aresta viva do sender).
    #[must_use]
    pub fn would_await_deadlock(&self, waiter: NodeId, target: NodeId) -> bool {
        lock(&self.waitfor).would_cycle(waiter, target)
    }

    /// Encerra TODOS os awaits de `waiter` — CONVENIÊNCIA (chegou o reply ou deu timeout). Para fechar
    /// SÓ um ticket (multi-aresta), use [`Supervisor::end_await_ticket`].
    pub fn end_await(&self, waiter: NodeId) {
        lock(&self.waitfor).remove_all(waiter);
    }

    /// **Round 5 #7: fecha SÓ a aresta `(id, target)`** de `waiter` — fechar um ticket não libera os
    /// outros awaits abertos do mesmo waiter.
    pub fn end_await_ticket(&self, waiter: NodeId, id: &str, target: NodeId) {
        lock(&self.waitfor).remove_edge(waiter, id, target);
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
                    Some(WriteOp::Submit { from: Some(s), .. }) => {
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

    /// **G2 (ADR 0047 add.1):** o sombra (role reservado) existe no roster de GERENCIAMENTO (`list` —
    /// o app precisa dele p/ o PTY) mas SOME do roster de TIME (`team_roster`, fonte do `agents.json`/
    /// `lina list`). A invisibilidade é só na LISTA; o `NodeAdded` segue no log (auditoria, inv #4).
    #[test]
    fn team_roster_hides_reserved_role_but_list_keeps_it() {
        let sup = Supervisor::new();
        sup.register("@Dev", Some("backend".into()), sink());
        let shadow = sup.register("@Sombra", Some(REFLECTOR_ROLE.into()), sink());

        assert!(
            sup.list().iter().any(|n| n.id == shadow),
            "list (gerenciamento) INCLUI o sombra"
        );
        assert!(
            sup.team_roster().iter().all(|n| n.id != shadow),
            "team_roster (lista do time) OMITE o sombra"
        );
        assert!(
            sup.team_roster()
                .iter()
                .any(|n| n.role.as_deref() == Some("backend")),
            "o nó normal permanece visível no time"
        );
    }

    /// **G2 (ADR 0047 add.1):** um broadcast de time NÃO entrega ao sombra (role reservado) — o
    /// sistema o endereça SÓ por entrega direta (`dispatch_reflection`). O nó normal recebe.
    #[test]
    fn broadcast_excludes_reserved_role() {
        let sup = Supervisor::new();
        let dev = sup.register("@Dev", Some("backend".into()), sink());
        let shadow = sup.register("@Sombra", Some(REFLECTOR_ROLE.into()), sink());

        let targets = sup.resolve(&Recipient::Broadcast, RolePolicy::All, None);
        assert!(targets.contains(&dev), "o nó de time recebe o broadcast");
        assert!(
            !targets.contains(&shadow),
            "o sombra (role reservado) é omitido do fan-out"
        );
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
                |e| matches!(e, BusEvent::NodeStatus { node, status: NodeStatus::Busy, .. } if *node == n)
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

    // ───────── F1-1-8 / ADR 0024 · porta atômica `deliver_approval_with_grid` ─────────

    use crate::approval::{PortOutcome, PROMPT_REGION_ROWS};
    const KK: usize = PROMPT_REGION_ROWS;

    /// Grid vivo do app (o MESMO tipo do `wire_terminal`): `Arc<Mutex<Box<dyn VtBackend>>>`.
    fn grid_with(bytes: &[u8]) -> Arc<Mutex<Box<dyn VtBackend>>> {
        let mut vt = AlacrittyBackend::new(80, 24);
        vt.advance(bytes);
        Arc::new(Mutex::new(Box::new(vt) as Box<dyn VtBackend>))
    }

    fn hash_of(grid: &Arc<Mutex<Box<dyn VtBackend>>>) -> String {
        let g = lock(grid);
        crate::approval::prompt_snapshot_hash(&**g, KK)
    }

    /// Sink observável (escreve num `Vec` compartilhado) — o "log de writes do master".
    struct VecSink(Arc<Mutex<Vec<u8>>>);
    impl Write for VecSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Sink que BLOQUEIA dentro do `write` até o teste liberar — simula um write lento/
    /// backpressured no master. Enquanto bloqueia, o grid-lock TEM que continuar seguro.
    struct BlockingSink {
        entered: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
        buf: Arc<Mutex<Vec<u8>>>,
    }
    impl Write for BlockingSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.buf).extend_from_slice(buf);
            self.entered.store(true, Ordering::SeqCst);
            while !self.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// **ATOMICIDADE (RED se o grid-lock não cobrir o write):** enquanto `deliver_approval_with_grid`
    /// está DENTRO do write (sink bloqueado), o grid-lock está SEGURO — uma thread concorrente NÃO
    /// consegue travar o grid (logo não pode `advance`) entre o check e o write. Se o método
    /// soltasse o grid-lock antes do write, o `try_lock` abaixo teria sucesso e o teste falharia.
    #[test]
    #[serial]
    fn deliver_approval_with_grid_holds_grid_lock_across_the_write() {
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(BlockingSink {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            buf: Arc::clone(&buf),
        });

        let sup = Arc::new(Supervisor::new());
        let node = sup.register("@T", None, sink);
        let grid = grid_with(b"$ deploy\r\nDeploy to prod? (y/n) ");
        let h = hash_of(&grid);

        let sup2 = Arc::clone(&sup);
        let grid2 = Arc::clone(&grid);
        let h2 = h.clone();
        let t =
            thread::spawn(move || sup2.deliver_approval_with_grid(node, &grid2, &h2, b"y\r", KK));

        // O write foi alcançado (tela casou) → o método está DENTRO do write, segurando o grid.
        assert!(
            poll_until(T, || entered.load(Ordering::SeqCst)),
            "deliver_approval_with_grid deveria alcançar o write (tela casou)"
        );
        // A PROVA da atomicidade: o grid-lock está vivo durante o write.
        assert!(
            grid.try_lock().is_err(),
            "grid-lock DEVE permanecer seguro durante o write (sem advance entre check e write)"
        );
        release.store(true, Ordering::SeqCst);

        let outcome = t.join().expect("join").expect("deliver ok");
        assert!(
            matches!(outcome, PortOutcome::Written { .. }),
            "tela casou → escreve: {outcome:?}"
        );
        assert_eq!(*lock(&buf), b"y\r".to_vec(), "exatamente as approval_keys");
    }

    /// Tela inalterada (hash casa) ⇒ escreve EXATAMENTE `keys` e devolve `Written`.
    #[test]
    #[serial]
    fn deliver_approval_with_grid_writes_keys_when_screen_matches() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sup = Supervisor::new();
        let node = sup.register("@T", None, Box::new(VecSink(Arc::clone(&buf))));
        let grid = grid_with(b"Continue? (y/n) ");
        let h = hash_of(&grid);

        let out = sup
            .deliver_approval_with_grid(node, &grid, &h, b"y\r", KK)
            .expect("deliver ok");
        assert!(matches!(out, PortOutcome::Written { .. }));
        // O write síncrono é direto no sync_writer — sem passar pela fila; o byte chega.
        assert!(
            poll_until(T, || *lock(&buf) == b"y\r".to_vec()),
            "o master recebe as approval_keys"
        );
    }

    /// Tela DIVERGIU entre a Captura 1 e a entrega (hash velho) ⇒ `ScreenChanged` e ZERO bytes.
    #[test]
    #[serial]
    fn deliver_approval_with_grid_aborts_with_zero_bytes_when_screen_changed() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sup = Supervisor::new();
        let node = sup.register("@T", None, Box::new(VecSink(Arc::clone(&buf))));
        let grid = grid_with(b"Deploy to prod? (y/n) ");
        let stale = hash_of(&grid);
        // A tela muda ANTES da entrega (output novo do CLI).
        lock(&grid).advance(b"\r\nWARNING: lock changed\r\nDeploy to prod? (y/n) ");

        let out = sup
            .deliver_approval_with_grid(node, &grid, &stale, b"y\r", KK)
            .expect("deliver ok");
        assert!(
            matches!(out, PortOutcome::ScreenChanged { .. }),
            "tela mudada deve abortar: {out:?}"
        );
        // Dá tempo de um write indevido aparecer (não deve): zero bytes.
        thread::sleep(Duration::from_millis(30));
        assert!(
            lock(&buf).is_empty(),
            "ZERO bytes ao master quando a tela divergiu"
        );
    }

    /// Nó inexistente erra limpo (`NodeNotFound`), sem panic.
    #[test]
    #[serial]
    fn deliver_approval_with_grid_unknown_node_errors() {
        let sup = Supervisor::new();
        let grid = grid_with(b"Continue? (y/n) ");
        let h = hash_of(&grid);
        let ghost = Uuid::now_v7();
        assert!(matches!(
            sup.deliver_approval_with_grid(ghost, &grid, &h, b"y\r", KK),
            Err(SupervisorError::NodeNotFound(_))
        ));
    }

    /// **AC-0021.7 (mecanismo, via Supervisor real) — espelha o `pty_port_...` do `approval.rs`,
    /// mas pelo caminho do APP (`Supervisor` + grid do app, não `PtyHost`).** Um `sh` bloqueado
    /// num `read` após `printf '(y/n) '`; uma thread-leitora avança o grid (como o `wire_terminal`);
    /// (1) a tela muda (humano digita `x` pela FILA SERIAL `write_human`); (2) entrega com o hash
    /// VELHO ⇒ `ScreenChanged`, o `read` segue bloqueado; (3) entrega com o hash FRESCO ⇒ `Written`
    /// e o programa destrava com EXATAMENTE `xy` — se o abort tivesse vazado bytes, o `read` teria
    /// consumido outra coisa e o passo final falharia (prova positiva do zero-byte). Coexistência
    /// provada: `write_human` (fila) e `deliver_approval_with_grid` (síncrono) no MESMO writer.
    #[cfg(unix)] // F1-6-8: spawna `sh -c` num PTY real — runtime-Unix (tabela ci-3so-triagem.md).
    #[test]
    #[serial]
    fn deliver_approval_with_grid_unblocks_real_prompt_via_supervisor() {
        let mut mgr = PtyManager::new();
        let pty_key = lina_pty::NodeId::new("approval-seam-supervisor".to_string());
        mgr.spawn(
            pty_key.clone(),
            PtyCommand::new("sh")
                .arg("-c")
                .arg("printf 'Continue? (y/n) '; read x; printf 'GOT:%s\\n' \"$x\""),
            80,
            24,
        )
        .expect("spawn do prompt sintético");
        let writer = mgr.take_writer(pty_key.clone()).expect("writer do master");
        let mut reader = mgr.clone_reader(pty_key.clone()).expect("reader do master");

        let sup = Arc::new(Supervisor::new());
        let node = sup.register("@claude-real", None, writer);

        // Grid do app + thread-leitora que o avança (réplica fiel do `wire_terminal`).
        let grid: Arc<Mutex<Box<dyn VtBackend>>> =
            Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(80, 24))));
        let reader_grid = Arc::clone(&grid);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_r = Arc::clone(&stop);
        let jh = thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                if stop_r.load(Ordering::Relaxed) {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        lock(&reader_grid).advance(&buf[..n]);
                    }
                }
            }
        });

        assert!(
            poll_until(T, || lock(&grid).last_nonempty_line() == "Continue? (y/n)"),
            "o prompt deveria aparecer no grid"
        );
        let captura1 = hash_of(&grid);

        // A tela MUDA: o humano digita `x` pela FILA SERIAL do Supervisor (eco altera a região).
        sup.write_human(node, b"x", Duration::from_secs(2))
            .expect("write_human");
        assert!(
            poll_until(T, || lock(&grid).last_nonempty_line()
                == "Continue? (y/n) x"),
            "o eco do 'x' deveria mudar a tela"
        );

        // (2) Hash VELHO ⇒ ScreenChanged, nenhum byte.
        let stale = sup
            .deliver_approval_with_grid(node, &grid, &captura1, b"y\r", KK)
            .expect("porta responde");
        assert!(
            matches!(stale, PortOutcome::ScreenChanged { .. }),
            "tela mudada deve abortar: {stale:?}"
        );

        // (3) Hash FRESCO ⇒ Written; o read destrava com EXATAMENTE "xy".
        let captura2 = hash_of(&grid);
        let ok = sup
            .deliver_approval_with_grid(node, &grid, &captura2, b"y\r", KK)
            .expect("porta responde");
        assert!(
            matches!(ok, PortOutcome::Written { .. }),
            "tela válida escreve: {ok:?}"
        );
        assert!(
            poll_until(T, || lock(&grid).last_nonempty_line() == "GOT:xy"),
            "o programa deveria destravar com EXATAMENTE o input 'xy'"
        );

        stop.store(true, Ordering::Relaxed);
        let _ = mgr.kill(pty_key, Duration::from_secs(2));
        let _ = jh.join();
    }
}
// TODO(W0-5): Event Store (rusqlite WAL + JSONL + snapshots; replay determinístico).
// TODO(W0-6): recuperação pós-crash visível (integrity_check -> preservar -> rebuild).

#[cfg(test)]
mod tests {
    use super::*;
    // F1-6-8: os únicos `#[serial]` deste mod são os testes de PTY real (gateados) — unused no Windows.
    #[cfg(unix)]
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
    #[cfg(unix)] // F1-6-8: helper exclusivo do flood-test Unix abaixo (dead_code no Windows).
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

    #[cfg(unix)] // F1-6-8: helper exclusivo dos testes de PTY real Unix abaixo (dead_code no Windows).
    fn last_line(host: &PtyHost, node: NodeId) -> Option<String> {
        host.with_grid(node, |vt| vt.last_nonempty_line())
    }

    /// Critério de aceite (a): sob flood (`yes`) sem ack, o flow-control LIMITA os
    /// bytes não-consumidos a um teto e o reader PARA de drenar (backpressure); outro
    /// terminal segue sendo servido. Mede o COMPORTAMENTO do watermark por condição,
    /// não timing absoluto. `#[serial]`: não compete por CPU com o outro flood-test.
    #[cfg(unix)]
    // F1-6-8: spawna `yes`/`sh` em PTY real — hang histórico de ~70min no Windows (tabela ci-3so-triagem.md).
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
    #[cfg(unix)] // F1-6-8: spawna `yes`/`cat` em PTY real — runtime-Unix (tabela ci-3so-triagem.md).
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
    #[cfg(unix)] // F1-6-8: spawna `sh -c printf` em PTY real — runtime-Unix (tabela ci-3so-triagem.md).
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
