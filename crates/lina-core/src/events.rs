//! Event Store (W0-5) + Recuperação pós-crash visível (W0-6).
//!
//! **Invariante do `CLAUDE.md`:** o event log é a **fonte da verdade** do domínio.
//! O estado projetado (roster, posições, contadores) é uma *projeção reconstruível*
//! por replay; nunca a autoridade.
//!
//! ## Estratificação (ver [[20 - Arquitetura Tecnica]] §4)
//! - **SQLite WAL** (`events`): log append-only imutável — NUNCA `UPDATE`/`DELETE`.
//!   Projeção rápida e queryável.
//! - **`log.jsonl`** append-only: **espelho redundante** do log (autoridade de
//!   disaster-recovery). Se o `.db` corromper, o estado é reconstruído 100% daqui.
//! - **`snapshots/snap-<seq>.json`**: estado projetado materializado (+ versão) que
//!   encurta o replay.
//!
//! ## Determinismo
//! O reducer é puro e o estado usa `BTreeMap` (ordem estável) → `serde_json` canônico
//! → `fingerprint` idêntico em replays repetidos.
//!
//! ## Upcasting versionado
//! Cada tipo de evento tem `event_version`. No replay, um payload de versão antiga
//! passa por [`upcast`] antes de desserializar — o schema evolui sem quebrar o log.

use std::collections::BTreeMap;
use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lina_host::{HostEvent, NodeId, UiHost};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cli_discovery::DiscoveredCli;
use crate::plan::Plan;

/// Erros do event store / recuperação.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ───────────────────────────── catálogo de eventos do domínio ─────────────────────────────

/// W3-7 (ADR 0003): motivo TIPADO de uma recusa de roteamento — projetável e assertável do log.
/// Serializa em `snake_case` (`"hop_limit"`, `"loop_detected"`, …) para casar a tabela do ADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    Duplicate,
    HopLimit,
    UnknownSender,
    NoTarget,
    BlockedByAutonomy,
    FanoutGated,
    BudgetExceeded,
    Deadlock,
    LoopDetected,
}

/// W3-7b (ADR 0002): motivo do fechamento de um `await`. Serializa em `snake_case`
/// (`"replied"`/`"timeout"`/`"deadlock"`). `Deadlock` existe para completude do contrato, mas o
/// `AwaitClosed{Deadlock}` NÃO é emitido — a pré-checagem recusa ANTES de abrir o ticket (ADR 0002 §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitReason {
    Replied,
    Timeout,
    Deadlock,
}

/// W4-4: durabilidade do último `append`, para a UI projetar **"salvando…" ↔ "Tudo salvo ✓"**
/// (rodapé, invariante #6 "estado sempre salvo e visível"). O core emite a transição via callback
/// (ver [`EventStore::append_with_flush`]); o shell a mapeia para seu evento de UI **sem o core
/// importar o contrato de UI** (`lina-host`). NÃO é evento de domínio (não vai ao log) — é estado
/// transitório de persistência.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushState {
    /// Escrita em andamento (antes do flush no disco).
    Saving,
    /// Escrita durável (JSONL append-only flushed) — "Tudo salvo ✓".
    Saved,
}

/// Catálogo canônico de eventos do domínio (subconjunto da [[20 - Arquitetura Tecnica]]
/// §4.2 suficiente para a Onda 0). Internamente *tagged* por `event` → o nome da
/// variante é o `kind` persistido.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum DomainEvent {
    WorkspaceCreated {
        name: String,
        /// W4-5: preset de Foco escolhido na galeria (`dev_app`/`research_content`/…). Taxonomia
        /// LIVRE aqui (a story W4-5 a define). `#[serde(default)]` + upcasting v1→v2 → logs antigos
        /// (sem o campo) replayam com `""` (não-setado), sem quebrar o replay.
        #[serde(default)]
        focus_preset: String,
    },
    NodeAdded {
        node: NodeId,
        kind: String,
        x: f64,
        y: f64,
    },
    NodeMoved {
        node: NodeId,
        x: f64,
        y: f64,
    },
    NodeRenamed {
        node: NodeId,
        name: String,
    },
    NodeRoleAssigned {
        node: NodeId,
        role: String,
    },
    NodeStatusChanged {
        node: NodeId,
        status: String,
    },
    NodeRemoved {
        node: NodeId,
    },
    TerminalSpawned {
        node: NodeId,
        cli: String,
    },
    TerminalExited {
        node: NodeId,
    },
    BusMessageSent {
        id: String,
        from: NodeId,
        to: String,
    },
    /// W3-4: um colega anunciou presença (`lina handshake`). Meta/observabilidade — o
    /// roster vivo é do Supervisor; aqui só registramos a entrada no log (`agent.joined`).
    Handshake {
        node: NodeId,
        name: String,
    },
    /// W3-4: o supervisor ROTEOU uma mensagem da mailbox (passou os guardrails 0-4) e a
    /// despachou para `to` (alvo resolvido). `id` = id da mensagem (dedupe/anti-loop).
    MessageRouted {
        id: String,
        from: NodeId,
        to: String,
        intent: String,
        /// W3-7 (A2): turno de origem propagado pelo supervisor — filtro do grafo anti-loop (§2.1).
        #[serde(default)]
        root_cause_id: String,
        /// W3-7 (A2): profundidade da cadeia neste salto (cresce a cada hop herdado).
        #[serde(default)]
        hops: u8,
        /// W3-7 (§2.1): alvo resolvido (NodeId) — aresta `from→to_node` do grafo de handoffs.
        /// `None` para broadcast (não entra no anti-loop por ciclo).
        #[serde(default)]
        to_node: Option<NodeId>,
    },
    /// W3-4: a mensagem `id` foi entregue ao PTY do alvo (`deliver_a2a` faseado concluiu).
    MessageDelivered {
        id: String,
        to: NodeId,
    },
    /// W3-7 (ADR 0003): recusa de guardrail no roteamento — o log é o livro-razão das recusas.
    /// `from`/`to` são NOMES (cobre `unknown_sender`, em que `from` não resolve a `NodeId`).
    /// `duplicate` NÃO é apendado (anti-amplificação A4 — ver `Router::route_message`).
    RouteBlocked {
        id: String,
        reason: BlockReason,
        from: String,
        to: String,
    },
    /// W3-7b (ADR 0002): `A --await--> B` aprovado → a aresta `waiter→target` foi ABERTA no
    /// wait-for-graph (persiste até o reply/timeout). `id` = id da mensagem-pergunta (chave do reply).
    AwaitOpened {
        id: String,
        waiter: NodeId,
        target: NodeId,
        root_cause_id: String,
    },
    /// W3-7b (ADR 0002): o ciclo de `await` `id` fechou (`Replied` pelo `reply_to` do alvo, ou
    /// `Timeout` pelo sweep). Libera a aresta do wait-for-graph (`end_await`).
    AwaitClosed {
        id: String,
        reason: AwaitReason,
    },
    /// W3-6 (AC-6.2): o gate de execução interceptou uma ação `gated-*` e a recusou/confirmou.
    /// É o livro-razão das ações barradas (mesmo princípio do `RouteBlocked`): apendado SÓ quando
    /// a decisão NÃO é `allow` (ação `routine` permitida não polui o log). `class` ∈
    /// {`gated-soft`,`gated-hard`}; `decision` ∈ {`ask`,`deny`}. META — sem efeito na projeção.
    ActionGated {
        cmd: String,
        class: String,
        decision: String,
    },
    /// W3-6c (ADR 0004): o broker `lina do` EXECUTOU uma ação custodiada (`gated-hard-external`)
    /// após confirmação humana, obtendo o segredo do SecretVault — que o agente NÃO tem no env do
    /// PTY. Livro-razão da execução brokerada. `requester` é a identidade auto-declarada do
    /// pedido (A3 pendente → autoria NÃO-autenticada). NUNCA carrega o segredo. META — sem projeção.
    BrokerExecuted {
        action: String,
        requester: String,
    },
    /// W3-6c (ADR 0004): o broker RECUSOU executar uma ação custodiada. `reason` ∈
    /// {`unconfirmed` (sem gate humano), `no_secret` (segredo ausente do cofre → a custódia
    /// bloqueia: sem segredo não há caminho de execução), `exec_failed`}. É a prova observável da
    /// custódia: sem confirmação E/OU sem segredo, a ação não roda. META — sem projeção.
    BrokerDenied {
        action: String,
        reason: String,
    },
    /// W3-7c (§2.2): uso de tokens REPORTADO para um nó. É a FONTE de uso do teto de custo — o app
    /// a emite ao observar o fim-de-resposta da CLI (W0-10); aqui o `CostLedger` (router) a soma por
    /// janela contábil. META — sem efeito na projeção do canvas; o ledger reconstrói por replay.
    TokenUsageReported {
        node: String,
        tokens: u64,
    },
    /// W3-7c (§2.2): a soma de uso da janela atingiu `token_budget_day` → o workspace entrou em
    /// `Paused`. Livro-razão do teto (mesmo princípio do `RouteBlocked`): apendado SÓ na TRANSIÇÃO
    /// para Paused (anti-amplificação A4 — não a cada delegação bloqueada). É GATE, não kill: o
    /// estado fica salvo e visível (invariante #6) até `lina resume --confirm`.
    CostCeilingHit {
        day: String,
        tokens: u64,
    },
    /// W3-7c (§2.2): humano confirmou a retomada (`lina resume --confirm`) → sai de `Paused` e ABRE
    /// uma nova janela contábil (o ledger zera a soma a partir daqui; sem isso, a soma anterior
    /// re-pausaria na próxima delegação).
    CostCeilingResumed {
        day: String,
    },
    /// W3-5: um item foi semeado no plano compartilhado (`status:todo`, `@owner:?`).
    PlanItemAdded {
        item: String,
        desc: String,
    },
    /// W3-5: uma decisão viva foi adicionada ao plano.
    PlanDecisionAdded {
        text: String,
    },
    /// W3-5: `lina plan claim` aplicado pelo supervisor. `id`=id da mensagem de origem;
    /// `item`=ref do item; `by`=quem reivindicou. Projeta `@owner:by` + `status:doing`.
    PlanClaimed {
        id: String,
        item: String,
        by: String,
    },
    /// W3-5: `lina plan check` aplicado pelo supervisor. Projeta `status:done` (campos como acima).
    PlanChecked {
        id: String,
        item: String,
        by: String,
    },
    NoteUpdated {
        name: String,
        hash: String,
    },
    /// W4-1: resultado do `CliDiscovery` (varredura do `PATH` no check-up de onboarding T1).
    /// Projeta a lista CORRENTE de CLIs disponíveis (último índice vence) — a fonte do "nenhum
    /// CLI encontrado" / "binário com versão" da tela T1. ZERO LLM, local-first.
    DiscoveryIndexed {
        clis: Vec<DiscoveredCli>,
    },
    /// W4-2 (M3): uma nota foi criada (`sticky` → `.md`). META — o nó vem de `NodeAdded{kind:"Note"}`;
    /// aqui registra-se o fato de criação do artefato (mesmo papel de `NoteUpdated`).
    NoteCreated {
        name: String,
    },
    /// W4-2 (M4): uma pasta foi criada. META — o nó vem de `NodeAdded{kind:"Folder"}`.
    FolderCreated {
        name: String,
    },
    /// W4-2 (P4 Inspetor): o CLI Profile de um nó foi setado/trocado. Projeta o `cli` do nó.
    CliProfileSet {
        node: NodeId,
        profile: String,
    },
    /// W4-4 (T6 Switcher): o foco de workspace mudou. Projeta `focused_workspace`. (O event store
    /// é por-workspace; este evento registra o foco vigente para a projeção do Switcher.)
    WorkspaceFocusSet {
        workspace: String,
    },
    /// W4-3 (freio do rodapé): a auto-orquestração foi PAUSADA pelo humano. Enquanto pausada, novas
    /// delegações ficam ENFILEIRADAS (não injetadas) — é GATE, não kill (inv #6): nada se perde, o
    /// estado fica salvo e visível. META — o estado de pausa vive no Router (escritor único); aqui só
    /// registra o fato no log (livro-razão do freio, par de [`DomainEvent::OrchestrationResumed`]).
    OrchestrationPaused,
    /// W4-3 (freio do rodapé): a auto-orquestração foi RETOMADA → a fila de delegações represadas é
    /// drenada (roteada/entregue agora). META — par de [`DomainEvent::OrchestrationPaused`].
    OrchestrationResumed,
    SnapshotTaken {
        seq: u64,
    },
}

impl DomainEvent {
    /// O `kind` persistido — idêntico à tag interna do serde.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            DomainEvent::WorkspaceCreated { .. } => "WorkspaceCreated",
            DomainEvent::NodeAdded { .. } => "NodeAdded",
            DomainEvent::NodeMoved { .. } => "NodeMoved",
            DomainEvent::NodeRenamed { .. } => "NodeRenamed",
            DomainEvent::NodeRoleAssigned { .. } => "NodeRoleAssigned",
            DomainEvent::NodeStatusChanged { .. } => "NodeStatusChanged",
            DomainEvent::NodeRemoved { .. } => "NodeRemoved",
            DomainEvent::TerminalSpawned { .. } => "TerminalSpawned",
            DomainEvent::TerminalExited { .. } => "TerminalExited",
            DomainEvent::BusMessageSent { .. } => "BusMessageSent",
            DomainEvent::Handshake { .. } => "Handshake",
            DomainEvent::MessageRouted { .. } => "MessageRouted",
            DomainEvent::MessageDelivered { .. } => "MessageDelivered",
            DomainEvent::RouteBlocked { .. } => "RouteBlocked",
            DomainEvent::AwaitOpened { .. } => "AwaitOpened",
            DomainEvent::AwaitClosed { .. } => "AwaitClosed",
            DomainEvent::ActionGated { .. } => "ActionGated",
            DomainEvent::BrokerExecuted { .. } => "BrokerExecuted",
            DomainEvent::BrokerDenied { .. } => "BrokerDenied",
            DomainEvent::TokenUsageReported { .. } => "TokenUsageReported",
            DomainEvent::CostCeilingHit { .. } => "CostCeilingHit",
            DomainEvent::CostCeilingResumed { .. } => "CostCeilingResumed",
            DomainEvent::PlanItemAdded { .. } => "PlanItemAdded",
            DomainEvent::PlanDecisionAdded { .. } => "PlanDecisionAdded",
            DomainEvent::PlanClaimed { .. } => "PlanClaimed",
            DomainEvent::PlanChecked { .. } => "PlanChecked",
            DomainEvent::NoteUpdated { .. } => "NoteUpdated",
            DomainEvent::DiscoveryIndexed { .. } => "DiscoveryIndexed",
            DomainEvent::NoteCreated { .. } => "NoteCreated",
            DomainEvent::FolderCreated { .. } => "FolderCreated",
            DomainEvent::CliProfileSet { .. } => "CliProfileSet",
            DomainEvent::WorkspaceFocusSet { .. } => "WorkspaceFocusSet",
            DomainEvent::OrchestrationPaused => "OrchestrationPaused",
            DomainEvent::OrchestrationResumed => "OrchestrationResumed",
            DomainEvent::SnapshotTaken { .. } => "SnapshotTaken",
        }
    }

    /// Versão CORRENTE do schema deste tipo. `NodeAdded` está em v2 (ganhou o campo
    /// `kind`); os demais em v1. O replay sobe versões antigas via [`upcast`].
    #[must_use]
    pub fn current_version(&self) -> u32 {
        match self {
            // `NodeAdded` v2 (campo `kind`), `WorkspaceCreated` v2 (campo `focus_preset`, W4-5).
            DomainEvent::NodeAdded { .. } | DomainEvent::WorkspaceCreated { .. } => 2,
            _ => 1,
        }
    }

    /// Reconstrói um evento a partir de um registro persistido: aplica [`upcast`] e
    /// então desserializa na forma corrente.
    fn from_record(
        kind: &str,
        version: u32,
        payload: serde_json::Value,
    ) -> Result<Self, StoreError> {
        let upcasted = upcast(kind, version, payload);
        Ok(serde_json::from_value(upcasted)?)
    }
}

/// Upcasting versionado: transforma um payload de versão antiga na forma corrente.
/// Aditivo e puro — base do replay à prova de evolução de schema.
fn upcast(kind: &str, version: u32, mut payload: serde_json::Value) -> serde_json::Value {
    // NodeAdded v1 → v2: não tinha o campo `kind` (tipo do nó). Default `Terminal`.
    if kind == "NodeAdded" && version < 2 {
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("kind")
                .or_insert_with(|| serde_json::Value::String("Terminal".into()));
        }
    }
    // WorkspaceCreated v1 → v2 (W4-5): não tinha `focus_preset`. Default `""` (não-setado).
    if kind == "WorkspaceCreated" && version < 2 {
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("focus_preset")
                .or_insert_with(|| serde_json::Value::String(String::new()));
        }
    }
    payload
}

/// Registro persistido de um evento (forma comum ao SQLite e ao espelho JSONL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: u64,
    pub ts: u64,
    pub kind: String,
    pub version: u32,
    pub payload: serde_json::Value,
}

// ───────────────────────────── estado projetado + reducer ─────────────────────────────

/// Nó no estado projetado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedNode {
    pub kind: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub x: f64,
    pub y: f64,
    pub cli: Option<String>,
}

/// Estado projetado do domínio. `BTreeMap` garante ordem estável → fingerprint estável.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectedState {
    pub workspace_name: Option<String>,
    pub nodes: BTreeMap<NodeId, ProjectedNode>,
    pub bus_messages: u64,
    /// W3-5: plano compartilhado projetado do log (`#[serde(default)]` → snapshots antigos
    /// desserializam com plano vazio). É a fonte do `.lina/plan.md` (escritor único: supervisor).
    #[serde(default)]
    pub plan: Plan,
    /// W4-4 (T6 Switcher): workspace em foco (do último `WorkspaceFocusSet`). `#[serde(default)]`
    /// → snapshots antigos desserializam como `None` (não quebra replay/recovery dos gates).
    #[serde(default)]
    pub focused_workspace: Option<String>,
    /// W4-1: CLIs de IA disponíveis (do último `DiscoveryIndexed`). `#[serde(default)]` → snapshots
    /// antigos desserializam com lista vazia.
    #[serde(default)]
    pub discovered_clis: Vec<DiscoveredCli>,
    /// W4-5: preset de Foco do workspace (de `WorkspaceCreated.focus_preset`). `None` = não-setado
    /// (inclui logs v1 upcasted). `#[serde(default)]` → snapshots antigos desserializam como `None`.
    #[serde(default)]
    pub focus_preset: Option<String>,
}

impl ProjectedState {
    /// Fingerprint determinístico (FNV-1a sobre o JSON canônico ordenado). Replays do
    /// mesmo log produzem o MESMO fingerprint.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let json = serde_json::to_string(self).unwrap_or_default();
        fnv1a(json.as_bytes())
    }
}

/// Reducer puro: aplica um evento ao estado projetado.
pub fn apply(state: &mut ProjectedState, event: &DomainEvent) {
    match event {
        DomainEvent::WorkspaceCreated {
            name,
            focus_preset,
        } => {
            state.workspace_name = Some(name.clone());
            // O cabeçalho do plano espelha o nome do workspace (projeção → `# Plano — <name>`).
            state.plan.workspace = name.clone();
            // W4-5: preset vazio (`""`, inclusive logs v1 upcasted) projeta `None`.
            state.focus_preset = if focus_preset.is_empty() {
                None
            } else {
                Some(focus_preset.clone())
            };
        }
        DomainEvent::NodeAdded { node, kind, x, y } => {
            state.nodes.insert(
                *node,
                ProjectedNode {
                    kind: kind.clone(),
                    name: None,
                    role: None,
                    status: None,
                    x: *x,
                    y: *y,
                    cli: None,
                },
            );
        }
        DomainEvent::NodeMoved { node, x, y } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.x = *x;
                n.y = *y;
            }
        }
        DomainEvent::NodeRenamed { node, name } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.name = Some(name.clone());
            }
        }
        DomainEvent::NodeRoleAssigned { node, role } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.role = Some(role.clone());
            }
        }
        DomainEvent::NodeStatusChanged { node, status } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.status = Some(status.clone());
            }
        }
        DomainEvent::NodeRemoved { node } => {
            state.nodes.remove(node);
        }
        DomainEvent::TerminalSpawned { node, cli } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cli = Some(cli.clone());
                n.status = Some("Running".into());
            }
        }
        DomainEvent::TerminalExited { node } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.status = Some("Dead".into());
            }
        }
        // W3-4: roteamento conta como mensagem do bus (mesma contagem de BusMessageSent).
        DomainEvent::BusMessageSent { .. } | DomainEvent::MessageRouted { .. } => {
            state.bus_messages += 1;
        }
        // W3-5: o plano é uma projeção do log — fatos já validados quando logados, aplicados
        // incondicionalmente no replay (a validação vive no comando, não na projeção).
        DomainEvent::PlanItemAdded { item, desc } => {
            state.plan.apply_item_added(item.clone(), desc.clone());
        }
        DomainEvent::PlanDecisionAdded { text } => state.plan.apply_decision_added(text.clone()),
        DomainEvent::PlanClaimed { item, by, .. } => state.plan.apply_claimed(item, by),
        DomainEvent::PlanChecked { item, by, .. } => state.plan.apply_checked(item, by),
        // W4-1: a indexação corrente substitui a anterior (último check-up vence).
        DomainEvent::DiscoveryIndexed { clis } => state.discovered_clis = clis.clone(),
        // W4-2 (P4): o CLI Profile do nó é projetado no campo `cli` (idempotente por nó).
        DomainEvent::CliProfileSet { node, profile } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cli = Some(profile.clone());
            }
        }
        // W4-4 (T6): o foco de workspace é projetado (último vence).
        DomainEvent::WorkspaceFocusSet { workspace } => {
            state.focused_workspace = Some(workspace.clone());
        }
        // Handshake/entrega/recusa/notas/snapshot são META (observabilidade no log) — sem efeito
        // na projeção do canvas; o roster vivo é do Supervisor, não da projeção.
        DomainEvent::Handshake { .. }
        | DomainEvent::MessageDelivered { .. }
        | DomainEvent::RouteBlocked { .. }
        | DomainEvent::AwaitOpened { .. }
        | DomainEvent::AwaitClosed { .. }
        | DomainEvent::ActionGated { .. }
        | DomainEvent::BrokerExecuted { .. }
        | DomainEvent::BrokerDenied { .. }
        // W3-7c: uso/teto de custo são META — o `CostLedger` (router) os reconstrói varrendo o log,
        // não pela projeção do canvas (mesmo padrão do grafo anti-loop §2.1).
        | DomainEvent::TokenUsageReported { .. }
        | DomainEvent::CostCeilingHit { .. }
        | DomainEvent::CostCeilingResumed { .. }
        | DomainEvent::NoteUpdated { .. }
        // W4-2 (M3/M4): criação de nota/pasta é META (o nó é criado via `NodeAdded`); registra o
        // fato do artefato no log, sem efeito na projeção do canvas (mesmo padrão de `NoteUpdated`).
        | DomainEvent::NoteCreated { .. }
        | DomainEvent::FolderCreated { .. }
        // W4-3: pausa/retomada da orquestração são META (livro-razão do freio); o estado vive no
        // Router (escritor único), reconstruível do log — sem efeito na projeção do canvas.
        | DomainEvent::OrchestrationPaused
        | DomainEvent::OrchestrationResumed
        | DomainEvent::SnapshotTaken { .. } => {}
    }
}

// ───────────────────────────── Event Store ─────────────────────────────

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    seq     INTEGER PRIMARY KEY,
    ts      INTEGER NOT NULL,
    kind    TEXT    NOT NULL,
    version INTEGER NOT NULL,
    payload TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS snapshots (
    seq   INTEGER PRIMARY KEY,
    ts    INTEGER NOT NULL,
    state TEXT    NOT NULL
);
";

/// **W3-7b §4.3 — reconciliação do path do espelho.** O design e o gate da Onda 3 verificam
/// `.lina/events/log.jsonl`; o código histórico escrevia `<store>/bus.jsonl`. DECISÃO: o espelho
/// JSONL chama-se **`log.jsonl`** (alinha com o gate + design; o `EventStore` já é aberto em
/// `.lina/events/`). Um único nome canônico — escritor (`open`) E leitor (`rebuild_from_jsonl`)
/// usam esta const; nada de 3º nome. (Stores de dev com `bus.jsonl` antigo ficam órfãos — sem
/// migração no MVP, pois não há dados persistentes de produção.)
const JSONL_FILE: &str = "log.jsonl";

/// Event store de um workspace: SQLite WAL (`events`) + espelho `log.jsonl` (§4.3) +
/// snapshots. Single-thread (a `Connection` do rusqlite não é `Sync`); o supervisor
/// futuro a embrulha sob lock.
pub struct EventStore {
    conn: Connection,
    dir: PathBuf,
    db_path: PathBuf,
    jsonl_path: PathBuf,
    snapshots_dir: PathBuf,
    next_seq: u64,
}

impl EventStore {
    /// Abre (ou cria) o event store em `dir`. **Não** faz recuperação — use
    /// [`EventStore::open_or_recover`] para a abertura resiliente a corrupção (W0-6).
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let snapshots_dir = dir.join("snapshots");
        std::fs::create_dir_all(&snapshots_dir)?;
        let db_path = dir.join("lina.db");
        let jsonl_path = dir.join(JSONL_FILE);

        let conn = Connection::open(&db_path)?;
        // FURO A (parte 1): com o store ws2/ws3 unificado, APP e bin (`lina do/guard`) escrevem no MESMO
        // log → escritores CONCORRENTES. Sem `busy_timeout`, o 2º escritor recebe `SQLITE_BUSY` IMEDIATO
        // ao disputar o write-lock e PERDE o evento (de segurança!). Com o timeout, ele ESPERA o lock
        // (3s é folgado; o WAL deixa leituras passarem em paralelo). DEFINIDO ANTES de qualquer escrita:
        // o próprio `journal_mode=WAL` e o `CREATE TABLE` (schema) pegam write-lock, então a ABERTURA
        // concorrente já precisa do timeout (senão é a `open()` do 2º processo que estoura `BUSY`).
        // Complementa o retry anti-colisão de `seq` (parte 2): este resolve a disputa de LOCK; aquele, a
        // colisão de PK (dois processos calculando o mesmo `seq` cacheado).
        conn.busy_timeout(std::time::Duration::from_millis(3000))?;
        enable_wal(&conn)?;
        conn.execute_batch(SCHEMA)?;
        let next_seq: i64 =
            conn.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM events", [], |r| {
                r.get(0)
            })?;

        Ok(Self {
            conn,
            dir,
            db_path,
            jsonl_path,
            snapshots_dir,
            next_seq: next_seq as u64,
        })
    }

    /// Abertura RESILIENTE (W0-6): roda `PRAGMA integrity_check`; se o `.db` estiver
    /// corrompido, **preserva** o arquivo (`lina.db.corrupt-<ts>`), reconstrói do
    /// `log.jsonl` e emite `Recovering` → `Recovered` no `UiHost` — nunca silencioso.
    pub fn open_or_recover(dir: impl AsRef<Path>, ui: &mut dyn UiHost) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("lina.db");

        if !db_path.exists() || !db_is_corrupt(&db_path) {
            return Self::open(&dir); // novo ou saudável → abertura normal, sem recovery
        }

        // Corrompido (lição Codex #21750): recuperação VISÍVEL.
        ui.on_event(HostEvent::Recovering);
        let preserved = preserve_corrupt(&db_path)?;
        tracing::warn!(corrupt = %preserved.display(), "banco corrompido preservado; reconstruindo do JSONL");
        let store = Self::rebuild_from_jsonl(&dir)?;
        ui.on_event(HostEvent::Recovered);
        Ok(store)
    }

    /// Reconstrói o `.db` (novo) reaplicando o espelho `log.jsonl`. Linhas ilegíveis
    /// são postas em **quarentena** (warn + segue) — não engole erro silenciosamente.
    fn rebuild_from_jsonl(dir: &Path) -> Result<Self, StoreError> {
        let mut store = Self::open(dir)?; // `lina.db` novo+schema (o corrompido já foi renomeado)
        let jsonl = dir.join(JSONL_FILE);
        if !jsonl.exists() {
            return Ok(store);
        }
        let file = std::fs::File::open(&jsonl)?;
        let reader = std::io::BufReader::new(file);
        for (lineno, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(line = lineno, "linha JSONL ilegível, quarentena: {e}");
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<EventRecord>(&line) {
                Ok(rec) => store.db_insert(&rec)?,
                Err(e) => tracing::warn!(line = lineno, "linha JSONL corrompida, quarentena: {e}"),
            }
        }
        Ok(store)
    }

    /// Anexa um evento do domínio (versão CORRENTE) ao log: INSERT no SQLite +
    /// linha no espelho JSONL append-only. Devolve o `seq` atribuído.
    pub fn append(&mut self, event: &DomainEvent) -> Result<u64, StoreError> {
        let payload = serde_json::to_value(event)?;
        self.insert_raw(event.kind(), event.current_version(), payload)
    }

    /// W4-4: como [`append`], mas **observa a durabilidade** para a UI: chama `on_flush(Saving)`
    /// antes de escrever e `on_flush(Saved)` após o append durável (JSONL flushed). O shell liga
    /// `on_flush` ao indicador "salvando…/Tudo salvo ✓" (e ao seu `HostEvent`) **sem o core importar
    /// o contrato de UI**. NÃO reimplementa persistência — só envolve [`append`] (W0-5) com a
    /// notificação. Devolve o `seq` atribuído.
    pub fn append_with_flush(
        &mut self,
        event: &DomainEvent,
        mut on_flush: impl FnMut(FlushState),
    ) -> Result<u64, StoreError> {
        on_flush(FlushState::Saving);
        let seq = self.append(event)?;
        on_flush(FlushState::Saved);
        Ok(seq)
    }

    /// Insere um registro CRU (`kind`/`version`/`payload`) — usado por [`append`], por
    /// importação e para gravar versões ANTIGAS (migração / teste de upcaster). Escreve
    /// no SQLite e no espelho JSONL.
    pub fn insert_raw(
        &mut self,
        kind: &str,
        version: u32,
        payload: serde_json::Value,
    ) -> Result<u64, StoreError> {
        // Monta com o `seq` OTIMISTA (cache em-memória) — o caminho rápido do escritor único NÃO paga
        // nada (1 tentativa, sem SELECT extra). `ts`/`payload` carimbados UMA vez: o retry reusa os
        // mesmos (a colisão não deve alterar o conteúdo do evento, só seu `seq`).
        let mut rec = EventRecord {
            seq: self.next_seq,
            ts: now_millis(),
            kind: kind.to_string(),
            version,
            payload,
        };
        // FURO A (parte 2): `next_seq` é cache POR-PROCESSO (nasce de `MAX(seq)+1` no open()). Sob 2
        // escritores, ambos podem calcular o MESMO `seq` → o 2º INSERT viola o PK e o evento de SEGURANÇA
        // some. Em colisão, re-lê `MAX(seq)` FRESCO do banco (o que o outro processo gravou), bumpa e
        // RE-INSERE — retry BOUNDED. Não é autoincrement (mudaria contrato/migração): é o mínimo correto.
        const MAX_RETRIES: u32 = 8;
        let mut attempt: u32 = 0;
        loop {
            match self.db_insert(&rec) {
                Ok(()) => break,
                Err(e) if is_seq_conflict(&e) && attempt < MAX_RETRIES => {
                    attempt += 1;
                    // Re-lê o seq REALMENTE gravado e mira o próximo livre. `db_insert` (no sucesso)
                    // re-sincroniza `self.next_seq`; aqui só corrigimos o alvo da re-tentativa.
                    let max: i64 = self.conn.query_row(
                        "SELECT COALESCE(MAX(seq), 0) FROM events",
                        [],
                        |r| r.get(0),
                    )?;
                    let next = (max as u64).saturating_add(1);
                    self.next_seq = next;
                    rec.seq = next;
                }
                Err(e) => return Err(e),
            }
        }
        // Espelha no JSONL com o `seq` EFETIVAMENTE gravado (não o otimista) — DB e espelho casam.
        append_jsonl(&self.jsonl_path, &rec)?;
        Ok(rec.seq)
    }

    /// INSERT apenas no SQLite (sem tocar no JSONL) — usado pelo rebuild, em que o
    /// JSONL é a FONTE (re-escrevê-lo duplicaria o espelho).
    fn db_insert(&mut self, rec: &EventRecord) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO events (seq, ts, kind, version, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rec.seq as i64,
                rec.ts as i64,
                rec.kind,
                rec.version as i64,
                serde_json::to_string(&rec.payload)?
            ],
        )?;
        if rec.seq + 1 > self.next_seq {
            self.next_seq = rec.seq + 1;
        }
        Ok(())
    }

    /// Estado projetado = último snapshot + replay dos eventos após ele (com upcasting).
    /// Determinístico.
    pub fn project(&self) -> Result<ProjectedState, StoreError> {
        let (mut state, from_seq) = match self.latest_snapshot()? {
            Some((seq, st)) => (st, seq),
            None => (ProjectedState::default(), 0),
        };
        let mut stmt = self
            .conn
            .prepare("SELECT kind, version, payload FROM events WHERE seq > ?1 ORDER BY seq ASC")?;
        let rows = stmt.query_map([from_seq as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (kind, version, payload_str) = row?;
            let payload: serde_json::Value = serde_json::from_str(&payload_str)?;
            let event = DomainEvent::from_record(&kind, version as u32, payload)?;
            apply(&mut state, &event);
        }
        Ok(state)
    }

    /// Materializa um snapshot do estado projetado (tabela + arquivo espelho em
    /// `snapshots/`, para resiliência §4.6). Devolve o `seq` coberto.
    pub fn take_snapshot(&mut self) -> Result<u64, StoreError> {
        let state = self.project()?;
        let seq = self.next_seq.saturating_sub(1);
        let state_json = serde_json::to_string(&state)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO snapshots (seq, ts, state) VALUES (?1, ?2, ?3)",
            params![seq as i64, now_millis() as i64, state_json],
        )?;
        std::fs::write(
            self.snapshots_dir.join(format!("snap-{seq}.json")),
            &state_json,
        )?;
        Ok(seq)
    }

    fn latest_snapshot(&self) -> Result<Option<(u64, ProjectedState)>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT seq, state FROM snapshots ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .ok();
        match row {
            Some((seq, state_json)) => {
                let state: ProjectedState = serde_json::from_str(&state_json)?;
                Ok(Some((seq as u64, state)))
            }
            None => Ok(None),
        }
    }

    /// Nº de eventos no log (queries só-leitura).
    pub fn event_count(&self) -> Result<u64, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Registros do log em ordem de `seq` (só-leitura). Permite inspecionar os CAMPOS de um evento
    /// no registro (ex.: asserir `id`/`item`/`by` de um `PlanClaimed`), não só sua presença.
    pub fn events(&self) -> Result<Vec<EventRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, ts, kind, version, payload FROM events ORDER BY seq ASC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, ts, kind, version, payload_str) = row?;
            out.push(EventRecord {
                seq: seq as u64,
                ts: ts as u64,
                kind,
                version: version as u32,
                payload: serde_json::from_str(&payload_str)?,
            });
        }
        Ok(out)
    }

    /// Caminho do diretório do workspace.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Caminho do banco SQLite.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

// ───────────────────────────── helpers ─────────────────────────────

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// `true` se `e` é uma violação de PK/UNIQUE do SQLite. O ÚNICO constraint do INSERT de evento é o PK
/// `seq` (schema: `seq INTEGER PRIMARY KEY`), então qualquer `ConstraintViolation` aqui É a colisão de
/// `seq` entre dois escritores — habilita o retry com re-leitura fresca (FURO A). Outros erros (I/O,
/// serde, db corrompido) NÃO casam → propagam sem retry (não mascaramos falha real).
fn is_seq_conflict(e: &StoreError) -> bool {
    matches!(
        e,
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(err, _))
            if err.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

/// `true` se o erro do rusqlite é `SQLITE_BUSY` ("database is locked") — disputa de lock transitória.
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _) if err.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

/// Liga `journal_mode=WAL` tolerando a CORRIDA DE SETUP entre conexões. Trocar o journal mode pede um
/// lock exclusivo momentâneo e — ao contrário dos writes — NÃO honra o `busy_timeout` de forma
/// confiável: sob abertura concorrente (app + bin abrindo o MESMO db), o 2º pode receber "database is
/// locked" exatamente AQUI (não no INSERT). WAL é PERSISTENTE e idempotente entre conexões (basta UM
/// processo setar), então re-tentamos BOUNDED com um respiro curto. Erro que NÃO seja `BUSY` propaga na
/// hora; se esgotar as tentativas ainda em `BUSY`, propaga o último — não silencia, só absorve a corrida.
fn enable_wal(conn: &Connection) -> Result<(), StoreError> {
    const TRIES: u32 = 50; // ~50 × 20ms = 1s — folga p/ a corrida de setup do WAL ceder
    let mut tries_left = TRIES;
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) && tries_left > 1 => {
                tries_left -= 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Anexa um registro de evento ao espelho JSONL (append-only, uma linha por evento).
fn append_jsonl(path: &Path, rec: &EventRecord) -> Result<(), StoreError> {
    // FURO B: UM único buffer (`linha` + `\n`) e UM único `write_all`. O espelho JSONL agora recebe
    // append CONCORRENTE de 2 processos (app + bin); com DOIS `write_all` (a linha e depois o `\n`), as
    // escritas O_APPEND dos dois podem INTERLEAVIAR no EOF → linha corrompida que cai na quarentena do
    // rebuild (W0-6) e some no replay. Com um `write_all` só, o kernel mantém a escrita contígua no EOF
    // (atomicidade do O_APPEND p/ um único write de uma linha de evento). O flush durável é mantido.
    let mut line = serde_json::to_string(rec)?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    file.flush()?;
    Ok(())
}

/// `true` se o `.db` está corrompido: `integrity_check` falha/≠ "ok", OU uma varredura
/// completa da tabela `events` (se existir) erra ("disk image is malformed").
fn db_is_corrupt(path: &Path) -> bool {
    let Ok(conn) = Connection::open(path) else {
        return true;
    };
    let integrity_bad = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .map(|s| s != "ok")
        .unwrap_or(true);
    if integrity_bad {
        return true;
    }
    // `integrity_check` pode passar com corrupção localizada; varre a tabela para
    // forçar a leitura das páginas. Tabela ausente = db novo/vazio (não é corrupção).
    let has_events = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='events'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .is_ok();
    if !has_events {
        return false;
    }
    let scan_ok = match conn.prepare("SELECT seq, kind, version, payload FROM events") {
        Ok(mut stmt) => match stmt.query_map([], |r| r.get::<_, i64>(0)) {
            Ok(rows) => rows.collect::<Result<Vec<i64>, _>>().is_ok(),
            Err(_) => false,
        },
        Err(_) => false,
    };
    !scan_ok
}

/// Renomeia o `.db` corrompido para `<nome>.corrupt-<ts>` (PRESERVA, nunca apaga) e
/// remove os arquivos `-wal`/`-shm` órfãos para não reanexar lixo ao banco novo.
fn preserve_corrupt(db_path: &Path) -> Result<PathBuf, StoreError> {
    let file_name = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("lina.db");
    let corrupt = db_path.with_file_name(format!("{file_name}.corrupt-{}", now_nanos()));
    std::fs::rename(db_path, &corrupt)?;
    for ext in ["-wal", "-shm"] {
        let mut side = db_path.as_os_str().to_os_string();
        side.push(ext);
        let _ = std::fs::remove_file(PathBuf::from(side));
    }
    Ok(corrupt)
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use uuid::Uuid;

    /// Diretório temporário único; removido no `Drop` (best-effort).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-evt-{tag}-{}", Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Host de UI gravador para checar `Recovering`/`Recovered`.
    #[derive(Default)]
    struct RecordingUi {
        events: Vec<HostEvent>,
    }
    impl UiHost for RecordingUi {
        fn on_event(&mut self, event: HostEvent) {
            self.events.push(event);
        }
    }

    fn seed(store: &mut EventStore) -> (NodeId, NodeId) {
        let dev = Uuid::now_v7();
        let qa = Uuid::now_v7();
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "App X".into(),
                focus_preset: String::new(),
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeAdded {
                node: dev,
                kind: "Terminal".into(),
                x: 10.0,
                y: 20.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeAdded {
                node: qa,
                kind: "Terminal".into(),
                x: 200.0,
                y: 50.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeMoved {
                node: dev,
                x: 15.0,
                y: 25.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::TerminalSpawned {
                node: dev,
                cli: "claude".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeRoleAssigned {
                node: dev,
                role: "dev".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::BusMessageSent {
                id: "msg_1".into(),
                from: dev,
                to: "role:qa".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::BusMessageSent {
                id: "msg_2".into(),
                from: qa,
                to: "node:dev".into(),
            })
            .unwrap();
        (dev, qa)
    }

    /// W0-5 (i): grava uma sequência, faz replay e o estado projetado bate.
    #[test]
    #[serial]
    fn replay_projects_expected_state() {
        let tmp = TempDir::new("replay");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let (dev, qa) = seed(&mut store);

        let state = store.project().expect("project");
        assert_eq!(state.workspace_name.as_deref(), Some("App X"));
        assert_eq!(state.nodes.len(), 2);
        assert_eq!(state.bus_messages, 2);

        let dev_node = state.nodes.get(&dev).expect("dev no estado");
        assert_eq!((dev_node.x, dev_node.y), (15.0, 25.0)); // NodeMoved aplicado
        assert_eq!(dev_node.cli.as_deref(), Some("claude")); // TerminalSpawned
        assert_eq!(dev_node.role.as_deref(), Some("dev"));
        assert_eq!(dev_node.status.as_deref(), Some("Running"));
        assert!(state.nodes.contains_key(&qa));
    }

    /// W0-5 (ii): replay 2x do mesmo log dá estado byte-idêntico (fingerprint igual).
    #[test]
    #[serial]
    fn replay_is_deterministic() {
        let tmp = TempDir::new("determinism");
        let mut store = EventStore::open(tmp.path()).expect("open");
        seed(&mut store);

        let f1 = store.project().expect("project 1").fingerprint();
        let f2 = store.project().expect("project 2").fingerprint();
        assert_eq!(
            f1, f2,
            "replays do mesmo log devem ter fingerprint idêntico"
        );

        // E com snapshot: snapshot + 0 eventos novos = mesmo estado.
        store.take_snapshot().expect("snapshot");
        let f3 = store.project().expect("project pós-snapshot").fingerprint();
        assert_eq!(f1, f3, "snapshot + replay vazio = mesmo estado");
    }

    /// W0-5 (iii): um evento com `event_version` ANTIGO passa pelo upcaster e
    /// reproduz sem erro (NodeAdded v1, sem o campo `kind`, vira `Terminal`).
    #[test]
    #[serial]
    fn legacy_event_version_is_upcasted_on_replay() {
        let tmp = TempDir::new("upcast");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let legacy_node = Uuid::now_v7();

        // payload v1: SEM `kind` (o campo só existe a partir da v2).
        let v1_payload = serde_json::json!({
            "event": "NodeAdded",
            "node": legacy_node,
            "x": 1.0,
            "y": 2.0
        });
        store
            .insert_raw("NodeAdded", 1, v1_payload)
            .expect("insert legado v1");

        // replay não pode errar e o nó deve ganhar kind=Terminal via upcaster.
        let state = store.project().expect("replay com upcasting");
        let n = state.nodes.get(&legacy_node).expect("nó legado projetado");
        assert_eq!(n.kind, "Terminal", "upcaster deve injetar kind=Terminal");
        assert_eq!((n.x, n.y), (1.0, 2.0));
    }

    /// W0-6: corrompe o `.db` (lixo no meio), reabre via `open_or_recover` e prova:
    /// corrompido PRESERVADO, estado reconstruído do JSONL IDÊNTICO, e o par
    /// `Recovering`→`Recovered` emitido.
    #[test]
    #[serial]
    fn corrupt_db_recovers_from_jsonl_visibly() {
        let tmp = TempDir::new("recover");

        // 1) Estado bom + snapshot; captura o fingerprint pré-corrupção.
        let pre_fp = {
            let mut store = EventStore::open(tmp.path()).expect("open");
            seed(&mut store);
            store.take_snapshot().expect("snapshot");
            let fp = store.project().expect("project").fingerprint();
            // checkpoint para materializar tudo no .db principal antes de corromper.
            store
                .conn
                .pragma_update(None, "wal_checkpoint", "TRUNCATE")
                .ok();
            fp
        }; // store dropado → conexão fechada

        // 2) Corrompe o .db: lixo no MEIO do arquivo (lição Codex #21750).
        let db_path = tmp.path().join("lina.db");
        corrupt_middle(&db_path);
        assert!(
            db_is_corrupt(&db_path),
            "o .db deveria ser detectado corrompido"
        );

        // 3) Reabre resiliente, com UI gravando os eventos de recuperação.
        let mut ui = RecordingUi::default();
        let store2 = EventStore::open_or_recover(tmp.path(), &mut ui).expect("open_or_recover");

        // 4a) o corrompido foi PRESERVADO (não apagado).
        let corrupt_files: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(
            corrupt_files.len(),
            1,
            "exatamente 1 arquivo *.corrupt-* preservado"
        );

        // 4b) estado reconstruído do JSONL é IDÊNTICO ao pré-corrupção.
        let post_fp = store2
            .project()
            .expect("project pós-recovery")
            .fingerprint();
        assert_eq!(post_fp, pre_fp, "reconstrução do JSONL deve ser idêntica");

        // 4c) recuperação VISÍVEL: par Recovering → Recovered.
        assert!(
            matches!(
                ui.events.as_slice(),
                [HostEvent::Recovering, HostEvent::Recovered]
            ),
            "deveria emitir exatamente Recovering depois Recovered; veio {:?}",
            ui.events
        );
    }

    /// `open_or_recover` num db SAUDÁVEL não dispara recuperação (sem eventos de UI).
    #[test]
    #[serial]
    fn healthy_open_does_not_recover() {
        let tmp = TempDir::new("healthy");
        {
            let mut store = EventStore::open(tmp.path()).expect("open");
            seed(&mut store);
        }
        let mut ui = RecordingUi::default();
        let _store = EventStore::open_or_recover(tmp.path(), &mut ui).expect("open_or_recover");
        assert!(
            ui.events.is_empty(),
            "db saudável não deve emitir Recovering/Recovered"
        );
    }

    /// W4-4 (T6): `WorkspaceFocusSet` projeta `focused_workspace` e sobrevive ao replay.
    #[test]
    #[serial]
    fn workspace_focus_set_projects_and_replays() {
        let tmp = TempDir::new("focus");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "App X".into(),
                focus_preset: String::new(),
            })
            .unwrap();
        store
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: "App X".into(),
            })
            .unwrap();
        let state = store.project().expect("project");
        assert_eq!(state.focused_workspace.as_deref(), Some("App X"));
    }

    /// W4-5: `WorkspaceCreated` v2 projeta `focus_preset`; um log v1 (sem o campo) passa pelo
    /// upcaster e projeta `None` — sem quebrar o replay.
    #[test]
    #[serial]
    fn workspace_focus_preset_v2_projects_and_v1_upcasts() {
        let tmp = TempDir::new("preset");
        let mut store = EventStore::open(tmp.path()).expect("open");
        // v2: com focus_preset.
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "App X".into(),
                focus_preset: "dev_app".into(),
            })
            .unwrap();
        assert_eq!(
            store.project().expect("project").focus_preset.as_deref(),
            Some("dev_app")
        );

        // v1: payload legado SEM focus_preset → upcaster injeta "" → projeta None.
        let tmp2 = TempDir::new("preset-v1");
        let mut store2 = EventStore::open(tmp2.path()).expect("open");
        let v1 = serde_json::json!({ "event": "WorkspaceCreated", "name": "Legacy" });
        store2
            .insert_raw("WorkspaceCreated", 1, v1)
            .expect("insert v1");
        let state = store2.project().expect("replay v1 upcasted");
        assert_eq!(state.workspace_name.as_deref(), Some("Legacy"));
        assert_eq!(state.focus_preset, None, "log v1 → preset não-setado");
    }

    /// W4-2 (P4): `CliProfileSet` projeta o `cli` do nó (idempotente, último vence).
    #[test]
    #[serial]
    fn cli_profile_set_projects_node_cli() {
        let tmp = TempDir::new("cliprofile");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let node = Uuid::now_v7();
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::CliProfileSet {
                node,
                profile: "claude".into(),
            })
            .unwrap();
        let state = store.project().expect("project");
        assert_eq!(
            state.nodes.get(&node).unwrap().cli.as_deref(),
            Some("claude")
        );
    }

    /// W4-1: `DiscoveryIndexed` projeta a lista corrente de CLIs e o round-trip preserva os campos.
    #[test]
    #[serial]
    fn discovery_indexed_projects_and_roundtrips() {
        let tmp = TempDir::new("discovery");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let clis = vec![
            DiscoveredCli {
                id: "claude".into(),
                version: Some("claude 1.2.3".into()),
                path: "/usr/local/bin/claude".into(),
            },
            DiscoveredCli {
                id: "codex".into(),
                version: None,
                path: "/usr/local/bin/codex".into(),
            },
        ];
        store
            .append(&DomainEvent::DiscoveryIndexed { clis: clis.clone() })
            .unwrap();
        let state = store.project().expect("project");
        assert_eq!(state.discovered_clis, clis, "projeção preserva os CLIs");
        // round-trip do payload no log (campos preservados).
        let rec = store
            .events()
            .unwrap()
            .into_iter()
            .find(|r| r.kind == "DiscoveryIndexed")
            .expect("evento no log");
        assert_eq!(rec.payload["clis"][0]["id"], "claude");
        assert_eq!(rec.payload["clis"][1]["version"], serde_json::Value::Null);
    }

    /// W4-2 (M3/M4): `NoteCreated`/`FolderCreated` são META — não alteram a projeção mas
    /// constam no log e o replay não erra.
    #[test]
    #[serial]
    fn note_and_folder_created_are_meta_but_logged() {
        let tmp = TempDir::new("notefolder");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let before = store.project().expect("project").fingerprint();
        store
            .append(&DomainEvent::NoteCreated {
                name: "ideia.md".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::FolderCreated {
                name: "docs".into(),
            })
            .unwrap();
        let after = store.project().expect("project");
        assert_eq!(
            after.fingerprint(),
            before,
            "Note/FolderCreated são META — projeção inalterada"
        );
        let kinds: Vec<String> = store
            .events()
            .unwrap()
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert!(kinds.contains(&"NoteCreated".to_string()));
        assert!(kinds.contains(&"FolderCreated".to_string()));
    }

    /// W4-4: `append_with_flush` notifica `Saving` → `Saved` na ordem e persiste o evento.
    #[test]
    #[serial]
    fn append_with_flush_signals_saving_then_saved() {
        let tmp = TempDir::new("flush");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let mut states = Vec::new();
        let seq = store
            .append_with_flush(
                &DomainEvent::WorkspaceCreated {
                    name: "App X".into(),
                    focus_preset: String::new(),
                },
                |s| states.push(s),
            )
            .expect("append_with_flush");
        assert_eq!(states, vec![FlushState::Saving, FlushState::Saved]);
        assert_eq!(store.event_count().unwrap(), 1);
        assert_eq!(seq, 1);
    }

    /// Sobrescreve um trecho contíguo no MEIO do arquivo com lixo (0xEE).
    fn corrupt_middle(path: &Path) {
        use std::io::{Seek, SeekFrom, Write};
        let len = std::fs::metadata(path).expect("metadata").len();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("abrir p/ corromper");
        let start = len / 4;
        let span = (len / 2).max(1);
        file.seek(SeekFrom::Start(start)).expect("seek");
        let garbage = vec![0xEE_u8; span as usize];
        file.write_all(&garbage).expect("escrever lixo");
        file.flush().expect("flush");
    }

    /// FUROS A+B — **append CONCORRENTE multi-conexão não perde nada**. Reproduz o cenário ATIVADO pela
    /// unificação do store (app + bin `lina do/guard` escrevendo no MESMO log de segurança): N threads,
    /// CADA UMA com sua PRÓPRIA `EventStore::open` apontando p/ o MESMO dir (= 2 processos/conexões).
    /// Prova: (a) ZERO evento perdido (`COUNT == total`); (b) seqs ÚNICOS e CONTÍGUOS (PK sem colisão
    /// perdida — FURO A); (c) o espelho JSONL tem `total` linhas TODAS desserializáveis (sem interleaving
    /// — FURO B). Sem os fixes, (a)/(b) quebram por colisão de `seq` e (c) por linha corrompida.
    #[test]
    #[serial]
    fn concurrent_appends_multi_connection_lose_nothing() {
        use std::io::BufRead;

        const WRITERS: u64 = 2; // app + bin (o caso canônico da unificação; o desenho generaliza p/ +)
        const PER_WRITER: u64 = 50;
        let total = WRITERS * PER_WRITER;

        let dir = TempDir::new("concurrent");
        let path = dir.path().to_path_buf();

        let handles: Vec<_> = (0..WRITERS)
            .map(|w| {
                let dir_w = path.clone();
                std::thread::spawn(move || {
                    // Conexão PRÓPRIA por thread (simula app+bin = processos distintos). Aberta DENTRO
                    // da thread → a `Connection` (não-`Send`) nunca cruza a fronteira.
                    let mut store = EventStore::open(&dir_w).expect("open store concorrente");
                    for i in 0..PER_WRITER {
                        store
                            .insert_raw("ConcTest", 1, serde_json::json!({ "writer": w, "i": i }))
                            .expect("insert_raw concorrente NÃO pode perder evento de segurança");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("writer thread");
        }

        let expected_seqs: Vec<u64> = (1..=total).collect();
        let store = EventStore::open(&path).expect("reabrir store");

        // (a) ZERO evento perdido.
        assert_eq!(
            store.event_count().unwrap(),
            total,
            "nenhum evento pode sumir sob escrita concorrente (FURO A: colisão de seq perderia o 2º)"
        );

        // (b) seqs ÚNICOS e CONTÍGUOS — sem buraco nem duplicata (PK retry-on-colisão).
        let db_seqs: Vec<u64> = store.events().unwrap().into_iter().map(|r| r.seq).collect();
        assert_eq!(
            db_seqs, expected_seqs,
            "seqs no SQLite devem ser únicos e contíguos 1..=total"
        );

        // (c) o espelho JSONL: `total` linhas, TODAS desserializáveis (FURO B: interleaving corromperia).
        let jsonl = path.join(JSONL_FILE);
        let file = std::fs::File::open(&jsonl).expect("abrir log.jsonl");
        let lines: Vec<String> = std::io::BufReader::new(file)
            .lines()
            .map(|l| l.expect("ler linha do jsonl"))
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert_eq!(
            lines.len() as u64,
            total,
            "o espelho JSONL deve ter exatamente 1 linha por evento (sem linha perdida/interleaviada)"
        );
        let mut jsonl_seqs: Vec<u64> = lines
            .iter()
            .map(|l| {
                serde_json::from_str::<EventRecord>(l)
                    .expect("cada linha do JSONL deve desserializar (sem interleaving — FURO B)")
                    .seq
            })
            .collect();
        jsonl_seqs.sort_unstable();
        assert_eq!(
            jsonl_seqs, expected_seqs,
            "o JSONL deve espelhar exatamente os mesmos seqs únicos/contíguos do SQLite"
        );
    }
}
