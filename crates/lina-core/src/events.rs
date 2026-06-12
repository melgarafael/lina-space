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
    /// F1-0-5 (`lina/msg@2`): `intent` fora do enum canônico, `reply` sem `reply_to`,
    /// ou schema desconhecido. Aditivo — logs antigos não o contêm (replay intacto).
    InvalidIntent,
    /// F1-0-5 (`lina/msg@2`): handoff sem o contrato completo (input/output_schema,
    /// timeout_sec, retry_policy). Aditivo como acima.
    InvalidContract,
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

/// F1-1-6: QUAL camada detectou o pedido de permissão (campo `evidence` do
/// [`DomainEvent::PermissionAsked`]). Serializa em `snake_case` (`"hook"`/`"grid"`).
/// `Hook` = detecção primária (Notification correlacionada ao `PreToolUse` pendente);
/// `Grid` = fallback por regex sobre o grid VT (heurística — FP medido, nunca "zero").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionEvidence {
    Hook,
    Grid,
}

/// `Grid` como default CONSERVADOR (replay de payload sem o campo lê a evidência de
/// MENOR confiança — nunca promove um registro degradado a detecção primária).
impl Default for PermissionEvidence {
    fn default() -> Self {
        Self::Grid
    }
}

/// F1-1-7/8 (ADR 0021 §2/§3): a DECISÃO de um [`DomainEvent::PermissionResolved`].
/// Serializa em `snake_case` (`"approve"`/`"deny"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

/// `Deny` como default CONSERVADOR: payload degradado/antigo NUNCA replaya como
/// aprovação (fail-safe na direção do ADR 0021 §3 — "deny é re-emissível; approve
/// não é des-apertável").
impl Default for ApprovalDecision {
    fn default() -> Self {
        Self::Deny
    }
}

/// F1-1-7/8 (ADR 0021 §2/§3): POR QUAL VIA a decisão nasceu — gesto humano na UI
/// autenticada ou SLA de timeout (auto-deny aos 10 min; nunca auto-approve).
/// Serializa em `snake_case` (`"human"`/`"timeout"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionVia {
    Human,
    Timeout,
}

/// `Timeout` como default CONSERVADOR: payload degradado NUNCA fabrica um gesto
/// humano que não foi registrado (doutrina do gate humano — ADRs 0004/0021 §5).
impl Default for ResolutionVia {
    fn default() -> Self {
        Self::Timeout
    }
}

/// R2b (direção do fundador: TODO bloqueante alerta): o FORMATO do prompt bloqueante
/// detectado — `yn` é aprovável/recusável pelo pipeline do ADR 0021; `choice` (caixa
/// de múltipla escolha do Claude Code, seleção numerada) e `trust` (diálogo "trust
/// this folder" 2.1.x) só ALERTAM + focam o terminal (a fila não oferece
/// aprovar/recusar — injetar `y` numa caixa de escolha seria input errado).
/// Serializa em `snake_case` (`"yn"`/`"choice"`/`"trust"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Yn,
    Choice,
    Trust,
}

/// `Yn` como default: payload antigo (pré-campo) era SEMPRE um prompt y/n — o
/// detector só emitia esse formato; replay de log antigo lê o comportamento da época.
impl Default for PromptKind {
    fn default() -> Self {
        Self::Yn
    }
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
        /// F1-3-6 (ADR 0019 §6): NodeId do agente que PEDIU este spawn via `lina spawn`
        /// (intent=spawn). `None` = criação direta pelo humano (UI/⌘T) — não passou pelo
        /// gate/cap de agente. Fecha o trio de auditoria intent-vs-action com
        /// [`DomainEvent::SpawnRequested`]/[`DomainEvent::SpawnGated`]. ADITIVO via
        /// `serde(default)`: logs antigos (shape `{node,kind,x,y}` v2) replayam com `None`
        /// SEM bump de versão nem entrada em [`upcast`] (campo `Option<T>` dispensa upcast —
        /// mesmo padrão de `TerminalSpawned.cwd`, ADR 0001 §2). META/auditoria: NÃO entra em
        /// [`ProjectedNode`] (consumido por replay — `lina retro`, painel de custo).
        #[serde(default)]
        requested_by: Option<NodeId>,
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
        /// F1-0-3 (ADR 0019): estado de ORIGEM da transição (`status` é o destino). Campo
        /// ADITIVO via `serde(default)` — logs antigos (shape `{node,status}`) replayam com
        /// `""`, sem upcast (ADR 0001 §2). Produzido pelo `lifecycle::LifecycleEngine`.
        #[serde(default)]
        from: String,
        /// F1-0-3: o SINAL que causou a transição (`pty_output`/`end_of_response`/`pty_exit`/
        /// `custody_gate`/…, consts em `lifecycle::reason`). Aditivo via `serde(default)`.
        #[serde(default)]
        reason: String,
    },
    NodeRemoved {
        node: NodeId,
    },
    TerminalSpawned {
        node: NodeId,
        cli: String,
        /// Rodada 360 (ADR 0022 §3): cwd REAL do spawn — o binding node↔cwd↔sessão que
        /// toda correlação de custo/atividade consome. ADITIVO via `serde(default)` —
        /// logs antigos (shape `{node,cli}`) replayam com `None`, sem upcast; as
        /// projeções degradam honestamente ("custo não rastreável aqui"), nunca chutam.
        #[serde(default)]
        cwd: Option<String>,
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
    /// F1-0-4 (P1 — entrega ciente de estado): a mensagem `id` foi RETIDA (não injetada)
    /// porque o alvo `to` estava `Busy` na state-machine F1-0-3 (`reason: target_busy` —
    /// inclui o Busy carimbado pelo próprio router na entrega anterior, que é o que
    /// serializa rajadas ao mesmo alvo). Apendado **1× por mensagem** (anti-
    /// amplificação A4) — a entrega posterior aparece como `MessageRouted`/
    /// `MessageDelivered` normais; o intervalo entre este evento e eles é a ESPERA
    /// auditável. META — sem efeito na projeção (livro-razão da espera, não estado de
    /// canvas); aditivo: logs antigos não o contêm (replay/fingerprint intactos).
    MessageRetained {
        id: String,
        to: NodeId,
        reason: String,
    },
    /// F1-0-7 (ADR 0020): uma TENTATIVA de entrega ao PTY falhou. `attempt` é 1-based;
    /// `next_retry_ms` é o agendamento (backoff exponencial + jitter determinístico por
    /// id) — a série destes eventos no log PROVA o backoff por timestamps E torna o
    /// estado de retry DURÁVEL (um restart retoma do attempt seguinte, não do zero).
    /// Bounded por `delivery_max_attempts` (anti-amplificação A4). META.
    MessageDeliveryFailed {
        id: String,
        to: NodeId,
        attempt: u32,
        error: String,
        next_retry_ms: u64,
    },
    /// F1-0-7 (ADR 0020): a mensagem foi movida à **dead-letter queue** durável
    /// (`<.lina>/dead-letter/<id>.json`) com motivo legível — estouro da retenção,
    /// esgotamento das tentativas de entrega, ou destino inexistente persistente.
    /// NADA some, NADA loopa: o retry é MANUAL (humano re-enfileira o arquivo). `to` é
    /// NOME (cobre destino que nunca resolveu a `NodeId`). META.
    MessageDeadLettered {
        id: String,
        to: String,
        reason: String,
    },
    /// F1-0-7 (ADR 0020): circuit breaker — `failures` falhas CONSECUTIVAS de entrega no
    /// nó → `Blocked` (`NodeStatusChanged{reason:"circuit_breaker"}`) e novas mensagens
    /// retêm/vão à DLQ SEM tentar (anti retry-storm), até reset externo (lifecycle/humano
    /// tirar o nó de `Blocked`). META.
    CircuitOpened {
        node: NodeId,
        failures: u32,
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
        /// FIX-2 (dogfood): NOME do terminal (`LINA_NODE_NAME`) que disparou o gate. A fila de
        /// atenção consome `ActionGated{decision:"ask", node:Some}` para ALERTAR + FOCAR o terminal
        /// bloqueado (o foco é POR NOME — `attention_goto_node`/ADR 0026). `None` nos caminhos de
        /// CUSTÓDIA (broker `run_custody`/`lina do`, que já têm item próprio na fila — `node:Some`
        /// ali viraria GuardAsk DUPLICANDO a custódia) e no log antigo. ADITIVO via `serde(default)`:
        /// payload pré-FIX-2 (`{cmd,class,decision}`) replaya com `None` SEM upcast (campo `Option`,
        /// padrão `NodeAdded.requested_by`). META — sem efeito na projeção.
        #[serde(default)]
        node: Option<String>,
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
    /// r5 perf-ws — **Descarregar Espaço** (modelo Maestri do fundador): os PTYs OCIOSOS de um
    /// Espaço de fundo foram desligados SEM remover os nós (religam no próximo foco via o fluxo
    /// de restore F1-4-3). `parked` = ids dos nós estacionados; `kept` = vivos preservados
    /// (Busy/Blocked — a restrição "sem interferir no que está rodando"). META — livro-razão do
    /// gesto; o executor (app) deriva o plano de [`crate::suspend::unload_plan`] e religa
    /// varrendo o log (mesmo padrão CostLedger).
    WorkspaceUnloaded {
        parked: Vec<String>,
        #[serde(default)]
        kept: Vec<String>,
    },
    /// W4-3 (freio do rodapé): a auto-orquestração foi PAUSADA pelo humano. Enquanto pausada, novas
    /// delegações ficam ENFILEIRADAS (não injetadas) — é GATE, não kill (inv #6): nada se perde, o
    /// estado fica salvo e visível. META — o estado de pausa vive no Router (escritor único); aqui só
    /// registra o fato no log (livro-razão do freio, par de [`DomainEvent::OrchestrationResumed`]).
    OrchestrationPaused,
    /// W4-3 (freio do rodapé): a auto-orquestração foi RETOMADA → a fila de delegações represadas é
    /// drenada (roteada/entregue agora). META — par de [`DomainEvent::OrchestrationPaused`].
    OrchestrationResumed,
    /// W5-4 (nó-Gatilho): um webhook foi CONFIGURADO — rota opaca (`hook_id` base32 não-enumerável)
    /// ligada a um `target_ref` (endereço A2A do destino: nome de nó, `role:…` ou `*`). O **secret
    /// HMAC NUNCA entra aqui**: vive no Secret Vault (W0-7) — o log é a fonte da config (rota+alvo), não
    /// do segredo (invariante #2 "nada em claro no disco"). META — sem efeito na projeção do canvas; o
    /// engine de webhooks reconstrói suas ligações varrendo o log (mesmo padrão do `CostLedger` §2.2).
    WebhookConfigured {
        hook_id: String,
        target_ref: String,
    },
    /// W5-4 (nó-Gatilho): um POST externo VÁLIDO (HMAC conferido) chegou em `hook_id` e foi aceito (202).
    /// `ts` = instante de RECEPÇÃO em millis, distinto do `ts` de persistência do `EventRecord` (o
    /// processamento roda FORA do caminho da resposta — invariante "202 imediato, processa async"). É o
    /// livro-razão do gatilho: prova observável de que o disparo externo entrou no sistema. META — o
    /// efeito (mensagem ao alvo) é o `BusEvent::Message` no bus; aqui registra-se o FATO no log (fonte
    /// da verdade, invariante #4). HMAC inválido NÃO chega a este evento (recusado com 401 sem publicar).
    WebhookReceived {
        hook_id: String,
        ts: u64,
        target_ref: String,
    },
    SnapshotTaken {
        seq: u64,
    },
    /// F1-5-8: um nó leu o histórico de OUTRO painel (`tail`/`search` cross-terminal) —
    /// livro-razão de auditoria da fronteira de pertencimento (ADR 0006). Toda leitura
    /// cross é um fato observável; same-owner NÃO entra aqui (alto volume/baixo sinal).
    /// META — sem efeito na projeção do canvas; `query` é o TIPO da consulta ("tail"/
    /// "search"), nunca o conteúdo (o payload da busca não vaza ao log).
    HistoryReadCross {
        reader: NodeId,
        panel: String,
        query: String,
    },
    /// F1-0-3 (ADR 0019 §3): nó `Busy` acumulou `stall_warn_samples` amostras consecutivas SEM
    /// progresso (tail_hash estagnado E zero evento de domínio atribuível) → WARN. Emitido **uma
    /// única vez, na transição para stalled** (anti-amplificação, ADR 0003/0005) pelo
    /// `lifecycle::LifecycleEngine`; re-arma se houver progresso e novo stall. As amostras cruas
    /// do heartbeat são EFÊMERAS (alto volume/baixo sinal) — só o veredito entra no log.
    /// `cycle_count` = contagem de advances-com-output no instante do veredito (diagnóstico).
    NodeStalled {
        node: NodeId,
        cycle_count: u64,
    },
    /// F1-5-5: nó ocioso SUSPENSO (overlay de render). O nó segue VIVO e drenando — só a montagem
    /// no shell pausa; portanto NÃO mexe no status do roster (suspensão ⟂ lifecycle). Projeta um
    /// flag consultável (`ProjectedNode.suspended`), no mesmo idioma do `stalled`. ADITIVO.
    NodeSuspended {
        node: NodeId,
        /// Por que suspendeu (canônico em `suspend::reason`, ex.: `idle_timeout`).
        #[serde(default)]
        reason: String,
    },
    /// F1-5-5: nó suspenso que ACORDOU (A2A dirigida, foco, prompt/custódia ou retomada de output).
    NodeResumed {
        node: NodeId,
        /// Por que acordou (canônico em `suspend::reason`, ex.: `a2a_delivered`, `focus`).
        #[serde(default)]
        reason: String,
    },
    /// F1-1-1 (coordenado via Maestro; consumidor: Dev 02): um CLI de IA foi DETECTADO num
    /// terminal — conversão campo-a-campo de `lina_cli_profiles::CliDetection`. A emissão no
    /// supervisor é fiação posterior (esta rodada só reserva o contrato no log). META — sem
    /// efeito na projeção do canvas; `node_id` é `String` espelhando o shape de origem.
    CliDetected {
        node_id: String,
        cli: String,
        confidence: f32,
        evidence: String,
    },
    /// F1-1-6 (consumidores F1-1-7/8 na próxima rodada): um terminal está BLOQUEADO
    /// aguardando aprovação humana — pedido de permissão detectado pelo
    /// `permission_detect::PermissionDetector` (hook `Notification` correlacionado ao
    /// `PreToolUse` pendente, ou fallback por regex no grid — ver `evidence`).
    /// `stable_id` é a chave de IDEMPOTÊNCIA derivada de `(session, tool_call, ts)`
    /// (padrão OpenCode, 13.13 achado 4): consumidores deduplicam por ela — dois
    /// pedidos seguidos do MESMO nó têm `stable_id` distintos (o `ts` difere), e o
    /// replay nunca duplica (o log guarda fatos; a detecção não roda no replay).
    /// META — sem efeito na projeção do canvas; a fila de atenção (F1-1-7) reconstrói
    /// as pendências varrendo o log (padrão `CostLedger`). Campos com `serde(default)`:
    /// payload de shape antigo/parcial replaya com defaults, nunca quebra (ADR 0001 §2).
    PermissionAsked {
        /// Nó atribuído pelo TOKEN de spawn (hook) ou pelo roster (grid) — `String`
        /// espelhando o shape de origem do pipeline de hooks (precedente: `CliDetected`).
        #[serde(default)]
        node_id: String,
        /// Tool correlacionada (`"Bash"`, …) quando o `PreToolUse` precedente existir.
        #[serde(default)]
        tool: Option<String>,
        /// Resumo legível do pedido: o COMANDO (`"git push origin/master"`) quando a
        /// origem o expõe, ou a linha do prompt no grid (fallback). É o que permite ao
        /// toast (F1-1-7) dizer O QUE pede aprovação, não só "precisa de aprovação".
        #[serde(default)]
        detail: Option<String>,
        /// Camada que detectou (`hook`/`grid`) — confiabilidade NÃO-uniforme por design.
        #[serde(default)]
        evidence: PermissionEvidence,
        #[serde(default)]
        stable_id: String,
        /// R2b (ADR 0021 §1): a CAPTURA 1 — hash da região do prompt no grid, anexado
        /// pelo detector na detecção. Correlaciona com o `vt_snapshot_hash` do
        /// `ApprovalInjected` (auditoria da pré-condição) e torna a fila reconstruída
        /// por replay APROVÁVEL (o gesto pós-crash valida contra a captura original).
        /// `None` = origem sem captura (ex.: caminho hook sem acesso ao grid).
        #[serde(default)]
        vt_snapshot_hash: Option<String>,
        /// R2b (direção do fundador: todo bloqueante alerta): formato do prompt —
        /// `yn` aprovável; `choice`/`trust` alertam + focam, sem aprovar/recusar.
        /// Default `yn` = comportamento do log antigo (pré-campo).
        #[serde(default)]
        prompt_kind: PromptKind,
    },
    /// F1-2-3 parte 2 (costura adiantada nesta rodada; consumidor é o modal de papéis,
    /// na próxima): um papel CUSTOM do usuário foi salvo (modelo duplicar+modificar da
    /// biblioteca) — persiste como evento e REAPARECE nos próximos modais. Heurística
    /// local + templates, ZERO LLM embutido (invariante #1). `ts` = instante do salvar
    /// em ms (relógio do usuário, distinto do `ts` de persistência do `EventRecord`).
    /// META — sem efeito na projeção do canvas; o modal reconstrói a biblioteca custom
    /// varrendo o log (último `name` vence — padrão `CostLedger`). `serde(default)` em
    /// todos os campos: replay de payload antigo/parcial nunca quebra.
    RoleTemplateSaved {
        /// Nome do papel em linguagem de leigo ("Revisora", "Designer de Landing").
        #[serde(default)]
        name: String,
        /// Descrição curta leiga (a doutrina técnica fica atrás de "ver detalhes").
        #[serde(default)]
        description: String,
        /// Doutrina/role completa (o texto técnico injetado no bootstrap turno-0).
        #[serde(default)]
        doctrine: String,
        #[serde(default)]
        ts: u64,
    },
    /// F1-1-7/8 (ADR 0021 §2 — ordem: Resolved → write → Injected): a DECISÃO sobre um
    /// pedido de permissão (`stable_id` correlaciona ao `PermissionAsked`), por gesto
    /// humano (via=human) ou SLA de 10 min (via=timeout, decision=deny — NUNCA
    /// auto-approve, sem knob). **Emitido pelo EXECUTOR de entrega (porta única,
    /// `deliver_approval`), que assina o desfecho inteiro Resolved → Injected/Aborted/
    /// DuplicateIgnored — um só escritor garante a ordem do §2 por construção; a fila
    /// apenas chama `deliver()`** (`AttentionQueue::resolve` é o VALIDADOR do gesto:
    /// alias→canônico + idempotência; não apenda — arbitragem registrada em f4256f0).
    /// A decisão NÃO é o efeito: o write no PTY é exclusivo do executor
    /// (`ApprovalInjected`).
    /// META — a fila reconstrói pendências varrendo o log (pendente = `PermissionAsked`
    /// sem Resolved/Dismissed posterior do mesmo `stable_id`).
    PermissionResolved {
        #[serde(default)]
        stable_id: String,
        /// `approve`/`deny` — default CONSERVADOR `deny` (replay degradado nunca
        /// fabrica aprovação).
        #[serde(default)]
        decision: ApprovalDecision,
        /// `human`/`timeout` — default CONSERVADOR `timeout` (replay degradado nunca
        /// fabrica gesto humano).
        #[serde(default)]
        via: ResolutionVia,
    },
    /// F1-1-8 (ADR 0021 §2 — o EFEITO): o write de `approval_keys` aconteceu no PTY
    /// alvo, com a tela validada no instante do write. `vt_snapshot_hash` é o hash da
    /// região do prompt que a validação conferiu — evidência auditável da pré-condição.
    /// META — consumido pelo ledger de idempotência do executor (porta única de escrita).
    ApprovalInjected {
        #[serde(default)]
        stable_id: String,
        #[serde(default)]
        vt_snapshot_hash: String,
    },
    /// F1-1-8 (ADR 0021 §1/§4): a injeção foi ABORTADA sem escrever nenhum byte —
    /// `reason` em snake_case aberto (`"screen_changed"` = snapshot divergiu entre a
    /// detecção e o gesto; `"target_mismatch"` = cross-check stable_id↔node_id falhou).
    /// String (não enum) de propósito: razões novas entram sem quebrar replay antigo.
    /// META — auditável como Resolved + Aborted, SEM Injected (ADR 0021 §2).
    ApprovalAborted {
        #[serde(default)]
        stable_id: String,
        #[serde(default)]
        reason: String,
    },
    /// F1-1-8 (ADR 0021 §2): segunda via de aprovação do MESMO `stable_id` já
    /// resolvido — no-op auditado, emitido NO MÁXIMO 1× por pedido (anti-amplificação,
    /// ADR 0003). Aprovar 2× injeta exatamente 1×. META.
    ApprovalDuplicateIgnored {
        #[serde(default)]
        stable_id: String,
    },
    /// F1-1-7 (mitigação de FP como produto — decisão do Maestro): o humano marcou o
    /// item como "não era um pedido" (botão da fila/toast). Remove a pendência SEM
    /// decidir permissão (não é deny — nada será escrito nem negado ao agente) e
    /// alimenta a telemetria de FP do detector (`record_false_positive`, rótulo
    /// externo — nunca auto-inferido, #28174). META.
    PermissionDismissed {
        #[serde(default)]
        stable_id: String,
    },
    /// F1-1-7 (allowlist por nó — decisão do Maestro): liga/desliga o fallback de
    /// detecção por grid PARA UM NÓ (CLI cujo output dispara FPs estruturais).
    /// Persistido como evento e REVERSÍVEL: o último evento do nó vence (padrão
    /// CostLedger) — `muted:false` re-liga. Só afeta a camada de GRID; a detecção
    /// por hook continua (é estrutural, não-heurística). META.
    NodeDetectionMuted {
        #[serde(default)]
        node_id: String,
        /// Default `true`: payload mínimo `{node_id}` significa "mutado" (casa com o
        /// nome do evento; `false` só existe explicitamente, no unmute).
        #[serde(default = "default_muted")]
        muted: bool,
    },
    /// R2b item 5 (pedido Dev 01 + UI, aprovado pelo Maestro): o prompt detectado
    /// SUMIU do grid sem decisão pela fila — o humano respondeu DIRETO no terminal
    /// (vale para y/n e choice; emitido pelo chamador de
    /// `PermissionDetector::take_cleared`). A fila trata como REMOÇÃO no fold (mesma
    /// família de Resolved/Dismissed), mas a semântica no log é distinta e
    /// deliberada: NÃO é `PermissionDismissed` (o pedido era REAL — alimentaria a
    /// telemetria de FP errado) e NÃO é `PermissionResolved` (ninguém decidiu pela
    /// fila — o log guarda fatos, não inferências). META.
    PermissionPromptCleared {
        #[serde(default)]
        node_id: String,
        #[serde(default)]
        stable_id: String,
    },
    /// F1-3-6 (ADR 0019 §6): um agente PEDIU criar um terminal via `lina spawn` (intent=spawn).
    /// META (livro-razão do PEDIDO — intent-vs-action 13.14 P1): registra os args reais + a cadeia
    /// EFETIVA, SEMPRE e ANTES da decisão do gate (par de [`DomainEvent::SpawnGated`]/
    /// `NodeAdded.requested_by`). `requested_by` é o sender AUTENTICADO pelo dir-dono do outbox
    /// (JAMAIS o campo `from`); `root_cause_id`/`hops` vêm do binding inforjável
    /// (`Router::derive_root_hops`), NUNCA dos campos forjáveis `msg.root_cause_id`/`msg.hops`.
    /// Consumido por replay (auditoria / `lina retro` / painel de custo); sem efeito na projeção
    /// do canvas (silêncio no [`apply`]). Variante NOVA → sem `serde(default)` nos campos (replay de
    /// log antigo nunca a encontra — precedente `AwaitOpened`).
    SpawnRequested {
        /// `= msg.id` (dedupe/correlação com `SpawnGated`/`NodeAdded` do mesmo pedido).
        id: String,
        /// Sender AUTENTICADO (dir-dono do outbox), NUNCA o campo `from` (forjável).
        requested_by: NodeId,
        /// `@Nome` pedido para o novo terminal.
        name: String,
        /// Papel pedido (`qa`/`designer`/…).
        role: String,
        /// Root EFETIVO da cadeia (do binding `delivered_root`, não do campo forjável).
        root_cause_id: String,
        /// Profundidade EFETIVA (`0` = origem; `>=1` = cascata).
        hops: u8,
        /// SEAM-1: 1º prompt do spawn (payload do envelope). Logado AQUI para o banner de um spawn
        /// GATED (cascata) ser recuperável do log (a fila de atenção reconstrói o pedido pendente do
        /// `SpawnRequested` no replay/pós-crash — o `RouteOutcome` é efêmero). ADITIVO via
        /// `serde(default)`: `SpawnRequested` de logs F1-3-6 (sem o campo) replaya com `""`.
        #[serde(default)]
        prompt: String,
    },
    /// F1-3-6 (ADR 0019 §6): o gate barrou/adiou um spawn (livro-razão da DECISÃO, par do
    /// `SpawnRequested`). `reason` ∈ {`cascade`,`over_cap`,`manual`,`cost`} — String (não enum) de
    /// propósito: razões novas entram sem quebrar replay antigo (mesmo padrão de
    /// `ApprovalAborted.reason`). META — sem efeito na projeção; o painel de custo/`lina retro` o
    /// reconstroem varrendo o log.
    SpawnGated {
        id: String,
        requested_by: NodeId,
        reason: String,
    },
    /// SEAM-1 (ADR 0022): o spawn `id` foi EXECUTADO — o app admitiu o terminal (`node`) via o funil
    /// `admit_node`. É a chave de DEDUPE DURÁVEL (M3): antes de admitir, o app varre o log por
    /// `SpawnAdmitted{id}`; presente → no-op (replay/re-drain/crash-recovery → 1 terminal só). Também
    /// RESOLVE o item de banner (`SpawnGated{cascade}` deixa de ser pendente). META — a criação
    /// física projeta via `NodeAdded` (este só correlaciona spawn↔nó, que `NodeAdded` não carrega).
    SpawnAdmitted {
        /// `= SpawnRequested.id` (== `root_cause_id` na origem) — o id durável do pedido de spawn.
        id: String,
        node: NodeId,
    },
    /// SEAM-1: o humano RECUSOU um spawn gated (cascata) na fila de atenção — NADA nasce. RESOLVE o
    /// item de banner (par de `SpawnAdmitted` para o ramo deny), para o pedido não reaparecer no
    /// replay/pós-crash. META.
    SpawnDeclined {
        /// `= SpawnRequested.id` do pedido recusado.
        id: String,
    },
    /// F1-3-7 (RESERVADO up-front por Dev 01 — Maestro decisão 3 §12.3: dono único de `events.rs`
    /// na Rodada 3 batcheia TODAS as variantes da onda, eliminando colisão de merge com F1-3-7).
    /// Uma skill foi INVOCADA por um nó. META — o `lina retro` conta invocações e deriva "última
    /// atividade" do `EventRecord.ts` (candidatas a stale >30d / archive >90d). **Shape = placeholder
    /// do seam §12.3**; o spec de F1-3-7 (Dev 02 / LLM Engineer) confirma os campos finais ANTES de
    /// consumir — ESCALADO ao Orquestrador (ver `.entrega-f1-3-6.md`). Variante NOVA → sem
    /// `serde(default)`.
    SkillInvoked {
        node: NodeId,
        skill: String,
    },
    /// F1-3-7 (RESERVADO up-front — ver [`DomainEvent::SkillInvoked`]): uma skill foi CRIADA por um
    /// nó. META. Shape placeholder do seam §12.3 (confirmar com Dev 02 antes de consumir).
    SkillCreated {
        node: NodeId,
        skill: String,
    },
    /// F1-4-1: identidade DURÁVEL do Espaço, atribuída na criação (`Workspace::create`).
    /// Variante NOVA em vez de campo aditivo em [`DomainEvent::WorkspaceCreated`]: o campo
    /// quebraria os 13 construtores existentes fora da fronteira desta rodada (gates, app,
    /// bootstrap); a variante preserva replay E compilação alheia. Logs legados não a
    /// contêm → a projeção degrada para `None` e o registry usa o path como id (honesto,
    /// nunca chuta). O registry global (`~/.lina/workspaces.json`) é PONTEIRO desta
    /// identidade (mini-ADR F1-4-1).
    ///
    /// ⚠️ Escopo da "identidade" (red-team F1-4-1): este evento é a verdade do *fato*
    /// "qual id este Espaço carrega" (invariante #4) — ele NÃO é, e nunca deve virar,
    /// autoridade de AUTORIZAÇÃO: o namespace de trust cross-Espaço do ADR 0010 deriva
    /// do roster VIVO do Supervisor (autenticação em 2 camadas, ADR 0006), nunca de um
    /// campo gravado em arquivo local que quem tem acesso de disco pode adulterar.
    WorkspaceIdAssigned {
        workspace_id: String,
    },
    /// F1-4-1 (fundador 2026-06-09; precedência do ADR 0022 §4): o **Diretório de
    /// Trabalho** do Espaço foi definido/trocado — vira o cwd PADRÃO de todo NOVO
    /// terminal/agente (nós vivos não migram — F1-2-4). `""` LIMPA (volta ao fallback
    /// gerenciado virgem `n-<key>`). Último vence (padrão de `WorkspaceFocusSet`).
    /// Evento-set em vez de campo num `WorkspaceCreated` v3: o critério 6 da story exige
    /// TROCAR depois da criação — o set cobre criação E edição com um só mecanismo.
    /// Consumidor: o seam único `workspace::resolve_spawn_cwd` (que NUNCA assume
    /// cwd-único-por-nó — N nós no mesmo projeto é o padrão; identidade vem do spawn,
    /// ADR 0026 em rascunho).
    WorkspaceDefaultCwdSet {
        cwd: String,
    },
    /// F1-4-1 (ciclo de vida): o Espaço foi renomeado. Último vence; o cabeçalho do
    /// plano espelha o nome corrente (mesma regra da criação).
    WorkspaceRenamed {
        name: String,
    },
    /// F1-4-1 (ciclo de vida): o Espaço foi ARQUIVADO — sai da contagem ATIVA do gating
    /// free/PRO (a vaga libera; ver `workspace::can_create_workspace`). Desarquivar, se a
    /// Fase 1 precisar, é evento aditivo futuro — o log preserva a história inteira.
    WorkspaceArchived,
    /// F2-0-1 (régua D0 camada d; épico 38 §VI): resumo de UMA janela da sonda `[PROF]`
    /// (~120 frames) persistido como fato — o gate de perf da F2 (p95 ≤ orçamento E
    /// excess-time <1%) fica re-derivável por replay. Numerador/denominador BRUTOS
    /// (`excess_ms`/`active_ms`): re-agregar janelas é soma de somas, exata — a % nunca
    /// é gravada (é projeção, não fato). META/medição: NÃO entra em projeções de roster.
    /// ADITIVO: variante nova; logs antigos replayam intactos (ADR 0001 §2).
    ProfWindowMetric {
        frames: usize,
        budget_ms: f64,
        active_ms: f64,
        excess_ms: f64,
        frames_over_budget: usize,
        frame_p50_ms: f64,
        frame_p95_ms: f64,
        frame_p99_ms: f64,
        /// Scale factor da janela na medição (1.0 = LoDPI — gap §II.8 da onda V);
        /// `None` = costura do render não fiada (medição sem carimbo).
        #[serde(default)]
        scale_factor: Option<f64>,
    },
    /// F2-2-4 (costura): um comando da paleta foi EXECUTADO (Enter). `key` é a chave estável
    /// de `palette::Command::key()` do shell (ex.: `"new_agent"`, `"focus_node:<uuid>"`).
    /// Alimenta o tier 4 do ranking (hit-count) como PROJEÇÃO por replay — o modelo da
    /// paleta nunca fabrica contagem (inv#4). META: não entra na projeção do canvas.
    /// ADITIVO: logs antigos replayam intactos (ranking degrada sem hit-count).
    PaletteCommandInvoked {
        key: String,
    },
}

/// Default do campo `muted` de [`DomainEvent::NodeDetectionMuted`] — `true` para o
/// payload mínimo casar com a semântica do nome do evento.
fn default_muted() -> bool {
    true
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
            DomainEvent::MessageRetained { .. } => "MessageRetained",
            DomainEvent::MessageDeliveryFailed { .. } => "MessageDeliveryFailed",
            DomainEvent::MessageDeadLettered { .. } => "MessageDeadLettered",
            DomainEvent::CircuitOpened { .. } => "CircuitOpened",
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
            DomainEvent::WorkspaceUnloaded { .. } => "WorkspaceUnloaded",
            DomainEvent::OrchestrationPaused => "OrchestrationPaused",
            DomainEvent::OrchestrationResumed => "OrchestrationResumed",
            DomainEvent::WebhookConfigured { .. } => "WebhookConfigured",
            DomainEvent::WebhookReceived { .. } => "WebhookReceived",
            DomainEvent::SnapshotTaken { .. } => "SnapshotTaken",
            DomainEvent::HistoryReadCross { .. } => "HistoryReadCross",
            DomainEvent::NodeStalled { .. } => "NodeStalled",
            DomainEvent::NodeSuspended { .. } => "NodeSuspended",
            DomainEvent::NodeResumed { .. } => "NodeResumed",
            DomainEvent::CliDetected { .. } => "CliDetected",
            DomainEvent::PermissionAsked { .. } => "PermissionAsked",
            DomainEvent::RoleTemplateSaved { .. } => "RoleTemplateSaved",
            DomainEvent::PermissionResolved { .. } => "PermissionResolved",
            DomainEvent::ApprovalInjected { .. } => "ApprovalInjected",
            DomainEvent::ApprovalAborted { .. } => "ApprovalAborted",
            DomainEvent::ApprovalDuplicateIgnored { .. } => "ApprovalDuplicateIgnored",
            DomainEvent::PermissionDismissed { .. } => "PermissionDismissed",
            DomainEvent::NodeDetectionMuted { .. } => "NodeDetectionMuted",
            DomainEvent::PermissionPromptCleared { .. } => "PermissionPromptCleared",
            DomainEvent::SpawnRequested { .. } => "SpawnRequested",
            DomainEvent::SpawnGated { .. } => "SpawnGated",
            DomainEvent::SpawnAdmitted { .. } => "SpawnAdmitted",
            DomainEvent::SpawnDeclined { .. } => "SpawnDeclined",
            DomainEvent::SkillInvoked { .. } => "SkillInvoked",
            DomainEvent::SkillCreated { .. } => "SkillCreated",
            DomainEvent::WorkspaceIdAssigned { .. } => "WorkspaceIdAssigned",
            DomainEvent::WorkspaceDefaultCwdSet { .. } => "WorkspaceDefaultCwdSet",
            DomainEvent::WorkspaceRenamed { .. } => "WorkspaceRenamed",
            DomainEvent::WorkspaceArchived => "WorkspaceArchived",
            DomainEvent::ProfWindowMetric { .. } => "ProfWindowMetric",
            DomainEvent::PaletteCommandInvoked { .. } => "PaletteCommandInvoked",
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
    /// então desserializa na forma corrente. `pub(crate)` para projeções-irmãs que
    /// varrem `EventRecord`s (F1-1-7: `attention::AttentionQueue::replay`).
    pub(crate) fn from_record(
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
    /// Rodada 360 (ADR 0022 §3): cwd REAL do último `TerminalSpawned` do nó — o binding
    /// node↔cwd que o painel de custo correlaciona (passo 7 do blueprint). `None` = log
    /// histórico sem o campo (degradação honesta, nunca chute). `serde(default)` →
    /// snapshots antigos desserializam com `None`.
    #[serde(default)]
    pub cwd: Option<String>,
    /// F1-0-3: WARN de travamento (`NodeStalled`) consultável; limpo na PRÓXIMA transição de
    /// status do nó (conservador — recuperação sem mudança de estado mantém o WARN até o estado
    /// confirmar saúde). `serde(default)` → snapshots antigos desserializam com `false`.
    #[serde(default)]
    pub stalled: bool,
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
    /// F1-4-1: identidade durável do Espaço (do `WorkspaceIdAssigned`). `None` = log
    /// legado sem o evento — o registry degrada para o path como id (honesto, nunca
    /// chuta). `#[serde(default)]` → snapshots antigos desserializam como `None`.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// F1-4-1 (fundador 2026-06-09 / ADR 0022): Diretório de Trabalho do Espaço — cwd
    /// PADRÃO de todo novo terminal/agente (do último `WorkspaceDefaultCwdSet`; `""`
    /// limpou). `None` = sem default → fallback dir gerenciado virgem `n-<key>` (seam
    /// único `workspace::resolve_spawn_cwd`). `#[serde(default)]` → snapshots antigos OK.
    #[serde(default)]
    pub default_cwd: Option<String>,
    /// F1-4-1: o Espaço foi arquivado (`WorkspaceArchived`) — fora da contagem ativa do
    /// gating free/PRO. `#[serde(default)]` → snapshots antigos desserializam `false`.
    #[serde(default)]
    pub archived: bool,
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
        DomainEvent::NodeAdded {
            node,
            kind,
            x,
            y,
            // F1-3-6: META/auditoria (quem pediu o spawn) — NÃO entra na projeção do canvas
            // (`ProjectedNode`); consumido por replay (`lina retro`/painel de custo).
            requested_by: _,
        } => {
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
                    cwd: None,
                    stalled: false,
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
        DomainEvent::NodeStatusChanged { node, status, .. } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.status = Some(status.clone());
                // F1-0-3: qualquer transição de status limpa o WARN de stall — o estado
                // novo é a confirmação de saúde (ADR 0019; ver doc de `ProjectedNode.stalled`).
                n.stalled = false;
            }
        }
        // F1-0-3: WARN de travamento — liga o flag consultável; NÃO muda o status (o stall é
        // um veredito sobre um nó que SEGUE Busy, não um estado da máquina).
        DomainEvent::NodeStalled { node, .. } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.stalled = true;
            }
        }
        DomainEvent::NodeRemoved { node } => {
            state.nodes.remove(node);
        }
        DomainEvent::TerminalSpawned { node, cli, cwd } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cli = Some(cli.clone());
                // Rodada 360 (ADR 0022 §3): o último spawn vence; `None` (log antigo)
                // projeta `None` — degradação honesta, nunca chute.
                n.cwd = cwd.clone();
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
        // F1-4-1: identidade durável do Espaço (a criação emite 1×; último vence).
        DomainEvent::WorkspaceIdAssigned { workspace_id } => {
            state.workspace_id = Some(workspace_id.clone());
        }
        // F1-4-1: Diretório de Trabalho do Espaço — último vence; `""` limpa (volta ao
        // fallback gerenciado do ADR 0022, via o seam `workspace::resolve_spawn_cwd`).
        DomainEvent::WorkspaceDefaultCwdSet { cwd } => {
            state.default_cwd = if cwd.is_empty() {
                None
            } else {
                Some(cwd.clone())
            };
        }
        // F1-4-1: rename projeta o nome corrente (e espelha o cabeçalho do plano, como
        // na criação — o `plan.md` derivado nunca fica com nome velho).
        DomainEvent::WorkspaceRenamed { name } => {
            state.workspace_name = Some(name.clone());
            state.plan.workspace = name.clone();
        }
        // F1-4-1: arquivado — sai da contagem ATIVA do gating free/PRO (o registry
        // re-deriva este flag do log; nunca o inverso).
        DomainEvent::WorkspaceArchived => {
            state.archived = true;
        }
        // Handshake/entrega/recusa/notas/snapshot são META (observabilidade no log) — sem efeito
        // na projeção do canvas; o roster vivo é do Supervisor, não da projeção.
        DomainEvent::Handshake { .. }
        | DomainEvent::MessageDelivered { .. }
        // F1-0-4: retenção é META (livro-razão da espera; a fila retida VIVE no `.inflight`
        // durável e é re-derivada pelo pump — mesmo padrão dos demais eventos de roteamento).
        | DomainEvent::MessageRetained { .. }
        // F1-0-7: retry/DLQ/breaker são META (livro-razão da entrega; a DLQ visível vive
        // em `<.lina>/dead-letter/` e o estado de retry é re-derivado destes eventos).
        | DomainEvent::MessageDeliveryFailed { .. }
        | DomainEvent::MessageDeadLettered { .. }
        | DomainEvent::CircuitOpened { .. }
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
        // W5-4: config/recepção de webhook são META (livro-razão do nó-Gatilho); o engine de webhooks
        // reconstrói suas ligações varrendo o log — sem efeito na projeção do canvas (padrão CostLedger).
        | DomainEvent::WebhookConfigured { .. }
        | DomainEvent::WebhookReceived { .. }
        // F1-1-1: detecção de CLI é META (livro-razão da descoberta); o consumidor (F1-1)
        // reconstrói o que precisar varrendo o log — sem efeito na projeção do canvas.
        | DomainEvent::CliDetected { .. }
        // F1-1-6: pedido de permissão é META (livro-razão da atenção pendente); a fila
        // unificada (F1-1-7) reconstrói as pendências varrendo o log, com dedupe por
        // `stable_id` — sem efeito na projeção do canvas.
        | DomainEvent::PermissionAsked { .. }
        // F1-2-3 (parte 2): papel custom salvo é META (biblioteca de papéis); o modal
        // reconstrói varrendo o log (último `name` vence) — sem efeito na projeção.
        | DomainEvent::RoleTemplateSaved { .. }
        // F1-1-7/8 (ADR 0021): decisão/efeito/abort/no-op da aprovação + dismiss de FP
        // + mute do fallback são META (livro-razão da fila de atenção e do ledger de
        // idempotência do executor) — a fila (attention.rs) e o executor (F1-1-8)
        // reconstroem varrendo o log; sem efeito na projeção do canvas.
        | DomainEvent::PermissionResolved { .. }
        | DomainEvent::ApprovalInjected { .. }
        | DomainEvent::ApprovalAborted { .. }
        | DomainEvent::ApprovalDuplicateIgnored { .. }
        | DomainEvent::PermissionDismissed { .. }
        | DomainEvent::NodeDetectionMuted { .. }
        // R2b item 5: prompt respondido direto no terminal é META (livro-razão da
        // fila — remoção sem decisão fabricada); a fila remove no fold.
        | DomainEvent::PermissionPromptCleared { .. }
        // F1-3-6: pedido/gate de spawn são META (livro-razão intent-vs-action 13.14 P1); a
        // criação física (`NodeAdded`) é que projeta. O painel de custo/auditoria e `lina retro`
        // reconstroem o trio varrendo o log — sem efeito na projeção do canvas.
        | DomainEvent::SpawnRequested { .. }
        | DomainEvent::SpawnGated { .. }
        // SEAM-1: admissão/recusa de spawn são META (correlação spawn↔nó p/ dedupe M3 + resolução do
        // banner); a criação física projeta via `NodeAdded`.
        | DomainEvent::SpawnAdmitted { .. }
        | DomainEvent::SpawnDeclined { .. }
        // F1-3-7 (reservado up-front): uso/criação de skill são META (dados do `lina retro`).
        | DomainEvent::SkillInvoked { .. }
        | DomainEvent::SkillCreated { .. }
        | DomainEvent::SnapshotTaken { .. }
        // F1-5-8: leitura cross do histórico é META (livro-razão de auditoria da fronteira
        // de pertencimento) — sem efeito na projeção do canvas.
        | DomainEvent::HistoryReadCross { .. }
        // F1-5-5: suspensão/retomada são META — overlay de RENDER consultável pelo shell via o
        // `SuspendController` (estado vivo), não pela projeção do canvas. O log é o livro-razão
        // (auditoria/retro/[PROF]); o nó NÃO muda de `status` (suspensão ⟂ lifecycle: segue vivo
        // e endereçável). Mantém ProjectedNode intocado → zero costura no shell (app/inspector.rs).
        | DomainEvent::NodeSuspended { .. }
        | DomainEvent::NodeResumed { .. }
        // r5 perf-ws: Descarregar é META (livro-razão do gesto; o estado vivo é dos PTYs e o
        // religar usa o fluxo de restore F1-4-3) — sem efeito na projeção do canvas.
        | DomainEvent::WorkspaceUnloaded { .. }
        // F2-0-1: resumo da janela [PROF] é META (livro-razão de medição; a régua D0
        // re-deriva % e percentis varrendo o log) — sem efeito na projeção do canvas.
        | DomainEvent::ProfWindowMetric { .. }
        // F2-2-4: uso de comando da paleta é META (livro-razão do ranking; o shell projeta
        // hit-count varrendo o log ao abrir a paleta) — sem efeito na projeção do canvas.
        | DomainEvent::PaletteCommandInvoked { .. } => {}
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
    /// r5 perf-ws: cache incremental de [`EventStore::project`] — `(último seq aplicado,
    /// estado)`. O log é append-only (nunca reescrito in-place), então "aplicar só
    /// `seq > último`" é sempre correto, inclusive para appends de OUTRO processo (o bin
    /// `lina` no mesmo `.db`). `RefCell` porque `project(&self)` é a API consolidada e a
    /// `Connection` já torna o tipo `!Sync` (o store vive sob `Mutex` nos consumidores).
    proj_cache: std::cell::RefCell<Option<(u64, ProjectedState)>>,
    /// Contador observável de eventos reaplicados por `project()` desde o open — a prova
    /// DETERMINÍSTICA do incremental nos testes (timing seria flaky).
    replayed_total: std::cell::Cell<u64>,
}

/// r5 perf-ws: cadência do snapshot AUTOMÁTICO no append (1 a cada N eventos). Sem isso,
/// `take_snapshot` não tinha chamador em produção e toda conexão NOVA re-parseava o log
/// inteiro (~91ms em 21k eventos, crescendo sem teto). Alto o bastante para testes
/// pequenos nunca cruzarem; baixo o bastante para limitar o replay frio a poucos ms.
const SNAPSHOT_EVERY: u64 = 4096;

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
            proj_cache: std::cell::RefCell::new(None),
            replayed_total: std::cell::Cell::new(0),
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
        // r5 perf-ws: snapshot AUTOMÁTICO a cada SNAPSHOT_EVERY appends — limita o replay
        // de conexões FRESCAS (sidebar/bin) ao delta pós-snapshot. Best-effort: o append já
        // é durável; falha de snapshot loga ALTO e nunca derruba o caminho de escrita.
        if rec.seq.is_multiple_of(SNAPSHOT_EVERY) {
            if let Err(e) = self.take_snapshot() {
                eprintln!(
                    "lina-core: snapshot automático no seq {} falhou ({e}); replay segue \
                     correto, só mais lento",
                    rec.seq
                );
            }
        }
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
    /// Determinístico. **r5 perf-ws:** INCREMENTAL na mesma instância — parte do cache em
    /// memória (ou do snapshot, na 1ª chamada) e reaplica SÓ `seq > último aplicado`; o
    /// caso quente do tick (nada novo) vira 1 SELECT vazio em vez de re-parsear o log
    /// inteiro (medido: 91ms em 21k eventos, a cada 120ms POR runtime). Appends de outro
    /// processo entram pelo mesmo delta de seq (log append-only — nunca reescrito).
    pub fn project(&self) -> Result<ProjectedState, StoreError> {
        let cached = self.proj_cache.borrow().clone();
        let (mut state, from_seq) = match cached {
            Some((seq, st)) => (st, seq),
            None => match self.latest_snapshot()? {
                Some((seq, st)) => (st, seq),
                None => (ProjectedState::default(), 0),
            },
        };
        let mut stmt = self.conn.prepare(
            "SELECT seq, kind, version, payload FROM events WHERE seq > ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([from_seq as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut last_seq = from_seq;
        for row in rows {
            let (seq, kind, version, payload_str) = row?;
            let payload: serde_json::Value = serde_json::from_str(&payload_str)?;
            let event = DomainEvent::from_record(&kind, version as u32, payload)?;
            apply(&mut state, &event);
            last_seq = seq as u64;
            self.replayed_total.set(self.replayed_total.get() + 1);
        }
        *self.proj_cache.borrow_mut() = Some((last_seq, state.clone()));
        Ok(state)
    }

    /// Total de eventos REAPLICADOS por [`EventStore::project`] desde o open — o
    /// observável determinístico do incremental (r5 perf-ws): sem evento novo, duas
    /// chamadas seguidas somam zero aqui.
    #[must_use]
    pub fn replayed_events_total(&self) -> u64 {
        self.replayed_total.get()
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

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
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

    /// **r5 perf-ws (BUG 3) — `project()` é INCREMENTAL na mesma instância.** Medido: 91ms
    /// por replay completo num log de 21k eventos, pago a cada 120ms POR runtime pelo tick
    /// do MailboxPump (N Espaços ≈ 291% de um core). O cache em memória reaplica SÓ os
    /// eventos novos (seq > último aplicado) — provado pelo contador determinístico
    /// `replayed_events_total` (nunca por timing). Appends EXTERNOS (outro processo: o bin
    /// `lina`) entram pelo mesmo delta de seq — a igualdade com um fold fresco é o juiz.
    #[test]
    fn project_is_incremental_and_matches_full_replay() {
        let tmp = TempDir::new("proj-inc");
        let mut store = EventStore::open(tmp.path()).expect("abrir");
        let (_dev, _qa) = seed(&mut store);
        let seeded = store.event_count().expect("count");

        let p1 = store.project().expect("project 1");
        let after_first = store.replayed_events_total();
        assert_eq!(after_first, seeded, "1º project reaplica o log inteiro");

        let p2 = store.project().expect("project 2");
        assert_eq!(p1, p2);
        assert_eq!(
            store.replayed_events_total(),
            after_first,
            "sem evento novo o 2º project reaplica ZERO (era 91ms de replay por tick)"
        );

        // Append EXTERNO (2º processo — o bin `lina` escreve no MESMO log): o cache da
        // instância quente precisa enxergar o delta pelo seq, nunca servir estado velho.
        let mut other = EventStore::open(tmp.path()).expect("2ª conexão");
        other
            .append(&DomainEvent::TokenUsageReported {
                node: "nó-externo".into(),
                tokens: 7,
            })
            .expect("append externo");

        let p3 = store.project().expect("project 3");
        assert_eq!(
            store.replayed_events_total(),
            after_first + 1,
            "só o delta (1 evento externo) é reaplicado"
        );
        let fresh = EventStore::open(tmp.path()).expect("conexão fresca");
        assert_eq!(
            p3,
            fresh.project().expect("fold fresco"),
            "cache nunca diverge do fold completo"
        );
    }

    /// **r5 perf-ws — auto-snapshot a cada [`SNAPSHOT_EVERY`] appends.** `take_snapshot`
    /// tinha ZERO chamadores em produção → conexões NOVAS (refresh do sidebar por Espaço,
    /// verbos do bin) re-parseavam o log INTEIRO, e o custo crescia sem teto com o log.
    /// Com a cadência, o replay de uma conexão fresca fica limitado ao delta pós-snapshot.
    #[test]
    fn append_auto_snapshots_to_bound_fresh_connection_replay() {
        let tmp = TempDir::new("auto-snap");
        let mut store = EventStore::open(tmp.path()).expect("abrir");
        for i in 0..(SNAPSHOT_EVERY + 10) {
            store
                .append(&DomainEvent::TokenUsageReported {
                    node: format!("nó-{}", i % 3),
                    tokens: i,
                })
                .expect("append");
        }
        let fresh = EventStore::open(tmp.path()).expect("conexão fresca");
        let p_fresh = fresh.project().expect("project fresco");
        assert!(
            fresh.replayed_events_total() < SNAPSHOT_EVERY,
            "snapshot automático limita o replay da conexão fresca (reaplicou {}, log tem {})",
            fresh.replayed_events_total(),
            SNAPSHOT_EVERY + 10
        );
        assert_eq!(
            p_fresh,
            store.project().expect("project quente"),
            "snapshot + delta = fold completo"
        );
    }

    /// **r5 perf-ws — `WorkspaceUnloaded` é META e aditivo:** replay com o evento no log
    /// produz a MESMA projeção (sem efeito no canvas) e o roundtrip preserva os campos —
    /// o executor do Descarregar (costura no app) lê `parked`/`kept` do log para religar.
    #[test]
    fn workspace_unloaded_is_meta_and_roundtrips() {
        let tmp = TempDir::new("ws-unload");
        let mut store = EventStore::open(tmp.path()).expect("abrir");
        seed(&mut store);
        let before = store.project().expect("projeção antes");
        store
            .append(&DomainEvent::WorkspaceUnloaded {
                parked: vec!["n1".into(), "n2".into()],
                kept: vec!["n3".into()],
            })
            .expect("append");
        assert_eq!(
            before,
            store.project().expect("projeção depois"),
            "META: zero efeito na projeção do canvas"
        );
        let rec = store
            .events()
            .expect("events")
            .into_iter()
            .rev()
            .find(|r| r.kind == "WorkspaceUnloaded")
            .expect("evento no log");
        assert_eq!(rec.payload["parked"][1].as_str(), Some("n2"));
        assert_eq!(rec.payload["kept"][0].as_str(), Some("n3"));
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
                requested_by: None,
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeAdded {
                node: qa,
                kind: "Terminal".into(),
                x: 200.0,
                y: 50.0,
                requested_by: None,
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
                cwd: None,
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
                requested_by: None,
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

    // ─────────────────────────── F1-0-3 · lifecycle no log ───────────────────────────

    /// F1-0-3: payload ANTIGO de `NodeStatusChanged` (só `node`+`status`, sem os campos
    /// aditivos `from`/`reason`) replaya com defaults vazios — compat por `serde(default)`,
    /// sem upcast (ADR 0001 §2: campo novo em variante existente, nunca evento paralelo).
    #[test]
    #[serial]
    fn node_status_changed_old_payload_replays_with_defaults() {
        let tmp = TempDir::new("nsc-compat");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let node = Uuid::now_v7();
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("add");
        // Shape antigo, gravado CRU — como um log pré-F1 teria persistido (o enum é
        // internamente tagged por `event`, então o tag faz parte do payload no disco).
        store
            .insert_raw(
                "NodeStatusChanged",
                1,
                serde_json::json!({ "event": "NodeStatusChanged", "node": node, "status": "Busy" }),
            )
            .expect("raw antigo");
        let state = store.project().expect("project");
        assert_eq!(
            state.nodes.get(&node).expect("nó").status.as_deref(),
            Some("Busy"),
            "payload antigo deve replayar (from/reason = default)"
        );

        // E a forma CORRENTE (com from/reason) faz round-trip e projeta normalmente.
        store
            .append(&DomainEvent::NodeStatusChanged {
                node,
                status: "Idle".into(),
                from: "Busy".into(),
                reason: "end_of_response".into(),
            })
            .expect("append forma nova");
        let state2 = store.project().expect("project 2");
        assert_eq!(
            state2.nodes.get(&node).expect("nó").status.as_deref(),
            Some("Idle")
        );
    }

    /// F1-0-3: `NodeStalled` liga o flag `stalled` na projeção (consultável pelo
    /// `lina check`/fila de atenção) e a PRÓXIMA transição de status do nó o limpa —
    /// o WARN persiste até o estado mudar (conservador, ADR 0019 §3).
    #[test]
    #[serial]
    fn node_stalled_projects_flag_and_next_transition_clears() {
        let tmp = TempDir::new("stall-proj");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let node = Uuid::now_v7();
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("add");
        store
            .append(&DomainEvent::NodeStatusChanged {
                node,
                status: "Busy".into(),
                from: "Ready".into(),
                reason: "pty_output".into(),
            })
            .expect("busy");
        store
            .append(&DomainEvent::NodeStalled {
                node,
                cycle_count: 42,
            })
            .expect("stalled");

        let state = store.project().expect("project");
        let n = state.nodes.get(&node).expect("nó");
        assert!(n.stalled, "NodeStalled deve ligar o flag na projeção");
        assert_eq!(n.status.as_deref(), Some("Busy"), "stall NÃO muda o status");

        store
            .append(&DomainEvent::NodeStatusChanged {
                node,
                status: "Idle".into(),
                from: "Busy".into(),
                reason: "end_of_response".into(),
            })
            .expect("idle");
        let state2 = store.project().expect("project 2");
        let n2 = state2.nodes.get(&node).expect("nó");
        assert!(!n2.stalled, "transição de status limpa o flag stalled");
        assert_eq!(n2.status.as_deref(), Some("Idle"));
    }

    /// F1-1-1 (coordenação via Maestro, Dev 02): `CliDetected` persiste e replaya; é META
    /// para a projeção do canvas (a emissão real no supervisor é fiação posterior).
    #[test]
    #[serial]
    fn cli_detected_round_trips_and_is_meta_for_projection() {
        let tmp = TempDir::new("cli-detected");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let node = Uuid::now_v7();
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 1.0,
                y: 2.0,
                requested_by: None,
            })
            .expect("add");
        let before = store.project().expect("project 1");

        store
            .append(&DomainEvent::CliDetected {
                node_id: node.to_string(),
                cli: "claude".into(),
                confidence: 0.95,
                evidence: "claude --version → 1.2.3".into(),
            })
            .expect("append CliDetected");

        let rec = store
            .events()
            .expect("events")
            .pop()
            .expect("último registro");
        assert_eq!(rec.kind, "CliDetected");
        assert_eq!(rec.payload["cli"], "claude");
        assert_eq!(rec.payload["node_id"], node.to_string());

        // Replay segue íntegro e a projeção do nó fica INTOCADA (evento meta).
        let after = store.project().expect("project 2");
        assert_eq!(
            before.nodes.get(&node),
            after.nodes.get(&node),
            "CliDetected é meta — não muda a projeção do nó"
        );
    }

    /// Rodada 360 (ADR 0022 §3): `TerminalSpawned.cwd` é ADITIVO — (a) payload antigo
    /// (sem o campo `cwd`) desserializa com `None`, sem upcast (replay de log antigo
    /// nunca quebra); (b) evento novo COM cwd round-trips pelo store e a projeção
    /// expõe o binding node↔cwd que o painel de custo consome (passo 7).
    #[test]
    #[serial]
    fn terminal_spawned_cwd_aditivo_replay_antigo_e_projecao() {
        // (a) Log antigo: payload v1 sem `cwd` → default `None`. O payload persistido
        // carrega a tag interna `event` (enum `#[serde(tag = "event")]`) — este é o
        // shape byte-fiel de um registro pré-rodada-360 no disco.
        let node = Uuid::now_v7();
        let old = DomainEvent::from_record(
            "TerminalSpawned",
            1,
            serde_json::json!({ "event": "TerminalSpawned", "node": node, "cli": "claude" }),
        )
        .expect("payload antigo sem cwd desserializa");
        assert_eq!(
            old,
            DomainEvent::TerminalSpawned {
                node,
                cli: "claude".into(),
                cwd: None,
            },
            "replay de log antigo projeta cwd ausente como None"
        );

        // (b) Evento novo COM cwd: round-trip pelo store + binding na projeção.
        let tmp = TempDir::new("cwd-aditivo");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("append NodeAdded");
        store
            .append(&DomainEvent::TerminalSpawned {
                node,
                cli: "claude".into(),
                cwd: Some("/Users/founder/projeto".into()),
            })
            .expect("append TerminalSpawned");
        let state = store.project().expect("project");
        let n = state.nodes.get(&node).expect("nó projetado");
        assert_eq!(n.cli.as_deref(), Some("claude"));
        assert_eq!(
            n.cwd.as_deref(),
            Some("/Users/founder/projeto"),
            "a projeção expõe o binding node↔cwd"
        );
    }

    // ─────────────────────────── F1-1-6 · PermissionAsked ───────────────────────────

    /// F1-1-6 (critério 4): dois pedidos seguidos do mesmo nó persistem com `stable_id`
    /// DISTINTOS; o evento é META (projeção intacta) e o replay repetido NÃO duplica —
    /// mesmo `event_count`, mesmo fingerprint (o log guarda fatos, a detecção não roda
    /// no replay).
    #[test]
    #[serial]
    fn permission_asked_two_asks_distinct_stable_ids_replay_no_dup() {
        let tmp = TempDir::new("perm-asked");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let node = Uuid::now_v7();
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("add");
        let before = store.project().expect("project 1");

        store
            .append(&DomainEvent::PermissionAsked {
                node_id: node.to_string(),
                tool: Some("Bash".into()),
                detail: Some("git push origin/master".into()),
                evidence: PermissionEvidence::Hook,
                stable_id: "pa-1".into(),
                vt_snapshot_hash: None,
                prompt_kind: PromptKind::Yn,
            })
            .expect("ask 1");
        store
            .append(&DomainEvent::PermissionAsked {
                node_id: node.to_string(),
                tool: None,
                detail: Some("Continue? (y/n)".into()),
                evidence: PermissionEvidence::Grid,
                stable_id: "pa-2".into(),
                vt_snapshot_hash: Some("cap1-abc".into()),
                prompt_kind: PromptKind::Choice,
            })
            .expect("ask 2");

        // Round-trip dos campos no log (incl. snake_case do evidence/prompt_kind e os
        // aditivos R2b: Captura 1 + formato do bloqueante).
        let recs: Vec<EventRecord> = store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "PermissionAsked")
            .collect();
        assert_eq!(recs.len(), 2, "2 pedidos = 2 eventos no log");
        assert_eq!(recs[0].payload["evidence"], "hook");
        assert_eq!(recs[0].payload["tool"], "Bash");
        assert_eq!(recs[0].payload["detail"], "git push origin/master");
        assert_eq!(recs[0].payload["prompt_kind"], "yn");
        assert_eq!(recs[1].payload["evidence"], "grid");
        assert_eq!(recs[1].payload["vt_snapshot_hash"], "cap1-abc");
        assert_eq!(recs[1].payload["prompt_kind"], "choice");
        assert_ne!(
            recs[0].payload["stable_id"], recs[1].payload["stable_id"],
            "pedidos seguidos do mesmo nó → stable_id distintos"
        );

        // META: projeção do canvas intacta; replay 2× = mesmo estado e mesma contagem.
        let after = store.project().expect("project 2");
        assert_eq!(before.fingerprint(), after.fingerprint());
        let count_1 = store.event_count().expect("count 1");
        let fp_1 = store.project().expect("replay 1").fingerprint();
        let fp_2 = store.project().expect("replay 2").fingerprint();
        assert_eq!(fp_1, fp_2, "replay não duplica nem altera nada");
        assert_eq!(store.event_count().expect("count 2"), count_1);
    }

    /// F1-1-6 + R2b (aditividade): payload PARCIAL de `PermissionAsked` (sem tool/
    /// detail/evidence e SEM os campos R2b — o shape do log antigo) replaya com
    /// defaults — `evidence` cai no conservador `grid`, `vt_snapshot_hash` em `None`
    /// e `prompt_kind` em `yn` (comportamento da época) — e o replay segue íntegro.
    #[test]
    #[serial]
    fn permission_asked_partial_payload_replays_with_defaults() {
        let tmp = TempDir::new("perm-partial");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .insert_raw(
                "PermissionAsked",
                1,
                serde_json::json!({
                    "event": "PermissionAsked",
                    "node_id": "n1",
                    "stable_id": "pa-x"
                }),
            )
            .expect("raw parcial");
        // Replay não erra; o registro desserializa com defaults.
        store.project().expect("replay com payload parcial");
        let rec = store.events().expect("events").pop().expect("registro");
        let ev = DomainEvent::from_record(&rec.kind, rec.version, rec.payload)
            .expect("desserializa com serde(default)");
        match ev {
            DomainEvent::PermissionAsked {
                tool,
                detail,
                evidence,
                vt_snapshot_hash,
                prompt_kind,
                ..
            } => {
                assert_eq!(tool, None);
                assert_eq!(detail, None);
                assert_eq!(
                    evidence,
                    PermissionEvidence::Grid,
                    "evidência ausente lê o default conservador (grid)"
                );
                assert_eq!(
                    vt_snapshot_hash, None,
                    "log antigo não tem Captura 1 — replay lê None"
                );
                assert_eq!(
                    prompt_kind,
                    PromptKind::Yn,
                    "log antigo era sempre y/n — replay lê o comportamento da época"
                );
            }
            other => panic!("kind errado: {other:?}"),
        }
    }

    // ──────────────── F1-1-7/8 · família Approval*/PermissionResolved (ADR 0021) ────────────────

    /// F1-1-7/8: a família inteira persiste com round-trip dos campos (incl. snake_case
    /// dos enums), é META (projeção do canvas intacta) e o replay repetido não duplica.
    #[test]
    #[serial]
    fn approval_family_round_trips_meta_and_replays_clean() {
        let tmp = TempDir::new("approval-family");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let before = store.project().expect("project 1");

        store
            .append(&DomainEvent::PermissionResolved {
                stable_id: "pa-1".into(),
                decision: ApprovalDecision::Approve,
                via: ResolutionVia::Human,
            })
            .expect("resolved");
        store
            .append(&DomainEvent::ApprovalInjected {
                stable_id: "pa-1".into(),
                vt_snapshot_hash: "abc123".into(),
            })
            .expect("injected");
        store
            .append(&DomainEvent::ApprovalAborted {
                stable_id: "pa-2".into(),
                reason: "screen_changed".into(),
            })
            .expect("aborted");
        store
            .append(&DomainEvent::ApprovalDuplicateIgnored {
                stable_id: "pa-1".into(),
            })
            .expect("dup");
        store
            .append(&DomainEvent::PermissionDismissed {
                stable_id: "pa-3".into(),
            })
            .expect("dismissed");
        store
            .append(&DomainEvent::NodeDetectionMuted {
                node_id: "n1".into(),
                muted: true,
            })
            .expect("muted");
        store
            .append(&DomainEvent::PermissionPromptCleared {
                node_id: "n1".into(),
                stable_id: "pa-4".into(),
            })
            .expect("cleared");

        let recs = store.events().expect("events");
        let by_kind = |k: &str| {
            recs.iter()
                .find(|r| r.kind == k)
                .unwrap_or_else(|| panic!("{k} ausente do log"))
        };
        let resolved = by_kind("PermissionResolved");
        assert_eq!(resolved.payload["decision"], "approve", "snake_case");
        assert_eq!(resolved.payload["via"], "human", "snake_case");
        assert_eq!(resolved.payload["stable_id"], "pa-1");
        assert_eq!(
            by_kind("ApprovalInjected").payload["vt_snapshot_hash"],
            "abc123"
        );
        assert_eq!(
            by_kind("ApprovalAborted").payload["reason"],
            "screen_changed"
        );
        assert_eq!(
            by_kind("ApprovalDuplicateIgnored").payload["stable_id"],
            "pa-1"
        );
        assert_eq!(by_kind("PermissionDismissed").payload["stable_id"], "pa-3");
        assert_eq!(by_kind("NodeDetectionMuted").payload["muted"], true);
        let cleared = by_kind("PermissionPromptCleared");
        assert_eq!(cleared.payload["stable_id"], "pa-4");
        assert_eq!(cleared.payload["node_id"], "n1");

        // META: projeção intacta; replay 2× = mesmo fingerprint, mesma contagem.
        let after = store.project().expect("project 2");
        assert_eq!(before.fingerprint(), after.fingerprint());
        let count = store.event_count().expect("count");
        assert_eq!(
            store.project().expect("replay").fingerprint(),
            after.fingerprint()
        );
        assert_eq!(store.event_count().expect("count 2"), count);
    }

    /// F1-1-7/8 (aditividade + doutrina): payload PARCIAL replaya com defaults
    /// CONSERVADORES — `PermissionResolved` sem decision/via lê `deny`/`timeout`
    /// (degradação nunca fabrica aprovação nem gesto humano — ADR 0021 §3/§5);
    /// `NodeDetectionMuted` mínimo lê `muted:true` (casa com o nome do evento).
    #[test]
    #[serial]
    fn approval_partial_payloads_replay_with_conservative_defaults() {
        let tmp = TempDir::new("approval-partial");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .insert_raw(
                "PermissionResolved",
                1,
                serde_json::json!({ "event": "PermissionResolved", "stable_id": "pa-x" }),
            )
            .expect("raw resolved");
        store
            .insert_raw(
                "NodeDetectionMuted",
                1,
                serde_json::json!({ "event": "NodeDetectionMuted", "node_id": "n1" }),
            )
            .expect("raw muted");
        store.project().expect("replay com payloads parciais");

        let recs = store.events().expect("events");
        let resolved = recs
            .iter()
            .find(|r| r.kind == "PermissionResolved")
            .expect("registro resolved");
        match DomainEvent::from_record(&resolved.kind, resolved.version, resolved.payload.clone())
            .expect("desserializa")
        {
            DomainEvent::PermissionResolved { decision, via, .. } => {
                assert_eq!(decision, ApprovalDecision::Deny, "default fail-safe");
                assert_eq!(via, ResolutionVia::Timeout, "default não-fabrica-humano");
            }
            other => panic!("kind errado: {other:?}"),
        }
        let muted = recs
            .iter()
            .find(|r| r.kind == "NodeDetectionMuted")
            .expect("registro muted");
        match DomainEvent::from_record(&muted.kind, muted.version, muted.payload.clone())
            .expect("desserializa")
        {
            DomainEvent::NodeDetectionMuted { muted, .. } => {
                assert!(muted, "payload mínimo = mutado (semântica do nome)");
            }
            other => panic!("kind errado: {other:?}"),
        }
    }

    // ─────────────────────── F1-2-3 (parte 2) · RoleTemplateSaved ───────────────────────

    /// F1-2-3: papel custom persiste como evento (round-trip dos 4 campos), é META para
    /// a projeção do canvas, sobrevive a replay, e payload PARCIAL (só `name`) replaya
    /// com defaults — log antigo nunca quebra.
    #[test]
    #[serial]
    fn role_template_saved_round_trips_meta_and_partial_payload_defaults() {
        let tmp = TempDir::new("role-template");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let before = store.project().expect("project 1").fingerprint();

        store
            .append(&DomainEvent::RoleTemplateSaved {
                name: "Revisora".into(),
                description: "Revisa textos e aponta melhorias".into(),
                doctrine: "Você é a revisora do time…".into(),
                ts: 1_700_000_000_000,
            })
            .expect("save template");

        let rec = store.events().expect("events").pop().expect("registro");
        assert_eq!(rec.kind, "RoleTemplateSaved");
        assert_eq!(rec.payload["name"], "Revisora");
        assert_eq!(
            rec.payload["description"],
            "Revisa textos e aponta melhorias"
        );
        assert_eq!(rec.payload["doctrine"], "Você é a revisora do time…");
        assert_eq!(rec.payload["ts"], 1_700_000_000_000_u64);

        // META + replay determinístico.
        let after = store.project().expect("project 2");
        assert_eq!(after.fingerprint(), before, "RoleTemplateSaved é META");

        // Payload parcial (só name) → defaults, replay íntegro.
        store
            .insert_raw(
                "RoleTemplateSaved",
                1,
                serde_json::json!({ "event": "RoleTemplateSaved", "name": "Faz-tudo" }),
            )
            .expect("raw parcial");
        store.project().expect("replay com payload parcial");
        let rec = store.events().expect("events").pop().expect("registro");
        let ev = DomainEvent::from_record(&rec.kind, rec.version, rec.payload)
            .expect("desserializa com serde(default)");
        match ev {
            DomainEvent::RoleTemplateSaved {
                name,
                description,
                doctrine,
                ts,
            } => {
                assert_eq!(name, "Faz-tudo");
                assert_eq!(description, "");
                assert_eq!(doctrine, "");
                assert_eq!(ts, 0);
            }
            other => panic!("kind errado: {other:?}"),
        }
    }

    /// **F1-3-6 (inv#4 / critério 5): `NodeAdded.requested_by` é ADITIVO.** Um payload v2 ANTIGO
    /// (shape `{node,kind,x,y}`, SEM o campo) replaya com `requested_by: None` via `serde(default)`
    /// — SEM bump de versão nem entrada em `upcast` — e a projeção do canvas NÃO quebra (campo META,
    /// fora de `ProjectedNode`). Um payload COM `requested_by:Some` roundtripa.
    #[test]
    #[serial]
    fn node_added_requested_by_is_additive() {
        let tmp = TempDir::new("reqby");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let node = Uuid::now_v7();

        // Payload v2 ANTIGO (sem o campo) — exatamente o que um log pré-F1-3-6 contém. O enum é
        // `#[serde(tag = "event")]`, então o payload armazenado carrega a tag `event` (como
        // `EventStore::append` serializa); só falta `requested_by` (o campo novo).
        store
            .insert_raw(
                "NodeAdded",
                2,
                serde_json::json!({ "event": "NodeAdded", "node": node, "kind": "Terminal", "x": 1.0, "y": 2.0 }),
            )
            .expect("insert raw v2-antigo");

        // Replaya sem erro: `from_record` desserializa com `requested_by=None` (sem upcast).
        let recs = store.events().expect("events");
        let ev = DomainEvent::from_record(&recs[0].kind, recs[0].version, recs[0].payload.clone())
            .expect("replay de NodeAdded v2 antigo");
        assert!(
            matches!(
                ev,
                DomainEvent::NodeAdded {
                    requested_by: None,
                    ..
                }
            ),
            "log antigo (sem o campo) replaya com requested_by=None"
        );
        // A projeção do canvas não quebra e ignora o campo META.
        let state = store.project().expect("project");
        assert!(
            state.nodes.contains_key(&node),
            "nó projetado mesmo sem o campo (requested_by é META, não entra em ProjectedNode)"
        );

        // Roundtrip COM `requested_by:Some` — o campo persiste e volta.
        let with_req = DomainEvent::NodeAdded {
            node: Uuid::now_v7(),
            kind: "Terminal".into(),
            x: 0.0,
            y: 0.0,
            requested_by: Some(Uuid::now_v7()),
        };
        let back: DomainEvent =
            serde_json::from_value(serde_json::to_value(&with_req).unwrap()).unwrap();
        assert_eq!(back, with_req, "NodeAdded com requested_by:Some roundtripa");
    }

    /// **FIX-2 (dogfood): `ActionGated.node` é ADITIVO.** Um log pré-FIX-2 (shape
    /// `{cmd,class,decision}`, SEM `node`) replaya com `node: None` via `serde(default)` — SEM bump
    /// nem `upcast` (campo `Option`, padrão `NodeAdded.requested_by`). Um payload COM `node:Some`
    /// roundtripa. É o que garante que a fila de atenção (consumidora de `ActionGated{ask, node:Some}`)
    /// nunca quebre o replay de um log antigo.
    #[test]
    #[serial]
    fn action_gated_node_is_additive() {
        let tmp = TempDir::new("agnode");
        let mut store = EventStore::open(tmp.path()).expect("open");

        // Payload PRÉ-FIX-2 (sem `node`) — exatamente o que um log antigo contém. O enum é
        // `#[serde(tag = "event")]`, então o payload carrega a tag `event`; só falta `node`.
        store
            .insert_raw(
                "ActionGated",
                1,
                serde_json::json!({ "event": "ActionGated", "cmd": "git push --force origin main", "class": "gated-hard", "decision": "ask" }),
            )
            .expect("insert raw pré-FIX-2");

        let recs = store.events().expect("events");
        let ev = DomainEvent::from_record(&recs[0].kind, recs[0].version, recs[0].payload.clone())
            .expect("replay de ActionGated antigo");
        assert!(
            matches!(ev, DomainEvent::ActionGated { node: None, .. }),
            "log antigo (sem `node`) replaya com node=None"
        );

        // Roundtrip COM `node:Some` — o campo persiste e volta.
        let with_node = DomainEvent::ActionGated {
            cmd: "deploy prod".into(),
            class: "gated-hard-external".into(),
            decision: "ask".into(),
            node: Some("Bug Finder".into()),
        };
        let back: DomainEvent =
            serde_json::from_value(serde_json::to_value(&with_node).unwrap()).unwrap();
        assert_eq!(back, with_node, "ActionGated com node:Some roundtripa");
    }

    /// **F1-3-6: roundtrip + `kind()` + META das variantes novas** (SpawnRequested/SpawnGated) e das
    /// RESERVADAS de F1-3-7 (SkillInvoked/SkillCreated). Variantes novas → replay limpo; silêncio no
    /// `apply` (não tocam a projeção do canvas).
    #[test]
    #[serial]
    fn spawn_and_skill_events_roundtrip_and_are_meta() {
        let by = Uuid::now_v7();
        let cases: Vec<(DomainEvent, &str)> = vec![
            (
                DomainEvent::SpawnRequested {
                    id: "msg_x".into(),
                    requested_by: by,
                    name: "@QA".into(),
                    role: "qa".into(),
                    root_cause_id: "msg_root".into(),
                    hops: 1,
                    prompt: "valide o checkout".into(),
                },
                "SpawnRequested",
            ),
            (
                DomainEvent::SpawnGated {
                    id: "msg_x".into(),
                    requested_by: by,
                    reason: "cascade".into(),
                },
                "SpawnGated",
            ),
            (
                DomainEvent::SkillInvoked {
                    node: by,
                    skill: "lina-retro".into(),
                },
                "SkillInvoked",
            ),
            (
                DomainEvent::SkillCreated {
                    node: by,
                    skill: "nova".into(),
                },
                "SkillCreated",
            ),
            (
                DomainEvent::SpawnAdmitted {
                    id: "msg_x".into(),
                    node: by,
                },
                "SpawnAdmitted",
            ),
            (
                DomainEvent::SpawnDeclined { id: "msg_x".into() },
                "SpawnDeclined",
            ),
        ];
        let mut state = ProjectedState::default();
        let before = state.clone();
        for (ev, kind) in &cases {
            assert_eq!(ev.kind(), *kind, "kind() casa a tag serde de {kind}");
            let back: DomainEvent =
                serde_json::from_value(serde_json::to_value(ev).unwrap()).unwrap();
            assert_eq!(&back, ev, "{kind} roundtripa");
            apply(&mut state, ev); // META: não muda a projeção.
        }
        assert_eq!(
            state, before,
            "eventos de spawn/skill são META (apply não projeta)"
        );
    }
}
