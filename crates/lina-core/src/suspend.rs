//! F1-5-5 · Suspensão real de ociosos — FATIA CORE (fonte: `ondas-5-6.md` 90-98; P0 não-condicional).
//!
//! ## A decisão load-bearing
//! Um nó suspenso **continua vivo e drenando**: o reader do PTY, o `advance` do VT e o harvest do
//! scrollback NUNCA param — parar de drenar travaria o CLI por flow-control (W0-3). O que para é a
//! **montagem/render no shell**, que consulta esta política (`should_render`). Por isso a suspensão é
//! um **overlay ORTOGONAL ao [`NodeStatus`]** do roster, não um novo estado de lifecycle: o nó segue
//! `Idle` e plenamente endereçável (um `lina ask` ENTREGA e o acorda). Modelar como `NodeStatus`
//! novo quebraria os `match` exaustivos do shell (app/) e confundiria "suspenso" (render pausado)
//! com "indisponível" (não é) — são eixos diferentes.
//!
//! ## A máquina (`Active → Idle → Suspended`)
//! Dirigida por um relógio INJETADO (`now_ms`) — puro, testável headless sem `sleep` (lição do
//! [`crate::lifecycle::LifecycleEngine`]). O chamador (loop de manutenção do app) dá `tick` na sua
//! cadência. Guards são consultados a cada `tick`:
//! - **prompt pendente · custódia pendente · pinado** BLOQUEIAM a suspensão (e ACORDAM se já suspenso
//!   — um prompt não pode ficar invisível esperando o humano).
//! - **A2A dirigida · foco · (opcional) retomada de output** ACORDAM.
//!
//! ## Costuras a fiar (registradas na entrega — fora desta fatia)
//! - **Despertar-por-A2A:** o roteador, após `RouteOutcome::Delivered`, chama
//!   [`SuspendController::note_a2a_delivered`] (1 linha em quem orquestra a entrega; `router.rs` é
//!   estável — não editado aqui).
//! - **Foco/clique** (shell) → [`SuspendController::note_focus`]; **render** consulta
//!   [`SuspendController::should_render`]; o reader segue drenando incondicionalmente
//!   ([`SuspendController::should_keep_draining`] é sempre `true`).
//! - **Guards:** o chamador projeta prompt/custódia pendentes (fila de atenção / broker) em
//!   [`SuspendGuards`]; pinagem é estado do shell via [`SuspendController::set_pinned`].

use std::collections::HashMap;

use crate::events::{DomainEvent, EventStore, StoreError};
use crate::NodeId;

/// Razões canônicas de transição (campo `reason` de `NodeSuspended`/`NodeResumed`). Estáveis por
/// contrato — mudar uma string quebra o replay/auditoria de logs existentes.
pub mod reason {
    /// Ocioso além do threshold, sem guards → `Suspended`.
    pub const IDLE_TIMEOUT: &str = "idle_timeout";
    /// Mensagem A2A entregue ao nó suspenso → acorda.
    pub const A2A_DELIVERED: &str = "a2a_delivered";
    /// Foco/clique do humano no terminal → acorda.
    pub const FOCUS: &str = "focus";
    /// Retomada de output do PTY (quando `wake_on_output`) → acorda.
    pub const OUTPUT_RESUMED: &str = "output_resumed";
    /// Prompt pendente detectado enquanto suspenso → acorda.
    pub const PROMPT_PENDING: &str = "prompt_pending";
    /// Custódia pendente detectada enquanto suspenso → acorda.
    pub const CUSTODY_PENDING: &str = "custody_pending";
}

/// Estado de suspensão — overlay ORTOGONAL ao [`NodeStatus`](crate::NodeStatus) do roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendState {
    /// Atividade recente; render normal.
    Active,
    /// Ocioso, mas ainda abaixo do threshold de suspensão; render normal.
    Idle,
    /// Ocioso além do threshold; o shell PAUSA a montagem (mas o core segue drenando).
    Suspended,
}

/// Configuração (thresholds conservadores por default).
#[derive(Debug, Clone, Copy)]
pub struct SuspendConfig {
    /// Sem atividade por este tempo → `Idle`.
    pub active_to_idle_ms: u64,
    /// Sem atividade por este tempo → `Suspended` (≥ `active_to_idle_ms`).
    pub idle_to_suspend_ms: u64,
    /// Se a retomada de output do PTY deve acordar o render (senão só foco/A2A acordam).
    pub wake_on_output: bool,
}

impl Default for SuspendConfig {
    fn default() -> Self {
        // Conservador: 1 min para Idle, 5 min para Suspended.
        Self {
            active_to_idle_ms: 60_000,
            idle_to_suspend_ms: 300_000,
            wake_on_output: true,
        }
    }
}

/// Sinais de guarda consultados a cada `tick` — bloqueiam a suspensão e acordam se já suspenso.
/// (Pinagem é estado sticky, não per-tick — ver [`SuspendController::set_pinned`].)
#[derive(Debug, Clone, Copy, Default)]
pub struct SuspendGuards {
    /// Há um prompt esperando resposta do humano neste nó.
    pub prompt_pending: bool,
    /// Há uma ação custodiada (gate de segurança) pendente neste nó.
    pub custody_pending: bool,
}

/// Transição resultante de um `tick`/sinal. Reasons são `&'static str` do módulo [`reason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendTransition {
    /// Sem mudança observável de estado.
    None,
    /// `Active → Idle`.
    ToIdle,
    /// `Idle/Active → Suspended`.
    ToSuspended { reason: &'static str },
    /// `Suspended → Active` (acordou).
    Woke { reason: &'static str },
}

#[derive(Debug, Clone, Copy)]
struct NodeSuspend {
    state: SuspendState,
    last_activity_ms: u64,
    pinned: bool,
}

/// O motor de suspensão: puro sobre `(now_ms, guards)`, sem thread nem relógio próprio. O chamador
/// dá `tick`/`note_*` na sua cadência. As transições que merecem auditoria
/// (`ToSuspended`/`Woke`) viram evento via [`record_transition`].
#[derive(Debug, Default)]
pub struct SuspendController {
    cfg: SuspendConfig,
    nodes: HashMap<NodeId, NodeSuspend>,
}

impl SuspendController {
    /// Controlador com os thresholds conservadores do default.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(SuspendConfig::default())
    }

    /// Controlador com configuração customizada.
    #[must_use]
    pub fn with_config(cfg: SuspendConfig) -> Self {
        Self {
            cfg,
            nodes: HashMap::new(),
        }
    }

    /// Começa a rastrear `node` como `Active` a partir de `now_ms` (chame ao spawn/admissão).
    pub fn track(&mut self, node: NodeId, now_ms: u64) {
        self.nodes.insert(
            node,
            NodeSuspend {
                state: SuspendState::Active,
                last_activity_ms: now_ms,
                pinned: false,
            },
        );
    }

    /// Para de rastrear `node` (chame ao `Dead`/remoção).
    pub fn forget(&mut self, node: NodeId) {
        self.nodes.remove(&node);
    }

    fn entry(&mut self, node: NodeId, now_ms: u64) -> &mut NodeSuspend {
        self.nodes.entry(node).or_insert(NodeSuspend {
            state: SuspendState::Active,
            last_activity_ms: now_ms,
            pinned: false,
        })
    }

    /// Estado de suspensão atual (`Active` para um nó nunca rastreado).
    #[must_use]
    pub fn state(&self, node: NodeId) -> SuspendState {
        self.nodes
            .get(&node)
            .map_or(SuspendState::Active, |n| n.state)
    }

    /// O shell deve montar/renderizar este nó? `false` SOMENTE quando suspenso.
    #[must_use]
    pub fn should_render(&self, node: NodeId) -> bool {
        self.state(node) != SuspendState::Suspended
    }

    /// O core deve seguir drenando o PTY/VT/harvest deste nó? SEMPRE `true` — a suspensão NUNCA
    /// para a drenagem (flow-control W0-3 travaria o CLI). É o outro lado da decisão load-bearing:
    /// só o render pausa.
    #[must_use]
    #[allow(clippy::unused_self)]
    pub fn should_keep_draining(&self, _node: NodeId) -> bool {
        true
    }

    /// Marca/desmarca o nó como pinado (sticky). Pinado nunca suspende; despinar reinicia o relógio
    /// de ociosidade a partir do próximo `tick`.
    pub fn set_pinned(&mut self, node: NodeId, pinned: bool) {
        let n = self.entry(node, 0);
        n.pinned = pinned;
    }

    /// Avalia a suspensão por TEMPO (e guards) em `now_ms`. Guards/pinagem bloqueiam (e acordam se
    /// já suspenso). Devolve a transição observada (`None` se nada mudou).
    pub fn tick(&mut self, node: NodeId, now_ms: u64, guards: SuspendGuards) -> SuspendTransition {
        let cfg = self.cfg;
        let n = self.entry(node, now_ms);

        let block_reason = if guards.prompt_pending {
            Some(reason::PROMPT_PENDING)
        } else if guards.custody_pending {
            Some(reason::CUSTODY_PENDING)
        } else if n.pinned {
            // pinado bloqueia, mas não é "wake-worthy" em si (o nó não some por estar pinado)
            Some("")
        } else {
            None
        };

        if let Some(why) = block_reason {
            // bloqueado: o relógio de ociosidade não corre — refresca a atividade.
            n.last_activity_ms = now_ms;
            if n.state == SuspendState::Suspended {
                n.state = SuspendState::Active;
                if !why.is_empty() {
                    return SuspendTransition::Woke { reason: why };
                }
            }
            n.state = SuspendState::Active;
            return SuspendTransition::None;
        }

        let elapsed = now_ms.saturating_sub(n.last_activity_ms);
        let target = if elapsed >= cfg.idle_to_suspend_ms {
            SuspendState::Suspended
        } else if elapsed >= cfg.active_to_idle_ms {
            SuspendState::Idle
        } else {
            SuspendState::Active
        };
        if target == n.state {
            return SuspendTransition::None;
        }
        n.state = target;
        match target {
            SuspendState::Idle => SuspendTransition::ToIdle,
            SuspendState::Suspended => SuspendTransition::ToSuspended {
                reason: reason::IDLE_TIMEOUT,
            },
            // recuperação sem sinal de atividade não acontece sem `note_*`; sem evento.
            SuspendState::Active => SuspendTransition::None,
        }
    }

    /// Sinal de atividade (output novo no PTY / VT avançou). Refresca o relógio; acorda se suspenso
    /// E `wake_on_output` (senão segue suspenso — render pausado, mas o output já foi harvested).
    pub fn note_activity(&mut self, node: NodeId, now_ms: u64) -> SuspendTransition {
        let wake = self.cfg.wake_on_output;
        let n = self.entry(node, now_ms);
        n.last_activity_ms = now_ms;
        if n.state == SuspendState::Suspended {
            if wake {
                n.state = SuspendState::Active;
                return SuspendTransition::Woke {
                    reason: reason::OUTPUT_RESUMED,
                };
            }
            return SuspendTransition::None;
        }
        n.state = SuspendState::Active;
        SuspendTransition::None
    }

    /// Despertar por A2A dirigida (o roteador chama após `Delivered` — costura registrada).
    pub fn note_a2a_delivered(&mut self, node: NodeId, now_ms: u64) -> SuspendTransition {
        self.wake(node, now_ms, reason::A2A_DELIVERED)
    }

    /// Despertar por foco/clique do humano (o shell chama).
    pub fn note_focus(&mut self, node: NodeId, now_ms: u64) -> SuspendTransition {
        self.wake(node, now_ms, reason::FOCUS)
    }

    fn wake(&mut self, node: NodeId, now_ms: u64, why: &'static str) -> SuspendTransition {
        let n = self.entry(node, now_ms);
        n.last_activity_ms = now_ms;
        let was_suspended = n.state == SuspendState::Suspended;
        n.state = SuspendState::Active;
        if was_suspended {
            SuspendTransition::Woke { reason: why }
        } else {
            SuspendTransition::None
        }
    }
}

/// Persiste uma transição de suspensão no log (aditivo; sem efeito de status no roster). Só
/// `ToSuspended`/`Woke` viram evento — `ToIdle`/`None` são transições internas sem auditoria.
/// Devolve `true` se um evento foi apendado.
pub fn record_transition(
    store: &mut EventStore,
    node: NodeId,
    transition: SuspendTransition,
) -> Result<bool, StoreError> {
    let event = match transition {
        SuspendTransition::ToSuspended { reason } => DomainEvent::NodeSuspended {
            node,
            reason: reason.to_string(),
        },
        SuspendTransition::Woke { reason } => DomainEvent::NodeResumed {
            node,
            reason: reason.to_string(),
        },
        SuspendTransition::None | SuspendTransition::ToIdle => return Ok(false),
    };
    store.append(&event)?;
    Ok(true)
}

/// **r5 perf-ws — plano de DESCARREGAR um Espaço de fundo** (modelo Maestri do fundador:
/// "desligar terminais SEM remover; religam ao focar"). Restrição LITERAL dele: *"sem
/// interferir nos processos que estiverem rodando nesse workspace"* — por isso a política
/// é conservadora: só o **`Idle`** (turno fechado, nada em curso) é estacionável; `Busy`
/// (trabalhando), `Blocked` (aguardando gesto humano) e `Starting/Running/Ready` (em
/// atividade ou prestes a receber o 1º prompt) são PRESERVADOS — o chamador narra/pede
/// confirmação para esses. `Dead` não é nada (já desligado). Função PURA: quem mata PTY,
/// apenda [`crate::DomainEvent::WorkspaceUnloaded`] e religa no foco é o executor do app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnloadPlan {
    /// Seguros de desligar agora (PTY morre; nó/posição/scrollback ficam).
    pub park: Vec<NodeId>,
    /// Vivos preservados (trabalhando/aguardando) — o "sem interferir" do fundador.
    pub keep: Vec<NodeId>,
}

/// Deriva o plano do roster vivo do Espaço (a mesma fonte do `lina list`).
#[must_use]
pub fn unload_plan(roster: &[crate::NodeInfo]) -> UnloadPlan {
    let mut plan = UnloadPlan::default();
    for n in roster {
        match n.status {
            crate::NodeStatus::Idle => plan.park.push(n.id),
            crate::NodeStatus::Dead => {}
            _ => plan.keep.push(n.id),
        }
    }
    plan
}

#[cfg(test)]
mod unload_tests {
    use super::*;
    use crate::{NodeInfo, NodeStatus};

    fn info(status: NodeStatus) -> NodeInfo {
        NodeInfo {
            id: uuid::Uuid::now_v7(),
            name: format!("{status:?}"),
            role: None,
            status,
        }
    }

    /// A restrição do fundador codificada: só Idle estaciona; Busy/Blocked/Starting/
    /// Running/Ready preservados; Dead ignorado. Vazio → plano vazio (não-vácuo: se a
    /// política regredir para "estaciona tudo", o keep esvazia e este teste quebra).
    #[test]
    fn unload_plan_parks_only_idle_and_preserves_working_nodes() {
        let idle = info(NodeStatus::Idle);
        let busy = info(NodeStatus::Busy);
        let blocked = info(NodeStatus::Blocked);
        let ready = info(NodeStatus::Ready);
        let dead = info(NodeStatus::Dead);
        let roster = vec![
            idle.clone(),
            busy.clone(),
            blocked.clone(),
            ready.clone(),
            dead,
        ];

        let plan = unload_plan(&roster);
        assert_eq!(plan.park, vec![idle.id], "só o ocioso desliga");
        assert_eq!(
            plan.keep,
            vec![busy.id, blocked.id, ready.id],
            "trabalhando/aguardando/prestes-a-receber ficam vivos (sem interferir)"
        );
        assert_eq!(unload_plan(&[]), UnloadPlan::default());
    }
}
