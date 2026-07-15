//! M8 fatia (i) — **runtime por-Espaço** (fundação do switch vivo F1-4-4).
//!
//! Extração PURA do boot de workspace do `main.rs` (passos [2]-[17] do mapa de boot) para um
//! construtor reutilizável: [`boot_ws_runtime`] monta TUDO que é por-Espaço ([`WsRuntime`]) a
//! partir do que é por-PROCESSO ([`SharedInfra`]). Com UM runtime o comportamento é idêntico ao
//! boot inline anterior; a fatia (ii) fará o processo hospedar N runtimes e o switch A↔B <1s com
//! PIDs preservados (decisão do fundador: Espaços de fundo VIVOS) — a tela só re-aponta.
//!
//! Doutrina (proposta runtime-por-Espaço + veredito do Arquiteto, 2026-06-11):
//! - **Isolamento por OBJETO DISTINTO** (F1-4-1): `Supervisor`/`MailboxPump`/store POR Espaço —
//!   não há tagging nem filtro; cruzar barramentos é impossível por construção.
//! - **Dreno por-runtime** (veredito §1, obrigatório nesta fatia): o `spawn_pump` consome
//!   `delta_rx`+`bus_rx` e alimenta `meter.record_output` (teto de custo) e `watch.note_output`
//!   (Busy/Idle R2b). Cada `WsRuntime` carrega o SEU pump VIVO — nada de pump global; um Espaço
//!   de fundo sem dreno seria memória sem teto (mpsc unbounded) + custo/idle cegos.
//! - **Handles vivos**: os pumps moram DENTRO do struct (`_pump`/`_mailbox_pump`/`_broker_pump`)
//!   e nunca são dropados enquanto o runtime existir — ciclo de vida explícito (inv do CLAUDE.md).

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lina_bootstrap::Autonomy;
use lina_cli_profiles::ProfileRegistry;
use lina_core::{CliProfile, EventStore, Mailbox, PtyManager, Supervisor};
use lina_host::NodeId;
// W3-6c (ADR 0004): cofre de segredos (demo: backend em memória `MockStore`).
use lina_secrets::{MockStore, SecretVault};

use crate::bridge::{
    self, load_injection_profile, lock, scrub_foreign_orchestrator_env, scrub_pty_secret_env,
    spawn_pump, ApprovalWiring, AttentionHub, BootstrapWriter, BrokerPump, CmdFactory, CoreInput,
    CustodyDesk, Desk, GpuiBridgeHost, Grid, HookDrain, HooksShared, MailboxPump, Model,
    NodeManager, Pump, SharedModel, WorkspaceHooks,
};
use crate::{agent_modal, dashboard, obsidian, persistence_ui, wiring};

/// Infra POR-PROCESSO: o que os N runtimes compartilham. `PtyManager` é dono dos processos do
/// SO (um só por app); a fábrica de shell e as dimensões do grid valem para todo Espaço. O tema
/// também é global, mas é APLICADO no `main` (antes de qualquer janela) — fica fora daqui.
#[derive(Clone)]
pub struct SharedInfra {
    pub pty: Arc<Mutex<PtyManager>>,
    pub cmd_factory: CmdFactory,
    pub cols: u16,
    pub rows: u16,
}

/// TUDO que é por-Espaço — o que a view consome hoje e o switch (fatia ii) vai re-apontar.
/// Os campos `_pump`/`_mailbox_pump`/`_broker_pump` são handles VIVOS: threads do Espaço que
/// continuam drenando/observando mesmo com o Espaço fora de cena (decisão "fundos VIVOS").
pub struct WsRuntime {
    pub ws_root: PathBuf,
    /// `<ws_root>/.lina` — mailbox A2A + event log + perfis, o "endereço" do Espaço.
    pub mailbox_dir: PathBuf,
    pub store: Arc<Mutex<EventStore>>,
    /// SEAM-3: bus escopado — UM `Supervisor` por Espaço; roster/presença/A2A nunca cruzam.
    pub sup: Arc<Supervisor>,
    /// SEAM-1: o funil ÚNICO de admissão de nós (⌘T/⌘N/seed/SpawnApproved — ADR 0022 §4).
    pub nodes: Arc<NodeManager>,
    pub model: Model,
    pub grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    pub input: Arc<CoreInput>,
    pub attention: Arc<AttentionHub>,
    pub desk: Desk,
    /// W4-3: freio por Espaço (pausa num Espaço não pausa outro — spec M8 §escopo).
    pub brake: wiring::Brake,
    pub autonomy: Autonomy,
    /// F1-0-2: o perfil de injeção REAL do boot (fallback dos alvos sem `profile_id`).
    pub injection_profile: CliProfile,
    pub profile_registry: Arc<ProfileRegistry>,
    /// Escopo deste workspace no listener global de hooks. Os tokens incluem o escopo,
    /// mas a timeline recebe o nome limpo depois do filtro.
    #[allow(dead_code)] // handle de POSSE (mantém o listener vivo) — como os `_pump`s.
    pub hooks: Option<WorkspaceHooks>,
    /// Assinatura filtrada da timeline; abortada ao desmontar este runtime.
    pub _hook_drain: Option<HookDrain>,
    /// F4-WA-1: a engine de webhook ativo do Espaço (handle de POSSE — mantém o runtime tokio +
    /// listener vivos). `None` = perfil sem webhook ou bind falhou. Como `hooks`, vive pelo runtime.
    #[allow(dead_code)]
    pub webhook: Option<Arc<bridge::WebhookShared>>,
    /// `Arc` (fatia ii): a view re-aponta para o dash do runtime ATIVO no switch — o wiring
    /// continua sendo alimentado pelas threads do runtime mesmo com o Espaço de fundo.
    pub dash: Arc<dashboard::DashWiring>,
    /// Dreno por-runtime (veredito §1): consome delta/bus e alimenta meter+watch — VIVO sempre.
    pub _pump: Pump,
    pub _mailbox_pump: Pump,
    pub _broker_pump: Pump,
}

/// F1-4-4 fatia (ii) — o processo hospeda N runtimes: raiz→runtime + qual está ATIVO.
/// Compartilhado entre a view (troca/leitura do ativo) e o epílogo do `main` (parada limpa).
pub struct RuntimeMap {
    pub map: BTreeMap<PathBuf, WsRuntime>,
    pub active: PathBuf,
    recent: VecDeque<PathBuf>,
}

pub type Runtimes = Arc<Mutex<RuntimeMap>>;

/// O processo medido com 13 runtimes ocupou todos os 256 descritores disponíveis. Quatro
/// runtimes deixam folga para GUI, stores transitórios e terminais novos; workspaces
/// adicionais continuam no catálogo e são remontados sob demanda.
const MAX_MOUNTED_WORKSPACES: usize = 4;
const FD_SAFETY_RESERVE: usize = 32;
const ESTIMATED_RUNTIME_FDS: usize = 16;
const UNLOAD_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(2);

fn workspace_switch_profile_enabled() -> bool {
    std::env::var("LINA_WORKSPACE_SWITCH_PROFILE").is_ok_and(|value| value.trim() == "1")
}

#[cfg(unix)]
fn nofile_limit() -> Option<usize> {
    let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: `getrlimit` inicializa o ponteiro quando retorna 0; lemos somente nesse caso.
    let status = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) };
    (status == 0).then(|| unsafe { limit.assume_init().rlim_cur as usize })
}

#[cfg(not(unix))]
fn nofile_limit() -> Option<usize> {
    None
}

fn open_fd_count() -> Option<usize> {
    std::fs::read_dir("/dev/fd")
        .ok()
        .map(|entries| entries.count())
}

/// Impede que uma operação nova seja iniciada na borda do `RLIMIT_NOFILE`.
/// O teto de runtimes mantém o caminho normal longe daqui; esta é a defesa para
/// um único workspace com muitos PTYs ou recursos externos.
pub(crate) fn ensure_fd_headroom(additional: usize) -> Result<(), String> {
    let Some(limit) = nofile_limit() else {
        return Ok(()); // plataforma sem getrlimit: o teto fixo segue valendo.
    };
    let Some(open) = open_fd_count() else {
        return Err(
            "o Lina não conseguiu reservar recursos do sistema; descarregue um Espaço ocioso e tente novamente"
                .to_string(),
        );
    };
    if open
        .saturating_add(additional)
        .saturating_add(FD_SAFETY_RESERVE)
        <= limit
    {
        Ok(())
    } else {
        Err(format!(
            "o Lina está usando {open} de {limit} recursos do sistema; descarregue um Espaço ocioso antes de abrir outro"
        ))
    }
}

fn terminal_unload_plan(rt: &WsRuntime) -> lina_core::suspend::UnloadPlan {
    runtime_unload_plan(&rt.sup.list())
}

fn runtime_unload_plan(roster: &[lina_core::NodeInfo]) -> lina_core::suspend::UnloadPlan {
    let mut plan = lina_core::suspend::UnloadPlan::default();
    for node in roster.iter().filter(|node| node.name != "@Trigger") {
        match node.status {
            // `Ready`/`Running` sem entrada recente estão parados no prompt. O seam
            // final ainda exige dois segundos de quietude e reserva atômica antes de
            // desligar; isso permite navegar além de quatro Espaços nunca usados.
            lina_core::NodeStatus::Idle
            | lina_core::NodeStatus::Ready
            | lina_core::NodeStatus::Running => plan.park.push(node.id),
            lina_core::NodeStatus::Dead => {}
            // `Starting` cobre admissão ainda sem key; Busy/Blocked são trabalho real.
            lina_core::NodeStatus::Starting
            | lina_core::NodeStatus::Busy
            | lina_core::NodeStatus::Blocked => plan.keep.push(node.id),
        }
    }
    plan
}

fn previous_generation_close_events(
    state: &lina_core::ProjectedState,
) -> Vec<lina_core::DomainEvent> {
    state
        .nodes
        .iter()
        .filter_map(|(node, projected)| {
            let status = projected.status.as_deref()?;
            (status != lina_core::NodeStatus::Dead.as_str()).then(|| {
                lina_core::DomainEvent::NodeStatusChanged {
                    node: *node,
                    status: lina_core::NodeStatus::Dead.as_str().to_string(),
                    from: status.to_string(),
                    reason: lina_core::lifecycle::reason::APP_REOPENED.to_string(),
                }
            })
        })
        .collect()
}

fn try_quiesce_for_unload(runtime: &WsRuntime) -> Option<bridge::RuntimeQuiescence> {
    let deadline = Instant::now() + UNLOAD_QUIESCENCE_TIMEOUT;
    let mut pumps_woken = false;
    loop {
        let observed = runtime.nodes.quiescence_generation();
        if let Some(quiescence) = runtime.nodes.try_begin_quiescence() {
            return quiescence.wait_until_idle(deadline).then_some(quiescence);
        }
        if !pumps_woken {
            runtime.wake_background_threads();
            pumps_woken = true;
        }
        if !runtime.nodes.wait_for_quiescence_change(observed, deadline) {
            return None;
        }
    }
}

impl WsRuntime {
    fn wake_background_threads(&self) {
        self._pump.wake();
        self._mailbox_pump.wake();
        self._broker_pump.wake();
    }

    fn stop_hook_drain(&mut self) -> Result<(), String> {
        if let Some(hook_drain) = self._hook_drain.as_mut() {
            hook_drain.stop_and_join()?;
        }
        Ok(())
    }

    fn stop_background_threads(&mut self) -> Result<(), String> {
        self.stop_hook_drain()?;
        self._pump.request_stop();
        self._mailbox_pump.request_stop();
        self._broker_pump.request_stop();
        self._pump.join();
        self._mailbox_pump.join();
        self._broker_pump.join();
        Ok(())
    }
}

impl RuntimeMap {
    fn note_focus(&mut self, root: &Path) {
        self.recent.retain(|known| known != root);
        self.recent.push_back(root.to_path_buf());
    }

    /// Libera o runtime menos recente que não possui terminal trabalhando ou aguardando
    /// resposta. O re-check dentro de `park_terminals` impede matar um turno que ficou
    /// ocupado entre o plano e a execução.
    fn evict_one_idle(&mut self) -> bool {
        let profile = workspace_switch_profile_enabled();
        let total_started = Instant::now();
        let candidates: Vec<PathBuf> = self
            .recent
            .iter()
            .filter(|root| *root != &self.active)
            .filter(|root| {
                self.map
                    .get(*root)
                    .is_some_and(|runtime| terminal_unload_plan(runtime).keep.is_empty())
            })
            .cloned()
            .collect();
        for root in candidates {
            let candidate_started = Instant::now();
            let Some(mut runtime) = self.map.remove(&root) else {
                continue;
            };
            let quiesce_started = Instant::now();
            let Some(quiescence) = try_quiesce_for_unload(&runtime) else {
                self.map.insert(root, runtime);
                continue;
            };
            let quiesce_us = quiesce_started.elapsed().as_micros();
            let plan = terminal_unload_plan(&runtime);
            let park_started = Instant::now();
            if let Err(error) = runtime.nodes.park_terminals(&plan.park) {
                eprintln!(
                    "lina-gpui: [WS] descarregamento de {} abortado: {error}",
                    root.display()
                );
                self.map.insert(root, runtime);
                continue;
            }
            let park_us = park_started.elapsed().as_micros();
            let cleanup_started = Instant::now();
            runtime.nodes.join_cleanup_tasks();
            let cleanup_us = cleanup_started.elapsed().as_micros();

            // Só desmonta depois de observar que NENHUM PTY continua sob posse do runtime.
            // Um candidato ainda na janela de quietude volta à mesma posição do LRU;
            // tentamos os seguintes em vez de bloquear a navegação prematuramente.
            if !runtime.nodes.terminal_ids().is_empty() {
                self.map.insert(root, runtime);
                continue;
            }

            let stop_started = Instant::now();
            if let Err(error) = runtime.stop_hook_drain() {
                eprintln!(
                    "lina-gpui: [WS] descarregamento de {} adiado: {error}",
                    root.display()
                );
                self.map.insert(root, runtime);
                continue;
            }
            if let Err(error) = runtime.stop_background_threads() {
                eprintln!(
                    "lina-gpui: [WS] descarregamento de {} adiado: {error}",
                    root.display()
                );
                self.map.insert(root, runtime);
                continue;
            }
            let stop_us = stop_started.elapsed().as_micros();
            quiescence.seal();
            self.recent.retain(|known| known != &root);
            let drop_started = Instant::now();
            drop(runtime);
            let drop_us = drop_started.elapsed().as_micros();
            eprintln!(
                "lina-gpui: [WS] runtime ocioso de {} desmontado para liberar recursos",
                root.display()
            );
            if profile {
                eprintln!(
                    "[WS-PROFILE evict] root={} total_us={} candidate_us={} quiesce_us={} park_us={} cleanup_us={} stop_us={} drop_us={}",
                    root.display(),
                    total_started.elapsed().as_micros(),
                    candidate_started.elapsed().as_micros(),
                    quiesce_us,
                    park_us,
                    cleanup_us,
                    stop_us,
                    drop_us
                );
            }
            return true;
        }
        false
    }

    fn reserve_runtime_slot(&mut self) -> Result<(), String> {
        loop {
            let count_full = self.map.len() >= MAX_MOUNTED_WORKSPACES;
            let pressure = ensure_fd_headroom(ESTIMATED_RUNTIME_FDS).err();
            if !count_full && pressure.is_none() {
                return Ok(());
            }
            if !self.evict_one_idle() {
                return Err(pressure.unwrap_or_else(|| {
                    format!(
                        "há {MAX_MOUNTED_WORKSPACES} Espaços com trabalho ativo; aguarde um deles terminar antes de abrir outro"
                    )
                }));
            }
            // O kill do PTY estacionado é assíncrono para não congelar a UI; dá
            // uma janela curta para os descritores fecharem antes de medir de novo.
            thread::sleep(Duration::from_millis(25));
        }
    }
}

fn validate_registry_target(
    registry: &lina_core::WorkspaceRegistry,
    target: &lina_core::WorkspaceEntry,
) -> Result<(), String> {
    let pointers: Vec<_> = registry
        .entries()
        .iter()
        .filter(|entry| entry.path == target.path)
        .collect();
    match pointers.as_slice() {
        [] => Ok(()),
        [pointer] if pointer.id == target.id && pointer.archived == target.archived => Ok(()),
        [pointer] => Err(format!(
            "a identidade/estado do Espaço não confere (store={} archived={}, registry={} archived={})",
            target.id, target.archived, pointer.id, pointer.archived
        )),
        _ => Err(format!(
            "o catálogo contém {} ponteiros para o mesmo caminho {}",
            pointers.len(),
            target.path.display()
        )),
    }
}

struct VerifiedWorkspaceTarget {
    registry: lina_core::WorkspaceRegistry,
}

fn verify_workspace_target(target_root: &Path) -> Result<VerifiedWorkspaceTarget, String> {
    let profile = workspace_switch_profile_enabled();
    let total_started = Instant::now();
    let events_dir = lina_core::Workspace::events_dir(target_root);
    if !events_dir.join("lina.db").exists() && !events_dir.join("log.jsonl").exists() {
        return Err("a pasta escolhida não contém dados de um Espaço".to_string());
    }
    let registry_path = lina_core::default_registry_path()
        .ok_or_else(|| "HOME/USERPROFILE indisponível para o catálogo de Espaços".to_string())?;
    let workspaces_base = target_root.parent().ok_or_else(|| {
        format!(
            "{} não tem diretório-base para validar o catálogo",
            target_root.display()
        )
    })?;
    let catalog_started = Instant::now();
    let catalog = persistence_ui::load_or_rebuild_workspace_registry(
        &registry_path,
        workspaces_base,
        &events_dir,
    )
    .map_err(|error| format!("não consegui validar a lista de Espaços ({error})"))?;
    let catalog_us = catalog_started.elapsed().as_micros();
    if catalog.quarantined_roots.contains(target_root) {
        return Err(format!(
            "o catálogo isolou {} porque sua identidade está ambígua",
            target_root.display()
        ));
    }
    let matching_entries = catalog
        .verified_entries
        .iter()
        .filter(|entry| entry.path == target_root)
        .cloned()
        .collect::<Vec<_>>();
    if matching_entries.len() > 1 {
        return Err(format!(
            "o catálogo contém mais de uma identidade verificada para {}",
            target_root.display()
        ));
    }
    let target = match matching_entries.into_iter().next() {
        Some(target) => target,
        None => {
            // Um erro transitório em OUTRA raiz obriga o catálogo a preservar o
            // snapshot last-good integral. Um alvo explícito novo ainda pode ser
            // aceito pela mesma projeção desta rodada, sem publicar o scan parcial
            // na sidebar nem reabrir/rederivar seu store.
            let projected = catalog
                .scan
                .projected_states
                .get(target_root)
                .ok_or_else(|| {
                    let issues = catalog.scan.issue_summary();
                    if issues.is_empty() {
                        format!(
                            "{} não aparece entre os Espaços completos e projetados",
                            target_root.display()
                        )
                    } else {
                        format!(
                            "não foi possível recuperar {} como Espaço completo nesta varredura: {issues}",
                            target_root.display()
                        )
                    }
                })?;
            let mut scanned = catalog.scan.entries.iter().filter(|entry| {
                crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir) == target_root
            });
            let scanned_target = scanned.next().ok_or_else(|| {
                format!(
                    "{} não aparece na varredura verificada desta rodada",
                    target_root.display()
                )
            })?;
            if scanned.next().is_some() {
                return Err(format!(
                    "a varredura contém mais de uma identidade para {}",
                    target_root.display()
                ));
            }
            if projected.workspace_id.as_deref() != Some(scanned_target.workspace_id.as_str())
                || projected.workspace_name.as_deref() != Some(scanned_target.name.as_str())
                || projected.archived != scanned_target.archived
            {
                return Err(format!(
                    "a projeção e a entrada verificada de {} não conferem",
                    target_root.display()
                ));
            }
            lina_core::WorkspaceEntry {
                id: scanned_target.workspace_id.clone(),
                name: scanned_target.name.clone(),
                path: target_root.to_path_buf(),
                last_focus: 0,
                archived: scanned_target.archived,
            }
        }
    };
    if target.archived {
        return Err("este Espaço está arquivado; restaure-o antes de abrir".to_string());
    }
    validate_registry_target(&catalog.registry, &target)?;
    if profile {
        eprintln!(
            "[WS-PROFILE verify] target={} total_us={} catalog_us={}",
            target_root.display(),
            total_started.elapsed().as_micros(),
            catalog_us
        );
    }
    Ok(VerifiedWorkspaceTarget {
        registry: catalog.registry,
    })
}

/// Confirma o foco de um runtime que JÁ terminou todo o boot e a religação. Antes do
/// evento, qualquer falha devolve `Err` e nada muda. Depois do append correlacionado
/// (commit point), falha do ponteiro vira reconciliação pendente e o runtime é publicado;
/// nunca reportamos um commit real como se a troca pudesse ser repetida.
pub fn register_ready_runtime(runtime: &WsRuntime) -> Result<String, String> {
    let outcome = register_ready_runtime_with_registry(runtime, None)?;
    if let Some(detail) = outcome.pending_detail() {
        eprintln!(
            "lina-gpui: [WS] foco em {} confirmado; catálogo será reconciliado ({detail})",
            runtime.ws_root.display()
        );
    }
    Ok(outcome.workspace_id().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FocusRegistration {
    Committed {
        workspace_id: String,
    },
    CommittedPending {
        workspace_id: String,
        detail: String,
    },
}

impl FocusRegistration {
    fn workspace_id(&self) -> &str {
        match self {
            Self::Committed { workspace_id } | Self::CommittedPending { workspace_id, .. } => {
                workspace_id
            }
        }
    }

    fn pending_detail(&self) -> Option<&str> {
        match self {
            Self::Committed { .. } => None,
            Self::CommittedPending { detail, .. } => Some(detail),
        }
    }
}

fn register_ready_runtime_with_registry(
    runtime: &WsRuntime,
    verified_registry: Option<lina_core::WorkspaceRegistry>,
) -> Result<FocusRegistration, String> {
    let profile = workspace_switch_profile_enabled();
    let total_started = Instant::now();
    let registry_path = lina_core::default_registry_path()
        .ok_or_else(|| "HOME/USERPROFILE indisponível para o catálogo de Espaços".to_string())?;
    let workspaces_base = runtime.ws_root.parent().ok_or_else(|| {
        format!(
            "{} não tem um diretório-base para reconciliar o catálogo",
            runtime.ws_root.display()
        )
    })?;
    let events_dir = lina_core::Workspace::events_dir(&runtime.ws_root);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("o relógio do sistema não permite registrar o foco ({error})"))?
        .as_millis() as u64;

    let project_started = Instant::now();
    let entry = {
        let store = lock(&runtime.store);
        let state = store
            .project()
            .map_err(|error| format!("não consegui projetar o Espaço alvo ({error})"))?;
        let name = state
            .workspace_name
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "o Espaço alvo não tem um nome durável".to_string())?;
        let id = state
            .workspace_id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "o Espaço alvo não tem uma identidade durável".to_string())?;
        if state.archived {
            return Err("o Espaço alvo está arquivado; restaure-o antes de abrir".to_string());
        }
        lina_core::WorkspaceEntry {
            id,
            name,
            path: runtime.ws_root.clone(),
            last_focus: 0,
            archived: false,
        }
    };
    let project_us = project_started.elapsed().as_micros();

    let catalog_started = Instant::now();
    let reused_catalog = verified_registry.is_some();
    let mut registry = match verified_registry {
        Some(registry) => registry,
        None => {
            persistence_ui::load_or_rebuild_workspace_registry(
                &registry_path,
                workspaces_base,
                &events_dir,
            )
            .map_err(|error| format!("não consegui reconciliar a lista de Espaços ({error})"))?
            .registry
        }
    };
    match registry
        .recover_pending_focus()
        .map_err(|error| format!("não consegui reconciliar uma troca anterior ({error})"))?
    {
        lina_core::WorkspaceFocusRecovery::Committed(
            lina_core::WorkspaceFocusCommit::CommittedPending { detail },
        ) => {
            return Err(format!(
                "uma troca anterior já foi confirmada, mas o catálogo ainda não pôde convergir ({detail})"
            ));
        }
        lina_core::WorkspaceFocusRecovery::NoPending
        | lina_core::WorkspaceFocusRecovery::Aborted { .. }
        | lina_core::WorkspaceFocusRecovery::Committed(
            lina_core::WorkspaceFocusCommit::Committed,
        ) => {}
    }
    let catalog_us = catalog_started.elapsed().as_micros();
    validate_registry_target(&registry, &entry)?;
    let requested_focus = lina_core::PendingWorkspaceFocus {
        focus_id: uuid::Uuid::now_v7().to_string(),
        target: entry.clone(),
        focused_at_ms: now_ms,
    };
    let (focus_transaction, pending) = registry
        .prepare_focus(&requested_focus)
        .map_err(|error| format!("não consegui preparar a troca de Espaço ({error})"))?;

    let append_started = Instant::now();
    let committed_seq = match lock(&runtime.store).append(
        &lina_core::DomainEvent::WorkspaceFocusSet {
            workspace: entry.name.clone(),
            focus_id: Some(pending.focus_id.clone()),
        },
    ) {
        Ok(seq) => seq,
        Err(error) => {
            let cleanup = registry.abort_prepared_focus(focus_transaction).err();
            return Err(match cleanup {
                Some(cleanup) => format!(
                    "não consegui confirmar o foco no Espaço alvo ({error}); a intenção será reconciliada na próxima abertura ({cleanup})"
                ),
                None => format!("não consegui confirmar o foco no Espaço alvo ({error})"),
            });
        }
    };
    let append_us = append_started.elapsed().as_micros();

    let commit_started = Instant::now();
    #[cfg(test)]
    let forced_commit_failure = tests::take_focus_registry_commit_fault();
    #[cfg(not(test))]
    let forced_commit_failure = false;
    let commit = if forced_commit_failure {
        drop(focus_transaction);
        lina_core::WorkspaceFocusCommit::CommittedPending {
            detail: "falha injetada depois do append e antes do save do registry".to_string(),
        }
    } else {
        match registry.commit_prepared_focus(focus_transaction, committed_seq) {
            Ok(commit) => commit,
            // O append acima é o ponto de commit. Qualquer erro posterior mantém o
            // journal preparado e vira reconciliação pendente, nunca falso `Err`.
            Err(error) => lina_core::WorkspaceFocusCommit::CommittedPending {
                detail: format!(
                    "o evento foi confirmado; o catálogo aguarda reconciliação ({error})"
                ),
            },
        }
    };
    let commit_us = commit_started.elapsed().as_micros();
    if profile {
        eprintln!(
            "[WS-PROFILE focus] target={} total_us={} project_us={} catalog_us={} catalog_reused={} append_us={} commit_us={}",
            runtime.ws_root.display(),
            total_started.elapsed().as_micros(),
            project_us,
            catalog_us,
            reused_catalog,
            append_us,
            commit_us
        );
    }
    match commit {
        lina_core::WorkspaceFocusCommit::Committed => Ok(FocusRegistration::Committed {
            workspace_id: entry.id,
        }),
        lina_core::WorkspaceFocusCommit::CommittedPending { detail } => {
            Ok(FocusRegistration::CommittedPending {
                workspace_id: entry.id,
                detail,
            })
        }
    }
}

/// Embrulha o runtime do boot como o ATIVO inicial do processo.
#[must_use]
pub fn runtimes_with_boot(rt: WsRuntime) -> Runtimes {
    let active = rt.ws_root.clone();
    let mut map = BTreeMap::new();
    map.insert(active.clone(), rt);
    Arc::new(Mutex::new(RuntimeMap {
        map,
        recent: VecDeque::from([active.clone()]),
        active,
    }))
}

/// **O switch vivo do M8.** Garante o runtime do alvo MONTADO (boot sob demanda + restore
/// F1-4-3 dele, dentro do `boot_ws_runtime`) e o marca ATIVO. Os PIDs do Espaço que sai de
/// cena NÃO são tocados (decisão do fundador: fundos VIVOS — pumps/PTYs seguem). Durável:
/// apenda `WorkspaceFocusSet` no log do ALVO (F1-4-4 crit 1) + carimba o foco no ponteiro
/// global pelo journal transacional (o PRÓXIMO boot abre o alvo). `Err` só existe antes
/// do commit e não troca nada. Se o evento já confirmou, a troca segue e o journal faz o
/// ponteiro convergir no restart, sem foco órfão nem retry duplicado.
pub fn activate_workspace(
    rts: &Runtimes,
    target_root: PathBuf,
    shared: &SharedInfra,
) -> Result<(), String> {
    let profile = workspace_switch_profile_enabled();
    let total_started = Instant::now();
    let lock_started = Instant::now();
    let mut r = lock(rts);
    let lock_us = lock_started.elapsed().as_micros();
    if r.active == target_root && r.map.contains_key(&target_root) {
        return Ok(());
    }
    let mut pending_runtime = None;
    let mut verified_registry = None;
    let mut verify_us = 0;
    let mut reserve_us = 0;
    let mut boot_us = 0;
    if !r.map.contains_key(&target_root) {
        let phase = Instant::now();
        verified_registry = Some(verify_workspace_target(&target_root)?.registry);
        verify_us = phase.elapsed().as_micros();
        let phase = Instant::now();
        r.reserve_runtime_slot()?;
        reserve_us = phase.elapsed().as_micros();
        let phase = Instant::now();
        pending_runtime = Some(boot_ws_runtime(target_root.clone(), shared, false)?);
        boot_us = phase.elapsed().as_micros();
    }
    // A religação faz parte da prontidão do alvo. Nenhum fato/carimbo de foco é
    // emitido enquanto houver agente estacionado sem sucessor integral.
    let relight_started = Instant::now();
    let target_runtime = pending_runtime
        .as_ref()
        .or_else(|| r.map.get(&target_root))
        .ok_or_else(|| "o runtime alvo desapareceu durante a troca".to_string())
        .and_then(relight_unloaded);
    let relit = match target_runtime {
        Ok(relit) => relit,
        Err(error) => {
            if let Some(mut runtime) = pending_runtime.take() {
                if let Err(stop_error) = runtime.stop_background_threads() {
                    eprintln!("lina-gpui: runtime parcial não encerrou limpo: {stop_error}");
                }
                runtime.nodes.shutdown_terminals_after_failed_boot();
            }
            return Err(format!(
                "não consegui reativar todos os agentes de {} ({error}); o Espaço atual foi mantido",
                target_root.display()
            ));
        }
    };
    if relit > 0 {
        eprintln!("lina-gpui: [PARK] {relit} terminal(is) religado(s) no foco");
    }
    let relight_us = relight_started.elapsed().as_micros();

    let focus_started = Instant::now();
    let focus_result = pending_runtime
        .as_ref()
        .or_else(|| r.map.get(&target_root))
        .ok_or_else(|| "o runtime alvo desapareceu antes de confirmar o foco".to_string())
        .and_then(|runtime| {
            register_ready_runtime_with_registry(runtime, verified_registry.take())
        });
    let focus_outcome = match focus_result {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Some(mut runtime) = pending_runtime.take() {
                if let Err(stop_error) = runtime.stop_background_threads() {
                    eprintln!("lina-gpui: runtime parcial não encerrou limpo: {stop_error}");
                }
                runtime.nodes.shutdown_terminals_after_failed_boot();
            }
            return Err(format!(
                "não consegui concluir a troca para {} ({error}); o Espaço atual foi mantido",
                target_root.display()
            ));
        }
    };
    if let Some(detail) = focus_outcome.pending_detail() {
        eprintln!(
            "lina-gpui: [WS] foco em {} confirmado; catálogo será reconciliado ({detail})",
            target_root.display()
        );
    }
    let focus_us = focus_started.elapsed().as_micros();

    // Não existe operação falível depois deste ponto: só agora o runtime e o foco
    // ficam visíveis para a janela.
    if let Some(runtime) = pending_runtime.take() {
        r.map.insert(target_root.clone(), runtime);
    }
    r.active = target_root;
    let active = r.active.clone();
    r.note_focus(&active);
    if profile {
        eprintln!(
            "[WS-PROFILE switch] target={} total_us={} lock_us={} verify_us={} reserve_us={} boot_us={} relight_us={} focus_us={}",
            r.active.display(),
            total_started.elapsed().as_micros(),
            lock_us,
            verify_us,
            reserve_us,
            boot_us,
            relight_us,
            focus_us
        );
    }
    Ok(())
}

// ───────────────── r5 park-seam · Descarregar Espaço de fundo + religar no foco ─────────────────

/// Resultado do Descarregar — o rail narra a partir daqui ("X desligados; Y seguem trabalhando").
#[allow(dead_code)] // consumido pelo botão do rail (costura do C) — padrão `RestoreBadge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnloadOutcome {
    /// Terminais ociosos desligados (PTY morto; card/nó preservados).
    pub parked: usize,
    /// Vivos preservados (Busy/Blocked/em atividade — "sem interferir", restrição do fundador).
    pub kept: usize,
}

/// **Callback público do botão «Descarregar Espaço» (rail/sidebar — o C fia).** Desliga os
/// PTYs OCIOSOS de um Espaço de FUNDO sem remover nada (religam no próximo foco, via
/// [`activate_workspace`]). Recusa o Espaço ATIVO (descarregar o que está na tela não faz
/// sentido e mataria a sessão em uso). Espaço não-montado → `Ok(0/0)` (já não consome nada).
///
/// # Errors
/// Alvo é o Espaço em foco.
#[allow(dead_code)] // consumido pelo botão do rail (costura do C) — padrão `plan_restore`.
pub fn unload_workspace(rts: &Runtimes, target_root: &Path) -> Result<UnloadOutcome, String> {
    let mut r = lock(rts);
    if r.active == target_root {
        return Err("este Espaço está em foco — troque para outro antes de descarregá-lo".into());
    }
    let Some(mut runtime) = r.map.remove(target_root) else {
        return Ok(UnloadOutcome { parked: 0, kept: 0 });
    };
    let Some(quiescence) = try_quiesce_for_unload(&runtime) else {
        r.map.insert(target_root.to_path_buf(), runtime);
        return Err(
            "este Espaço está concluindo uma criação, limpeza ou operação externa; tente novamente em instantes"
                .into(),
        );
    };
    let plan = terminal_unload_plan(&runtime);
    let parked = match runtime.nodes.park_terminals(&plan.park) {
        Ok(parked) => parked,
        Err(error) => {
            r.map.insert(target_root.to_path_buf(), runtime);
            return Err(error);
        }
    };
    runtime.nodes.join_cleanup_tasks();
    let live_terminals = runtime.nodes.terminal_ids().len();
    let outcome = UnloadOutcome {
        parked,
        kept: live_terminals,
    };
    if live_terminals == 0 {
        if let Err(error) = runtime.stop_hook_drain() {
            r.map.insert(target_root.to_path_buf(), runtime);
            return Err(format!(
                "este Espaço ainda está concluindo o dreno de tela; tente novamente ({error})"
            ));
        }
        if let Err(error) = runtime.stop_background_threads() {
            r.map.insert(target_root.to_path_buf(), runtime);
            return Err(format!(
                "este Espaço ainda está concluindo tarefas de fundo; tente novamente ({error})"
            ));
        }
        quiescence.seal();
        r.recent.retain(|known| known != target_root);
    } else {
        r.map.insert(target_root.to_path_buf(), runtime);
    }
    Ok(outcome)
}

/// **PURO — ids estacionados que AINDA pendem religação:** união dos `parked` de todos os
/// `WorkspaceUnloaded` do log, filtrada a quem segue `Dead` NA PROJEÇÃO. O supersede do
/// restore (`NodeRemoved`) os tira da projeção → religar é idempotente por construção.
fn parked_pending(
    recs: &[lina_core::EventRecord],
    proj: &lina_core::ProjectedState,
) -> std::collections::BTreeSet<NodeId> {
    let mut out = std::collections::BTreeSet::new();
    for r in recs {
        if r.kind != "WorkspaceUnloaded" {
            continue;
        }
        let Some(parked) = r
            .payload
            .get("parked")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for v in parked {
            let Some(id) = v.as_str().and_then(|s| s.parse::<NodeId>().ok()) else {
                continue;
            };
            let dead = proj
                .nodes
                .get(&id)
                .is_some_and(|n| n.status.as_deref() == Some("Dead"));
            if dead {
                out.insert(id);
            }
        }
    }
    out
}

/// Religa os terminais estacionados de um runtime JÁ MONTADO (o caso "Espaço de fundo
/// descarregado voltou ao foco") — reusa o fluxo F1-4-3 inteiro: `plan_restore` (resgate de
/// órfão-nomeado) filtrado aos estacionados + `restore_terminals` (re-ergue, badge honesto,
/// supersede com `NodeRemoved`). Devolve quantos religou. (Runtime NOVO não passa aqui: o
/// restore de boot dentro de `boot_ws_runtime` já resgata os estacionados do log.)
fn parked_restore_snapshot(
    rt: &WsRuntime,
) -> Result<
    (
        Vec<lina_core::EventRecord>,
        lina_core::ProjectedState,
        std::collections::BTreeSet<NodeId>,
    ),
    String,
> {
    let (recs, proj) = {
        let s = lock(&rt.store);
        match (s.events(), s.project()) {
            (Ok(r), Ok(p)) => (r, p),
            (Err(error), _) | (_, Err(error)) => {
                return Err(format!(
                    "não consegui ler os agentes estacionados ({error})"
                ));
            }
        }
    };
    let pending = parked_pending(&recs, &proj);
    Ok((recs, proj, pending))
}

fn relight_unloaded(rt: &WsRuntime) -> Result<usize, String> {
    let (_, _, observed_pending) = parked_restore_snapshot(rt)?;
    if observed_pending.is_empty() {
        return Ok(0);
    }
    let _quiescence = try_quiesce_for_unload(rt).ok_or_else(|| {
        "o Espaço recebeu uma atividade enquanto os agentes seriam religados; tente novamente"
            .to_string()
    })?;
    // O primeiro snapshot só decide se vale congelar. Toda decisão de restore é
    // refeita sob a guarda para fechar a corrida com unload/mensagem/admissão.
    let (recs, proj, pending) = parked_restore_snapshot(rt)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let mut plans: Vec<bridge::RestoredTerminal> = bridge::plan_restore_resuming(
        &proj,
        &rt.profile_registry,
        rt.nodes.scrollback().as_ref(),
        &lina_core::resume_session::ResumeSessionStore::from_records(&recs),
    )
    .into_iter()
    .filter(|p| pending.contains(&p.node))
    .collect();
    // ADR 0037: re-lançar o CLI por caminho absoluto (o PATH pode estar pobre no boot via Dock).
    crate::absolutize_restore_commands(&mut plans);
    let covered: std::collections::BTreeSet<NodeId> = plans
        .iter()
        .flat_map(|plan| std::iter::once(plan.node).chain(plan.shadows.iter().copied()))
        .collect();
    let missing: Vec<String> = pending
        .difference(&covered)
        .map(ToString::to_string)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{} agente(s) estacionado(s) não puderam ser reconstruídos: {}",
            missing.len(),
            missing.join(", ")
        ));
    }
    let expected = plans.len();
    let restored = rt.nodes.restore_terminals_quiesced(&plans)?;
    if restored != expected {
        return Err(format!(
            "religação incompleta: {restored}/{expected} agentes"
        ));
    }
    Ok(restored)
}

/// Epílogo do processo: para o dreno de TODOS os runtimes (o `main` chama após `app.run`).
pub fn stop_all(rts: &Runtimes) {
    let mut r = lock(rts);
    for rt in r.map.values_mut() {
        if let Err(error) = rt.stop_background_threads() {
            eprintln!("lina-gpui: runtime não encerrou limpo no epílogo: {error}");
        }
    }
}

/// R2b: `LINA_ATTENTION_DEMO` setado = teatro determinístico do fundador — o loop
/// VIVO de detecção desliga (demo OU real, nunca os dois; sem duplicar pedidos).
/// Lido em DOIS pontos (aqui: desliga a detecção viva do pump; `main`: semeia o pedido do
/// teatro DEPOIS do boot) — helper único para as duas leituras usarem o MESMO parse.
#[must_use]
pub fn attention_demo_kind() -> Option<String> {
    std::env::var("LINA_ATTENTION_DEMO")
        .ok()
        .map(|v| v.trim().to_lowercase())
        .filter(|k| !k.is_empty() && k != "0")
}

/// Monta UM Espaço vivo a partir de `ws_root` — passos [2]-[17] do boot, sem mudança de
/// comportamento (refactor puro da fatia i). O `main` resolve QUAL Espaço abre (registry/ponteiro)
/// e opera demo/load-gen/janela SOBRE o runtime retornado. `Err` = falha que antes encerrava o
/// processo (store de fallback morto, ponte core→UI não sobe) — quem decide encerrar é o chamador
/// (inv#6: mensagem clara, nunca backtrace; um runtime de FUNDO falhando não pode matar o app).
pub fn boot_ws_runtime(
    ws_root: PathBuf,
    shared: &SharedInfra,
    demo: bool,
) -> Result<WsRuntime, String> {
    let profile_boot = workspace_switch_profile_enabled();
    let boot_started = Instant::now();
    let mut previous_phase = boot_started;
    let mut note_boot_phase = |phase: &str| {
        if profile_boot {
            let now = Instant::now();
            eprintln!(
                "[WS-PROFILE boot] root={} phase={} delta_us={} total_us={}",
                ws_root.display(),
                phase,
                now.duration_since(previous_phase).as_micros(),
                now.duration_since(boot_started).as_micros()
            );
            previous_phase = now;
        }
    };
    let mailbox_dir = ws_root.join(".lina");
    let dir = mailbox_dir.join("events");
    // F1-0-2 critério 5: o perfil de injeção REAL (claude-code.toml — ready_timeout/busy_markers
    // calibrados) substitui o demo hardcoded; fallback-com-warning no loader (nunca panic). O
    // caminho carregado + os campos do fix saem AQUI no log de boot, p/ inspeção.
    let injection_profile = load_injection_profile(Some(&mailbox_dir));
    // A2A UNIVERSAL: registry de TODOS os CLI Profiles (claude-code, codex, gemini, antigravity —
    // semeados write-if-absent no dir do usuário). A entrega A2A resolve o profile do ALVO por aqui
    // (pelo `profile_id` do nó, projetado de `CliProfileSet`), com `injection_profile` como FALLBACK.
    // Sem isto, a entrega usava o prompt_ready/busy do Claude p/ TODO alvo → a TUI do Codex (glifo
    // `›`) nunca casava → timeout (msg não injetada). Compartilhado (Arc) com o ⚡ demo e o pump.
    let profile_registry = Arc::new(agent_modal::load_profiles(&agent_modal::profiles_dir(
        &mailbox_dir,
    )));
    note_boot_phase("profiles");
    eprintln!(
        "lina-gpui: workspace em {} ({})",
        ws_root.display(),
        if demo {
            "DEMO/temp"
        } else {
            "produção/persistente"
        }
    );
    // Falha de UM workspace não pode montar um store temporário compartilhado fingindo ser
    // o alvo: isso mistura identidade/eventos entre raízes e ainda carimba foco incorreto.
    // O switch mantém o runtime atual; os dados do alvo ficam preservados para recuperação.
    let store = Arc::new(Mutex::new(persistence_ui::open_store_resilient(&dir).map_err(|e| {
        format!(
            "não consegui abrir os dados de {} ({e}); eles foram preservados e o Espaço atual continuará aberto",
            dir.display()
        )
    })?));

    // F1-4-3: foto da geração ANTERIOR — capturada ANTES do fechamento abaixo, então quem
    // estava vivo no último quit ainda NÃO está `dead` nela (nós `dead` NÃO saem da projeção —
    // só `NodeRemoved` remove; o `plan_restore` PULA os `dead` de gerações mais antigas e o
    // executor aposenta cada re-erguido com `NodeRemoved`). É a fonte do plano de restore
    // (posições/nomes/papéis/cwd/CLI do log — inv#4); o opt-out por Espaço decide o uso adiante.
    let restore_proj = lock(&store).project().map_err(|error| {
        format!(
            "não consegui reconstruir o estado de {} ({error}); nenhum agente foi iniciado",
            ws_root.display()
        )
    })?;

    // A morte póstuma da geração anterior participa do MESMO lote do restore. Até
    // todos os recursos do novo runtime estarem preparados, o log permanece intacto.
    let previous_generation_events = previous_generation_close_events(&restore_proj);
    note_boot_phase("store_project");

    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    // W3-2 · BOOTSTRAP turno-0: o app escreve `<cwd>/CLAUDE.md` (8 blocos) por terminal e o
    // reescreve a cada mudança de roster. `ws_root`/`mailbox_dir` já foram definidos acima (o event
    // store do app mora em `<ws_root>/.lina/events`, o MESMO log do bin); vault/autonomia/bin `lina`
    // configuráveis por env (LINA_VAULT/LINA_BIN). O bootstrap é best-effort (não trava o app).
    // W3-4: a mailbox compartilhada do workspace (`<ws_root>/.lina`). `LINA_HOME` aponta os
    // terminais (que herdam o env) para cá → `lina ask`/`handshake` depositam no MESMO outbox que
    // o supervisor (o `MailboxPump`) observa, e o bin `lina do/guard` apenda no MESMO event log.
    if let Err(e) = std::fs::create_dir_all(&mailbox_dir) {
        // Não fatal (o resto do app — canvas, terminais — funciona sem A2A), mas VISÍVEL: sem a
        // mailbox, `lina ask` não terá para onde escrever.
        eprintln!(
            "lina-gpui: não criou a mailbox {} (A2A indisponível): {e}",
            mailbox_dir.display()
        );
    }
    // F1-4-4 fatia (ii) — LINA_HOME é POR SPAWN (`node_identity_env`, veredito §3): cada
    // terminal nasce apontando para o `.lina` do SEU Espaço. O `set_var` global foi REMOVIDO:
    // auditoria 2026-06-11 = zero leitores in-process no app (únicas ocorrências eram este
    // setter e uma allowlist de teste), e com N runtimes o global apontaria sempre para o
    // último Espaço bootado — spawns do Espaço A nasceriam com o `.lina` de B.
    // O segundo cérebro (onboarding) grava o vault escolhido em `.lina/vault.json`. Se houver um
    // `primary`, ele tem prioridade — reflete a escolha REAL do usuário e faz `{{vault_writable_paths}}`/
    // `{{vault_tino_path}}` apontarem pro vault linkado na próxima reescrita do bootstrap. Senão, cai no
    // override de dev `LINA_VAULT` e, por fim, no default `<ws_root>/vault`.
    // ADR 0056: o vault é config do USUÁRIO → mora no `~/.lina` global e é herdado por todo Espaço.
    // Migração one-shot (idempotente): promove p/ o global o vault que o usuário já linkou em qualquer
    // Espaço conhecido — é o que faz um Espaço NOVO enxergar o vault sem refazer o onboarding.
    obsidian::migrate_vault_to_global();
    note_boot_phase("vault_migrate");
    let vault_path = obsidian::read_primary_vault_effective(&mailbox_dir)
        .or_else(|| std::env::var("LINA_VAULT").ok())
        .unwrap_or_else(|| ws_root.join("vault").display().to_string());
    note_boot_phase("vault_read");
    // Self-heal do segundo cérebro: se o vault está conectado (vault.json) mas o índice (PageIndex)
    // sumiu — a thread fire-and-forget do onboarding morreu, ou a 1ª execução foi negada no TCC de
    // Documentos — regenera-o agora, no contexto do app (já com o grant de Documentos do bundle).
    // No-op quando os índices já existem. Sem isso, o usuário não-técnico fica "conectado mas vazio".
    // ADR 0056: o índice vive ao lado do vault — no global (onde ele agora mora) e, se houver override
    // de projeto, no `.lina` do Espaço.
    if let Some(global) = obsidian::global_lina_dir() {
        obsidian::heal_missing_indices(&global);
    }
    note_boot_phase("vault_heal_global");
    obsidian::heal_missing_indices(&mailbox_dir);
    note_boot_phase("vault_heal_workspace");
    let lina_bin = std::env::var("LINA_BIN").unwrap_or_else(|_| "lina".to_string());
    // SEAM-1 (M2): FONTE ÚNICA da autonomia do workspace — passada AO MESMO TEMPO ao `BootstrapWriter`
    // (doutrina do bin) E ao `RouterConfig` da `MailboxPump` (enforcement do gate). `LINA_AUTONOMY`
    // (default `assistido`, sem regressão); origem futura = `bootstrap.json`/workspace.
    // FIX-3: a MESMA função alimenta a UI (`WorkspaceView.ws_autonomy` — badge do card + prefill
    // do modal), para a superfície nunca divergir do enforcement.
    let autonomy = crate::workspace_autonomy();
    let mut bootstrap = BootstrapWriter::new(ws_root.clone(), vault_path, autonomy, lina_bin)
        .map_err(|e| eprintln!("lina-gpui: bootstrap desativado: {e}"))
        .ok();
    note_boot_phase("bootstrap_registry");

    // ── F1-1-3/F1-1-5 (wiring) · HOOKS + TIMELINE + SESSÕES — antes de qualquer write/spawn. ──
    // Um listener por PROCESSO; cada terminal ganha token escopado por workspace SE o perfil
    // declarar a capability. O drain valida/retira o escopo antes de tocar a timeline, então
    // workspaces com nomes de terminal iguais seguem isolados sem multiplicar listeners.
    let dash = Arc::new(dashboard::DashWiring::new(
        injection_profile.id.clone(),
        ws_root.display().to_string(),
    ));
    let hook_scope = restore_proj
        .workspace_id
        .clone()
        .unwrap_or_else(|| format!("path:{}", ws_root.display()));
    let (hooks, hook_drain) = if injection_profile.capabilities.has("hooks") {
        if let Some(shared_hooks) = HooksShared::start() {
            let hooks = shared_hooks.for_workspace(hook_scope);
            if let Some(bw) = bootstrap.as_mut() {
                bw.set_hooks(hooks.clone());
            }
            let tl = Arc::clone(&dash.timeline);
            let drain = hooks.spawn_drain(move |ev| {
                // Sonda [DASH]: latência de INGESTÃO (chegada no listener → timeline).
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                eprintln!(
                    "lina-gpui: [DASH] hook node={} kind={:?} tool={:?} lat_ingest_ms={}",
                    ev.node_id,
                    ev.kind,
                    ev.tool_name,
                    now.saturating_sub(ev.ts)
                );
                if let Ok(mut t) = tl.lock() {
                    t.note(ev);
                }
            });
            (Some(hooks), Some(drain))
        } else {
            (None, None)
        }
    } else {
        eprintln!(
            "lina-gpui: [DASH] perfil '{}' sem capability hooks — settings sem hooks (como antes)",
            injection_profile.id
        );
        (None, None)
    };
    // Sessões (camada 3 — custo~): r5 perf-ws — o runtime se INSCREVE no hub ÚNICO do
    // processo (antes: 1 thread + 1 SessionWatch ZERADO por Espaço = N cold-parses do HOME
    // inteiro + N caminhadas/stat a cada 500ms — a regressão de perf do multi-workspace).
    // O pattern segue vindo do TOML do perfil (F1-1-1); HOME real do usuário (não é teste).
    if let Some(pattern) = injection_profile.session_dir_pattern.clone() {
        subscribe_sessions(&injection_profile.id, &pattern, &dash.sessions);
    }
    note_boot_phase("hooks_sessions");

    // ── W3-6c (ADR 0004) · COFRE DE SEGREDO + ENV LIMPO (fios 1 e 3) — ANTES de spawnar qualquer PTY.
    // Cofre do workspace. DEMO: backend em memória (`MockStore`) p/ não tocar o keyring do SO no
    // teatro do fundador; PRODUÇÃO trocaria por `SecretVault::new("lina-space/<ws>")` (KeyringStore).
    // Semente OPCIONAL via `LINA_DEPLOY_TOKEN`: com ela, a custódia EXECUTA (segredo do cofre); sem
    // ela, o cofre fica vazio e a custódia BLOQUEIA (DeniedNoSecret) — ambos provam a blindagem.
    let custody_vault = SecretVault::with_store("lina-space/walking-skeleton", MockStore::new());
    // F4-1-4: cofre de CANAIS, COMPARTILHADO (Arc) entre a `MailboxPump` (a tela de conectar grava a
    // chave aqui via `credential.store`) e a `BrokerPump` (o envio gated a lê do MESMO cofre). Um único
    // cofre; sem isso, a chave gravada por uma pump seria invisível à outra (MockStore é por-instância).
    let channel_vault = Arc::new(SecretVault::with_store(
        "lina-space/channels",
        MockStore::new(),
    ));
    if let Ok(token) = std::env::var("LINA_DEPLOY_TOKEN") {
        match custody_vault.set("deploy", "prod", &token) {
            Ok(()) => {
                eprintln!("lina-gpui: cofre semeado (deploy/prod) a partir de LINA_DEPLOY_TOKEN")
            }
            Err(e) => eprintln!("lina-gpui: nao semeou o cofre (deploy/prod): {e}"),
        }
    }
    // Fio 3: os PTYs filhos herdam o env do app no spawn → remover as vars de segredo do PRÓPRIO env
    // AQUI (antes de qualquer `wire_terminal`) garante que nenhum terminal do agente nasça com o token.
    let scrubbed = scrub_pty_secret_env();
    if !scrubbed.is_empty() {
        eprintln!("lina-gpui: env limpo — vars de segredo removidas do env do app (PTYs nao as herdam): {scrubbed:?}");
    }
    // R2c (bug de tela 21:40): se o Lina.app foi lançado de um terminal hospedado no
    // Maestri, `MAESTRI_*` vaza no env → a skill GLOBAL do Maestri vira "funcional" p/ o
    // agente do ⌘N (maestri list exit 127; "$MAESTRI_CLI" exit 126 — a var EXISTE). Scrub
    // ANTES de qualquer `wire_terminal`: nenhum agente nasce enxergando plumbing de fora
    // do Espaço (a comunicação entre terminais é EXCLUSIVA pelos verbos `lina`).
    let foreign = scrub_foreign_orchestrator_env();
    if !foreign.is_empty() {
        eprintln!(
            "lina-gpui: env limpo — vars de orquestrador estrangeiro removidas (PTYs nao as herdam; comunicacao so via `lina`): {foreign:?}"
        );
    }
    // Mesa de custódia compartilhada: a UI confirma (⌘⏎); a BrokerPump executa COM o segredo do cofre.
    let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
    // W4-3: FREIO compartilhado — a UI pede pausa/retoma; a MailboxPump aplica no Router e espelha.
    let brake: wiring::Brake = wiring::new_brake();

    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));

    // Rodada 360 (ADR 0022): o NodeManager — dono dos nós em runtime E o FUNIL ÚNICO de
    // admissão — nasce ANTES do seed do demo, para as TRÊS portas (⌘T, ⌘N e o seed) entrarem
    // pelo MESMO `admit_node`. `seq_start = 0`: o demo admite A/B pelo funil (seq 0/1 → keys
    // t0/t1); em produção o 1º ⌘T do aluno nasce como "Terminal A".
    // SEAM-1: `Arc` — compartilhado com a `MailboxPump`/`AttentionHub` (SpawnApproved/banner criam
    // pelo MESMO funil `admit_node`, ADR 0022). O gpui View detém um clone; as pumps, outro.
    let nodes = Arc::new(NodeManager::new(
        Arc::clone(&shared.pty),
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&model),
        Arc::clone(&grids),
        BTreeMap::new(),
        delta_tx,
        shared.cols,
        shared.rows,
        0,
        Arc::clone(&shared.cmd_factory),
        bootstrap,
        mailbox_dir.clone(), // W4-2 M3/M4: `.lina/` onde notas/pastas persistem
    ));
    nodes.set_prompt_profiles(Arc::clone(&profile_registry), injection_profile.clone());
    let prompt_nodes = Arc::downgrade(&nodes);
    sup.set_input_observer(Arc::new(move |node| {
        if let Some(nodes) = prompt_nodes.upgrade() {
            nodes.capture_prompt_before_input(node);
        }
    }));
    let boot_quiescence = nodes
        .try_quiesce()
        .ok_or_else(|| "não consegui reservar o runtime para a restauração inicial".to_string())?;

    // F4-WA-1: engine de webhook ativo (POST externo → input no terminal vivo). Runtime tokio próprio
    // (molde HooksShared): bind loopback + serve; deposita a entrega-de-sistema na fila que o pump drena.
    let webhook = bridge::WebhookShared::start(
        Arc::clone(&store),
        Arc::clone(&sup),
        nodes.system_deliveries(),
    );
    note_boot_phase("nodes_webhook");

    // F1-4-3 · RESTORE do quit limpo: reabrir o app = o Espaço volta vivo (posições/nomes/
    // papéis do log + scrollback re-hidratado + resume do CLI quando o TOML declara + badge
    // honesto). Opt-out por Espaço (`restore_on_open` no settings.json — «Abrir este Espaço
    // sem religar os Agentes») e por env (`LINA_NO_RESTORE=1`, escape de emergência). Fora do
    // demo (o demo semeia A/B fixos) e best-effort: falha degrada com log, nunca trava o boot.
    let mut restore_plans = if !demo
        && persistence_ui::load_settings(&dir).restore_on_open
        && !std::env::var("LINA_NO_RESTORE").is_ok_and(|v| v.trim() == "1")
    {
        let resume_sessions = lina_core::resume_session::ResumeSessionStore::replay(&lock(&store))
            .map_err(|error| format!("não consegui ler as sessões para restaurar ({error})"))?;
        bridge::plan_restore_resuming(
            &restore_proj,
            &profile_registry,
            nodes.scrollback().as_ref(),
            &resume_sessions,
        )
    } else {
        Vec::new()
    };
    // ADR 0037: re-lançar o CLI por caminho absoluto — no boot via Dock o PATH é mínimo.
    crate::absolutize_restore_commands(&mut restore_plans);

    // W3-7c · TETO DE CUSTO REAL. `LINA_TOKEN_BUDGET_DAY` (tokens/dia ESTIMADOS ≈ bytes/4 de output;
    // ver cost.rs) ARMA o teto; 0/ausente = desligado (default, sem regressão). A bomba mede o output
    // dos PTYs e apenda TokenUsageReported no MESMO store (`<ws_root>/.lina/events`, o log
    // compartilhado com o bin `lina`) que o CostLedger soma.
    let token_budget_day: u64 = std::env::var("LINA_TOKEN_BUDGET_DAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let idle_ms = injection_profile.idle_ms.unwrap_or(200);
    let bridge_host = GpuiBridgeHost::new(Arc::clone(&model));
    // F1-1-7: a FILA DE ATENÇÃO unificada — a projeção `AttentionQueue` do core cabeada
    // ao MESMO event log (replay no boot reconstrói pendência pós-crash — critério 5) e
    // à mesa de custódia. NENHUM byte vai ao PTY por este caminho (ADR 0021 §6).
    // Criada ANTES do pump (R2b): a varredura viva da detecção usa a projeção de mute
    // do hub (mesma fonte do log — defesa em profundidade nas duas pontas).
    // F1-1-8 / ADR 0024: o hub ganha a porta de escrita validada (gesto humano + auto-deny do
    // SLA digitam por aqui). `sup`/`grids` são os MESMOS do app (Arc compartilhado); as
    // `approval_keys` vêm do CLI Profile de injeção (lido ANTES do move para a MailboxPump).
    let attention = Arc::new(AttentionHub::new(
        Arc::clone(&store),
        Arc::clone(&desk),
        Arc::clone(&model),
        ApprovalWiring::new(
            Arc::clone(&sup),
            Arc::clone(&grids),
            injection_profile.approval_keys.clone(),
        ),
        // SEAM-1: aprovar um banner de spawn na fila cria o terminal pelo MESMO funil (admit_node).
        Arc::clone(&nodes),
    ));
    let live_detection = attention_demo_kind().is_none();
    if !live_detection {
        eprintln!("lina-gpui: [ATT] modo DEMO — detecção viva de grid DESLIGADA nesta sessão");
    }
    // inv#6/no-panic: sem a ponte core→UI o app não renderiza terminais — encerra com mensagem clara
    // em vez de backtrace. (Falha só em condição catastrófica de sistema; instalação nova não cai aqui.)
    // FIX DE GATE F1-0: o pump precisa do Supervisor p/ devolver o nó a Idle no fim-de-resposta
    // (transição event-sourced — destrava a retenção F1-0-4 na 2ª mensagem ao mesmo alvo).
    // R2b: + grids/hub/flag — a metade-app da detecção viva roda no tick do pump.
    let mut pump = match spawn_pump(
        bridge_host,
        delta_rx,
        bus_rx,
        Arc::clone(&store),
        idle_ms,
        Arc::clone(&sup),
        Arc::clone(&grids),
        Arc::clone(&attention),
        live_detection,
        Some(Arc::clone(&nodes)),
    ) {
        Ok(p) => p,
        Err(e) => {
            nodes.shutdown_terminals_after_failed_boot();
            return Err(format!(
                "não subi a ponte core→UI ({e}); o app não consegue renderizar"
            ));
        }
    };

    let input = Arc::new(CoreInput::new(Arc::clone(&sup)));

    // W3-2: garante o CLAUDE.md/estado do roster do boot (vazio em produção — best-effort).
    // (O NodeManager nasce ANTES do seed, lá em cima — rodada 360/ADR 0022.)
    nodes.rewrite_bootstrap();

    // W3-4: sobe o SUPERVISOR que observa a mailbox (`<ws_root>/.lina/outbox/`). A partir daqui,
    // um `lina ask @B "oi"` de qualquer terminal trafega: outbox → guardrails → deliver_a2a → B.
    // Thread DAEMON (observador de fundo): o app sai via `process::exit`, que encerra a thread —
    // o handle é mantido só para deixar o ciclo de vida explícito.
    let mut _mailbox_pump = match MailboxPump::new(
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&grids),
        Mailbox::new(&mailbox_dir),
        nodes.reinject_queue(), // FIX-A3: fila EM-PROCESSO de re-injeção (sem superfície de filesystem)
        Arc::clone(&model),
        Arc::clone(&brake), // W4-3: freio (pausa/retoma a auto-orquestração)
        token_budget_day,   // W3-7c: arma o teto de custo no Router (0 = desligado)
        // F1-0-2: o claude-code.toml real (fallback-com-warning no loader). Clone: o original
        // fica no `WsRuntime` (o demo ⚡ e a fatia ii leem de lá).
        injection_profile.clone(),
        // A2A UNIVERSAL: resolve o profile do ALVO (codex/gemini/…) na entrega.
        Arc::clone(&profile_registry),
        autonomy, // SEAM-1 (M2): autonomia REAL fiada no RouterConfig (manual bloqueia spawn)
        Arc::clone(&nodes), // SEAM-1: funil de admissão — SpawnApproved cria o terminal
        // F3-5-1: o MESMO snapshot de sessões que o dashboard recebe do hub — o pump correlaciona por
        // `cwd` e nasce o `SessionPersisted` do 1º prompt (vazio quando o CLI não grava session-file).
        Arc::clone(&dash.sessions),
    )
    .with_webhook(webhook.as_deref())
    // F4-1-4: a tela "Conectar WhatsApp" grava a chave (`credential.store`) NESTE cofre compartilhado.
    .with_channel_vault(Arc::clone(&channel_vault))
    .spawn_stoppable()
    {
        Ok(pump) => pump,
        Err(error) => {
            pump.stop();
            nodes.shutdown_terminals_after_failed_boot();
            return Err(format!(
                "não subi o canal interno do Espaço ({error}); o runtime parcial foi desmontado"
            ));
        }
    };

    // W3-6c (ADR 0004): a BROKER PUMP observa a FILA DE BROKER (`<ws_root>/.lina/broker/`, irmã do
    // outbox A2A), aplica o gate humano (⌘⏎ na janela) e chama `run_custody` — que obtém o segredo do
    // cofre e executa. O agente NUNCA tem o token. Thread daemon (encerra com o `process::exit`).
    let mut _broker_pump = match BrokerPump::new(
        Mailbox::new(mailbox_dir.join("broker")),
        Arc::clone(&store),
        custody_vault,
        Arc::clone(&desk),
        Arc::clone(&model),
        Arc::clone(&sup), // roster vivo: valida que a origem do pedido é um nó REAL (hole 3)
    )
    .with_webhook(webhook.as_deref()) // F4-WA-2b: cofre+porta p/ o gesto webhook.test (dogfooding)
    // F4-1-4: o envio gated de canal lê a chave do MESMO cofre que a tela gravou.
    .with_channel_vault(Some(Arc::clone(&channel_vault)))
    // Item #4 (ADR 0060): toggle mestre da custódia. Com `<ws_root>/.lina/workspace.json → guard:off`
    // E autonomia autônoma, os pedidos `lina do` executam direto (sem o gate humano ⌘⏎). Fonte relida
    // a cada pedido; default (arquivo ausente / não-autônomo) mantém o gate — ADR 0004 preservado.
    .with_guard(mailbox_dir.clone(), autonomy)
    .with_runtime_activity(nodes.runtime_activity())
    .spawn_stoppable()
    {
        Ok(pump) => pump,
        Err(error) => {
            _mailbox_pump.stop();
            pump.stop();
            nodes.shutdown_terminals_after_failed_boot();
            return Err(format!(
                "não subi a fila protegida do Espaço ({error}); o runtime parcial foi desmontado"
            ));
        }
    };
    note_boot_phase("pumps");

    let restored = nodes
        .restore_terminals_with_prefix(&restore_plans, &previous_generation_events)
        .map_err(|error| {
            _broker_pump.stop();
            _mailbox_pump.stop();
            pump.stop();
            format!(
                "o Espaço não terminou de carregar: 0/{} agentes restaurados atomicamente ({error})",
                restore_plans.len()
            )
        })?;
    if restored != restore_plans.len() {
        _broker_pump.stop();
        _mailbox_pump.stop();
        pump.stop();
        return Err(format!(
            "o Espaço não terminou de carregar: {restored}/{} agentes restaurados",
            restore_plans.len()
        ));
    }
    eprintln!(
        "lina-gpui: F1-0-8 — geração anterior fechada no commit: {} fantasma(s)",
        previous_generation_events.len()
    );
    if restored == 0 {
        eprintln!("lina-gpui: [RESTORE] nada a re-erguer (Espaço novo ou sem terminais)");
    }
    drop(boot_quiescence);
    note_boot_phase("restore_commit");

    Ok(WsRuntime {
        ws_root,
        mailbox_dir,
        store,
        sup,
        nodes,
        model,
        grids,
        input,
        attention,
        desk,
        brake,
        autonomy,
        injection_profile,
        profile_registry,
        hooks,
        _hook_drain: hook_drain,
        webhook,
        dash,
        _pump: pump,
        _mailbox_pump,
        _broker_pump,
    })
}

// ───────────────── r5 perf-ws · hub ÚNICO de session-watch por PROCESSO ─────────────────
//
// ANTES (a regressão do multi-workspace): cada `boot_ws_runtime` subia SUA thread com um
// `SessionWatch` ZERADO — N Espaços = N cold-parses do HOME inteiro (medido na máquina do
// fundador: **10,8s** de parse por boot, 3.735 jsonl/1,3GB) + N caminhadas+stat a cada
// 500ms (medido: ~10ms/poll real, 37ms em cache frio ⇒ 4 Espaços ≈ 30k stats/s contínuos).
// DEPOIS: UM watcher por processo; fontes deduplicadas; cold-parse 1×; cada Espaço novo só
// se INSCREVE e recebe o snapshot corrente na hora. As sessões são dado GLOBAL da máquina
// (o HOME é um só) — não há nada por-Espaço numa varredura dessas.

/// Hub testável (sem thread/relógio próprios): o estado da varredura única + os
/// inscritos (`Weak` — runtime removido sai sozinho do fan-out).
struct SessionHub {
    watch: lina_session_watch::SessionWatch,
    sources: std::collections::BTreeSet<(String, String)>,
    known: std::collections::BTreeSet<(String, String)>,
    last_snapshot: Vec<lina_session_watch::Session>,
    subs: Vec<std::sync::Weak<Mutex<Vec<lina_session_watch::Session>>>>,
}

impl SessionHub {
    fn new(home: PathBuf) -> Self {
        Self {
            watch: lina_session_watch::SessionWatch::with_home(home),
            sources: std::collections::BTreeSet::new(),
            known: std::collections::BTreeSet::new(),
            last_snapshot: Vec::new(),
            subs: Vec::new(),
        }
    }

    /// Registra uma fonte (cli + pattern do TOML), DEDUPLICADA: N Espaços com o mesmo
    /// perfil = 1 varredura. Fonte nova (outro CLI/pattern) entra no próximo poll.
    fn add_source(&mut self, cli: &str, pattern: &str) {
        if self.sources.insert((cli.to_string(), pattern.to_string())) {
            self.watch.add_source(cli, pattern);
        }
    }

    #[cfg(test)]
    fn sources_len(&self) -> usize {
        self.sources.len()
    }

    /// Inscreve o destino de UM runtime e o SEMEIA com o snapshot corrente — um Espaço
    /// aberto depois do cold-parse não fica vazio esperando o próximo update.
    fn subscribe(&mut self, out: &Arc<Mutex<Vec<lina_session_watch::Session>>>) {
        if let Ok(mut dst) = out.lock() {
            dst.clone_from(&self.last_snapshot);
        }
        self.subs.push(Arc::downgrade(out));
    }

    /// Um passo: varre 1× e, se algo mudou, refaz o snapshot e alimenta TODOS os
    /// inscritos vivos (poda os mortos). Devolve quantos inscritos foram alimentados.
    fn poll_and_fanout(&mut self) -> usize {
        let updated = match self.watch.poll_once() {
            Ok(o) if !o.sessions_updated.is_empty() => {
                self.known.extend(o.sessions_updated);
                self.last_snapshot = self
                    .known
                    .iter()
                    .filter_map(|(c, s)| self.watch.scanner().session(c, s))
                    .collect();
                true
            }
            Ok(_) => false,
            Err(e) => {
                eprintln!("lina-gpui: [SESSÕES] poll do hub falhou (re-tenta): {e}");
                false
            }
        };
        if !updated {
            self.subs.retain(|w| w.strong_count() > 0);
            return 0;
        }
        let mut fed = 0;
        self.subs.retain(|w| match w.upgrade() {
            Some(out) => {
                if let Ok(mut dst) = out.lock() {
                    dst.clone_from(&self.last_snapshot);
                    fed += 1;
                }
                true
            }
            None => false,
        });
        fed
    }
}

/// Hub global do processo (nasce no 1º Espaço com `session_dir_pattern`).
static SESSION_HUB: std::sync::OnceLock<Arc<Mutex<SessionHub>>> = std::sync::OnceLock::new();

/// Inscreve o `dash.sessions` de um runtime no hub único (cria hub + thread na 1ª chamada).
fn subscribe_sessions(
    cli: &str,
    pattern: &str,
    out: &Arc<Mutex<Vec<lina_session_watch::Session>>>,
) {
    let Some(home) = std::env::var_os("HOME") else {
        return; // sem HOME não há o que varrer (mesma degradação de antes)
    };
    let hub = SESSION_HUB.get_or_init(|| {
        let hub = Arc::new(Mutex::new(SessionHub::new(PathBuf::from(home))));
        let for_thread = Arc::clone(&hub);
        let spawned = thread::Builder::new()
            .name("lina-session-hub".into())
            .spawn(move || loop {
                lock(&for_thread).poll_and_fanout();
                thread::sleep(Duration::from_millis(500));
            });
        if let Err(e) = spawned {
            eprintln!("lina-gpui: [SESSÕES] hub sem thread ({e}); dashboards sem sessões");
        }
        hub
    });
    let mut h = lock(hub);
    h.add_source(cli, pattern);
    h.subscribe(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        /// Sabota exatamente a janela pós-append/pré-save. O evento já está
        /// confirmado; a função deve devolver sucesso pendente e publicar o alvo.
        static FOCUS_REGISTRY_COMMIT_FAULT: Cell<bool> = const { Cell::new(false) };
    }

    fn arm_focus_registry_commit_fault() {
        FOCUS_REGISTRY_COMMIT_FAULT.with(|fault| fault.set(true));
    }

    pub(super) fn take_focus_registry_commit_fault() -> bool {
        FOCUS_REGISTRY_COMMIT_FAULT.with(|fault| fault.replace(false))
    }

    #[test]
    fn workspace_reliability_boot_wires_prompt_epoch_observer() {
        let root = std::env::temp_dir().join(format!(
            "lina-boot-prompt-observer-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        drop(
            lina_core::Workspace::create(&root, "Prompt observer", "blank", None)
                .expect("workspace"),
        );
        let shared = SharedInfra {
            pty: Arc::new(Mutex::new(PtyManager::new())),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let mut runtime = boot_ws_runtime(root.clone(), &shared, false).expect("boot real");
        let mut profile = bridge::demo_profile();
        profile.prompt_ready_regex = "(?m)^READY> ".to_string();
        profile.busy_markers.clear();
        runtime
            .nodes
            .set_prompt_profiles(Arc::new(ProfileRegistry::new()), profile);
        let node = runtime.nodes.add_node().expect("terminal");
        let grid = lock(&runtime.grids).get(&node).cloned().expect("grid");
        lock(&grid).advance(b"READY> ");
        assert!(runtime.nodes.terminal_has_fresh_prompt(node));

        runtime
            .sup
            .write_human(node, b"x", Duration::from_secs(1))
            .expect("entrada pela porta real");
        assert_eq!(runtime.sup.terminal_input_generation(node), Some(1));
        assert!(
            !runtime.nodes.terminal_has_fresh_prompt(node),
            "o observer instalado pelo boot capturou o prompt anterior à entrada"
        );

        runtime
            .stop_background_threads()
            .expect("threads encerram no teste");
        runtime.nodes.shutdown_terminals_after_failed_boot();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_reliability_unload_waits_for_observed_tick_completion() {
        let root = std::env::temp_dir().join(format!(
            "lina-unload-wait-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        drop(lina_core::Workspace::create(&root, "Unload wait", "blank", None).expect("workspace"));
        let shared = SharedInfra {
            pty: Arc::new(Mutex::new(PtyManager::new())),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let mut runtime = boot_ws_runtime(root.clone(), &shared, false).expect("boot real");
        let pump_tick = runtime
            .nodes
            .begin_external_task()
            .expect("tick controlado em voo");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                started_tx.send(()).expect("sinalizar espera iniciada");
                let converged = try_quiesce_for_unload(&runtime).is_some();
                result_tx.send(converged).expect("publicar desfecho");
            });
            started_rx.recv().expect("waiter iniciou");
            assert!(
                matches!(
                    result_rx.recv_timeout(Duration::from_millis(40)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ),
                "um tick acima da antiga janela de 25ms permanece protegido"
            );
            drop(pump_tick);
            assert!(
                result_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("mudança observada acorda o unload"),
                "o unload converge assim que a atividade realmente termina"
            );
        });

        runtime
            .stop_background_threads()
            .expect("threads encerram no teste");
        runtime.nodes.shutdown_terminals_after_failed_boot();
        drop(runtime);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_reliability_system_delivery_reservation_drop_wakes_unload() {
        let root = std::env::temp_dir().join(format!(
            "lina-delivery-reservation-wait-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        drop(
            lina_core::Workspace::create(&root, "Delivery wait", "blank", None).expect("workspace"),
        );
        let shared = SharedInfra {
            pty: Arc::new(Mutex::new(PtyManager::new())),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let mut runtime = boot_ws_runtime(root.clone(), &shared, false).expect("boot real");
        let queue = runtime.nodes.system_deliveries();
        let reservation = queue.try_reserve().expect("reserva webhook pendente");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        thread::scope(|scope| {
            scope.spawn(|| {
                started_tx.send(()).expect("sinalizar espera iniciada");
                let converged = try_quiesce_for_unload(&runtime).is_some();
                result_tx.send(converged).expect("publicar desfecho");
            });
            started_rx.recv().expect("waiter iniciou");
            assert!(
                matches!(
                    result_rx.recv_timeout(Duration::from_millis(40)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ),
                "a reserva aceita impede unload"
            );
            let released_at = Instant::now();
            drop(reservation);
            assert!(
                result_rx
                    .recv_timeout(Duration::from_millis(500))
                    .expect("drop da reserva acorda unload"),
                "a fila vazia converge sem esperar o timeout de 2s"
            );
            assert!(released_at.elapsed() < Duration::from_millis(500));
        });

        runtime
            .stop_background_threads()
            .expect("threads encerram no teste");
        runtime.nodes.shutdown_terminals_after_failed_boot();
        drop(runtime);
        let _ = std::fs::remove_dir_all(root);
    }

    #[derive(Debug, Clone, Copy)]
    struct ResourceSample {
        fds: usize,
        threads: usize,
        listeners: usize,
        hook_listeners: usize,
        webhook_listeners: usize,
        pump_handles: usize,
        flush_registrations: usize,
        flush_threads: usize,
        flush_signal_registrations: usize,
        flush_handlers_installed: bool,
        runtimes: usize,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct InternalResourceCounts {
        pump_handles: usize,
        hook_listeners: usize,
        webhook_listeners: usize,
        flush_registrations: usize,
        flush_threads: usize,
        flush_signal_registrations: usize,
        flush_handlers_installed: bool,
    }

    fn internal_resource_counts() -> InternalResourceCounts {
        let runtime = bridge::runtime_listener_stats();
        let flush = lina_core::scrollback::flush_coordinator_stats();
        InternalResourceCounts {
            pump_handles: runtime.pump_handles,
            hook_listeners: runtime.hook_listeners,
            webhook_listeners: runtime.webhook_listeners,
            flush_registrations: flush.registrations,
            flush_threads: flush.threads,
            flush_signal_registrations: flush.signal_registrations,
            flush_handlers_installed: flush.handlers_installed,
        }
    }

    fn process_thread_count() -> usize {
        lina_core::bench::thread_count().expect("contador nativo de threads disponível")
    }

    #[cfg(target_os = "macos")]
    fn process_thread_names() -> Vec<String> {
        const PROC_PIDLISTTHREADS: libc::c_int = 6;
        let mut thread_ids = vec![0_u64; 256];
        let buffer_size = (thread_ids.len() * std::mem::size_of::<u64>()) as libc::c_int;
        // SAFETY: `thread_ids` é um buffer gravável de `buffer_size` bytes e o PID
        // é o processo atual. `proc_pidinfo` nunca conserva o ponteiro após a chamada.
        let bytes = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                PROC_PIDLISTTHREADS,
                0,
                thread_ids.as_mut_ptr().cast::<libc::c_void>(),
                buffer_size,
            )
        };
        assert!(bytes >= 0, "proc_pidinfo lista as threads do processo");
        thread_ids.truncate(bytes as usize / std::mem::size_of::<u64>());

        let mut names = Vec::with_capacity(thread_ids.len());
        for thread_id in thread_ids {
            // SAFETY: `proc_threadinfo` é POD e será totalmente inicializado pela
            // chamada bem-sucedida abaixo antes de lermos `pth_name`.
            let mut info: libc::proc_threadinfo = unsafe { std::mem::zeroed() };
            let info_size = std::mem::size_of::<libc::proc_threadinfo>() as libc::c_int;
            // SAFETY: o buffer aponta para `info`, tem exatamente `info_size` bytes
            // e a API só escreve durante a chamada.
            let read = unsafe {
                libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDTHREADINFO,
                    thread_id,
                    std::ptr::addr_of_mut!(info).cast::<libc::c_void>(),
                    info_size,
                )
            };
            if read != info_size {
                continue;
            }
            let name_end = info
                .pth_name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(info.pth_name.len());
            let name_bytes = info.pth_name[..name_end]
                .iter()
                .map(|byte| *byte as u8)
                .collect::<Vec<_>>();
            let name = String::from_utf8_lossy(&name_bytes).into_owned();
            names.push(if name.is_empty() {
                format!("<unnamed:{thread_id}>")
            } else {
                name
            });
        }
        names.sort();
        names
    }

    #[cfg(not(target_os = "macos"))]
    fn process_thread_names() -> Vec<String> {
        Vec::new()
    }

    fn process_listener_count() -> usize {
        // `lsof` usa exit 1 para uma seleção vazia. Primeiro provamos que ele
        // consegue inspecionar este processo; só então exit 1 + saída vazia do
        // filtro significa legitimamente “zero listeners”, não medição vacuosa.
        let pid = std::process::id().to_string();
        let probe = std::process::Command::new("/usr/sbin/lsof")
            .args(["-nP", "-p", &pid])
            .output()
            .expect("lsof disponível para validar a inspeção do processo");
        assert!(
            probe.status.success()
                && String::from_utf8_lossy(&probe.stdout)
                    .lines()
                    .next()
                    .is_some_and(|header| header.starts_with("COMMAND")),
            "lsof precisa inspecionar o processo: status={} stderr={}",
            probe.status,
            String::from_utf8_lossy(&probe.stderr)
        );
        let output = std::process::Command::new("/usr/sbin/lsof")
            .args(["-nP", "-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN"])
            .output()
            .expect("lsof disponível para medir listeners");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            return stdout.lines().skip(1).count();
        }
        if output.status.code() == Some(1) && stdout.trim().is_empty() && stderr.trim().is_empty() {
            return 0;
        }
        panic!(
            "lsof precisa medir listeners: status={} stdout={stdout} stderr={stderr}",
            output.status
        );
    }

    fn resource_sample(runtimes: usize) -> ResourceSample {
        let listeners = bridge::runtime_listener_stats();
        let flush = lina_core::scrollback::flush_coordinator_stats();
        ResourceSample {
            fds: std::fs::read_dir("/dev/fd")
                .expect("/dev/fd disponível")
                .count(),
            threads: process_thread_count(),
            listeners: process_listener_count(),
            hook_listeners: listeners.hook_listeners,
            webhook_listeners: listeners.webhook_listeners,
            pump_handles: listeners.pump_handles,
            flush_registrations: flush.registrations,
            flush_threads: flush.threads,
            flush_signal_registrations: flush.signal_registrations,
            flush_handlers_installed: flush.handlers_installed,
            runtimes,
        }
    }

    fn assert_live_listener_inventory(sample: ResourceSample, phase: &str) {
        assert_eq!(
            sample.hook_listeners, 1,
            "{phase}: o listener global de hooks precisa estar realmente vivo"
        );
        assert_eq!(
            sample.webhook_listeners, sample.runtimes,
            "{phase}: cada runtime montado precisa manter seu webhook vivo"
        );
        assert_eq!(
            sample.listeners,
            sample.hook_listeners + sample.webhook_listeners,
            "{phase}: o inventário interno precisa corresponder aos listeners TCP observados pelo lsof"
        );
    }

    fn wait_for_grid_marker(grid: &Grid, marker: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let marker_visible = {
                let grid = lock(grid);
                let (_, rows) = grid.dims();
                (0..rows).any(|row| grid.row_text(row).contains(marker))
            };
            if marker_visible {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn is_per_workspace_thread(name: &str) -> bool {
        [
            "lina-mailbox-pump",
            "lina-broker-pump",
            "lina-bridge-pump",
            "lina-reader-",
            "lina-kill-",
            "lina-bus-writer-",
            "lina-pty-reader-",
            "lina-sync-watchdog-",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    }

    #[cfg(target_os = "macos")]
    fn assert_only_process_global_threads_remain(
        baseline_names: &[String],
        final_names: &[String],
        hook_listeners: usize,
    ) {
        let named_count = |expected: &str| {
            final_names
                .iter()
                .filter(|name| name.as_str() == expected)
                .count()
        };
        assert_eq!(named_count("lina-session-hub"), 1);
        assert_eq!(named_count("lina-scrollback-flush-coordinator"), 1);
        assert_eq!(named_count("tokio-rt-worker"), hook_listeners);

        // `sample` identificou a única thread sem nome como o workqueue ocioso do
        // libdispatch (`start_wqthread`), criado uma vez pelo macOS após o primeiro PTY.
        let unnamed = final_names
            .iter()
            .filter(|name| name.starts_with("<unnamed:"))
            .count();
        assert!(
            unnamed <= 1,
            "no máximo um workqueue global: {final_names:?}"
        );
        assert!(
            final_names.iter().all(|name| {
                baseline_names.contains(name)
                    || matches!(
                        name.as_str(),
                        "lina-session-hub"
                            | "lina-scrollback-flush-coordinator"
                            | "tokio-rt-worker"
                    )
                    || name.starts_with("<unnamed:")
            }),
            "nenhuma thread por-workspace pode sobreviver: {final_names:?}"
        );
        assert!(
            final_names.len() <= baseline_names.len() + 3 + hook_listeners,
            "somente serviços globais podem permanecer: {baseline_names:?} → {final_names:?}"
        );
    }

    #[cfg(not(target_os = "macos"))]
    fn assert_only_process_global_threads_remain(
        _baseline_names: &[String],
        _final_names: &[String],
        _hook_listeners: usize,
    ) {
    }

    fn runtime_cleanup_inventory_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let root = home.join("espacos").join("unico");
        lina_core::Workspace::create(&root, "Único", "blank", None)
            .expect("workspace fixture completo");
        let baseline = resource_sample(0);
        let baseline_names = process_thread_names();
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty: Arc::clone(&pty),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let runtime = boot_ws_runtime(root, &shared, false).expect("boot real");
        let node = runtime.nodes.add_node().expect("PTY de controle");
        runtime
            .sup
            .set_status(node, lina_core::NodeStatus::Busy)
            .expect("marcar Busy");
        let runtimes = runtimes_with_boot(runtime);
        stop_all(&runtimes);
        for runtime in lock(&runtimes).map.values() {
            runtime.nodes.shutdown_terminals_after_failed_boot();
        }
        drop(runtimes);
        assert_eq!(lock(&pty).len(), 0);

        let deadline = Instant::now() + Duration::from_secs(15);
        let (final_sample, final_names) = loop {
            let sample = resource_sample(0);
            let names = process_thread_names();
            let per_workspace_names_gone = names.iter().all(|name| !is_per_workspace_thread(name));
            let process_globals_only = sample.threads
                <= baseline.threads + 3 + sample.hook_listeners
                && sample.fds <= baseline.fds + 4
                && sample.webhook_listeners == 0
                && sample.pump_handles == 0
                && sample.listeners == sample.hook_listeners
                && names
                    .iter()
                    .filter(|name| name.starts_with("<unnamed:"))
                    .count()
                    <= 1;
            if (per_workspace_names_gone && process_globals_only) || Instant::now() >= deadline {
                break (sample, names);
            }
            thread::sleep(Duration::from_millis(25));
        };
        eprintln!(
            "workspace-thread-inventory: baseline={baseline:?} names={baseline_names:?} final={final_sample:?} names={final_names:?}"
        );
        assert_eq!(final_sample.pump_handles, 0);
        assert_eq!(final_sample.flush_registrations, 0);
        assert_eq!(final_sample.flush_threads, 1);
        assert_eq!(final_sample.webhook_listeners, 0);
        assert_eq!(final_sample.listeners, final_sample.hook_listeners);
        assert!(final_sample.fds <= baseline.fds + 4);
        assert!(
            final_sample.threads <= baseline.threads + 3 + final_sample.hook_listeners,
            "somente serviços globais permanecem: {baseline:?} → {final_sample:?}"
        );
        assert!(final_names
            .iter()
            .all(|name| !is_per_workspace_thread(name)));
        assert_only_process_global_threads_remain(
            &baseline_names,
            &final_names,
            final_sample.hook_listeners,
        );
    }

    #[test]
    fn workspace_reliability_runtime_cleanup_thread_inventory() {
        const CHILD: &str = "LINA_WORKSPACE_THREAD_INVENTORY_CHILD";
        if std::env::var_os(CHILD).is_some() {
            runtime_cleanup_inventory_child();
            return;
        }
        let home = std::env::temp_dir().join(format!(
            "lina-runtime-thread-inventory-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado");
        let output = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args([
                "--exact",
                "runtime::tests::workspace_reliability_runtime_cleanup_thread_inventory",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LINA_NO_RESTORE", "1")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .output()
            .expect("subprocesso do inventário");
        let _ = std::fs::remove_dir_all(&home);
        assert!(
            output.status.success(),
            "inventário isolado falhou: status={} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }

    fn runtime_resource_soak_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let roots: Vec<PathBuf> = (0..20)
            .map(|index| {
                let root = home.join("espacos").join(format!("ws-{index:02}"));
                lina_core::Workspace::create(&root, &format!("WS {index:02}"), "blank", None)
                    .expect("workspace fixture completo");
                root
            })
            .collect();
        let baseline = resource_sample(0);
        let baseline_thread_names = process_thread_names();
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty: Arc::clone(&pty),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let first = boot_ws_runtime(roots[0].clone(), &shared, false).expect("boot inicial");
        let busy = first.nodes.add_node().expect("PTY ocupado de controle");
        first
            .sup
            .set_status(busy, lina_core::NodeStatus::Busy)
            .expect("marcar Busy");
        let busy_root = first.ws_root.clone();
        let runtimes = runtimes_with_boot(first);

        // Um segundo runtime nasce `Running`, ainda na janela de quietude. Ao encher
        // o mapa ele não pode bloquear a busca por outro candidato vazio; depois da
        // janela ele deve ser estacionado e liberar seus PTYs.
        activate_workspace(&runtimes, roots[1].clone(), &shared).expect("visitar ws-01");
        let quiet_prompt = {
            let map = lock(&runtimes);
            let runtime = map.map.get(&roots[1]).expect("ws-01 montado");
            let node = runtime.nodes.add_node().expect("PTY parado no prompt");
            let grid = lock(&runtime.grids)
                .get(&node)
                .cloned()
                .expect("grid do prompt");
            lock(&grid).advance(b"> ");
            runtime
                .sup
                .set_status(node, lina_core::NodeStatus::Running)
                .expect("Running");
            node
        };
        for root in roots.iter().take(4).skip(2) {
            activate_workspace(&runtimes, root.clone(), &shared).expect("visitar workspace");
            assert!(lock(&runtimes).map.len() <= MAX_MOUNTED_WORKSPACES);
        }
        assert!(
            lock(&runtimes).map.contains_key(&roots[1]),
            "Running recente não foi morto"
        );
        assert_eq!(lock(&pty).len(), 2, "Busy + prompt recente vivos");
        thread::sleep(Duration::from_millis(2_100));
        for root in roots.iter().skip(4) {
            activate_workspace(&runtimes, root.clone(), &shared).expect("visitar workspace");
            assert!(lock(&runtimes).map.len() <= MAX_MOUNTED_WORKSPACES);
        }
        assert!(lock(&runtimes).map.contains_key(&busy_root));
        assert!(
            !lock(&runtimes).map.contains_key(&roots[1]),
            "prompt quieto foi evictado"
        );
        let kill_deadline = std::time::Instant::now() + Duration::from_secs(3);
        while lock(&pty).len() > 1 && std::time::Instant::now() < kill_deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(lock(&pty).len(), 1, "só o PTY Busy original segue vivo");
        assert!(
            lock(&runtimes)
                .map
                .values()
                .all(|runtime| runtime.sup.get(quiet_prompt).is_none()),
            "geração quieta saiu dos runtimes montados"
        );

        assert_eq!(lock(&runtimes).map.len(), MAX_MOUNTED_WORKSPACES);
        let mut midpoint = None;
        for turn in 0..1_000 {
            let target = roots[(turn * 17 + 3) % roots.len()].clone();
            activate_workspace(&runtimes, target, &shared).expect("switch entre 20 Espaços");
            assert!(lock(&runtimes).map.len() <= MAX_MOUNTED_WORKSPACES);
            if turn == 499 {
                midpoint = Some(resource_sample(lock(&runtimes).map.len()));
            }
        }
        let end = resource_sample(lock(&runtimes).map.len());
        assert_eq!(end.runtimes, MAX_MOUNTED_WORKSPACES);
        let midpoint = midpoint.expect("amostra 500");
        eprintln!("workspace-soak: baseline={baseline:?} mid={midpoint:?} end={end:?}");
        assert_eq!(midpoint.runtimes, MAX_MOUNTED_WORKSPACES);
        assert_live_listener_inventory(midpoint, "amostra após 500 trocas");
        assert_live_listener_inventory(end, "amostra após 1.000 trocas");
        {
            let mounted = lock(&runtimes);
            assert!(
                mounted.map.values().all(|runtime| runtime.hooks.is_some()),
                "todo runtime montado precisa estar inscrito no listener global de hooks"
            );
            assert!(
                mounted
                    .map
                    .values()
                    .all(|runtime| runtime.webhook.is_some()),
                "todo runtime montado precisa possuir seu webhook"
            );
        }
        assert!(
            end.fds <= midpoint.fds + 4,
            "FDs precisam atingir platô: {midpoint:?} → {end:?}"
        );
        assert!(
            end.fds <= 96,
            "reserva mínima de 32 sob ulimit 128: {end:?}"
        );
        assert!(
            end.threads <= midpoint.threads + 2,
            "threads sem crescimento linear: {midpoint:?} → {end:?}"
        );
        assert!(
            end.listeners <= midpoint.listeners,
            "listeners não crescem depois do aquecimento: {midpoint:?} → {end:?}"
        );
        assert_eq!(midpoint.pump_handles, MAX_MOUNTED_WORKSPACES * 3);
        assert_eq!(end.pump_handles, MAX_MOUNTED_WORKSPACES * 3);
        assert_eq!(midpoint.flush_registrations, MAX_MOUNTED_WORKSPACES);
        assert_eq!(end.flush_registrations, MAX_MOUNTED_WORKSPACES);
        assert!(midpoint.flush_threads <= 1 && end.flush_threads <= 1);
        assert_eq!(midpoint.flush_threads, end.flush_threads);
        assert_eq!(midpoint.flush_threads, 1, "coordenador global único vivo");
        assert_eq!(
            midpoint.flush_signal_registrations, end.flush_signal_registrations,
            "handlers fatais globais não multiplicam"
        );
        assert_eq!(
            midpoint.flush_handlers_installed,
            end.flush_handlers_installed
        );
        assert!(
            !midpoint.flush_handlers_installed && !end.flush_handlers_installed,
            "binário de teste não instala handlers fatais do processo"
        );
        assert!(lock(&runtimes)
            .map
            .get(&busy_root)
            .is_some_and(|runtime| runtime
                .sup
                .get(busy)
                .is_some_and(|node| { node.status == lina_core::NodeStatus::Busy })));

        let (busy_supervisor, busy_grid) = {
            let mounted = lock(&runtimes);
            let runtime = mounted
                .map
                .get(&busy_root)
                .expect("runtime Busy preservado após 1.000 trocas");
            let grid = lock(&runtime.grids)
                .get(&busy)
                .cloned()
                .expect("grid do agente Busy preservado");
            (Arc::clone(&runtime.sup), grid)
        };
        let busy_marker = format!("LINA_BUSY_AFTER_1000_SWITCHES_{}", std::process::id());
        let mut busy_input = busy_marker.as_bytes().to_vec();
        busy_input.push(b'\r');
        busy_supervisor
            .write_human(busy, &busy_input, Duration::from_secs(1))
            .expect("o PTY cat do agente Busy continua aceitando trabalho");
        assert!(
            wait_for_grid_marker(&busy_grid, &busy_marker, Duration::from_secs(3)),
            "o marcador enviado após 1.000 trocas precisa aparecer no grid do agente Busy"
        );

        stop_all(&runtimes);
        for runtime in lock(&runtimes).map.values() {
            runtime.nodes.shutdown_terminals_after_failed_boot();
        }
        drop(runtimes);
        assert_eq!(lock(&pty).len(), 0, "cleanup encerrou o PTY de controle");

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let (final_sample, final_thread_names) = loop {
            let final_sample = resource_sample(0);
            let final_thread_names = process_thread_names();
            let resources_released = final_sample.fds <= baseline.fds + 4
                && final_sample.threads <= baseline.threads + 3 + final_sample.hook_listeners
                && final_sample.flush_registrations == 0
                && final_sample.flush_signal_registrations == 0
                && !final_sample.flush_handlers_installed
                && final_sample.flush_threads == 1
                && final_sample.hook_listeners == 1
                && final_sample.webhook_listeners == 0
                && final_sample.pump_handles == 0
                && final_sample.listeners == final_sample.hook_listeners;
            if resources_released {
                break (final_sample, final_thread_names);
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "recursos por-Espaço retornam ao baseline: {baseline:?} → {final_sample:?}; threads={final_thread_names:?}"
                );
            }
            thread::sleep(Duration::from_millis(25));
        };
        assert_only_process_global_threads_remain(
            &baseline_thread_names,
            &final_thread_names,
            final_sample.hook_listeners,
        );
    }

    #[test]
    fn workspace_reliability_runtime_resources_reach_plateau() {
        const CHILD: &str = "LINA_WORKSPACE_RESOURCE_SOAK_CHILD";
        if std::env::var_os(CHILD).is_some() {
            runtime_resource_soak_child();
            return;
        }

        let home = std::env::temp_dir().join(format!(
            "lina-runtime-soak-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado");
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args([
                "--exact",
                "runtime::tests::workspace_reliability_runtime_resources_reach_plateau",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LINA_NO_RESTORE", "1")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .status()
            .expect("subprocesso do soak");
        let _ = std::fs::remove_dir_all(&home);
        assert!(status.success(), "soak isolado falhou: {status}");
    }

    fn switch_failure_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let failing_root = home.join("espacos").join("restore-falha");
        lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
            .expect("workspace ativo completo");
        lina_core::Workspace::create(&failing_root, "Falha", "blank", None)
            .expect("workspace alvo completo");

        let previous_node = uuid::Uuid::now_v7();
        let mut failing_store =
            EventStore::open(lina_core::Workspace::events_dir(&failing_root)).expect("store alvo");
        failing_store
            .append(&lina_core::DomainEvent::NodeAdded {
                node: previous_node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("terminal anterior");
        failing_store
            .append(&lina_core::DomainEvent::NodeRenamed {
                node: previous_node,
                name: "Terminal A".into(),
            })
            .expect("nome anterior");
        drop(failing_store);

        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty: Arc::clone(&pty),
            cmd_factory: Arc::new(|_| {
                lina_core::PtyCommand::new("/definitely/missing/lina-restore-engine")
            }),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("catálogo inicial");
        let runtimes = runtimes_with_boot(active);
        let fds_before = open_fd_count();
        let resources_before = internal_resource_counts();
        let active_store_before = {
            let map = lock(&runtimes);
            Arc::clone(&map.map.get(&active_root).expect("runtime ativo").store)
        };
        let active_projection_before = format!(
            "{:?}",
            lock(&active_store_before)
                .project()
                .expect("projeção ativa antes")
        );

        let error = activate_workspace(&runtimes, failing_root.clone(), &shared)
            .expect_err("restore incompleto precisa abortar a troca");
        assert!(
            error.contains("0/1 agentes restaurados"),
            "erro explica que o alvo não ficou pronto: {error}"
        );
        let map = lock(&runtimes);
        assert_eq!(map.active, active_root, "o foco em memória não mudou");
        assert_eq!(map.map.len(), 1, "runtime parcial não foi publicado");
        assert!(map.map.contains_key(&map.active));
        drop(map);
        assert_eq!(lock(&pty).len(), 0, "nenhum PTY parcial sobreviveu");
        assert_eq!(
            internal_resource_counts(),
            resources_before,
            "pumps/listeners/flush do boot parcial foram encerrados e aguardados"
        );
        {
            let map = lock(&runtimes);
            let surviving = map.map.get(&active_root).expect("ativo sobrevive");
            assert!(
                Arc::ptr_eq(&surviving.store, &active_store_before),
                "a falha não troca o store do Espaço atual por fallback"
            );
            assert_eq!(
                lock(&surviving.store).dir(),
                lina_core::Workspace::events_dir(&active_root),
                "cada raiz mantém seu store próprio"
            );
        }
        assert_eq!(
            format!(
                "{:?}",
                lock(&active_store_before)
                    .project()
                    .expect("projeção ativa depois")
            ),
            active_projection_before,
            "falha no alvo não escreve no store do Espaço atual"
        );

        let registry = lina_core::WorkspaceRegistry::load(
            lina_core::default_registry_path().expect("registry isolado"),
        )
        .expect("reabrir registry");
        assert_eq!(
            registry.focused().map(|entry| entry.path.as_path()),
            Some(active_root.as_path()),
            "o foco durável só muda depois de o alvo ficar pronto"
        );
        let failing_entry = registry
            .entries()
            .iter()
            .find(|entry| entry.path == failing_root)
            .expect("a reconciliação pode descobrir o store completo antes do boot");
        assert_eq!(
            failing_entry.last_focus, 0,
            "o boot parcial não publica foco no alvo descoberto"
        );

        if let Some(before) = fds_before {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                let after = open_fd_count().expect("FDs depois da falha");
                if after <= before + 2 || std::time::Instant::now() >= deadline {
                    assert!(
                        after <= before + 2,
                        "listeners/stores do boot parcial foram encerrados: {before} -> {after}"
                    );
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        stop_all(&runtimes);
    }

    #[test]
    fn workspace_reliability_switch_failure_keeps_current_ready_workspace() {
        const CHILD: &str = "LINA_WORKSPACE_SWITCH_FAILURE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            switch_failure_child();
            return;
        }

        let home = std::env::temp_dir().join(format!(
            "lina-switch-failure-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado");
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args([
                "--exact",
                "runtime::tests::workspace_reliability_switch_failure_keeps_current_ready_workspace",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("LINA_NO_RESTORE")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .status()
            .expect("subprocesso do teste de troca");
        let _ = std::fs::remove_dir_all(&home);
        assert!(status.success(), "teste isolado de troca falhou: {status}");
    }

    fn focus_commit_failure_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let target_root = home.join("espacos").join("alvo");
        drop(
            lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
                .expect("workspace ativo"),
        );
        drop(
            lina_core::Workspace::create(&target_root, "Alvo", "blank", None)
                .expect("workspace alvo"),
        );

        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty,
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("foco inicial confirmado");
        let runtimes = runtimes_with_boot(active);

        let registry_path = lina_core::default_registry_path().expect("registry isolado");
        std::fs::remove_file(&registry_path).expect("remover ponteiro para injetar falha");
        std::fs::create_dir(&registry_path).expect("registry ilegível determinístico");

        let error = activate_workspace(&runtimes, target_root.clone(), &shared)
            .expect_err("falha do catálogo precisa abortar a publicação do foco");
        assert!(
            error.contains("validar a lista de Espaços"),
            "erro do catálogo chega ao chamador: {error}"
        );
        let map = lock(&runtimes);
        assert_eq!(map.active, active_root, "o foco em memória não mudou");
        assert_eq!(map.map.len(), 1, "o runtime alvo não foi publicado");
        assert!(!map.map.contains_key(&target_root));
        drop(map);

        let target_events = EventStore::open(lina_core::Workspace::events_dir(&target_root))
            .expect("reabrir log alvo")
            .events()
            .expect("eventos alvo");
        assert!(
            target_events
                .iter()
                .all(|record| record.kind != "WorkspaceFocusSet"),
            "a validação inconclusiva falha antes de publicar qualquer foco no alvo"
        );
        stop_all(&runtimes);
    }

    #[test]
    fn workspace_reliability_registry_failure_never_publishes_new_focus() {
        const CHILD: &str = "LINA_WORKSPACE_FOCUS_COMMIT_FAILURE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            focus_commit_failure_child();
            return;
        }

        let home = std::env::temp_dir().join(format!(
            "lina-focus-commit-failure-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado");
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args([
                "--exact",
                "runtime::tests::workspace_reliability_registry_failure_never_publishes_new_focus",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LINA_NO_RESTORE", "1")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .status()
            .expect("subprocesso da falha de catálogo");
        let _ = std::fs::remove_dir_all(&home);
        assert!(status.success(), "teste isolado de foco falhou: {status}");
    }

    fn post_append_focus_failure_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let target_root = home.join("espacos").join("alvo");
        drop(
            lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
                .expect("workspace ativo"),
        );
        drop(
            lina_core::Workspace::create(&target_root, "Alvo", "blank", None)
                .expect("workspace alvo"),
        );

        let shared = SharedInfra {
            pty: Arc::new(Mutex::new(PtyManager::new())),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("foco inicial");
        let runtimes = runtimes_with_boot(active);
        let registry_path = lina_core::default_registry_path().expect("registry isolado");
        let registry_before = std::fs::read(&registry_path).expect("snapshot do registry");
        arm_focus_registry_commit_fault();
        activate_workspace(&runtimes, target_root.clone(), &shared)
            .expect("append confirmado não volta como falso erro");
        assert_eq!(
            lock(&runtimes).active,
            target_root,
            "o commit point publica o runtime alvo"
        );
        assert_eq!(
            std::fs::read(&registry_path).expect("registry ainda legível"),
            registry_before,
            "a falha injetada ocorreu antes do save do registry"
        );
        let target_store =
            EventStore::open(lina_core::Workspace::events_dir(&target_root)).expect("reabrir alvo");
        let committed_focuses = target_store
            .events()
            .expect("eventos alvo")
            .into_iter()
            .filter_map(|record| {
                serde_json::from_value::<lina_core::DomainEvent>(record.payload).ok()
            })
            .filter(|event| {
                matches!(
                    event,
                    lina_core::DomainEvent::WorkspaceFocusSet {
                        focus_id: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            committed_focuses, 1,
            "um único foco correlacionado foi apendado"
        );
        drop(target_store);

        let mut registry = lina_core::WorkspaceRegistry::load(&registry_path)
            .expect("registry anterior continua válido");
        let journal = registry.pending_focus_path();
        assert!(
            journal.exists(),
            "intent durável sobrevive à falha pós-append"
        );
        assert!(matches!(
            registry
                .recover_pending_focus()
                .expect("restart reconcilia o commit"),
            lina_core::WorkspaceFocusRecovery::Committed(
                lina_core::WorkspaceFocusCommit::Committed
            )
        ));
        assert_eq!(
            registry.focused().map(|entry| entry.path.as_path()),
            Some(target_root.as_path()),
            "restart converge o ponteiro para o evento confirmado"
        );
        assert!(
            !journal.exists(),
            "journal sai após a reconciliação durável"
        );
        assert!(matches!(
            registry
                .recover_pending_focus()
                .expect("segunda recuperação é idempotente"),
            lina_core::WorkspaceFocusRecovery::NoPending
        ));
        let final_focuses = EventStore::open(lina_core::Workspace::events_dir(&target_root))
            .expect("store final")
            .events()
            .expect("eventos finais")
            .into_iter()
            .filter_map(|record| {
                serde_json::from_value::<lina_core::DomainEvent>(record.payload).ok()
            })
            .filter(|event| {
                matches!(
                    event,
                    lina_core::DomainEvent::WorkspaceFocusSet {
                        focus_id: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(final_focuses, 1, "recovery não duplica WorkspaceFocusSet");

        stop_all(&runtimes);
        for runtime in lock(&runtimes).map.values() {
            runtime.nodes.shutdown_terminals_after_failed_boot();
        }
    }

    #[test]
    fn workspace_reliability_post_append_focus_failure_recovers_exactly_once() {
        const CHILD: &str = "LINA_POST_APPEND_FOCUS_FAILURE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            post_append_focus_failure_child();
            return;
        }
        let home = std::env::temp_dir().join(format!(
            "lina-focus-post-append-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado");
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args([
                "--exact",
                "runtime::tests::workspace_reliability_post_append_focus_failure_recovers_exactly_once",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LINA_NO_RESTORE", "1")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .status()
            .expect("subprocesso da janela pós-append");
        let _ = std::fs::remove_dir_all(home);
        assert!(status.success(), "teste isolado de foco falhou: {status}");
    }

    fn stale_registry_pointer_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let target_root = home.join("espacos").join("alvo");
        drop(
            lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
                .expect("workspace ativo"),
        );
        drop(
            lina_core::Workspace::create(&target_root, "Alvo", "blank", None)
                .expect("workspace alvo"),
        );
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty,
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("foco inicial");
        let runtimes = runtimes_with_boot(active);

        let registry_path = lina_core::default_registry_path().expect("registry isolado");
        let mut registry_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&registry_path).expect("ler registry"))
                .expect("registry JSON");
        let entries = registry_json["workspaces"]
            .as_array_mut()
            .expect("lista de workspaces");
        let target_path = target_root.to_string_lossy();
        let target = entries
            .iter_mut()
            .find(|entry| entry["path"].as_str() == Some(target_path.as_ref()))
            .expect("ponteiro alvo");
        target["id"] = serde_json::Value::String("id-divergente-injetado".to_string());
        std::fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry_json).expect("serializar conflito"),
        )
        .expect("injetar conflito");
        activate_workspace(&runtimes, target_root.clone(), &shared)
            .expect("um ponteiro stale isolado é reparado pelo store autoritativo");
        let map = lock(&runtimes);
        assert_eq!(map.active, target_root);
        assert_eq!(map.map.len(), 2);
        assert!(map.map.contains_key(&target_root));
        drop(map);

        let canonical = lina_core::WorkspaceRegistry::rederive_entry(&target_root)
            .expect("rederivar alvo")
            .id;
        let repaired =
            lina_core::WorkspaceRegistry::load(&registry_path).expect("registry reparado e focado");
        let target_entries = repaired
            .entries()
            .iter()
            .filter(|entry| entry.path == target_root)
            .collect::<Vec<_>>();
        assert_eq!(
            target_entries.len(),
            1,
            "o reparo não duplica o ponteiro alvo"
        );
        assert_eq!(
            target_entries[0].id, canonical,
            "a identidade do store substitui o ID stale"
        );
        assert_eq!(
            repaired.focused().map(|entry| entry.path.as_path()),
            Some(target_root.as_path()),
            "o alvo reparado recebe o foco confirmado"
        );
        stop_all(&runtimes);
    }

    #[test]
    fn workspace_reliability_stale_registry_pointer_is_repaired_before_focus() {
        const CHILD: &str = "LINA_WORKSPACE_STALE_REGISTRY_POINTER_CHILD";
        if std::env::var_os(CHILD).is_some() {
            stale_registry_pointer_child();
            return;
        }
        run_isolated_switch_recovery_test(
            "runtime::tests::workspace_reliability_stale_registry_pointer_is_repaired_before_focus",
            CHILD,
        );
    }

    fn ambiguous_registry_pointer_child() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let target_root = home.join("espacos").join("alvo");
        drop(
            lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
                .expect("workspace ativo"),
        );
        drop(
            lina_core::Workspace::create(&target_root, "Alvo", "blank", None)
                .expect("workspace alvo"),
        );
        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty,
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("foco inicial");
        let runtimes = runtimes_with_boot(active);

        let registry_path = lina_core::default_registry_path().expect("registry isolado");
        let mut registry_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&registry_path).expect("ler registry"))
                .expect("registry JSON");
        let entries = registry_json["workspaces"]
            .as_array_mut()
            .expect("lista de workspaces");
        let target_path = target_root.to_string_lossy();
        let mut duplicate = entries
            .iter()
            .find(|entry| entry["path"].as_str() == Some(target_path.as_ref()))
            .expect("ponteiro alvo")
            .clone();
        duplicate["id"] = serde_json::Value::String("id-duplicado-injetado".to_string());
        entries.push(duplicate);
        std::fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry_json).expect("serializar ambiguidade"),
        )
        .expect("injetar ponteiro duplicado");
        let target_count_before = EventStore::open(lina_core::Workspace::events_dir(&target_root))
            .expect("store alvo")
            .event_count()
            .expect("count alvo");

        let error = activate_workspace(&runtimes, target_root.clone(), &shared)
            .expect_err("dois ponteiros para o mesmo path precisam bloquear a troca");
        assert!(
            error.contains("ambígua") || error.contains("isolou"),
            "erro explica a quarentena: {error}"
        );
        let map = lock(&runtimes);
        assert_eq!(map.active, active_root);
        assert_eq!(map.map.len(), 1);
        assert!(!map.map.contains_key(&target_root));
        drop(map);
        let reconciled = lina_core::WorkspaceRegistry::load(&registry_path)
            .expect("registry saudável após quarentena");
        assert!(
            reconciled
                .entries()
                .iter()
                .all(|entry| entry.path != target_root),
            "ponteiros ambíguos são removidos do catálogo sem ressuscitar o alvo"
        );
        assert!(
            reconciled
                .entries()
                .iter()
                .any(|entry| entry.path == active_root),
            "o reparo preserva ponteiros saudáveis"
        );
        assert_eq!(
            EventStore::open(lina_core::Workspace::events_dir(&target_root))
                .expect("reabrir alvo")
                .event_count()
                .expect("count final"),
            target_count_before,
            "a falha acontece antes de apendar foco no alvo"
        );
        stop_all(&runtimes);
    }

    #[test]
    fn workspace_reliability_ambiguous_registry_pointer_preserves_current_focus() {
        const CHILD: &str = "LINA_WORKSPACE_AMBIGUOUS_REGISTRY_POINTER_CHILD";
        if std::env::var_os(CHILD).is_some() {
            ambiguous_registry_pointer_child();
            return;
        }
        run_isolated_switch_recovery_test(
            "runtime::tests::workspace_reliability_ambiguous_registry_pointer_preserves_current_focus",
            CHILD,
        );
    }

    fn remove_sqlite_files(events_dir: &Path) {
        for name in ["lina.db", "lina.db-wal", "lina.db-shm"] {
            match std::fs::remove_file(events_dir.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("remover {name}: {error}"),
            }
        }
    }

    #[derive(Clone, Copy)]
    enum SwitchRecoveryCase {
        MissingDatabase,
        CorruptDatabase,
        AmbiguousMirror,
    }

    fn switch_recovery_child(case: SwitchRecoveryCase) {
        use std::io::Write as _;

        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let target_root = home.join("espacos").join(match case {
            SwitchRecoveryCase::MissingDatabase => "espelho-recuperavel",
            SwitchRecoveryCase::CorruptDatabase => "sqlite-corrompido",
            SwitchRecoveryCase::AmbiguousMirror => "espelho-ambiguo",
        });
        drop(
            lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
                .expect("workspace ativo"),
        );
        drop(
            lina_core::Workspace::create(&target_root, "Alvo", "blank", None)
                .expect("workspace alvo"),
        );
        let events_dir = lina_core::Workspace::events_dir(&target_root);
        if matches!(case, SwitchRecoveryCase::AmbiguousMirror) {
            let original = EventStore::open(&events_dir)
                .expect("store alvo")
                .events()
                .expect("eventos alvo")[0]
                .clone();
            let mut forged = original;
            forged.payload = serde_json::json!({"event":"WorkspaceCreated","name":"Divergente","focus_preset":"blank"});
            let mut mirror = std::fs::OpenOptions::new()
                .append(true)
                .open(events_dir.join("log.jsonl"))
                .expect("espelho");
            writeln!(
                mirror,
                "{}",
                serde_json::to_string(&forged).expect("registro forjado")
            )
            .expect("conflito no espelho");
            mirror.sync_data().expect("persistir conflito");
        }
        remove_sqlite_files(&events_dir);
        if matches!(case, SwitchRecoveryCase::CorruptDatabase) {
            std::fs::write(events_dir.join("lina.db"), b"sqlite-corrompido-injetado")
                .expect("corromper SQLite");
        }

        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty: Arc::clone(&pty),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("publicar ativo");
        let runtimes = runtimes_with_boot(active);

        if matches!(case, SwitchRecoveryCase::AmbiguousMirror) {
            let error = activate_workspace(&runtimes, target_root.clone(), &shared)
                .expect_err("espelho conflitante deve ser recusado");
            assert!(
                error.contains("recuperável")
                    || error.contains("recuperar")
                    || error.contains("completos e verificados"),
                "erro humano descreve a recuperação recusada: {error}"
            );
            let map = lock(&runtimes);
            assert_eq!(map.active, active_root, "o Espaço atual permanece em foco");
            assert!(!map.map.contains_key(&target_root));
        } else {
            activate_workspace(&runtimes, target_root.clone(), &shared)
                .expect("espelho íntegro recupera antes da validação");
            assert_eq!(lock(&runtimes).active, target_root);
            assert!(
                events_dir.join("lina.db").exists(),
                "SQLite foi reconstruído"
            );
            if matches!(case, SwitchRecoveryCase::CorruptDatabase) {
                assert!(
                    std::fs::read_dir(&events_dir)
                        .expect("listar preservados")
                        .any(|entry| entry.is_ok_and(|entry| entry
                            .file_name()
                            .to_string_lossy()
                            .contains(".corrupt-"))),
                    "o SQLite inválido foi preservado antes da reconstrução"
                );
            }
        }

        stop_all(&runtimes);
        for runtime in lock(&runtimes).map.values() {
            runtime.nodes.shutdown_terminals_after_failed_boot();
        }
    }

    fn run_isolated_switch_recovery_test(test_name: &str, child_env: &str) {
        let home = std::env::temp_dir().join(format!(
            "lina-switch-recovery-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado");
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args(["--exact", test_name, "--test-threads=1", "--nocapture"])
            .env(child_env, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LINA_NO_RESTORE", "1")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .status()
            .expect("subprocesso de recuperação");
        let _ = std::fs::remove_dir_all(home);
        assert!(status.success(), "teste isolado falhou: {status}");
    }

    #[test]
    fn workspace_reliability_switch_recovers_valid_jsonl_before_identity_validation() {
        const CHILD: &str = "LINA_SWITCH_RECOVER_VALID_CHILD";
        if std::env::var_os(CHILD).is_some() {
            switch_recovery_child(SwitchRecoveryCase::MissingDatabase);
            return;
        }
        run_isolated_switch_recovery_test(
            "runtime::tests::workspace_reliability_switch_recovers_valid_jsonl_before_identity_validation",
            CHILD,
        );
    }

    #[test]
    fn workspace_reliability_switch_recovers_corrupt_store_before_focus() {
        const CHILD: &str = "LINA_SWITCH_RECOVER_CORRUPT_CHILD";
        if std::env::var_os(CHILD).is_some() {
            switch_recovery_child(SwitchRecoveryCase::CorruptDatabase);
            return;
        }
        run_isolated_switch_recovery_test(
            "runtime::tests::workspace_reliability_switch_recovers_corrupt_store_before_focus",
            CHILD,
        );
    }

    #[test]
    fn workspace_reliability_ambiguous_jsonl_keeps_current_workspace() {
        const CHILD: &str = "LINA_SWITCH_REJECT_AMBIGUOUS_CHILD";
        if std::env::var_os(CHILD).is_some() {
            switch_recovery_child(SwitchRecoveryCase::AmbiguousMirror);
            return;
        }
        run_isolated_switch_recovery_test(
            "runtime::tests::workspace_reliability_ambiguous_jsonl_keeps_current_workspace",
            CHILD,
        );
    }

    #[test]
    fn workspace_reliability_runtime_plan_evicts_quiet_prompt_but_keeps_real_work() {
        let node = |name: &str, status| lina_core::NodeInfo {
            id: uuid::Uuid::now_v7(),
            name: name.to_string(),
            role: None,
            status,
        };
        let idle = node("Idle", lina_core::NodeStatus::Idle);
        let ready = node("Ready", lina_core::NodeStatus::Ready);
        let running = node("Running", lina_core::NodeStatus::Running);
        let starting = node("Starting", lina_core::NodeStatus::Starting);
        let busy = node("Busy", lina_core::NodeStatus::Busy);
        let blocked = node("Blocked", lina_core::NodeStatus::Blocked);
        let trigger = node("@Trigger", lina_core::NodeStatus::Starting);

        let plan = runtime_unload_plan(&[
            idle.clone(),
            ready.clone(),
            running.clone(),
            starting.clone(),
            busy.clone(),
            blocked.clone(),
            trigger,
        ]);
        assert_eq!(plan.park, vec![idle.id, ready.id, running.id]);
        assert_eq!(plan.keep, vec![starting.id, busy.id, blocked.id]);
    }

    #[test]
    fn workspace_reliability_switch_validation_rejects_dead_and_partial_targets() {
        let base = std::env::temp_dir().join(format!(
            "lina-runtime-invalid-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        let dead = base.join("ausente");
        assert!(verify_workspace_target(&dead).is_err());
        assert!(
            !lina_core::Workspace::events_dir(&dead).exists(),
            "ponteiro morto não cria store durante a validação"
        );

        let partial = base.join("parcial");
        let mut store =
            EventStore::open(lina_core::Workspace::events_dir(&partial)).expect("store parcial");
        store
            .append(&lina_core::DomainEvent::WorkspaceCreated {
                name: "Parcial".into(),
                focus_preset: "dev_app".into(),
            })
            .expect("evento parcial");
        drop(store);
        assert!(verify_workspace_target(&partial).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_switch_validation_accepts_complete_unregistered_target() {
        let base = std::env::temp_dir().join(format!(
            "lina-runtime-complete-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        let root = base.join("completo");
        lina_core::Workspace::create(&root, "Completo", "dev_app", Some(&base))
            .expect("workspace completo");

        verify_workspace_target(&root).expect("o store é autoridade mesmo sem registry");
        let _ = std::fs::remove_dir_all(base);
    }

    fn run_gate_f_case(test_name: &str, child_env: &str, disable_restore: bool) {
        let home = std::env::temp_dir().join(format!(
            "lina-gate-f-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&home).expect("HOME isolado do caso F");
        let mut command = std::process::Command::new(std::env::current_exe().expect("test exe"));
        command
            .args(["--exact", test_name, "--test-threads=1", "--nocapture"])
            .env(child_env, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO");
        if disable_restore {
            command.env("LINA_NO_RESTORE", "1");
        } else {
            command.env_remove("LINA_NO_RESTORE");
        }
        let status = command.status().expect("subprocesso do caso F");
        let _ = std::fs::remove_dir_all(home);
        assert!(status.success(), "caso F falhou ({test_name}): {status}");
    }

    #[test]
    fn workspace_reliability_switch_recovery_matrix() {
        let cases = [
            (
                "runtime::tests::workspace_reliability_switch_validation_rejects_dead_and_partial_targets",
                "LINA_GATE_F_DEAD_PARTIAL_CHILD",
                true,
            ),
            (
                "runtime::tests::workspace_reliability_stale_registry_pointer_is_repaired_before_focus",
                "LINA_WORKSPACE_STALE_REGISTRY_POINTER_CHILD",
                true,
            ),
            (
                "runtime::tests::workspace_reliability_switch_recovers_corrupt_store_before_focus",
                "LINA_SWITCH_RECOVER_CORRUPT_CHILD",
                true,
            ),
            (
                "runtime::tests::workspace_reliability_switch_failure_keeps_current_ready_workspace",
                "LINA_WORKSPACE_SWITCH_FAILURE_CHILD",
                false,
            ),
            (
                "runtime::tests::workspace_reliability_registry_failure_never_publishes_new_focus",
                "LINA_WORKSPACE_FOCUS_COMMIT_FAILURE_CHILD",
                true,
            ),
            (
                "runtime::tests::workspace_reliability_post_append_focus_failure_recovers_exactly_once",
                "LINA_POST_APPEND_FOCUS_FAILURE_CHILD",
                true,
            ),
            (
                "runtime::tests::workspace_reliability_ambiguous_registry_pointer_preserves_current_focus",
                "LINA_WORKSPACE_AMBIGUOUS_REGISTRY_POINTER_CHILD",
                true,
            ),
            (
                "runtime::tests::workspace_reliability_ambiguous_jsonl_keeps_current_workspace",
                "LINA_SWITCH_REJECT_AMBIGUOUS_CHILD",
                true,
            ),
        ];
        for (test_name, child_env, disable_restore) in cases {
            run_gate_f_case(test_name, child_env, disable_restore);
        }
    }

    fn boot_restore_failure_matrix_child() {
        const AGENTS: usize = 3;
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let active_root = home.join("espacos").join("ativo");
        let target_root = home.join("espacos").join("restore-tres-agentes");
        drop(
            lina_core::Workspace::create(&active_root, "Ativo", "blank", None)
                .expect("workspace ativo"),
        );
        drop(
            lina_core::Workspace::create(&target_root, "Três agentes", "blank", None)
                .expect("workspace alvo"),
        );
        {
            let mut store = EventStore::open(lina_core::Workspace::events_dir(&target_root))
                .expect("store alvo");
            let mut events = Vec::new();
            for index in 0..AGENTS {
                let node = NodeId::from_u128(0xc000 + index as u128);
                events.extend([
                    lina_core::DomainEvent::NodeAdded {
                        node,
                        kind: "Terminal".to_owned(),
                        x: 40.0 + index as f64 * 320.0,
                        y: 96.0,
                        requested_by: None,
                    },
                    lina_core::DomainEvent::NodeRenamed {
                        node,
                        name: format!("Agente {}", index + 1),
                    },
                    lina_core::DomainEvent::NodeRoleAssigned {
                        node,
                        role: "developer".to_owned(),
                    },
                    lina_core::DomainEvent::TerminalSpawned {
                        node,
                        cli: "shell".to_owned(),
                        cwd: None,
                    },
                ]);
            }
            store
                .append_batch(&events)
                .expect("roster anterior em lote");
        }
        let registry_path = lina_core::default_registry_path().expect("registry isolado");
        let mut registry = lina_core::WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(
            lina_core::WorkspaceRegistry::rederive_entry(&active_root).expect("entrada ativa"),
        );
        registry.upsert(
            lina_core::WorkspaceRegistry::rederive_entry(&target_root).expect("entrada alvo"),
        );
        registry.save().expect("publicar catálogo inicial");

        let pty = Arc::new(Mutex::new(PtyManager::new()));
        let shared = SharedInfra {
            pty: Arc::clone(&pty),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        };
        let active = boot_ws_runtime(active_root.clone(), &shared, false).expect("boot ativo");
        register_ready_runtime(&active).expect("foco inicial");
        let active_store = Arc::clone(&active.store);
        let runtimes = runtimes_with_boot(active);

        for prepared_agents in 1..=AGENTS {
            let target_log_before = format!(
                "{:?}",
                EventStore::open(lina_core::Workspace::events_dir(&target_root))
                    .expect("abrir alvo antes")
                    .events()
                    .expect("log alvo antes")
            );
            let target_projection_before = format!(
                "{:?}",
                EventStore::open(lina_core::Workspace::events_dir(&target_root))
                    .expect("abrir projeção antes")
                    .project()
                    .expect("projeção alvo antes")
            );
            let active_projection_before = format!(
                "{:?}",
                lock(&active_store).project().expect("projeção ativa antes")
            );
            let resources_before = internal_resource_counts();
            let ptys_before = lock(&pty).len();
            bridge::arm_restore_failure_after_agent(prepared_agents);

            let error = activate_workspace(&runtimes, target_root.clone(), &shared)
                .expect_err("corte após agente preparado aborta o boot");
            assert!(
                error.contains(&format!("preparar {prepared_agents} agente")),
                "erro identifica o corte real: {error}"
            );
            let map = lock(&runtimes);
            assert_eq!(map.active, active_root, "foco atual preservado no corte");
            assert_eq!(map.map.len(), 1, "runtime parcial nunca é publicado");
            let surviving = map.map.get(&active_root).expect("ativo sobrevive");
            assert!(Arc::ptr_eq(&surviving.store, &active_store));
            assert!(!map.map.contains_key(&target_root));
            drop(map);
            assert_eq!(lock(&pty).len(), ptys_before, "zero PTY provisório");
            assert_eq!(
                internal_resource_counts(),
                resources_before,
                "pumps/listeners/scrollback voltam ao baseline do ativo"
            );
            assert_eq!(
                format!(
                    "{:?}",
                    EventStore::open(lina_core::Workspace::events_dir(&target_root))
                        .expect("abrir alvo depois")
                        .events()
                        .expect("log alvo depois")
                ),
                target_log_before,
                "o lote de restore não publica prefixo nem subconjunto"
            );
            assert_eq!(
                format!(
                    "{:?}",
                    EventStore::open(lina_core::Workspace::events_dir(&target_root))
                        .expect("abrir projeção depois")
                        .project()
                        .expect("projeção alvo depois")
                ),
                target_projection_before,
                "a projeção alvo permanece idêntica"
            );
            assert_eq!(
                format!(
                    "{:?}",
                    lock(&active_store)
                        .project()
                        .expect("projeção ativa depois")
                ),
                active_projection_before,
                "o store ativo jamais vira fallback do alvo"
            );
        }

        activate_workspace(&runtimes, target_root.clone(), &shared)
            .expect("sem fault o mesmo roster restaura inteiro");
        let map = lock(&runtimes);
        assert_eq!(map.active, target_root);
        let restored = map.map.get(&target_root).expect("alvo pronto");
        assert_eq!(
            restored
                .sup
                .list()
                .into_iter()
                .filter(|node| node.name != "@Trigger")
                .count(),
            AGENTS,
            "o retry íntegro publica todos os agentes, exatamente uma vez"
        );
        drop(map);
        stop_all(&runtimes);
        for runtime in lock(&runtimes).map.values() {
            runtime.nodes.shutdown_terminals_after_failed_boot();
        }
    }

    #[test]
    fn workspace_reliability_boot_restore_and_focus_failure_matrix() {
        const CHILD: &str = "LINA_BOOT_RESTORE_FAILURE_MATRIX_CHILD";
        if std::env::var_os(CHILD).is_some() {
            boot_restore_failure_matrix_child();
            return;
        }
        run_gate_f_case(
            "runtime::tests::workspace_reliability_boot_restore_and_focus_failure_matrix",
            CHILD,
            false,
        );
        run_gate_f_case(
            "runtime::tests::workspace_reliability_registry_failure_never_publishes_new_focus",
            "LINA_WORKSPACE_FOCUS_COMMIT_FAILURE_CHILD",
            true,
        );
        run_gate_f_case(
            "runtime::tests::workspace_reliability_post_append_focus_failure_recovers_exactly_once",
            "LINA_POST_APPEND_FOCUS_FAILURE_CHILD",
            true,
        );
    }

    /// **r5 park-seam — `parked_pending` (puro sobre log+projeção):** só ids de
    /// `WorkspaceUnloaded` que AINDA estão `Dead` na projeção pendem religação; depois do
    /// supersede (`NodeRemoved` do restore) saem sozinhos — religar 2× é impossível por
    /// construção (idempotência do foco).
    #[test]
    fn parked_pending_tracks_unloaded_until_superseded() {
        let dir = std::env::temp_dir().join(format!("lina-relight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(dir.join(".lina/events")).expect("store");
        let node: NodeId = uuid::Uuid::now_v7();
        store
            .append(&lina_core::DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 1.0,
                y: 2.0,
                requested_by: None,
            })
            .expect("NodeAdded");
        store
            .append(&lina_core::DomainEvent::NodeRenamed {
                node,
                name: "Pesquisador".into(),
            })
            .expect("NodeRenamed");
        store
            .append(&lina_core::DomainEvent::NodeStatusChanged {
                node,
                status: "Dead".into(),
                from: "Idle".into(),
                reason: "unloaded_bg".into(),
            })
            .expect("morte registrada");
        store
            .append(&lina_core::DomainEvent::WorkspaceUnloaded {
                parked: vec![node.to_string()],
                kept: vec![],
            })
            .expect("gesto registrado");

        let recs = store.events().expect("events");
        let proj = store.project().expect("project");
        let pending = parked_pending(&recs, &proj);
        assert!(pending.contains(&node), "estacionado pende religação");

        // Supersede (o restore aposenta o antigo) → sai do pendente (idempotência).
        store
            .append(&lina_core::DomainEvent::NodeRemoved { node })
            .expect("NodeRemoved");
        let pending2 = parked_pending(&store.events().expect("e"), &store.project().expect("p"));
        assert!(pending2.is_empty(), "aposentado não religa de novo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **r5 perf-ws (regressão do multi-workspace):** N Espaços compartilham UMA varredura —
    /// fonte duplicada não duplica o walk, todo inscrito recebe o MESMO snapshot, e um
    /// inscrito TARDIO (Espaço aberto depois do cold-parse) nasce já alimentado.
    #[test]
    fn session_hub_single_scan_feeds_all_subscribers_and_dedupes_sources() {
        let home = std::env::temp_dir().join(format!("lina-hub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let proj = home.join(".claude/projects/proj-a");
        std::fs::create_dir_all(&proj).expect("corpus");
        std::fs::write(
            proj.join("sess-1.jsonl"),
            concat!(
                r#"{"type":"assistant","sessionId":"sess-1","requestId":"r1","message":{"model":"claude-opus-4-8","usage":{"input_tokens":10,"output_tokens":5}}}"#,
                "\n"
            ),
        )
        .expect("fixture");

        let mut hub = SessionHub::new(home.clone());
        hub.add_source("claude-code", "~/.claude/projects/*/*.jsonl");
        // N runtimes com o MESMO perfil registram a MESMA fonte → 1 varredura, não N.
        hub.add_source("claude-code", "~/.claude/projects/*/*.jsonl");
        assert_eq!(hub.sources_len(), 1, "fonte deduplicada");

        let a: Arc<Mutex<Vec<lina_session_watch::Session>>> = Arc::default();
        hub.subscribe(&a);
        let fed = hub.poll_and_fanout();
        assert_eq!(fed, 1, "1 inscrito alimentado no update");
        assert_eq!(lock(&a).len(), 1, "snapshot com a sessão do corpus");

        // Inscrito TARDIO: recebe o snapshot corrente NA INSCRIÇÃO (sem esperar update).
        let b: Arc<Mutex<Vec<lina_session_watch::Session>>> = Arc::default();
        hub.subscribe(&b);
        assert_eq!(*lock(&a), *lock(&b), "tardio nasce com o snapshot corrente");

        // Sem mudança no disco: poll não refaz snapshot nem realimenta ninguém.
        assert_eq!(hub.poll_and_fanout(), 0, "estável → zero fan-out");

        // Inscrito morto sai sozinho do fan-out (Weak podado).
        drop(a);
        std::fs::write(
            proj.join("sess-2.jsonl"),
            concat!(
                r#"{"type":"assistant","sessionId":"sess-2","requestId":"r2","message":{"model":"claude-opus-4-8","usage":{"input_tokens":1,"output_tokens":1}}}"#,
                "\n"
            ),
        )
        .expect("2ª sessão");
        assert_eq!(hub.poll_and_fanout(), 1, "só o inscrito VIVO é alimentado");
        assert_eq!(lock(&b).len(), 2, "snapshot cresceu com a sessão nova");

        let _ = std::fs::remove_dir_all(&home);
    }
}
