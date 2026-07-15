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
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lina_host::{HostEvent, NodeId, UiHost};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
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

/// [12]/[9] Nível de raciocínio que o Maestro pede a um terminal (cap. 09, ponto [9] de
/// Meadows — delay-de-raciocínio). Contrato **NEUTRO** entre CLIs: o mapeamento para a flag
/// real de cada CLI (`--reasoning high`, env de thinking, …) é do **CLI Profile** (invariante
/// #3). O core NUNCA conhece a sintaxe de um CLI específico — só `low/medium/high`. Serializa
/// em `lowercase` para casar o vocabulário dos eventos (`EffortAssigned.effort: "high"`) e do
/// `params.json` do expert. Default `Medium` (espelha [`SystemParams`](crate::SystemParams)).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    #[default]
    Medium,
    High,
}

impl Effort {
    /// O rótulo neutro `low`/`medium`/`high` — o vocabulário do `LINA_EFFORT` (ENV do PTY filho,
    /// F3-0-4) e do campo `effort` dos eventos. Mantido como `&'static str` (espelha
    /// `Autonomy::label`) para carimbar o ENV sem alocar nem depender do serializador.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }
}

/// F3-0-4 ([12]/[9]): de ONDE veio o effort de um [`DomainEvent::EffortAssigned`]. `Observed` = o
/// app LEU o effort que o CLI já usa (linha 57 do doc-fonte — captura via session-watch); `Assigned`
/// = o Maestro/humano/preset ESCOLHEU (linha 58 — atribuição). Serializa `lowercase`
/// (`"observed"`/`"assigned"`) para a F3-0-7 auditar por string. Default CONSERVADOR `Observed`: um
/// payload degradado/antigo NUNCA replaya como uma atribuição com autoridade (regra-mãe ADR 0007).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffortOrigin {
    #[default]
    Observed,
    Assigned,
}

/// F3-1 ([3] Objetivos, spec 52 §1): um critério de aceite OBSERVÁVEL de uma Goal/item — "algo que
/// roda e se mede", NÃO "implementado". É o *setpoint* do laço de qualidade ([8] Feedback): o
/// [`DomainEvent::ReviewVerdict`] confere a entrega contra estes critérios. `Debug` derivado (estilo
/// da casa; exigido por [`crate::PlanItem`], que o contém e deriva `Debug`/`Eq`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    /// Legível ao humano leigo (narrado no card, sem jargão).
    pub desc: String,
    /// Como o critério é VERIFICADO. Aditivo: default `HumanReview` (degrada honesto — exige gate).
    #[serde(default)]
    pub check_kind: CheckKind,
    /// Argumento do check: comando a rodar, caminho ou glob de teste. `None` em `HumanReview`.
    #[serde(default)]
    pub check_arg: Option<String>,
}

/// F3-1 ([3]/[8], spec 52 §1): COMO um [`AcceptanceCriterion`] é verificado. Default CONSERVADOR
/// `HumanReview` (replay/payload sem o campo degrada para "exige humano", nunca para um check
/// automático que se auto-aprova). **Segurança (spec 52 §Segurança 3):** `Command` é o VERIFICADOR,
/// não um portal de execução arbitrária — o comando roda sob o mesmo `guard::decide` de qualquer
/// ação; um critério `GatedHard` (deploy/pay/`rm -rf`) continua pedindo `Ask`. O critério verifica;
/// ele não autoriza.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckKind {
    /// Julgado por um humano (default seguro — exige gate).
    #[default]
    HumanReview,
    /// Roda um comando; PASS sse exit 0. NUNCA roda ação `GatedHard` (ver doc do tipo).
    Command,
    /// Existência de um arquivo (caminho em `check_arg`).
    FileExists,
    /// Uma suíte de teste passa (glob/cmd em `check_arg`).
    TestPass,
}

/// F3-1 ([8] Feedback negativo, spec 52 §3): o veredito de uma revisão — o sinal de erro do
/// termostato de qualidade. `Pass` fecha o item; `Fail` realimenta o laço (re-despacho informado →
/// escala de effort → gate humano no teto). É DADO transportado por [`DomainEvent::ReviewVerdict`],
/// JAMAIS autoridade: um `Pass` NUNCA libera uma ação `GatedHard` (spec 52 §Segurança 1; o gate de
/// `guard.rs` permanece soberano).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    Fail,
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

/// Sinal de RESULTADO de uma seleção de skill (ADR 0045 C2, spec 0045 §3) — inferido
/// PASSIVAMENTE, a Lina NUNCA pergunta. `Reworked`/`HumanOverride` são variantes distintas de
/// falha para o painel distinguir "devolveram" de "trocaram"; no ranking ambas pesam negativo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSignal {
    /// Skill usada, sem troca e sem re-trabalho na janela seguinte (reforço barato: gate/cold-review PASS).
    Success,
    /// Falha genérica (negativo no ranking).
    Failure,
    /// Entrega devolvida / re-trabalho — conta como falha.
    Reworked,
    /// Humano/Maestro trocou a skill escolhida — conta como falha.
    HumanOverride,
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
    /// F2-3-2: redimensionamento DURÁVEL do terminal vivo. Emitido UMA vez no FIM do
    /// gesto de arrasto da borda (nunca por frame; gesto cancelado ⇒ ZERO evento —
    /// espelha a doutrina de `NodeMoved`, ADR 0029 §1). `cols`/`rows` restauram a grade
    /// do PTY; `w`/`h` (px) restauram o tamanho EXATO do card (o resto do `floor` de
    /// `px_to_grid` não é recuperável só de cols×rows). Variante NOVA ⇒ replay de log
    /// antigo (sem ela) nunca a vê; sem upcast (v1). Tipos `u16` espelham o pipeline de
    /// resize ponta-a-ponta (`portable-pty`/`VtBackend`/`px_to_grid`) — zero casting.
    TerminalRedimensionado {
        node: NodeId,
        cols: u16,
        rows: u16,
        w: f64,
        h: f64,
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
    /// F3-0-2 ([12] Parâmetros, spec 51 §4): um parâmetro do sistema foi sobrescrito acima do
    /// default, numa das 4 camadas. META — sem efeito na projeção do canvas; o [`ParamsLedger`]
    /// (`params.rs`) reconstrói a camada efetiva por replay (mesmo padrão do `CostLedger`). Livro-
    /// razão das calibrações: apendado SÓ quando o valor MUDA (anti-amplificação, como o
    /// `RouteBlocked`). ADITIVO: `#[serde(default)]` nos opcionais → logs antigos sem o evento
    /// replayam intactos.
    ///
    /// **Segurança (regra-mãe, família ADR 0007):** `by`/`scope`/`target` são CARIMBO server-side
    /// derivado do binding de quem aplica (humano via CLI = `"human"`; agente em cascata =
    /// `"maestro"`; preset = `"preset:<slug>"`), JAMAIS lidos do payload de um agente — um nó não
    /// forja `by:"human"` nem escala de camada. O carimbo é do emissor (F3-0-5); o evento transporta.
    SystemParamsChanged {
        /// Chave canônica do parâmetro (ex.: `"fanout_gate"`, `"effort"`).
        key: String,
        /// Camada que aplicou: `"global"` | `"workspace"` | `"preset"` | `"terminal"`.
        scope: String,
        /// Valor APLICADO, serializado como string (auditável sem schema rígido — espelha `old`).
        new: String,
        /// Alvo quando `scope="terminal"` (NodeId) ou `"preset"` (slug); `None` p/ workspace/global.
        #[serde(default)]
        target: Option<String>,
        /// Valor ANTERIOR efetivo nesta camada (`""` = era None/herdado).
        #[serde(default)]
        old: String,
        /// Quem aplicou (carimbo server-side): `"human"` | `"maestro"` | `"preset:<slug>"`.
        #[serde(default)]
        by: Option<String>,
    },
    /// F3-0-4 ([12]/[9], spec 51 §4): effort (e modelo correlacionado) OBSERVADO ou ATRIBUÍDO a um
    /// nó. É a PROJEÇÃO por-nó que alimenta o badge `modelo·effort` do card (linha 57) e o knob de
    /// atribuição do Maestro (linha 58). META — sem efeito na projeção do canvas (o badge varre o
    /// log, padrão CostLedger). ADITIVO (`#[serde(default)]` nos opcionais).
    ///
    /// **Segurança (regra-mãe, ADR 0007):** `origin`/`by`/`task_difficulty` são CARIMBO server-side
    /// (do binding de quem atribui), JAMAIS lidos do payload de um agente — um nó não se
    /// auto-atribui `origin:assigned`/`effort:high` para gastar mais tokens (a auto-declaração é, no
    /// máximo, `observed`). O carimbo é do emissor (admit_node / `lina effort`); o evento transporta.
    EffortAssigned {
        /// O nó alvo (nome `@Nome` ou a `key` durável — o que o emissor tiver no momento).
        node: String,
        /// Nível efetivo (low/medium/high).
        effort: Effort,
        /// Modelo correlacionado, se conhecido (`None` = só o effort).
        #[serde(default)]
        model: Option<String>,
        /// Origem do carimbo: observado (leitura) vs atribuído (escolha). Default `observed`.
        #[serde(default)]
        origin: EffortOrigin,
        /// Dificuldade que motivou a atribuição (`"trivial"`/`"standard"`/`"hard"`) — só em
        /// `assigned`, carimbado server-side. `None` em `observed`.
        #[serde(default)]
        task_difficulty: Option<String>,
        /// Quem atribuiu (carimbo server-side do binding): `Some(NodeId)` = um agente/Maestro;
        /// `None` = gesto humano direto OU observação. Nunca lido do payload do agente.
        #[serde(default)]
        by: Option<NodeId>,
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
    /// F3-1 ([3] Objetivos, spec 52 §1): a Goal foi DECLARADA — âncora de todos os eventos da meta.
    /// Variante NOVA → sem `serde(default)` nos campos (replay antigo nunca a encontra; precedente
    /// `SpawnRequested`). META no `apply` do canvas; a projeção [`crate::Goal`] reconstrói por replay.
    GoalDefined {
        /// UUIDv7 estável, cunhado pelo SUPERVISOR (NÃO pelo payload do agente — spec 52 §Segurança 2).
        goal_id: String,
        /// O pedido bruto do humano/origem (doc-fonte linha 10). Dado, JAMAIS autoridade.
        statement: String,
        /// Liga a Goal ao âncora de escopo existente (`Router::derive_root_hops`). Carimbado pelo
        /// binding interno — NUNCA de `msg.root_cause_id` (spec 52 §Segurança 2).
        root_cause_id: String,
        /// Quem originou: `"@Maestro"` | `"@Tradutor"` | `"cron:<id>"` (ponto 5 / doc linha 86).
        /// Rótulo de PROVENIÊNCIA, não credencial.
        origin: String,
        /// Orçamento de tokens da meta. `0` = herda o `token_budget_day` do workspace.
        budget_tokens: u64,
    },
    /// F3-1 ([3], spec 52 §1): o Maestro/Tradutor INTERPRETOU a meta e propõe estratégia + critérios.
    /// SUGERE — não autoriza nada (o gate é o `GoalConfirmed` seguinte). Variante NOVA → sem
    /// `serde(default)` nos campos.
    GoalInterpreted {
        goal_id: String,
        /// O que o Maestro ENTENDEU do statement (doc linha 11). Dado transportado.
        interpretation: String,
        /// Estratégia de execução multi-terminal proposta (texto livre).
        strategy: String,
        /// Papéis sugeridos para o time (doc linha 12): `["Arquiteto","Backend",...]`.
        proposed_team: Vec<String>,
        /// Critérios OBSERVÁVEIS — o setpoint do laço (não "implementado").
        acceptance_criteria: Vec<AcceptanceCriterion>,
    },
    /// F3-1 ([3]/[5], spec 52 §1): GATE HUMANO — o humano confirmou ou corrigiu a interpretação
    /// (doc linha 11). Variante NOVA → sem `serde(default)` nos campos (exceto o marcado).
    GoalConfirmed {
        goal_id: String,
        /// IDENTIDADE da camada de auth — carimbada pelo dir-dono do outbox autenticado (mesmo
        /// binding de [`DomainEvent::SpawnRequested`]`.requested_by`), JAMAIS lida do campo `from`
        /// do payload (spec 52 §Segurança 2). DADO transportado no evento, mas a AUTORIDADE é do
        /// binding server-side: um `GoalConfirmed` forjado com `by` de outro nó é ignorado.
        by: NodeId,
        /// Se o humano corrigiu o entendimento antes de confirmar.
        #[serde(default)]
        amended_statement: Option<String>,
    },
    /// F3-1 ([3]/[4], spec 52 §1): a Goal foi decomposta em itens do plano (liga Goal → plano).
    /// Variante NOVA → sem `serde(default)` nos campos.
    GoalDecomposed {
        goal_id: String,
        /// ids dos `PlanItem` gerados (cada um atribuído via [`DomainEvent::PlanItemAttributed`]).
        plan_items: Vec<String>,
    },
    /// F3-1 ([8] Feedback negativo, spec 52 §1): a Goal FECHOU. Só emitido quando TODO item da Goal
    /// tem [`DomainEvent::ReviewVerdict`] `Pass` — fecha o termostato de qualidade. Variante NOVA →
    /// sem `serde(default)` nos campos.
    GoalAchieved {
        goal_id: String,
        /// Total de voltas do OUTER loop (turn-budget gasto).
        iterations: u32,
        /// id do `EventRecord` do `ReviewVerdict::Pass` que fechou o último item.
        verdict_event_id: String,
    },
    /// F3-1 ([8]/[9], spec 52 §1): a Goal foi ESCALADA ao humano sem fechar (backstop do loop).
    /// PAUSA, não mata — resumível (mesma doutrina de `CostCeilingHit`/`CircuitOpened`: gate, nunca
    /// kill). Variante NOVA → sem `serde(default)` nos campos.
    GoalEscalated {
        goal_id: String,
        /// String tolerante a evolução (precedente `SpawnGated.reason`): razões novas não quebram
        /// replay. Vocabulário canônico: `"turn_budget_exhausted"` | `"budget_exceeded"` |
        /// `"human_cancelled"` | `"judge_unreliable"` | `"stalled"`.
        reason: String,
    },
    /// F3-1 ([3]/[4], spec 52 §2): atribui Goal + dependências + DoD a um item JÁ semeado.
    /// Idempotente por item (último vence). EFETIVO no `apply` (muta o [`crate::Plan`] projetado).
    /// Variante NOVA → sem `serde(default)` nos campos.
    PlanItemAttributed {
        item: String,
        goal_id: Option<String>,
        parents: Vec<String>,
        acceptance: Vec<AcceptanceCriterion>,
        budget_tokens: u64,
        /// F3-4-3 (spec 36 §2, ADR 0041): arquivos/globs que o item RESERVA (relativos ao repo),
        /// event-sourcing o `PlanItem.paths`. ADITIVO via `#[serde(default)]` — replay F3-1 (shape
        /// sem `paths`) reconstrói com `[]`, sem upcast (inv #4). Cruzado com `CodeChanged.paths` pela
        /// projeção de pertencimento (Trilha B); DADO declarado, JAMAIS autoridade (família ADR 0007).
        #[serde(default)]
        paths: Vec<String>,
    },
    /// F3-1 ([8] Feedback negativo, spec 52 §3): veredito de revisão sobre a entrega de um item — o
    /// sinal de erro do laço de qualidade. Variante NOVA → sem `serde(default)` nos campos (exceto os
    /// marcados). META no `apply`; a projeção [`crate::Goal`] varre por `goal_id`.
    ///
    /// **Segurança (spec 52 §Segurança 1):** `evidence`/`defect_class` são DADO transportado, JAMAIS
    /// autoridade — um `verdict: Pass` NUNCA libera uma ação `GatedHard` (`guard.rs` segue soberano).
    ReviewVerdict {
        goal_id: String,
        plan_item: String,
        /// Nó revisado (o "desenvolvedor" do doc).
        target: NodeId,
        /// Quem emitiu o veredito — juiz estrutural (supervisor) ou nó QA. Carimbado server-side,
        /// NUNCA o `target` (juiz ≠ executor, spec 52 §3): o auto-veredito é rejeitado pela lógica.
        reviewer: NodeId,
        verdict: Verdict,
        /// Evidência arquivo:linha do FAIL (vazia em `Pass`) — saída OBSERVADA do check, não "ok".
        #[serde(default)]
        evidence: String,
        /// `qualidade` | `seguranca` | `regressao` | `criterio_nao_cumprido`.
        #[serde(default)]
        defect_class: String,
        /// nº da iteração do OUTER loop para este item (ganho corrente do termostato).
        #[serde(default)]
        iteration: u32,
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
    /// ADR 0037: a **Pasta de Trabalho POR-NÓ** foi editada/trocada DEPOIS da criação. Projeta o
    /// `cwd` do nó (último vence — mesma semântica de `TerminalSpawned.cwd`); o restore
    /// (`plan_restore` lê `ProjectedNode.cwd`) re-entra na nova pasta na próxima abertura, sem
    /// re-spawn. Evento ISOLADO em vez de campo novo em `TerminalSpawned` (18 construções
    /// literais em gates de várias ondas): editar o cwd é um gesto humano pontual, como
    /// `CliProfileSet`. A cwd é DADO (nunca autoridade — identidade vem do env de spawn, ADR 0026).
    NodeCwdSet {
        node: NodeId,
        cwd: String,
    },
    /// W4-4 (T6 Switcher): o foco de workspace mudou. Projeta `focused_workspace`. (O event store
    /// é por-workspace; este evento registra o foco vigente para a projeção do Switcher.)
    WorkspaceFocusSet {
        workspace: String,
        /// Identifica uma tentativa de foco de ponta a ponta. Ausente nos logs anteriores
        /// à troca transacional de workspace; não altera a projeção último-vence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        focus_id: Option<String>,
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
        /// F4-WA (ADITIVO, spec 55 §3): o terminal DONO deste webhook (por NOME — ADR 0026).
        /// Vazio no payload antigo (replay degrada p/ o fluxo `publish()` legado — ADR 0001 §2).
        #[serde(default)]
        target_node: String,
        /// F4-WA (ADITIVO): a INSTRUÇÃO do usuário = AUTORIDADE. OBRIGATÓRIA na config nova (a UI
        /// recusa salvar vazio — é ela que decide a ação); replay antigo → "" (fluxo legado, sem ação).
        #[serde(default)]
        instruction: String,
        /// F4-WA (ADITIVO): nível de ação `"autonomous"|"notify_before"`. Default CONSERVADOR
        /// `notify_before` — replay/payload incompleto NUNCA fabrica autonomia. String, não enum
        /// (níveis novos entram sem upcast — spec 55 §3).
        #[serde(default = "default_notify_before")]
        autonomy_level: String,
        /// F4-WA (ADITIVO): REFERÊNCIA ao secret no Secret Vault (escopo `"webhook"`, key = hook_id) —
        /// NUNCA o valor. Normalmente vazio (a chave É o hook_id); reservado p/ secret nomeado/rotacionado.
        #[serde(default)]
        secret_ref: String,
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
        /// F4-WA (ADITIVO, spec 55 §5): método HTTP (POST/GET/…) — metadado; informa o tipo de evento.
        #[serde(default)]
        method: String,
        /// F4-WA (ADITIVO): tamanho do payload em bytes — metadado p/ correlação. NUNCA o conteúdo.
        #[serde(default)]
        payload_size: u64,
        /// F4-WA (ADITIVO): SHA-256 do payload (hex) — hash p/ correlação; o conteúdo NÃO vai ao log
        /// (mesma doutrina de `HistoryReadCross`: registra o TIPO/hash, jamais o conteúdo — ADR 0035 §5).
        #[serde(default)]
        payload_sha256: String,
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
        /// F3-0-3 (ADR 0031): modelo PEDIDO para o novo terminal (alias do CLI Profile, invariante
        /// #3) — preferência de EXECUÇÃO, NÃO autoridade (a autoridade de origem é `requested_by`/
        /// `root_cause_id`, server-side). `None` = o que o CLI já usa. ADITIVO via `serde(default)`:
        /// logs de spawn pré-F3-0-3 replayam com `None`.
        #[serde(default)]
        model: Option<String>,
        /// F3-0-3 (ADR 0031): nível de raciocínio PEDIDO (contrato neutro low/medium/high). Mesma
        /// natureza de `model`: preferência transportada, não autoridade. O effort EFETIVO (ENV +
        /// `EffortAssigned`) é aplicado em F3-0-4; aqui só transita o pedido. ADITIVO.
        #[serde(default)]
        effort: Option<Effort>,
        /// F3-1 (spec 52 §4): a Goal que JUSTIFICA o recurso — auditoria (`SpawnRequested` ligado à
        /// meta que o motivou). `None` = spawn não atrelado a Goal (comportamento legado). ADITIVO
        /// via `serde(default)`: logs de spawn pré-F3-1 replayam com `None`.
        #[serde(default)]
        goal_id: Option<String>,
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

    // ───────────────────────── F3-3 Mentality (spec 35 §3) ─────────────────────────
    // Ciclo de vida da CRENÇA de um papel. Variantes TOTALMENTE novas (não campos
    // aditivos a um evento existente): nenhum log F0/F1/F2/F3-anterior as contém, então
    // o replay nunca as encontra (precedente `SpawnRequested`) — por isso os campos
    // OBRIGATÓRIOS não levam `serde(default)`; só os EXTENSÍVEIS (refs/evidence/flag) o
    // levam, para a própria variante crescer barato depois. REGRA-MÃE (spec 35 §6.1;
    // família ADR 0007): a crença é DADO comportamental ("como pensar"), JAMAIS
    // autoridade — `role`/proveniência são carimbados server-side por quem EMITE
    // (M-DETECTOR/Refletor), nunca lidos do payload do agente. A projeção
    // `Mentality(papel)` é por REPLAY externo (módulo `mentality`, padrão `CostLedger`):
    // estes eventos NÃO tocam `ProjectedState` (reducer no-op).
    /// PORTA DE ENTRADA do aprendizado (spec 35 §3/§4.1): o usuário corrigiu o terminal
    /// de um papel — captado pela sentinela `[LINA::CORRECTION ...]` (M-DETECTOR). `role`
    /// é o binding server-side do nó (ADR 0007/0026), JAMAIS o texto do agente. `summary`
    /// e `refs` são DADO (o "o quê" + ids dos eventos-fonte), nunca decidem nada.
    CorrectionObserved {
        /// Papel do roster cujo terminal foi corrigido — server-side, não do payload.
        role: String,
        /// Resumo curto da correção observada (o "o quê"). Dado, não autoridade.
        summary: String,
        /// Refs às fontes (ids de eventos/transcript) que evidenciam a correção.
        #[serde(default)]
        refs: Vec<String>,
    },
    /// O Refletor (job assíncrono FORA do caminho crítico, 1 CLI-call — inv #1: NÃO é
    /// LLM no core) destilou a correção numa crença PROVISÓRIA (spec 35 §3/§4.2). Entra
    /// em quarentena: só vira regra por evidência diversa (política determinística,
    /// M-PROMO). `statement` é falseável (Why/How-to-apply, 1-2 frases) e nunca instrução
    /// de segurança/permissão (anti-poisoning §6.3 — barrado no Refletor antes de virar
    /// evento).
    BeliefProposed {
        /// Id durável da crença (gerado na emissão, persistido — replay lê do log).
        belief_id: String,
        /// Papel dono da crença (server-side; liga `belief_id`→papel na projeção).
        role: String,
        /// Statement falseável ("como pensar"), Why/How-to-apply. DADO comportamental.
        statement: String,
        /// Proveniência humanizável (de qual correção/quando nasceu) — para o painel.
        provenance: String,
        /// §6.3: crença nascida em sessão que processou conteúdo externo não-confiável
        /// leva flag de origem (sinaliza, não bloqueia). Default `false` (origem confiável).
        #[serde(default)]
        untrusted_origin: bool,
    },
    /// A crença se aplicou com sucesso numa situação DISTINTA (spec 35 §3/§6.4). O
    /// `situation_hash` é o anti-gaming: a MESMA situação 2× NÃO conta — a política
    /// M-PROMO só promove com N hashes distintos (default N=2). `evidence` é auditável.
    BeliefReinforced {
        belief_id: String,
        /// Hash-de-situação — exige diversidade real de evidência (anti-gaming §6.4).
        situation_hash: String,
        /// Evidência textual do reforço (auditoria/painel). Opcional.
        #[serde(default)]
        evidence: String,
    },
    /// O usuário corrigiu DE NOVO no mesmo tema (spec 35 §3): a crença foi contestada —
    /// ZERA o progresso de promoção (política M-PROMO). Crença nunca é deletada, só
    /// rebaixada — o histórico permanece reconstruível por replay (inv #4).
    BeliefChallenged {
        belief_id: String,
        #[serde(default)]
        evidence: String,
    },
    /// Política DETERMINÍSTICA (sem LLM, inv #1): N situações distintas confirmaram
    /// (default N=2) → a crença vira ESTABELECIDA (spec 35 §3/§4.3) e entra na doutrina
    /// renderizada como REGRA do papel (M-INJETOR). Só o `belief_id` — o "como/porquê" se
    /// reconstrói pelos eventos anteriores do mesmo id no replay.
    BeliefEstablished {
        belief_id: String,
    },
    /// A crença saiu de circulação (spec 35 §3): refutada, TTL de provisória expirado
    /// (default 30d sem reforço — mesmo vocabulário stale do `lina retro`), ou APOSENTADA
    /// pelo humano no painel (1 clique — humano é o árbitro, §6.7). `reason` ∈
    /// {`refuted`,`expired`,`retired_by_human`} — String (não enum), como o `reason` de
    /// `SpawnGated`. Não é deletada: some da injeção, mas o histórico replaya intacto.
    BeliefRetired {
        belief_id: String,
        /// Motivo da aposentadoria. String aberta (precedente `SpawnGated.reason`).
        reason: String,
    },
    // ──────────────── F3-4 Coordenação de Código Multi-Agente (spec 36) ────────────────
    // Sinal de mudança + prova de integração para N agentes codando em git worktrees
    // isoladas no MESMO PC. Variantes TOTALMENTE novas (precedente `SpawnRequested`):
    // campos OBRIGATÓRIOS sem `serde(default)` (replay antigo nunca as encontra). REGRA-MÃE
    // (família ADR 0007/0026): `author_node` é o binding server-side do nó que commitou
    // (do outbox autenticado), JAMAIS o texto do payload escrito por um agente — `paths`/
    // `branch`/`commit` são DADO transportado, nunca decidem identidade, ordem ou
    // autorização. O event log é a FONTE do sinal (nunca regex). As projeções (CodeChanged
    // por branch; branches não-integradas — módulo `code`) reconstroem por REPLAY externo
    // (padrão `CostLedger`/`Mentality`): estes eventos NÃO tocam `ProjectedState` (no-op).
    /// SINAL DE MUDANÇA (spec 36 §2): um commit numa worktree de agente apendou trabalho.
    /// Emitido pelo hook git pós-commit via verbo `lina code-changed` — o supervisor
    /// carimba `author_node` server-side; o hook só transporta os fatos do commit
    /// (`paths` do `diff-tree`, `branch`, `commit`). Broadcast por pertencimento (Trilha B)
    /// compara `paths`×claims: sem interseção → ignora; com → o nó dono PARA e pede direção.
    CodeChanged {
        /// Branch da worktree onde o commit entrou (ex.: `lina/<nome-do-agente>`).
        branch: String,
        /// Caminhos tocados pelo commit (relativos ao repo) — DADO p/ a comparação ×claims.
        paths: Vec<String>,
        /// Nó autor — binding server-side do outbox autenticado, JAMAIS do payload (ADR 0007).
        author_node: String,
        /// SHA do commit que disparou o sinal (prova auditável no log).
        commit: String,
    },
    /// PROVA DE INTEGRAÇÃO (spec 36 §3): uma branch foi mergeada na linha de integração.
    /// É a ÚNICA prova de "branch fechada" — só com `BranchIntegrated` no log a projeção
    /// de branches-não-integradas a remove do conjunto pendente. Emitido pelo DevOps
    /// integrador por merge PROVADO; trabalho órfão nunca vira lixo (vira item de atenção).
    BranchIntegrated {
        /// Branch de origem que foi integrada.
        branch: String,
        /// Linha de integração de destino (ex.: `develop`).
        into: String,
        /// SHA do commit de merge resultante (prova auditável no log).
        commit: String,
    },
    // ─────────── F3-5 Auto-aprimoramento, Sessões & Contexto (spec 53 §11) ───────────
    // Buffers geridos + sessões --resume + disco custodiado + skills/pistas. Variantes
    // TOTALMENTE novas (precedente `SpawnRequested`/`CodeChanged`): campos OBRIGATÓRIOS sem
    // `#[serde(default)]` (replay antigo nunca as encontra); só os EXTENSÍVEIS (`*_at_ms`,
    // `after_first_prompt`, Option, Vec opcional) levam default. REGRA-MÃE (spec 53 §Segurança;
    // família ADR 0007): `session_id`/`approved_by`/`pressure`/`paths`/conteúdo de skill/pista é
    // DADO transportado, JAMAIS autoridade — a identidade do nó é `LINA_NODE_ID` (ADR 0026); a
    // poda de disco exige o gesto CUSTODIADO (`AttentionKind::Custody`), nunca o campo. ZERO LLM
    // no core (inv #1): tempo/pressão/seleção/validação são determinísticos; julgamento semântico
    // é 1 CLI-call fora do caminho crítico. As projeções (`ResumeSessionStore`/`BufferRegistry`/
    // `DiskBudget`/skill_index — frentes paralelas) reconstroem por REPLAY externo (padrão
    // `CostLedger`): estes eventos NÃO tocam `ProjectedState` (reducer no-op).
    /// F3-5-1 (§11.B): uma sessão `--resume` de um nó ficou retomável. Emitido pelo core SÓ após
    /// o nó receber ≥1 prompt (o shell observa o 1º turno via `lina-session-watch`). `session_id`
    /// é PROJEÇÃO (ponteiro de --resume validado pelo funil `admit_node`, ADR 0022), nunca
    /// identidade — forjá-lo não confere nada (a identidade segue `LINA_NODE_ID`, ADR 0026).
    SessionPersisted {
        /// node_id do nó cuja sessão foi salva (binding server-side, não do payload).
        node: String,
        /// CLI Profile (os `resume_args` vivem lá — `lina-cli-profiles`).
        cli: String,
        /// id capturado do JSONL pelo `lina-session-watch` — DADO, jamais autoridade.
        session_id: String,
        /// Invariante de admissão legível e auditável: só emitido após ≥1 prompt. Sempre `true`.
        #[serde(default = "default_true")]
        after_first_prompt: bool,
        #[serde(default)]
        first_prompt_at_ms: u64,
        /// Nome amigável p/ o leigo escolher no modal "Sessões salvas".
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        persisted_at_ms: u64,
    },
    /// F3-5-1 (§11.B): poda EXPLÍCITA de uma sessão salva. Sessão "ativa" = `SessionPersisted`
    /// sem `SessionRetired` posterior (projeção `ResumeSessionStore`). Nunca deletada do log.
    SessionRetired {
        node: String,
        session_id: String,
        /// `"lru"` | `"ttl"` | `"user_discarded"` | `"cli_invalidated"`. String aberta (precedente `SpawnGated.reason`).
        reason: String,
        #[serde(default)]
        retired_at_ms: u64,
    },
    /// F3-5-7 (§11.A): amostra de ocupação de UM buffer (poliforme via `buffer_id`).
    /// Anti-amplificação (doutrina `CostCeilingHit`): só emitido quando `pressure_ratio` cruza um
    /// bucket (passos de 10%) OU muda warn/critical — nunca um evento por linha. META (projeção
    /// `BufferRegistry` por replay; último por `buffer_id` vence).
    BufferOccupancySampled {
        /// `"scrollback:<panel>"` | `"mailbox:<node>"` | `"dlq"` | `"disk:<scope>"` | `"session_store"` | `"cost:day"` | `"pending"`.
        buffer_id: String,
        /// ocupação atual, na unidade de `unit`.
        used: u64,
        /// capacidade declarada; `0` = ilimitado/desligado (convenção de `token_budget_day`).
        capacity: u64,
        /// `"lines"` | `"bytes"` | `"messages"` | `"tokens"`.
        unit: String,
        /// `used/capacity`; `0.0` quando `capacity==0` (evita div-por-zero e sinaliza "off").
        pressure_ratio: f32,
        #[serde(default)]
        sampled_at_ms: u64,
    },
    /// F3-5-8 (§11.C): sinal de pressão de disco. DETECTAR/sinalizar é livre em todo nível de
    /// autonomia; APAGAR exige o gate humano custodiado (ver `DiskReclaimApproved`). META.
    DiskPressureSignaled {
        /// `"workspace_target"` | `"app_target"` | `"disk_total"`.
        path: String,
        used_bytes: u64,
        free_bytes: u64,
        /// `used/total` (o campo pedido no critério de aceite).
        used_pct: f32,
        /// `"warn"` | `"critical"`.
        threshold_crossed: String,
        #[serde(default)]
        signaled_at_ms: u64,
    },
    /// F3-5-8 (§11.C): o Lina PROPÕE candidatos de poda; jamais apaga sozinho. Proposta ≠ execução.
    DiskReclaimProposed {
        candidates: Vec<ReclaimCandidate>,
        reclaimable_bytes: u64,
        #[serde(default)]
        proposed_at_ms: u64,
    },
    /// F3-5-8 (§11.C): o GATE HUMANO — a ÚNICA porta para a execução. `approved_by` é EXIBIÇÃO;
    /// a AUTORIDADE é o gesto custodiado (`AttentionKind::Custody`, attention.rs), nunca este
    /// campo (forjar `approved_by` NÃO dispara poda — ADR 0007). Gate humano em TODOS os níveis.
    DiskReclaimApproved {
        candidate_paths: Vec<String>,
        /// EXIBIÇÃO, não autoridade — quem aprovou (humanizável), carimbado pelo gesto custodiado.
        approved_by: String,
        #[serde(default)]
        approved_at_ms: u64,
    },
    /// F3-5-8 (§11.C): a poda IRREVERSÍVEL aconteceu — SÓ após `DiskReclaimApproved` (zero
    /// `Executed` sem `Approved`; inv #6: apagar é irreversível). Prova auditável no log.
    DiskReclaimExecuted {
        reclaimed_bytes: u64,
        paths: Vec<String>,
        #[serde(default)]
        executed_at_ms: u64,
    },
    /// F3-5-4 (Hermes C.1): o seletor escolheu uma skill por gatilho DETERMINÍSTICO (ZERO LLM:
    /// match de gatilho + filtro por tools presentes). DADO observável de adoção; nunca autoridade.
    SkillSelected {
        node: String,
        skill: String,
        /// gatilho que casou (palavra-chave/contexto), se houver.
        #[serde(default)]
        trigger: Option<String>,
        /// de onde a skill veio (catálogo/hub/fábrica).
        source: String,
        // ── ADR 0045 C2 (spec 0045 §2): campos ADITIVOS de roteamento por retrieval+outcome.
        // Todos `#[serde(default)]` → um `SkillSelected` antigo (4 campos) reidrata trivialmente.
        // `skill` acima É o `chosen` do ADR — NÃO há campo `chosen` (duplicação é dívida).
        /// tipo de tarefa (`canon(role):top_terms(query)`, spec §4) — unidade de agregação do
        /// ranking; MESMA do `situation_hash` do ADR 0048 (um conceito, dois usos).
        #[serde(default)]
        task_kind: String,
        /// os skill_ids do top-k de C1 (retrieval) — o conjunto entre os quais esta venceu.
        #[serde(default)]
        candidates: Vec<String>,
        /// papel do nó (agregação cross-instância/multi-workspace, ADR 0010) — `node` é a
        /// instância (auditoria local), `by_role` agrega o aprendizado entre projetos.
        #[serde(default)]
        by_role: String,
        /// por que esta venceu (retrieval-only no MVP; ganha o termo de outcome quando C2 amadurece).
        #[serde(default)]
        rank_reason: String,
        /// id DETERMINÍSTICO da seleção (spec §3.1) — chave que o `SkillOutcome` referencia.
        #[serde(default)]
        selection_id: String,
    },
    /// F3-5-5 (Hermes C.4): faltou skill especialista → a fábrica PROPÕE uma via deep-research.
    /// SUGERE, nunca aplica (gate humano antes de carregar; `inline-shell` passa pelo guard,
    /// Hermes C.2). `references` são as fontes da pesquisa — DADO, não autoridade.
    SkillFactoryProposed {
        skill_name: String,
        /// `"deep-research"` (o método de proposta).
        via: String,
        /// fontes/referências que a pesquisa reuniu.
        #[serde(default)]
        references: Vec<String>,
    },
    /// ADR 0045 C2 (spec 0045 §3): RESULTADO de uma seleção de skill, ligado por `selection_ref`
    /// ao `SkillSelected.selection_id`. Sinal INFERIDO PASSIVAMENTE (a Lina nunca pergunta). META
    /// — no-op no `apply`; o score por replay é projeção dedicada (padrão `CostLedger`/`Mentality`).
    /// Skill é DADO, jamais autoridade: o outcome RANQUEIA, nunca dispensa gate (ADR 0045 §Segurança).
    SkillOutcome {
        /// = `selection_id` do `SkillSelected` casado (ligação determinística, replay reconstrói).
        selection_ref: String,
        /// Success | Failure | Reworked | HumanOverride.
        signal: SkillSignal,
        /// observabilidade/painel (ex.: "trocou para X"); opcional.
        #[serde(default)]
        evidence: String,
    },
    /// F3-5-6 (doc-fonte 65): pistas — pastas/arquivos que o terminal de um escopo passa a
    /// enxergar no briefing. `paths` é DADO de contexto, nunca autoridade (não concede acesso
    /// além do que o nó já tem — só informa o que a IA olha).
    ClueSetDefined {
        /// escopo da pista (ex.: nó/projeto/foco).
        scope: String,
        /// pastas/arquivos que entram no contexto.
        paths: Vec<String>,
        #[serde(default)]
        label: Option<String>,
    },
    // ───────── Rito de paradigma (spec-mestre 50, Meadows [2]; épico 39 §IV) ─────────
    /// Registro AUDITÁVEL do rito de red-team de paradigma ao FECHAR uma fase (ADR 0044).
    /// Aplica o invariante #4 ("o event log é a fonte") ao META-PROCESSO: a auto-crítica do
    /// paradigma vira FATO no log, não só um `.md`. Variante nova (campos obrigatórios sem
    /// `serde(default)`; `adr_ref` opcional). META — no-op no `apply` (auditoria por replay
    /// externo). Emissão = verbo `lina paradigm` futuro OU gesto do Maestro ao aceitar o ADR;
    /// `verdict`/`invariant_challenged` são DADO, jamais autoridade.
    ParadigmReviewed {
        /// fase revisada (ex.: `"F3"`).
        phase: String,
        /// o que foi desafiado (ex.: `"7-invariantes"` ou o invariante específico).
        invariant_challenged: String,
        /// veredito do rito (ex.: `"sem-dogma; 3 impostos nomeados"`).
        verdict: String,
        /// ADR que registra o rito (ex.: `"0044"`).
        #[serde(default)]
        adr_ref: Option<String>,
    },
    // ───────── F4-0 · Substrato de canais & credenciais (épico 41 §III/§IV) ─────────
    // LARGADA da onda F4-0 (dono único: Maestro). Os 5 eventos abaixo congelam o CONTRATO que as
    // frentes consomem (channel.rs/tool_scope.rs/broker.rs/credenciais) — cada uma só EMITE/projeta,
    // ninguém re-edita events.rs (elimina a costura nº1; espelha a largada F3-5, lib.rs §196-205).
    // Todos META (no-op no `apply`): reconstruídos por projeções dedicadas via replay (padrão
    // CostLedger/ClueSet). O instante de cada fato vive no `ts` do `EventRecord` — NÃO duplicado no
    // payload (convenção events.rs; espelha `ClueSetDefined`/`SkillSelected`). Decisão registrada na
    // peça `tasks/epico-f4/onda-0.md` §Decisões (diverge do §IV do épico, que listava `*_at`).
    /// F4-0-2 (ADR 0004 custódia · spec 53 §credencial): uma credencial de canal foi GUARDADA no
    /// keyring (`lina-secrets`), indexada por canal/escopo. **`key_ref` é REFERÊNCIA ao keyring,
    /// JAMAIS o valor** — o segredo vive no cofre, fora do log e fora do env do agente (invariante
    /// #2 "nada em claro no disco"). O backup silencioso antes de sobrescrever (Hermes doc 40 §3) é
    /// fato do cofre, não evento. META.
    CredentialStored {
        /// canal dono da credencial (ex.: `"whatsapp"`, `"email"`).
        channel: String,
        /// escopo lógico do segredo no cofre (`vault.set(scope, key, …)`).
        scope: String,
        /// REFERÊNCIA ao segredo no keyring (a chave/conta), nunca o valor.
        key_ref: String,
    },
    /// F4-0-2: revogação de uma credencial de canal — o broker para de injetar o segredo (o cofre
    /// apaga a entrada). `key_ref` é referência, jamais o valor. META.
    CredentialRevoked {
        /// canal dono da credencial revogada.
        channel: String,
        /// REFERÊNCIA ao segredo removido (a chave/conta), nunca o valor.
        key_ref: String,
    },
    /// F4-0-1 (TAG [10] Estrutura · doc 40 §4/§6): um canal foi REGISTRADO no `ChannelRegistry` a
    /// partir do seu manifesto declarativo (`channels/<nome>/manifest.toml`). **Manifesto é DADO (o
    /// gate é o broker, F4-0-3), JAMAIS autoridade.** `trust_tier ∈ {core,curado,comunidade}`;
    /// `install_ref` é pinado em SHA/tag (nunca flutua HEAD). Registrar ≠ conectar (default-deny por
    /// pertencimento, ADR 0006): o canal nasce "declarado, não conectado". META.
    ChannelRegistered {
        /// nome do canal (chave no registry).
        channel: String,
        /// referência ao manifesto lido (caminho relativo ao repo/dir de canais).
        manifest_ref: String,
        /// tier de confiança: `"core"` | `"curado"` | `"comunidade"`.
        trust_tier: String,
        /// ref de instalação pinada (SHA/tag), nunca HEAD flutuante.
        install_ref: String,
    },
    /// F4-0-4 (TAG [6] Fluxo · doc-fonte 63): o leigo DECLAROU, por projeto, uma ferramenta/grupo que
    /// a IA passa a enxergar. **Declarar ≠ autorizar** — é fluxo de contexto, não autoridade; a ação
    /// externa continua passando pelo broker+gate (F4-0-3). A projeção (`tool_scope.rs`) reconstrói o
    /// conjunto por replay (padrão `ClueSet`: último vence). META.
    ToolScopeDeclared {
        /// projeto/escopo dono da declaração.
        project: String,
        /// canal a que a ferramenta/grupo pertence.
        channel: String,
        /// a ferramenta/grupo declarado (ex.: `"grupo:Vendas"`).
        scope: String,
    },
    /// F4-0-4: a declaração foi RETIRADA — o acesso some no próximo turno (push, sem restart). Par de
    /// `ToolScopeDeclared`; o fold determinístico faz o escopo sumir no replay seguinte. META.
    ToolScopeRevoked {
        /// projeto/escopo dono da declaração retirada.
        project: String,
        /// canal a que a ferramenta/grupo pertencia.
        channel: String,
        /// a ferramenta/grupo retirado.
        scope: String,
    },
    // ───────── F4-WA · Webhook Ativo (épico 41 §III · spec 55 · ADR 0035) ─────────
    // LARGADA da onda F4-WA (dono único: Maestro 01). Os 2 eventos abaixo + os campos aditivos em
    // WebhookConfigured/WebhookReceived congelam o CONTRATO do fio recepção→injeção. Todos META (no-op
    // no `apply`): o engine de webhooks + o router reconstroem por replay. **NUNCA o payload cru nem o
    // secret no log** (só metadados — invariante #2/#4; ADR 0035 §5). Decisão na peça onda-wa.md.
    /// F4-WA-1 (spec 55 §5 · ADR 0035 §1): o SERVIDOR DESPACHOU um input de webhook a um terminal vivo
    /// — o fio recepção→injeção. Livro-razão do despacho server-side com origem `sistema/webhook`.
    /// **NUNCA carrega o payload cru** (só metadados — o conteúdo vive efêmero no PTY, não no log). META.
    WebhookDispatched {
        #[serde(default)]
        webhook_id: String,
        #[serde(default)]
        target_node: String,
        #[serde(default)]
        method: String,
        /// id da entrega A2A (correlaciona com `MessageDelivered`/`MessageRetained`/`MessageDeadLettered`).
        #[serde(default)]
        delivery_id: String,
        #[serde(default)]
        payload_sha256: String,
        #[serde(default)]
        payload_size: u64,
    },
    /// F4-WA-4 (spec 55 §5 · ADR 0035 §3 · ADR 0004): uma ação irreversível DISPARADA por webhook foi
    /// enfileirada no gate (custódia + fila de atenção) em vez de executar. Prova observável de que o
    /// webhook NÃO furou o gate humano. A custódia em si segue emitindo `ActionGated`/`BrokerExecuted`/
    /// `BrokerDenied` (reuso); este é o marcador específico do canal webhook. META.
    WebhookActionGated {
        #[serde(default)]
        webhook_id: String,
        #[serde(default)]
        target_node: String,
        /// classe da ação (espelha guard/broker: ex. `"gated-hard-external"`).
        #[serde(default)]
        action_class: String,
        /// nível efetivo aplicado: `min(webhook.autonomy_level, workspace.autonomy)`.
        #[serde(default)]
        effective_level: String,
    },
    // ───────── F4-1 · Canal WhatsApp/Waha (épico 41 §III · Onda F4-1) ─────────
    // LARGADA da onda F4-1 (dono único: Maestro 01). Os 4 eventos abaixo congelam o CONTRATO da vida
    // do canal: conectar (QR) → ler (fluido, auditado) → enviar (gated humano) → desconectar (zera o
    // cano). Todos META (no-op no `apply`): a projeção `ChannelRegistry`/`channel_read` reconstrói por
    // replay. **Doutrina dura (épico §IV):** `session_ref` é REFERÊNCIA custodiada (jamais o token);
    // conteúdo de mensagem NUNCA vai cru ao log (só `count`/refs — espelha `HistoryReadCross`);
    // `approved_by` é rótulo de PROVENIÊNCIA, jamais autoridade. Convenção D2: o instante vive no `ts`
    // do `EventRecord` (sem `*_at` no payload). Decisão na peça onda-1.md.
    /// F4-1-2 (TAG [10] Estrutura): uma conta de canal foi CONECTADA por ação humana de tela (o leigo
    /// escaneou o QR). Opt-in sinalizado + gesto explícito (nunca silencioso). A projeção passa a
    /// derivar `ChannelStatus::Connected`. `session_ref` é REFERÊNCIA à sessão custodiada (keyring),
    /// jamais o token — o agente nunca o vê (ADR 0004). META.
    ChannelConnected {
        /// nome do canal (ex.: `"whatsapp"`).
        #[serde(default)]
        channel: String,
        /// REFERÊNCIA à sessão custodiada no cofre (a chave/conta), nunca o token de sessão.
        #[serde(default)]
        session_ref: String,
        /// escopo da conexão (ex.: provedor/instância Waha) — DADO declarativo, nunca autoridade.
        #[serde(default)]
        scope: String,
    },
    /// F4-1-2 (TAG [10] Estrutura): a conexão foi DESFEITA — o cano zera; o próximo `lina do <canal>`
    /// falha "não conectado". Par de `ChannelConnected`; o fold determinístico derruba o estado para
    /// `Declared` no replay seguinte. META.
    ChannelDisconnected {
        /// nome do canal desconectado.
        #[serde(default)]
        channel: String,
        /// REFERÊNCIA à sessão encerrada (correlaciona com o `ChannelConnected`), nunca o token.
        #[serde(default)]
        session_ref: String,
    },
    /// F4-1-3 (TAG [6] Fluxo): a IA LEU uma conversa/grupo DECLARADO como contexto de entrada. Leitura
    /// é fluida (sem efeito externo → sem gate) mas AUDITADA. **Conteúdo NUNCA vai cru ao log** — só
    /// metadados (`count`/refs), espelhando `HistoryReadCross`. Só grupos no `tool_scope` (default-deny
    /// F4-1-5): ler um não-declarado é recusado e auditado, não emite este evento de sucesso. META.
    ChannelMessageRead {
        /// nome do canal (ex.: `"whatsapp"`).
        #[serde(default)]
        channel: String,
        /// REFERÊNCIA à conversa/grupo lido (ex.: `"grupo:Vendas"`), nunca o conteúdo.
        #[serde(default)]
        conversation_ref: String,
        /// quantidade de mensagens lidas (metadado, nunca o texto).
        #[serde(default)]
        count: u32,
        /// nó que leu (proveniência da leitura), para a trilha de auditoria.
        #[serde(default)]
        read_by_node: String,
    },
    /// F4-1-4 (TAG [10] Estrutura · ADR 0004): uma mensagem FOI ENVIADA — só APÓS o gate humano. Encadeia
    /// a `ActionGated`+`BrokerExecuted` (reusados via `broker_exec_ref`). **Texto da mensagem fora do log
    /// cru** (só refs). `approved_by` é rótulo de PROVENIÊNCIA do gesto humano (lição
    /// `human-gesture-sentinela`), JAMAIS autoridade — quem libera é a custódia, não este campo. META.
    ChannelMessageSent {
        /// nome do canal (ex.: `"whatsapp"`).
        #[serde(default)]
        channel: String,
        /// REFERÊNCIA à conversa/grupo destino, nunca o conteúdo.
        #[serde(default)]
        conversation_ref: String,
        /// correlação com o `BrokerExecuted` da custódia (prova de que passou pelo gate).
        #[serde(default)]
        broker_exec_ref: String,
        /// rótulo de proveniência de quem aprovou (humano) — auditoria, JAMAIS autoridade.
        #[serde(default)]
        approved_by: String,
    },
    // ───────── F2-4 · Área de Poderes (épico 38 §III F2-4 · ADR 0052) ─────────
    // LARGADA da onda F2-4 (dono único: Maestro 01). Evento de AUDITORIA anti-manipulação do scan
    // de Poderes — METADADOS apenas. META: no-op no `apply`; o painel NÃO depende dele (lê o scan
    // de disco direto via `powers::scan_powers`). ADITIVO (`#[serde(default)]`): replay de log antigo
    // nunca quebra. Limite duro (ADR 0052 §2): ZERO nome/descrição/conteúdo de poder; NENHUM campo
    // decide autorização (mostrar ≠ autorizar).
    /// F2-4: um scan da Área de Poderes ocorreu — só CONTADORES + timestamp, para auditoria.
    PowerScanned {
        /// total de poderes vistos (soma de todos os kinds).
        total: u32,
        /// contagem por kind — chave = `PowerKind` em minúsculo ("skill","plugin",…). String, não
        /// enum, para o evento sobreviver a um `PowerKind` novo (replay-safe).
        #[serde(default)]
        counts: BTreeMap<String, u32>,
        #[serde(default)]
        scanned_at_ms: u64,
    },
}

/// Default do campo `muted` de [`DomainEvent::NodeDetectionMuted`] — `true` para o
/// payload mínimo casar com a semântica do nome do evento.
fn default_muted() -> bool {
    true
}

/// Default de `autonomy_level` em [`DomainEvent::WebhookConfigured`] (F4-WA, spec 55 §3): nível
/// CONSERVADOR `notify_before` — replay de payload antigo/incompleto NUNCA fabrica autonomia.
fn default_notify_before() -> String {
    "notify_before".to_string()
}

/// Default de `after_first_prompt` em [`DomainEvent::SessionPersisted`] (F3-5-1, spec 53 §11.B):
/// a invariante de admissão ("só após ≥1 prompt") é sempre `true` — o campo existe para tornar a
/// regra legível e auditável no log, e o default cobre payloads mínimos.
fn default_true() -> bool {
    true
}

/// Tipo de apoio de [`DomainEvent::DiskReclaimProposed`] (F3-5-8, spec 53 §11.C): um candidato de
/// poda de disco. DADO descritivo (caminho/tamanho/tipo), NUNCA autoridade — a poda só ocorre após
/// o gesto custodiado (`DiskReclaimApproved`). Aditivo (serde): proposta antiga sem o campo carrega.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReclaimCandidate {
    /// caminho do candidato (ex.: `target/` de cargo — o dominante do ENOSPC #25).
    pub path: String,
    /// tamanho recuperável em bytes.
    pub bytes: u64,
    /// `"cargo_target"` | `"old_scrollback_db"` | `"dlq_archive"`.
    pub kind: String,
}

impl DomainEvent {
    /// O `kind` persistido — idêntico à tag interna do serde.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            DomainEvent::WorkspaceCreated { .. } => "WorkspaceCreated",
            DomainEvent::NodeAdded { .. } => "NodeAdded",
            DomainEvent::NodeMoved { .. } => "NodeMoved",
            DomainEvent::TerminalRedimensionado { .. } => "TerminalRedimensionado",
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
            DomainEvent::SystemParamsChanged { .. } => "SystemParamsChanged",
            DomainEvent::EffortAssigned { .. } => "EffortAssigned",
            DomainEvent::PlanItemAdded { .. } => "PlanItemAdded",
            DomainEvent::PlanDecisionAdded { .. } => "PlanDecisionAdded",
            DomainEvent::PlanClaimed { .. } => "PlanClaimed",
            DomainEvent::PlanChecked { .. } => "PlanChecked",
            DomainEvent::GoalDefined { .. } => "GoalDefined",
            DomainEvent::GoalInterpreted { .. } => "GoalInterpreted",
            DomainEvent::GoalConfirmed { .. } => "GoalConfirmed",
            DomainEvent::GoalDecomposed { .. } => "GoalDecomposed",
            DomainEvent::GoalAchieved { .. } => "GoalAchieved",
            DomainEvent::GoalEscalated { .. } => "GoalEscalated",
            DomainEvent::PlanItemAttributed { .. } => "PlanItemAttributed",
            DomainEvent::ReviewVerdict { .. } => "ReviewVerdict",
            DomainEvent::NoteUpdated { .. } => "NoteUpdated",
            DomainEvent::DiscoveryIndexed { .. } => "DiscoveryIndexed",
            DomainEvent::NoteCreated { .. } => "NoteCreated",
            DomainEvent::FolderCreated { .. } => "FolderCreated",
            DomainEvent::CliProfileSet { .. } => "CliProfileSet",
            DomainEvent::NodeCwdSet { .. } => "NodeCwdSet",
            DomainEvent::WorkspaceFocusSet { .. } => "WorkspaceFocusSet",
            DomainEvent::WorkspaceUnloaded { .. } => "WorkspaceUnloaded",
            DomainEvent::OrchestrationPaused => "OrchestrationPaused",
            DomainEvent::OrchestrationResumed => "OrchestrationResumed",
            DomainEvent::WebhookConfigured { .. } => "WebhookConfigured",
            DomainEvent::WebhookReceived { .. } => "WebhookReceived",
            DomainEvent::WebhookDispatched { .. } => "WebhookDispatched",
            DomainEvent::WebhookActionGated { .. } => "WebhookActionGated",
            DomainEvent::ChannelConnected { .. } => "ChannelConnected",
            DomainEvent::ChannelDisconnected { .. } => "ChannelDisconnected",
            DomainEvent::ChannelMessageRead { .. } => "ChannelMessageRead",
            DomainEvent::ChannelMessageSent { .. } => "ChannelMessageSent",
            // F2-4 Área de Poderes (épico 38 · ADR 0052): auditoria do scan (só metadados).
            DomainEvent::PowerScanned { .. } => "PowerScanned",
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
            // F3-3 Mentality (spec 35 §3): ciclo de vida da crença.
            DomainEvent::CorrectionObserved { .. } => "CorrectionObserved",
            DomainEvent::BeliefProposed { .. } => "BeliefProposed",
            DomainEvent::BeliefReinforced { .. } => "BeliefReinforced",
            DomainEvent::BeliefChallenged { .. } => "BeliefChallenged",
            DomainEvent::BeliefEstablished { .. } => "BeliefEstablished",
            DomainEvent::BeliefRetired { .. } => "BeliefRetired",
            // F3-4 Coordenação de código (spec 36): sinal de mudança + prova de integração.
            DomainEvent::CodeChanged { .. } => "CodeChanged",
            DomainEvent::BranchIntegrated { .. } => "BranchIntegrated",
            // F3-5 Auto-aprimoramento, Sessões & Contexto (spec 53 §11).
            DomainEvent::SessionPersisted { .. } => "SessionPersisted",
            DomainEvent::SessionRetired { .. } => "SessionRetired",
            DomainEvent::BufferOccupancySampled { .. } => "BufferOccupancySampled",
            DomainEvent::DiskPressureSignaled { .. } => "DiskPressureSignaled",
            DomainEvent::DiskReclaimProposed { .. } => "DiskReclaimProposed",
            DomainEvent::DiskReclaimApproved { .. } => "DiskReclaimApproved",
            DomainEvent::DiskReclaimExecuted { .. } => "DiskReclaimExecuted",
            DomainEvent::SkillSelected { .. } => "SkillSelected",
            DomainEvent::SkillFactoryProposed { .. } => "SkillFactoryProposed",
            DomainEvent::SkillOutcome { .. } => "SkillOutcome",
            DomainEvent::ClueSetDefined { .. } => "ClueSetDefined",
            // Rito de paradigma (épico 39 §IV): registro auditável do red-team de fechamento de fase.
            DomainEvent::ParadigmReviewed { .. } => "ParadigmReviewed",
            DomainEvent::CredentialStored { .. } => "CredentialStored",
            DomainEvent::CredentialRevoked { .. } => "CredentialRevoked",
            DomainEvent::ChannelRegistered { .. } => "ChannelRegistered",
            DomainEvent::ToolScopeDeclared { .. } => "ToolScopeDeclared",
            DomainEvent::ToolScopeRevoked { .. } => "ToolScopeRevoked",
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: u64,
    pub ts: u64,
    pub kind: String,
    pub version: u32,
    pub payload: serde_json::Value,
}

struct PreparedEvent {
    kind: String,
    version: u32,
    payload: serde_json::Value,
}

struct CommittedBatch {
    seqs: Vec<u64>,
    event_count: u64,
}

struct ProjectionAtSeq {
    state: ProjectedState,
    seq: u64,
    replayed: u64,
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
    /// F2-3-2: grade do PTY do último `TerminalRedimensionado` do nó. `None` = nó nunca
    /// redimensionado pelo usuário (cai no tamanho fixo do card). `serde(default)` →
    /// snapshots antigos (sem o campo) desserializam com `None`, sem quebrar o replay.
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
    /// F2-3-2: tamanho EXATO do card em px (espelha `x`/`y`); restaura o frame ao reabrir.
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
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
                    cols: None,
                    rows: None,
                    w: None,
                    h: None,
                },
            );
        }
        DomainEvent::NodeMoved { node, x, y } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.x = *x;
                n.y = *y;
            }
        }
        DomainEvent::TerminalRedimensionado {
            node,
            cols,
            rows,
            w,
            h,
        } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cols = Some(*cols);
                n.rows = Some(*rows);
                n.w = Some(*w);
                n.h = Some(*h);
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
        // F3-1 (spec 52 §2): atributos da Goal são EFETIVOS no plano projetado (último vence por
        // item) — mesma família validada-no-comando/infalível-no-replay dos demais `Plan*`.
        DomainEvent::PlanItemAttributed {
            item,
            goal_id,
            parents,
            acceptance,
            budget_tokens,
            paths,
        } => state.plan.apply_item_attributed(
            item,
            goal_id.clone(),
            parents.clone(),
            acceptance.clone(),
            *budget_tokens,
            paths.clone(),
        ),
        // W4-1: a indexação corrente substitui a anterior (último check-up vence).
        DomainEvent::DiscoveryIndexed { clis } => state.discovered_clis = clis.clone(),
        // W4-2 (P4): o CLI Profile do nó é projetado no campo `cli` (idempotente por nó).
        DomainEvent::CliProfileSet { node, profile } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cli = Some(profile.clone());
            }
        }
        // ADR 0037: a edição da Pasta de Trabalho por-nó projeta o `cwd` (último vence). O
        // restore consome `ProjectedNode.cwd`, então a nova pasta passa a valer na próxima
        // abertura mesmo sem re-spawn (o caminho "só na próxima vez" da escolha do fundador).
        DomainEvent::NodeCwdSet { node, cwd } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cwd = Some(cwd.clone());
            }
        }
        // W4-4 (T6): o foco de workspace é projetado (último vence).
        DomainEvent::WorkspaceFocusSet { workspace, .. } => {
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
        // F3-0-2: mudança de parâmetro é META — o `ParamsLedger` (params.rs) reconstrói a config
        // efetiva varrendo o log (padrão CostLedger); sem efeito na projeção do canvas.
        | DomainEvent::SystemParamsChanged { .. }
        // F3-0-4: effort observado/atribuído é META — o badge `modelo·effort` varre o log; o nó
        // não muda de status nem de posição por isto (sem efeito na projeção do canvas).
        | DomainEvent::EffortAssigned { .. }
        // F3-1: o ciclo da Goal é META no apply do canvas; a projeção `Goal` (goal.rs) reconstrói por
        // replay varrendo por `goal_id` — padrão CostLedger/ParamsLedger; sem efeito na projeção do
        // canvas. (`PlanItemAttributed` é a exceção EFETIVA — muta o Plan, tratado acima.)
        | DomainEvent::GoalDefined { .. }
        | DomainEvent::GoalInterpreted { .. }
        | DomainEvent::GoalConfirmed { .. }
        | DomainEvent::GoalDecomposed { .. }
        | DomainEvent::GoalAchieved { .. }
        | DomainEvent::GoalEscalated { .. }
        | DomainEvent::ReviewVerdict { .. }
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
        // F4-WA: despacho server-side / gate de ação por webhook são META — o engine + o router
        // reconstroem por replay; payload externo e secret NUNCA no log (ADR 0035 §5).
        | DomainEvent::WebhookDispatched { .. }
        | DomainEvent::WebhookActionGated { .. }
        // F4-1: vida do canal WhatsApp/Waha (conectar/ler/enviar/desconectar) é META — a projeção
        // `ChannelRegistry`/`channel_read` reconstrói por replay; conteúdo de mensagem e token de
        // sessão NUNCA no log (só refs/metadados — épico 41 §IV).
        | DomainEvent::ChannelConnected { .. }
        | DomainEvent::ChannelDisconnected { .. }
        | DomainEvent::ChannelMessageRead { .. }
        | DomainEvent::ChannelMessageSent { .. }
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
        | DomainEvent::PaletteCommandInvoked { .. }
        // F3-3 Mentality (spec 35 §3): o ciclo de vida da crença é META no apply do canvas — a
        // projeção `Mentality(papel)` reconstrói por REPLAY externo (módulo `mentality`, padrão
        // `CostLedger`). Estes eventos NÃO tocam `ProjectedState`.
        | DomainEvent::CorrectionObserved { .. }
        | DomainEvent::BeliefProposed { .. }
        | DomainEvent::BeliefReinforced { .. }
        | DomainEvent::BeliefChallenged { .. }
        | DomainEvent::BeliefEstablished { .. }
        | DomainEvent::BeliefRetired { .. }
        // F3-4 Coordenação de código (spec 36): o sinal de mudança e a prova de integração
        // são META no apply do canvas — as projeções (`code`) reconstroem por REPLAY externo
        // (padrão `CostLedger`/`Mentality`). Estes eventos NÃO tocam `ProjectedState`.
        | DomainEvent::CodeChanged { .. }
        | DomainEvent::BranchIntegrated { .. }
        // F3-5 Auto-aprimoramento, Sessões & Contexto (spec 53 §11): buffers/sessões/disco/skills/
        // pistas são META no apply do canvas — as projeções (`ResumeSessionStore`/`BufferRegistry`/
        // `DiskBudget`/skill_index, frentes paralelas) reconstroem por REPLAY externo. NÃO tocam
        // `ProjectedState`.
        | DomainEvent::SessionPersisted { .. }
        | DomainEvent::SessionRetired { .. }
        | DomainEvent::BufferOccupancySampled { .. }
        | DomainEvent::DiskPressureSignaled { .. }
        | DomainEvent::DiskReclaimProposed { .. }
        | DomainEvent::DiskReclaimApproved { .. }
        | DomainEvent::DiskReclaimExecuted { .. }
        | DomainEvent::SkillSelected { .. }
        | DomainEvent::SkillFactoryProposed { .. }
        | DomainEvent::SkillOutcome { .. }
        | DomainEvent::ClueSetDefined { .. }
        // Rito de paradigma: registro de auditoria (META) — reconstruído por replay, não toca o canvas.
        | DomainEvent::ParadigmReviewed { .. }
        // F4-0: credenciais/canais/tool-scope são META — projeções dedicadas (channel.rs/tool_scope.rs)
        // e o cofre (lina-secrets) reconstroem o estado por replay; sem efeito na projeção do canvas.
        | DomainEvent::CredentialStored { .. }
        | DomainEvent::CredentialRevoked { .. }
        | DomainEvent::ChannelRegistered { .. }
        | DomainEvent::ToolScopeDeclared { .. }
        | DomainEvent::ToolScopeRevoked { .. }
        // F2-4: auditoria do scan de Poderes — META (ADR 0052); o painel lê o scan de disco direto.
        | DomainEvent::PowerScanned { .. } => {}
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
CREATE TABLE IF NOT EXISTS jsonl_mirror (
    seq INTEGER PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS jsonl_state (
    singleton    INTEGER PRIMARY KEY CHECK (singleton = 1),
    file_len     INTEGER NOT NULL,
    modified_ns  TEXT    NOT NULL,
    event_count  INTEGER NOT NULL
);
";

const EVENT_BY_SEQ_SQL: &str = "SELECT seq, ts, kind, version, payload FROM events WHERE seq = ?1";

/// **W3-7b §4.3 — reconciliação do path do espelho.** O design e o gate da Onda 3 verificam
/// `.lina/events/log.jsonl`; o código histórico escrevia `<store>/bus.jsonl`. DECISÃO: o espelho
/// JSONL chama-se **`log.jsonl`** (alinha com o gate + design; o `EventStore` já é aberto em
/// `.lina/events/`). Um único nome canônico — escritor (`open`) E leitor (`rebuild_from_jsonl`)
/// usam esta const; nada de 3º nome. (Stores de dev com `bus.jsonl` antigo ficam órfãos — sem
/// migração no MVP, pois não há dados persistentes de produção.)
const JSONL_FILE: &str = "log.jsonl";

/// Certificado externo do espelho. Não é uma terceira autoridade: só prova que o
/// JSONL contém exatamente o último conjunto que o SQLite confirmou. Recuperação
/// sem este certificado, com `pending` ou com digest divergente é recusada.
const JSONL_RECOVERY_STATE_FILE: &str = "log.recovery.json";
const JSONL_RECOVERY_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JsonlRecoveryState {
    version: u32,
    #[serde(flatten)]
    status: JsonlRecoveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum JsonlRecoveryStatus {
    /// Gravado e sincronizado ANTES do commit SQLite. Enquanto existir, nenhum
    /// prefixo do JSONL pode ser promovido a banco recuperado.
    Pending {
        target_event_count: u64,
        target_last_seq: u64,
    },
    /// Publicado somente depois do `sync_data` do JSONL canônico.
    Complete {
        event_count: u64,
        last_seq: u64,
        digest_sha256: String,
    },
}

impl JsonlRecoveryState {
    fn pending(target_event_count: u64, target_last_seq: u64) -> Self {
        Self {
            version: JSONL_RECOVERY_STATE_VERSION,
            status: JsonlRecoveryStatus::Pending {
                target_event_count,
                target_last_seq,
            },
        }
    }

    fn complete(event_count: u64, last_seq: u64, digest_sha256: String) -> Self {
        Self {
            version: JSONL_RECOVERY_STATE_VERSION,
            status: JsonlRecoveryStatus::Complete {
                event_count,
                last_seq,
                digest_sha256,
            },
        }
    }
}

/// Event store de um workspace: SQLite WAL (`events`) + espelho `log.jsonl` (§4.3) +
/// snapshots. Single-thread (a `Connection` do rusqlite não é `Sync`); o supervisor
/// futuro a embrulha sob lock.
pub struct EventStore {
    conn: Connection,
    dir: PathBuf,
    db_path: PathBuf,
    jsonl_path: PathBuf,
    jsonl_recovery_state_path: PathBuf,
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
    /// Estado SHA-256 incremental do arquivo físico observado por esta conexão.
    /// Outra conexão alterando o arquivo invalida o cache por tamanho/mtime/count.
    jsonl_digest_cache: Option<JsonlDigestCache>,
}

/// r5 perf-ws: cadência do snapshot AUTOMÁTICO no append (1 a cada N eventos). Sem isso,
/// `take_snapshot` não tinha chamador em produção e toda conexão NOVA re-parseava o log
/// inteiro (~91ms em 21k eventos, crescendo sem teto). Alto o bastante para testes
/// pequenos nunca cruzarem; baixo o bastante para limitar o replay frio a poucos ms.
const SNAPSHOT_EVERY: u64 = 4096;

impl EventStore {
    /// Abre (ou cria) o event store em `dir` pelo caminho rápido. Um banco existente
    /// recebe somente validação estrutural e posicional (`events` + primeiro/último
    /// `seq`); o histórico e o JSONL nunca são varridos nesta etapa. Corrupção interna
    /// aparece naturalmente no consumidor que projeta o log — use
    /// [`EventStore::project_or_recover`] quando esse consumidor puder recuperar.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref();
        let db = dir.join("lina.db");
        let jsonl = dir.join(JSONL_FILE);
        let recovery_state = dir.join(JSONL_RECOVERY_STATE_FILE);
        if !db.try_exists()? && recovery_artifacts_exist(&jsonl, &recovery_state)? {
            return Err(invalid_recovery_data(format!(
                "há artefatos de recuperação em {} sem o SQLite autoritativo; use open_or_recover",
                dir.display()
            )));
        }
        Self::open_inner(dir, true)
    }

    /// Abre a conexão e o schema. O rebuild usa `reconcile=false` porque, nesse instante,
    /// o JSONL é a única cópia íntegra: reconciliá-lo contra um SQLite recém-criado e vazio
    /// apagaria justamente a fonte de recuperação.
    fn open_inner(dir: &Path, reconcile: bool) -> Result<Self, StoreError> {
        let dir = dir.to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let snapshots_dir = dir.join("snapshots");
        std::fs::create_dir_all(&snapshots_dir)?;
        let db_path = dir.join("lina.db");
        let jsonl_path = dir.join(JSONL_FILE);
        let jsonl_recovery_state_path = dir.join(JSONL_RECOVERY_STATE_FILE);
        let db_existed = db_path.try_exists()?;

        let conn = Connection::open(&db_path)?;
        // FURO A (parte 1): com o store ws2/ws3 unificado, APP e bin (`lina do/guard`) escrevem no MESMO
        // log → escritores CONCORRENTES. Sem `busy_timeout`, o 2º escritor recebe `SQLITE_BUSY` IMEDIATO
        // ao disputar o write-lock e PERDE o evento (de segurança!). Com o timeout, ele ESPERA o lock
        // (30s cobre inclusive uma sequência de fsyncs do certificado durável; o WAL deixa leituras
        // passarem em paralelo). DEFINIDO ANTES de qualquer escrita:
        // o próprio `journal_mode=WAL` e o `CREATE TABLE` (schema) pegam write-lock, então a ABERTURA
        // concorrente já precisa do timeout (senão é a `open()` do 2º processo que estoura `BUSY`).
        // O mesmo lock serializa a leitura fresca de `MAX(seq)` e o INSERT transacional do batch;
        // nenhum escritor aloca sequência a partir de cache local.
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        let existing_position = db_existed
            .then(|| validate_existing_store_head(&conn))
            .transpose()?;
        enable_wal(&conn)?;
        conn.execute_batch(SCHEMA)?;
        let (event_count, last_seq) = match existing_position {
            Some(position) => position,
            None => event_log_position(&conn)?,
        };
        let next_seq = last_seq.checked_add(1).ok_or_else(|| {
            invalid_recovery_data("sequência do event log excedeu u64".to_owned())
        })?;

        let mut store = Self {
            conn,
            dir,
            db_path,
            jsonl_path,
            jsonl_recovery_state_path,
            snapshots_dir,
            next_seq,
            proj_cache: std::cell::RefCell::new(None),
            replayed_total: std::cell::Cell::new(0),
            jsonl_digest_cache: None,
        };

        if reconcile {
            // SQLite é a autoridade. Se um append anterior confirmou o INSERT e o espelho falhou
            // depois (por exemplo EMFILE), repõe o arquivo canônico inteiro. A reconciliação é
            // best-effort porque indisponibilidade do espelho não pode tornar indisponível o log
            // autoritativo; a falha permanece observável e será tentada no próximo append/open.
            if let Err(error) = store.reconcile_jsonl_from_db(event_count, last_seq) {
                tracing::error!(
                    path = %store.jsonl_path.display(),
                    %error,
                    "espelho JSONL degradado; SQLite segue autoritativo e a reconciliação será repetida"
                );
            }
        }

        Ok(store)
    }

    /// Abertura resiliente a falhas ESTRUTURAIS detectáveis pelo cabeçalho do store.
    /// O caminho saudável chama [`EventStore::open`] uma única vez e permanece O(1).
    /// Para corrupção encontrada durante o replay, abra com [`EventStore::open`] e
    /// finalize com [`EventStore::project_or_recover`].
    pub fn open_or_recover(dir: impl AsRef<Path>, ui: &mut dyn UiHost) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("lina.db");
        let jsonl_path = dir.join(JSONL_FILE);
        let recovery_state_path = dir.join(JSONL_RECOVERY_STATE_FILE);

        if !db_path.try_exists()? {
            if recovery_artifacts_exist(&jsonl_path, &recovery_state_path)? {
                let records = recoverable_jsonl_records(&jsonl_path, &recovery_state_path)?;
                ui.on_event(HostEvent::Recovering);
                let store = Self::rebuild_database_atomic(&dir, &records, |_| Ok(()))?;
                ui.on_event(HostEvent::Recovered);
                return Ok(store);
            }
            return Self::open(&dir); // store realmente novo: não há DB nem espelho recuperável
        }

        let open_error = match Self::open(&dir) {
            Ok(store) => return Ok(store),
            Err(error) => error,
        };
        if !is_recoverable_store_corruption(&open_error) || !db_is_corrupt(&db_path) {
            return Err(open_error);
        }
        Self::recover_existing_database(&dir, ui)
    }

    /// Abre e projeta com recuperação completa nos dois pontos onde corrupção pode
    /// aparecer: no cabeçalho estrutural ou durante o replay semântico. Este é o helper
    /// indicado para boot/loaders que precisam do estado projetado imediatamente.
    pub fn open_projected_or_recover(
        dir: impl AsRef<Path>,
        ui: &mut dyn UiHost,
    ) -> Result<(Self, ProjectedState), StoreError> {
        Self::open_or_recover(dir, ui)?.project_or_recover(ui)
    }

    /// Projeta um store já aberto pelo fast path e só aciona disaster recovery quando
    /// o replay observa corrupção determinística. Erros transitórios (lock, permissão,
    /// I/O) continuam sendo devolvidos sem preservar ou substituir um banco saudável.
    ///
    /// O retorno conserva o store porque consumidores quentes normalmente precisam do
    /// estado projetado e da conexão que o produziu.
    pub fn project_or_recover(
        self,
        ui: &mut dyn UiHost,
    ) -> Result<(Self, ProjectedState), StoreError> {
        match self.project() {
            Ok(projected) => Ok((self, projected)),
            Err(error) if is_recoverable_store_corruption(&error) => {
                let dir = self.dir.clone();
                drop(self);
                let recovered = Self::recover_existing_database(&dir, ui)?;
                let projected = recovered.project()?;
                Ok((recovered, projected))
            }
            Err(error) => Err(error),
        }
    }

    fn recover_existing_database(dir: &Path, ui: &mut dyn UiHost) -> Result<Self, StoreError> {
        let db_path = dir.join("lina.db");
        let jsonl_path = dir.join(JSONL_FILE);
        let recovery_state_path = dir.join(JSONL_RECOVERY_STATE_FILE);
        let records =
            recoverable_jsonl_records(&jsonl_path, &recovery_state_path).map_err(|error| {
                invalid_recovery_data(format!(
                    "{} está corrompido e o espelho não é recuperável: {error}",
                    db_path.display()
                ))
            })?;

        // Corrompido (lição Codex #21750): recuperação VISÍVEL.
        ui.on_event(HostEvent::Recovering);
        let preserved = preserve_corrupt(&db_path)?;
        tracing::warn!(corrupt = %preserved.display(), "banco corrompido preservado; reconstruindo do JSONL");
        let store = Self::rebuild_database_atomic(dir, &records, |_| Ok(()))?;
        ui.on_event(HostEvent::Recovered);
        Ok(store)
    }

    /// Constrói o SQLite recuperado fora do caminho final, fecha/checkpointa a conexão e
    /// só então publica `lina.db` por rename. Qualquer falha intermediária deixa o JSONL
    /// original intacto e nenhum banco parcial que um retry possa confundir com saudável.
    fn rebuild_database_atomic(
        dir: &Path,
        records: &[EventRecord],
        mut checkpoint: impl FnMut(usize) -> Result<(), StoreError>,
    ) -> Result<Self, StoreError> {
        let staging = dir.join(format!(
            ".lina-db-recovery-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::create_dir(&staging)?;
        let build = (|| -> Result<(), StoreError> {
            let mut store = Self::open_inner(&staging, false)?;
            {
                let tx = store
                    .conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                for (index, record) in records.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO events (seq, ts, kind, version, payload) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            i64::try_from(record.seq).map_err(|_| {
                                invalid_recovery_data(format!(
                                    "seq {} não cabe no SQLite",
                                    record.seq
                                ))
                            })?,
                            i64::try_from(record.ts).map_err(|_| {
                                invalid_recovery_data(format!(
                                    "timestamp {} não cabe no SQLite",
                                    record.ts
                                ))
                            })?,
                            record.kind,
                            i64::from(record.version),
                            serde_json::to_string(&record.payload)?
                        ],
                    )?;
                    checkpoint(index + 1)?;
                }
                tx.commit()?;
            }
            if query_all_events(&store.conn)? != records {
                return Err(invalid_recovery_data(
                    "o SQLite de staging não reproduziu todos os eventos do espelho".to_owned(),
                ));
            }
            let integrity: String = store
                .conn
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            if integrity != "ok" {
                return Err(invalid_recovery_data(format!(
                    "integrity_check do DB recuperado retornou {integrity}"
                )));
            }
            store
                .conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
            drop(store);

            let staged_db = staging.join("lina.db");
            std::fs::File::open(&staged_db)?.sync_all()?;
            let final_db = dir.join("lina.db");
            if final_db.try_exists()? {
                return Err(invalid_recovery_data(format!(
                    "{} apareceu durante a recuperação; o candidato completo ficou isolado",
                    final_db.display()
                )));
            }
            std::fs::rename(&staged_db, &final_db)?;
            #[cfg(unix)]
            if let Err(error) = std::fs::File::open(dir).and_then(|parent| parent.sync_all()) {
                tracing::error!(
                    path = %dir.display(),
                    %error,
                    "DB recuperado já foi publicado; fsync do diretório falhou"
                );
            }
            Ok(())
        })();
        match std::fs::remove_dir_all(&staging) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if build.is_ok() => return Err(error.into()),
            Err(error) => tracing::warn!(
                path = %staging.display(),
                %error,
                "não consegui remover staging de recuperação após falha"
            ),
        }
        build?;
        Self::open(dir)
    }

    /// Anexa um evento do domínio (versão CORRENTE) ao log: INSERT no SQLite +
    /// linha no espelho JSONL append-only. Devolve o `seq` atribuído.
    pub fn append(&mut self, event: &DomainEvent) -> Result<u64, StoreError> {
        let committed = self.insert_prepared_batch(vec![PreparedEvent {
            kind: event.kind().to_owned(),
            version: event.current_version(),
            payload: serde_json::to_value(event)?,
        }])?;
        committed.seqs.into_iter().next().ok_or_else(|| {
            invalid_recovery_data("append unitário não recebeu sequência".to_owned())
        })
    }

    /// Persiste `events` em uma única transação SQLite, preservando a ordem e
    /// devolvendo a contagem confirmada do log (igual ao último `seq`, pois o log é
    /// contíguo). Slice vazio devolve a contagem atual. Qualquer erro até o commit
    /// deixa zero subconjunto visível. Depois do commit confirmado, falha do espelho
    /// não vira falso `Err`: o certificado permanece `pending` e um DB saudável o
    /// repara na próxima abertura/append.
    pub fn append_batch(&mut self, events: &[DomainEvent]) -> Result<u64, StoreError> {
        let prepared = events
            .iter()
            .map(|event| {
                Ok(PreparedEvent {
                    kind: event.kind().to_owned(),
                    version: event.current_version(),
                    payload: serde_json::to_value(event)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(self.insert_prepared_batch(prepared)?.event_count)
    }

    /// W4-4: como [`append`], mas **observa a durabilidade** para a UI: chama `on_flush(Saving)`
    /// antes de escrever e `on_flush(Saved)` após o append autoritativo no SQLite. O espelho
    /// JSONL é reconciliável e pode estar temporariamente degradado sem tornar o fato incerto. O shell liga
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
        let committed = self.insert_prepared_batch(vec![PreparedEvent {
            kind: kind.to_owned(),
            version,
            payload,
        }])?;
        committed.seqs.into_iter().next().ok_or_else(|| {
            invalid_recovery_data("insert_raw unitário não recebeu sequência".to_owned())
        })
    }

    fn insert_prepared_batch(
        &mut self,
        prepared: Vec<PreparedEvent>,
    ) -> Result<CommittedBatch, StoreError> {
        if prepared.is_empty() {
            return Ok(CommittedBatch {
                seqs: Vec::new(),
                event_count: self.event_count()?,
            });
        }

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (_, current_last_seq) = event_log_position(&tx)?;
        let max_seq = i64::try_from(current_last_seq)
            .map_err(|_| invalid_recovery_data("sequência atual não cabe no SQLite".to_owned()))?;
        // `seq` é alocado sob `BEGIN IMMEDIATE`, sempre como `MAX(seq) + 1`, e nunca
        // sofre UPDATE/DELETE. Logo o próprio head é a contagem confirmada; `COUNT(*)`
        // aqui faria todo append crescer linearmente com o histórico.
        let event_count = max_seq;
        let batch_len = i64::try_from(prepared.len())
            .map_err(|_| invalid_recovery_data("batch grande demais para o SQLite".to_owned()))?;
        let target_last_seq = max_seq.checked_add(batch_len).ok_or_else(|| {
            invalid_recovery_data("sequência do event log excedeu i64".to_owned())
        })?;
        let target_event_count = event_count
            .checked_add(batch_len)
            .ok_or_else(|| invalid_recovery_data("contagem do event log excedeu i64".to_owned()))?;

        let ts = now_millis();
        let mut records = Vec::with_capacity(prepared.len());
        for (index, event) in prepared.into_iter().enumerate() {
            let offset = i64::try_from(index + 1)
                .map_err(|_| invalid_recovery_data("offset do batch excedeu i64".to_owned()))?;
            let seq = u64::try_from(max_seq + offset)
                .map_err(|_| invalid_recovery_data("sequência inválida no batch".to_owned()))?;
            records.push(EventRecord {
                seq,
                ts,
                kind: event.kind,
                version: event.version,
                payload: event.payload,
            });
        }
        let payloads = records
            .iter()
            .map(|record| serde_json::to_string(&record.payload))
            .collect::<Result<Vec<_>, _>>()?;
        let target_event_count_u64 = u64::try_from(target_event_count)
            .map_err(|_| invalid_recovery_data("contagem alvo inválida".to_owned()))?;
        let target_last_seq_u64 = u64::try_from(target_last_seq)
            .map_err(|_| invalid_recovery_data("sequência alvo inválida".to_owned()))?;

        // O certificado pending é a barreira de write-ahead externa: se não ficar
        // durável, a transação nem começa a inserir eventos.
        write_recovery_state_atomic(
            &self.jsonl_recovery_state_path,
            &JsonlRecoveryState::pending(target_event_count_u64, target_last_seq_u64),
        )?;
        for (record, payload) in records.iter().zip(payloads) {
            tx.execute(
                "INSERT INTO events (seq, ts, kind, version, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    i64::try_from(record.seq).map_err(|_| {
                        invalid_recovery_data("sequência não cabe no SQLite".to_owned())
                    })?,
                    i64::try_from(record.ts).map_err(|_| {
                        invalid_recovery_data("timestamp não cabe no SQLite".to_owned())
                    })?,
                    &record.kind,
                    i64::from(record.version),
                    payload
                ],
            )?;
        }
        tx.commit()?;

        self.next_seq = target_last_seq_u64.saturating_add(1);
        if let Err(error) = self.mirror_committed_records(&records) {
            tracing::error!(
                first_seq = records.first().map(|record| record.seq),
                last_seq = records.last().map(|record| record.seq),
                path = %self.jsonl_path.display(),
                %error,
                "batch confirmado no SQLite; espelho permanece pending e será reconciliado"
            );
        }
        if records
            .iter()
            .any(|record| record.seq.is_multiple_of(SNAPSHOT_EVERY))
        {
            if let Err(error) = self.take_snapshot() {
                tracing::error!(%error, "snapshot automático falhou; replay segue correto");
            }
        }
        Ok(CommittedBatch {
            seqs: records.iter().map(|record| record.seq).collect(),
            event_count: target_event_count_u64,
        })
    }

    /// Reconstrói o espelho canônico quando o arquivo físico diverge do estado confirmado.
    /// Reescrever por temporário + rename evita cauda parcial, duplicata e payload divergente;
    /// o SQLite permanece a única autoridade durante toda a reparação.
    fn reconcile_jsonl_from_db(
        &mut self,
        event_count: u64,
        last_seq: u64,
    ) -> Result<(), StoreError> {
        if event_count == 0
            && !recovery_artifacts_exist(&self.jsonl_path, &self.jsonl_recovery_state_path)?
        {
            return Ok(());
        }
        if jsonl_state_matches(&self.conn, &self.jsonl_path, event_count)?
            && jsonl_certificate_matches_position(
                &self.jsonl_recovery_state_path,
                event_count,
                last_seq,
            )?
        {
            return Ok(());
        }

        // O lock SQLite serializa reconciliação e appends do espelho entre processos.
        let jsonl_path = self.jsonl_path.clone();
        let recovery_state_path = self.jsonl_recovery_state_path.clone();
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (event_count, last_seq) = event_log_position(&tx)?;
        if jsonl_state_matches(&tx, &jsonl_path, event_count)?
            && jsonl_certificate_matches_position(&recovery_state_path, event_count, last_seq)?
        {
            tx.commit()?;
            return Ok(());
        }

        let records = query_all_events(&tx)?;
        write_recovery_state_atomic(
            &recovery_state_path,
            &JsonlRecoveryState::pending(event_count, last_seq),
        )?;
        let synced = rewrite_jsonl_atomic(&jsonl_path, &records)?;
        replace_jsonl_confirmation(&tx, &records, &synced.file_state)?;
        write_complete_recovery_state(&recovery_state_path, event_count, last_seq, &synced.hasher)?;
        tx.commit()?;
        self.jsonl_digest_cache = Some(JsonlDigestCache::from_synced(event_count, &synced));
        Ok(())
    }

    /// Espelha um batch já confirmado no SQLite sob o mesmo lock usado pela reconciliação.
    /// Se outro escritor já reparou toda a cauda, apenas confirma o estado atual. Se o
    /// prefixo observado por esta conexão ainda é o confirmado, faz append incremental;
    /// qualquer dúvida força reescrita canônica a partir da autoridade SQLite.
    fn mirror_committed_records(&mut self, committed: &[EventRecord]) -> Result<(), StoreError> {
        if committed.is_empty() {
            return Ok(());
        }
        let jsonl_path = self.jsonl_path.clone();
        let recovery_state_path = self.jsonl_recovery_state_path.clone();
        let cached_digest = self.jsonl_digest_cache.clone();
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (event_count, last_seq) = event_log_position(&tx)?;
        if jsonl_state_matches(&tx, &jsonl_path, event_count)?
            && jsonl_certificate_matches_position(&recovery_state_path, event_count, last_seq)?
        {
            tx.commit()?;
            return Ok(());
        }

        write_recovery_state_atomic(
            &recovery_state_path,
            &JsonlRecoveryState::pending(event_count, last_seq),
        )?;

        let committed_count = u64::try_from(committed.len())
            .map_err(|_| invalid_recovery_data("batch espelhado excede u64".to_owned()))?;
        let previous_count = event_count.checked_sub(committed_count).ok_or_else(|| {
            invalid_recovery_data("batch espelhado é maior que o log SQLite".to_owned())
        })?;
        let expected_first_seq = previous_count
            .checked_add(1)
            .ok_or_else(|| invalid_recovery_data("sequência do espelho excedeu u64".to_owned()))?;
        let is_current_tail = committed.first().map(|record| record.seq)
            == Some(expected_first_seq)
            && committed.last().map(|record| record.seq) == Some(last_seq)
            && committed.iter().enumerate().all(|(index, record)| {
                record.seq == expected_first_seq.saturating_add(index as u64)
            });
        let prefix_is_confirmed =
            is_current_tail && jsonl_state_matches(&tx, &jsonl_path, previous_count)?;

        let (synced, canonical_records) =
            if prefix_is_confirmed && previous_count == 0 && mirror_is_empty(&jsonl_path)? {
                (
                    append_jsonl_records(&jsonl_path, committed, Sha256::new())?,
                    None,
                )
            } else if prefix_is_confirmed {
                let physical_state = mirror_file_state(&jsonl_path)?;
                if let Some(cache) = cached_digest.as_ref().filter(|cache| {
                    cache.event_count == previous_count
                        && physical_state.as_ref() == Some(&cache.file_state)
                }) {
                    (
                        append_jsonl_records(&jsonl_path, committed, cache.hasher.clone())?,
                        None,
                    )
                } else {
                    let records = query_all_events(&tx)?;
                    let synced = rewrite_jsonl_atomic(&jsonl_path, &records)?;
                    (synced, Some(records))
                }
            } else {
                let records = query_all_events(&tx)?;
                let synced = rewrite_jsonl_atomic(&jsonl_path, &records)?;
                (synced, Some(records))
            };

        if let Some(records) = canonical_records {
            replace_jsonl_confirmation(&tx, &records, &synced.file_state)?;
        } else {
            for record in committed {
                let seq = i64::try_from(record.seq).map_err(|_| {
                    invalid_recovery_data("sequência não cabe no SQLite".to_owned())
                })?;
                tx.execute("INSERT INTO jsonl_mirror (seq) VALUES (?1)", [seq])?;
            }
            upsert_jsonl_state(&tx, &synced.file_state, event_count)?;
        }
        write_complete_recovery_state(&recovery_state_path, event_count, last_seq, &synced.hasher)?;
        tx.commit()?;
        self.jsonl_digest_cache = Some(JsonlDigestCache::from_synced(event_count, &synced));
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
        let (state, from_seq) = match cached {
            Some((seq, st)) => (st, seq),
            None => match self.latest_snapshot()? {
                Some((seq, st)) => (st, seq),
                None => (ProjectedState::default(), 0),
            },
        };
        let projection = replay_projection(&self.conn, state, from_seq, None)?;
        let replayed_total = self
            .replayed_total
            .get()
            .checked_add(projection.replayed)
            .ok_or_else(|| invalid_recovery_data("contador de replay excedeu u64".to_owned()))?;
        self.replayed_total.set(replayed_total);
        *self.proj_cache.borrow_mut() = Some((projection.seq, projection.state.clone()));
        Ok(projection.state)
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
        let cached = self.proj_cache.borrow().clone();
        let projection = {
            // A transação é somente de leitura durante o replay. Assim o head e o
            // estado vêm da mesma visão WAL, mas nenhum escritor fica serializado
            // enquanto o snapshot é persistido em SQLite/arquivo.
            let transaction = self
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let (_, head_seq) = event_log_position(&transaction)?;
            let (state, from_seq) = match cached {
                Some((seq, state)) => (state, seq),
                None => match latest_snapshot_from(&transaction)? {
                    Some((seq, state)) => (state, seq),
                    None => (ProjectedState::default(), 0),
                },
            };
            if from_seq > head_seq {
                return Err(invalid_recovery_data(format!(
                    "checkpoint da projeção está adiante do event log: seq {from_seq}, head {head_seq}"
                )));
            }
            let projection = replay_projection(&transaction, state, from_seq, Some(head_seq))?;
            transaction.commit()?;
            projection
        };

        let replayed_total = self
            .replayed_total
            .get()
            .checked_add(projection.replayed)
            .ok_or_else(|| invalid_recovery_data("contador de replay excedeu u64".to_owned()))?;
        self.replayed_total.set(replayed_total);
        *self.proj_cache.borrow_mut() = Some((projection.seq, projection.state.clone()));

        let seq = projection.seq;
        let seq_sql = i64::try_from(seq)
            .map_err(|_| invalid_recovery_data("seq do snapshot excedeu i64".to_owned()))?;
        let timestamp_sql = i64::try_from(now_millis())
            .map_err(|_| invalid_recovery_data("timestamp do snapshot excedeu i64".to_owned()))?;
        let state_json = serde_json::to_string(&projection.state)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO snapshots (seq, ts, state) VALUES (?1, ?2, ?3)",
            params![seq_sql, timestamp_sql, &state_json],
        )?;
        std::fs::write(
            self.snapshots_dir.join(format!("snap-{seq}.json")),
            &state_json,
        )?;
        Ok(seq)
    }

    fn latest_snapshot(&self) -> Result<Option<(u64, ProjectedState)>, StoreError> {
        latest_snapshot_from(&self.conn)
    }

    /// Nº de eventos no log (queries só-leitura).
    pub fn event_count(&self) -> Result<u64, StoreError> {
        let (event_count, _) = event_log_position(&self.conn)?;
        Ok(event_count)
    }

    /// Registros do log em ordem de `seq` (só-leitura). Permite inspecionar os CAMPOS de um evento
    /// no registro (ex.: asserir `id`/`item`/`by` de um `PlanClaimed`), não só sua presença.
    pub fn events(&self) -> Result<Vec<EventRecord>, StoreError> {
        query_all_events(&self.conn)
    }

    /// Busca pontual pelo `seq`, que é a chave primária do log. Não projeta nem
    /// percorre o histórico; valida o registro antes de devolvê-lo ao chamador.
    pub fn event_at(&self, seq: u64) -> Result<Option<EventRecord>, StoreError> {
        if seq == 0 {
            return Err(invalid_recovery_data(
                "seq zero não identifica um evento".to_owned(),
            ));
        }
        let seq_sql = i64::try_from(seq)
            .map_err(|_| invalid_recovery_data(format!("seq {seq} não cabe no SQLite")))?;
        let raw = self
            .conn
            .query_row(EVENT_BY_SEQ_SQL, [seq_sql], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .optional()?;
        let Some((stored_seq, timestamp, kind, version, payload)) = raw else {
            return Ok(None);
        };
        let stored_seq = u64::try_from(stored_seq)
            .map_err(|_| invalid_recovery_data("evento contém seq negativo".to_owned()))?;
        if stored_seq != seq {
            return Err(invalid_recovery_data(format!(
                "índice retornou seq {stored_seq} para a busca por {seq}"
            )));
        }
        let record = EventRecord {
            seq: stored_seq,
            ts: u64::try_from(timestamp).map_err(|_| {
                invalid_recovery_data(format!("evento seq {seq} contém timestamp negativo"))
            })?,
            kind,
            version: u32::try_from(version).map_err(|_| {
                invalid_recovery_data(format!("evento seq {seq} contém versão fora de u32"))
            })?,
            payload: serde_json::from_str(&payload)?,
        };
        validate_event_record(&record)?;
        Ok(Some(record))
    }

    /// Registros do log com `seq > after_seq`, em ordem de `seq` (só-leitura). É a leitura
    /// INCREMENTAL: o consumidor mantém um cursor do último `seq` aplicado e busca só o delta —
    /// `seq > último` é sempre correto, inclusive para appends de OUTRO processo (mesma garantia
    /// do cache de [`project`]). Tira o `O(N)` do full-`events()` dos consumidores quentes
    /// (ex.: o heartbeat da Fila de Atenção, que roda na thread de UI a até 30Hz).
    pub fn events_since(&self, after_seq: u64) -> Result<Vec<EventRecord>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, kind, version, payload FROM events WHERE seq > ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([after_seq as i64], |r| {
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

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FullReadProbe {
    sqlite_history_scans: u64,
    jsonl_history_reads: u64,
    head_queries: u64,
    head_vm_steps: u64,
}

#[cfg(test)]
thread_local! {
    static FULL_READ_PROBE: std::cell::Cell<FullReadProbe> =
        const { std::cell::Cell::new(FullReadProbe {
            sqlite_history_scans: 0,
            jsonl_history_reads: 0,
            head_queries: 0,
            head_vm_steps: 0,
        }) };
}

#[cfg(test)]
fn reset_full_read_probe() {
    FULL_READ_PROBE.with(|probe| probe.set(FullReadProbe::default()));
}

#[cfg(test)]
fn full_read_probe() -> FullReadProbe {
    FULL_READ_PROBE.with(std::cell::Cell::get)
}

#[cfg(test)]
fn record_sqlite_history_scan() {
    FULL_READ_PROBE.with(|probe| {
        let mut value = probe.get();
        value.sqlite_history_scans += 1;
        probe.set(value);
    });
}

#[cfg(test)]
fn record_jsonl_history_read() {
    FULL_READ_PROBE.with(|probe| {
        let mut value = probe.get();
        value.jsonl_history_reads += 1;
        probe.set(value);
    });
}

#[cfg(test)]
fn record_head_query(vm_steps: i32) {
    FULL_READ_PROBE.with(|probe| {
        let mut value = probe.get();
        value.head_queries += 1;
        value.head_vm_steps += u64::try_from(vm_steps).unwrap_or(0);
        probe.set(value);
    });
}

fn latest_snapshot_from(conn: &Connection) -> Result<Option<(u64, ProjectedState)>, StoreError> {
    let row = conn
        .query_row(
            "SELECT seq, state FROM snapshots ORDER BY seq DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    match row {
        Some((seq, state_json)) => {
            let seq = u64::try_from(seq).map_err(|_| {
                invalid_recovery_data("snapshot contém sequência negativa".to_owned())
            })?;
            let state = serde_json::from_str(&state_json)?;
            Ok(Some((seq, state)))
        }
        None => Ok(None),
    }
}

fn replay_projection(
    conn: &Connection,
    mut state: ProjectedState,
    from_seq: u64,
    through_seq: Option<u64>,
) -> Result<ProjectionAtSeq, StoreError> {
    let from_seq_sql = i64::try_from(from_seq)
        .map_err(|_| invalid_recovery_data("seq inicial do replay excedeu i64".to_owned()))?;
    let mut statement = match through_seq {
        Some(_) => conn.prepare(
            "SELECT seq, kind, version, payload FROM events \
             WHERE seq > ?1 AND seq <= ?2 ORDER BY seq ASC",
        )?,
        None => conn.prepare(
            "SELECT seq, kind, version, payload FROM events WHERE seq > ?1 ORDER BY seq ASC",
        )?,
    };
    let mut rows = match through_seq {
        Some(seq) => {
            let through_seq_sql = i64::try_from(seq)
                .map_err(|_| invalid_recovery_data("seq final do replay excedeu i64".to_owned()))?;
            statement.query(params![from_seq_sql, through_seq_sql])?
        }
        None => statement.query([from_seq_sql])?,
    };

    let mut last_seq = from_seq;
    let mut replayed = 0_u64;
    while let Some(row) = rows.next()? {
        let seq = u64::try_from(row.get::<_, i64>(0)?).map_err(|_| {
            invalid_recovery_data("evento projetado contém seq negativo".to_owned())
        })?;
        let expected_seq = last_seq
            .checked_add(1)
            .ok_or_else(|| invalid_recovery_data("sequência projetada excedeu u64".to_owned()))?;
        if seq != expected_seq {
            return Err(invalid_recovery_data(format!(
                "event log não é contíguo durante o replay: esperava seq {expected_seq}, encontrou {seq}"
            )));
        }
        let kind = row.get::<_, String>(1)?;
        let version = u32::try_from(row.get::<_, i64>(2)?).map_err(|_| {
            invalid_recovery_data(format!(
                "evento seq {seq} contém versão fora do intervalo u32"
            ))
        })?;
        let payload = serde_json::from_str(&row.get::<_, String>(3)?)?;
        let event = decode_validated_domain_event(seq, &kind, version, &payload)?;
        apply(&mut state, &event);
        last_seq = seq;
        replayed = replayed
            .checked_add(1)
            .ok_or_else(|| invalid_recovery_data("replay excedeu u64".to_owned()))?;
    }

    if let Some(expected_head) = through_seq.filter(|expected_head| last_seq != *expected_head) {
        return Err(invalid_recovery_data(format!(
            "replay não alcançou o head da mesma visão SQLite: terminou em {last_seq}, head {expected_head}"
        )));
    }

    Ok(ProjectionAtSeq {
        state,
        seq: last_seq,
        replayed,
    })
}

fn query_all_events(conn: &Connection) -> Result<Vec<EventRecord>, StoreError> {
    #[cfg(test)]
    record_sqlite_history_scan();
    let mut stmt =
        conn.prepare("SELECT seq, ts, kind, version, payload FROM events ORDER BY seq ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (seq, ts, kind, version, payload) = row?;
        records.push(EventRecord {
            seq: seq as u64,
            ts: ts as u64,
            kind,
            version: version as u32,
            payload: serde_json::from_str(&payload)?,
        });
    }
    Ok(records)
}

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

/// Confere somente o contrato estrutural e as duas extremidades do log existente.
/// Preparar a projeção valida as colunas obrigatórias sem ler linhas; buscar primeiro
/// e último `seq` usa a chave primária. A validação semântica fica para `project()`.
fn validate_existing_store_head(conn: &Connection) -> Result<(u64, u64), StoreError> {
    let has_events = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'events'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some();
    if !has_events {
        return Err(invalid_recovery_data(
            "SQLite existente não contém a tabela events".to_owned(),
        ));
    }
    let mut columns_statement = conn.prepare("PRAGMA table_info(events)")?;
    let columns = columns_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let required_columns = ["seq", "ts", "kind", "version", "payload"];
    if required_columns
        .iter()
        .any(|required| !columns.iter().any(|(name, _)| name == required))
        || !columns
            .iter()
            .any(|(name, primary_key)| name == "seq" && *primary_key > 0)
    {
        return Err(invalid_recovery_data(
            "tabela events existente não tem o schema posicional exigido".to_owned(),
        ));
    }
    conn.prepare("SELECT seq, ts, kind, version, payload FROM events WHERE seq > ?1")?;
    event_log_position(conn)
}

fn is_recoverable_store_corruption(error: &StoreError) -> bool {
    match error {
        StoreError::Json(_) => true,
        StoreError::Io(error) => error.kind() == std::io::ErrorKind::InvalidData,
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _)) => matches!(
            error.code,
            rusqlite::ErrorCode::DatabaseCorrupt
                | rusqlite::ErrorCode::NotADatabase
                | rusqlite::ErrorCode::TypeMismatch
        ),
        StoreError::Sqlite(
            rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::Utf8Error(..)
            | rusqlite::Error::InvalidColumnType(..),
        ) => true,
        StoreError::Sqlite(_) => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MirrorFileState {
    len: u64,
    modified_ns: u128,
}

#[derive(Clone)]
struct JsonlDigestCache {
    file_state: MirrorFileState,
    event_count: u64,
    hasher: Sha256,
}

struct SyncedJsonl {
    file_state: MirrorFileState,
    hasher: Sha256,
}

impl JsonlDigestCache {
    fn from_synced(event_count: u64, synced: &SyncedJsonl) -> Self {
        Self {
            file_state: synced.file_state,
            event_count,
            hasher: synced.hasher.clone(),
        }
    }
}

fn validate_indexed_seq_bounds(
    label: &str,
    first_seq: Option<i64>,
    last_seq: Option<i64>,
) -> Result<Option<u64>, StoreError> {
    match (first_seq, last_seq) {
        (None, None) => Ok(None),
        (Some(1), Some(last_seq)) if last_seq >= 1 => {
            let last_seq = u64::try_from(last_seq)
                .map_err(|_| invalid_recovery_data(format!("{label} contém sequência inválida")))?;
            Ok(Some(last_seq))
        }
        bounds => Err(invalid_recovery_data(format!(
            "{label} tem extremidades inválidas: primeiro seq={:?}, último seq={:?}",
            bounds.0, bounds.1
        ))),
    }
}

fn event_log_position(conn: &Connection) -> Result<(u64, u64), StoreError> {
    let mut statement = conn.prepare(
        "SELECT \
         (SELECT seq FROM events ORDER BY seq ASC LIMIT 1), \
         (SELECT seq FROM events ORDER BY seq DESC LIMIT 1)",
    )?;
    let (first_seq, last_seq): (Option<i64>, Option<i64>) =
        statement.query_row([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    #[cfg(test)]
    record_head_query(statement.get_status(rusqlite::StatementStatus::VmStep));
    let last_seq: u64 =
        validate_indexed_seq_bounds("event log SQLite", first_seq, last_seq)?.unwrap_or_default();
    // O escritor mantém `seq` contíguo sob a mesma transação e o log é imutável.
    // Assim o último `seq` também é a contagem, sem `COUNT(*)` linear.
    Ok((last_seq, last_seq))
}

fn mirror_file_state(path: &Path) -> Result<Option<MirrorFileState>, StoreError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    Ok(Some(MirrorFileState {
        len: metadata.len(),
        modified_ns,
    }))
}

/// Valida o marcador lógico E a identidade física observada após `sync_data`. Metadados
/// iguais permitem o fast path sem reler históricos grandes; qualquer alteração externa
/// (truncate, replace, corrupção in-place) muda tamanho ou mtime e força rewrite canônico.
fn jsonl_state_matches(
    conn: &Connection,
    path: &Path,
    expected_count: u64,
) -> Result<bool, StoreError> {
    let (first_marker, last_marker): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT \
         (SELECT seq FROM jsonl_mirror ORDER BY seq ASC LIMIT 1), \
         (SELECT seq FROM jsonl_mirror ORDER BY seq DESC LIMIT 1)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let marker_last = validate_indexed_seq_bounds("jsonl_mirror", first_marker, last_marker)?;
    let markers_match = match (expected_count, marker_last) {
        (0, None) => true,
        (expected_count, Some(last_marker)) => last_marker == expected_count,
        _ => false,
    };
    if !markers_match {
        return Ok(false);
    }
    if expected_count == 0 {
        return Ok(match mirror_file_state(path)? {
            None => true,
            Some(state) => state.len == 0,
        });
    }
    let Some((stored_len, stored_modified, stored_count)) = conn
        .query_row(
            "SELECT file_len, modified_ns, event_count FROM jsonl_state WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(false);
    };
    if stored_count < 0 || stored_count as u64 != expected_count || stored_len < 0 {
        return Ok(false);
    }
    let Some(actual) = mirror_file_state(path)? else {
        return Ok(false);
    };
    Ok(actual.len == stored_len as u64 && actual.modified_ns.to_string() == stored_modified)
}

fn upsert_jsonl_state(
    conn: &Connection,
    state: &MirrorFileState,
    event_count: u64,
) -> Result<(), StoreError> {
    let file_len = i64::try_from(state.len).map_err(|_| {
        invalid_recovery_data(format!(
            "espelho JSONL grande demais para registrar: {} bytes",
            state.len
        ))
    })?;
    let event_count = i64::try_from(event_count)
        .map_err(|_| invalid_recovery_data("contagem do espelho excede i64".to_owned()))?;
    conn.execute(
        "INSERT INTO jsonl_state (singleton, file_len, modified_ns, event_count) \
         VALUES (1, ?1, ?2, ?3) \
         ON CONFLICT(singleton) DO UPDATE SET \
         file_len = excluded.file_len, modified_ns = excluded.modified_ns, \
         event_count = excluded.event_count",
        params![file_len, state.modified_ns.to_string(), event_count],
    )?;
    Ok(())
}

fn replace_jsonl_confirmation(
    conn: &Connection,
    records: &[EventRecord],
    state: &MirrorFileState,
) -> Result<(), StoreError> {
    conn.execute("DELETE FROM jsonl_mirror", [])?;
    for record in records {
        let seq = i64::try_from(record.seq)
            .map_err(|_| invalid_recovery_data(format!("seq {} não cabe no SQLite", record.seq)))?;
        conn.execute("INSERT INTO jsonl_mirror (seq) VALUES (?1)", [seq])?;
    }
    let event_count = u64::try_from(records.len())
        .map_err(|_| invalid_recovery_data("event log excede u64".to_owned()))?;
    upsert_jsonl_state(conn, state, event_count)
}

/// Anexa o batch em uma única abertura O_APPEND. Cada linha é enviada por um único
/// `write_all`, e o arquivo inteiro é sincronizado antes de qualquer certificado `complete`.
fn append_jsonl_records(
    path: &Path,
    records: &[EventRecord],
    mut hasher: Sha256,
) -> Result<SyncedJsonl, StoreError> {
    let existed = path.try_exists()?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for record in records {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        file.write_all(&line)?;
        hasher.update(&line);
    }
    file.sync_data()?;
    drop(file);
    let parent = path.parent().ok_or_else(|| {
        invalid_recovery_data(format!("{} não tem diretório-pai", path.display()))
    })?;
    #[cfg(unix)]
    if !existed {
        std::fs::File::open(parent)?.sync_all()?;
    }
    let file_state = mirror_file_state(path)?.ok_or_else(|| {
        invalid_recovery_data(format!(
            "{} deixou de ser um arquivo após o append",
            path.display()
        ))
    })?;
    Ok(SyncedJsonl { file_state, hasher })
}

/// Reescreve o espelho completo sem expor um arquivo intermediário a leitores JSONL-only.
fn rewrite_jsonl_atomic(path: &Path, records: &[EventRecord]) -> Result<SyncedJsonl, StoreError> {
    let parent = path.parent().ok_or_else(|| {
        invalid_recovery_data(format!("{} não tem diretório-pai", path.display()))
    })?;
    let tmp = parent.join(format!(
        ".log.jsonl.reconcile-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let mut hasher = Sha256::new();
    let write_result = (|| -> Result<(), StoreError> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        for record in records {
            let mut line = serde_json::to_vec(record)?;
            line.push(b'\n');
            file.write_all(&line)?;
            hasher.update(&line);
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                path = %tmp.display(),
                %error,
                "não consegui remover temporário de reconciliação"
            ),
        }
    }
    write_result?;
    let file_state = mirror_file_state(path)?.ok_or_else(|| {
        invalid_recovery_data(format!(
            "{} não é arquivo depois da reconciliação",
            path.display()
        ))
    })?;
    Ok(SyncedJsonl { file_state, hasher })
}

fn recovery_artifacts_exist(
    jsonl_path: &Path,
    recovery_state_path: &Path,
) -> Result<bool, StoreError> {
    Ok(jsonl_path.try_exists()? || recovery_state_path.try_exists()?)
}

fn mirror_is_empty(path: &Path) -> Result<bool, StoreError> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && metadata.len() == 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn write_recovery_state_atomic(path: &Path, state: &JsonlRecoveryState) -> Result<(), StoreError> {
    let parent = path.parent().ok_or_else(|| {
        invalid_recovery_data(format!("{} não tem diretório-pai", path.display()))
    })?;
    let tmp = parent.join(format!(
        ".log.recovery-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let bytes = serde_json::to_vec(state)?;
    let write_result = (|| -> Result<(), StoreError> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                path = %tmp.display(),
                %error,
                "não consegui remover certificado temporário"
            ),
        }
    }
    write_result
}

fn write_complete_recovery_state(
    path: &Path,
    event_count: u64,
    last_seq: u64,
    hasher: &Sha256,
) -> Result<(), StoreError> {
    write_recovery_state_atomic(
        path,
        &JsonlRecoveryState::complete(event_count, last_seq, sha256_hex(hasher)),
    )
}

fn read_recovery_state(path: &Path) -> Result<Option<JsonlRecoveryState>, StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn sha256_hex(hasher: &Sha256) -> String {
    format!("{:x}", hasher.clone().finalize())
}

fn hash_jsonl_file(path: &Path) -> Result<SyncedJsonl, StoreError> {
    #[cfg(test)]
    record_jsonl_history_read();
    let before = mirror_file_state(path)?.ok_or_else(|| {
        invalid_recovery_data(format!("{} não é um arquivo JSONL", path.display()))
    })?;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let after = mirror_file_state(path)?.ok_or_else(|| {
        invalid_recovery_data(format!("{} desapareceu durante o hash", path.display()))
    })?;
    if before != after {
        return Err(invalid_recovery_data(format!(
            "{} mudou durante o cálculo SHA-256",
            path.display()
        )));
    }
    Ok(SyncedJsonl {
        file_state: after,
        hasher,
    })
}

/// Confere apenas o certificado pequeno que acompanha a posição já confirmada no
/// SQLite. O SHA continua sendo obrigatório em [`recoverable_jsonl_records`], mas
/// calculá-lo em toda reabertura tornaria o caminho saudável O(tamanho do histórico).
fn jsonl_certificate_matches_position(
    recovery_state_path: &Path,
    expected_count: u64,
    expected_last_seq: u64,
) -> Result<bool, StoreError> {
    let state = match read_recovery_state(recovery_state_path) {
        Ok(Some(state)) if state.version == JSONL_RECOVERY_STATE_VERSION => state,
        Ok(Some(_)) | Ok(None) => return Ok(false),
        Err(error) => {
            tracing::warn!(
                path = %recovery_state_path.display(),
                %error,
                "certificado JSONL ilegível; SQLite saudável fará reconciliação"
            );
            return Ok(false);
        }
    };
    let JsonlRecoveryStatus::Complete {
        event_count,
        last_seq,
        digest_sha256,
    } = state.status
    else {
        return Ok(false);
    };
    Ok(event_count == expected_count
        && last_seq == expected_last_seq
        && digest_sha256.len() == 64
        && digest_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn load_canonical_jsonl_records(path: &Path) -> Result<Vec<EventRecord>, StoreError> {
    #[cfg(test)]
    record_jsonl_history_read();
    let bytes = std::fs::read(path)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err(invalid_recovery_data(format!(
            "{} termina em linha JSONL parcial",
            path.display()
        )));
    }
    let content = if bytes.is_empty() {
        bytes.as_slice()
    } else {
        &bytes[..bytes.len() - 1]
    };
    if content.is_empty() {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        return Err(invalid_recovery_data(format!(
            "{} contém uma linha JSONL vazia",
            path.display()
        )));
    }
    let mut records = Vec::new();
    for (index, raw_line) in content.split(|byte| *byte == b'\n').enumerate() {
        let line_number = index + 1;
        if raw_line.is_empty() || raw_line.ends_with(b"\r") {
            return Err(invalid_recovery_data(format!(
                "{} contém linha não canônica em {line_number}",
                path.display()
            )));
        }
        let record: EventRecord = serde_json::from_slice(raw_line).map_err(|error| {
            invalid_recovery_data(format!(
                "{} contém JSON inválido na linha {line_number}: {error}",
                path.display()
            ))
        })?;
        if serde_json::to_vec(&record)? != raw_line {
            return Err(invalid_recovery_data(format!(
                "{} contém JSON não canônico na linha {line_number}",
                path.display()
            )));
        }
        let expected_seq = u64::try_from(line_number)
            .map_err(|_| invalid_recovery_data("JSONL excede u64".to_owned()))?;
        if record.seq != expected_seq {
            return Err(invalid_recovery_data(format!(
                "{} não é contíguo: esperava seq {expected_seq}, encontrou {}",
                path.display(),
                record.seq
            )));
        }
        records.push(record);
    }
    Ok(records)
}

fn recoverable_jsonl_records(
    jsonl_path: &Path,
    recovery_state_path: &Path,
) -> Result<Vec<EventRecord>, StoreError> {
    let state = read_recovery_state(recovery_state_path)?.ok_or_else(|| {
        invalid_recovery_data(format!(
            "{} não tem certificado de recuperação",
            jsonl_path.display()
        ))
    })?;
    if state.version != JSONL_RECOVERY_STATE_VERSION {
        return Err(invalid_recovery_data(format!(
            "versão incompatível do certificado JSONL: {}",
            state.version
        )));
    }
    let JsonlRecoveryStatus::Complete {
        event_count,
        last_seq,
        digest_sha256,
    } = state.status
    else {
        return Err(invalid_recovery_data(format!(
            "{} está pending; um prefixo não prova recuperação completa",
            recovery_state_path.display()
        )));
    };
    if digest_sha256.len() != 64 || !digest_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_recovery_data(
            "certificado JSONL não contém SHA-256 válido".to_owned(),
        ));
    }
    let records = load_canonical_jsonl_records(jsonl_path)?;
    let actual_count = u64::try_from(records.len())
        .map_err(|_| invalid_recovery_data("JSONL excede u64".to_owned()))?;
    let actual_last_seq = records.last().map_or(0, |record| record.seq);
    if actual_count != event_count || actual_last_seq != last_seq {
        return Err(invalid_recovery_data(format!(
            "certificado promete count={event_count}/last_seq={last_seq}, mas o JSONL tem count={actual_count}/last_seq={actual_last_seq}"
        )));
    }
    let synced = hash_jsonl_file(jsonl_path)?;
    if sha256_hex(&synced.hasher) != digest_sha256.to_ascii_lowercase() {
        return Err(invalid_recovery_data(
            "SHA-256 do JSONL diverge do certificado complete".to_owned(),
        ));
    }
    for record in &records {
        validate_event_record(record)?;
    }
    Ok(records)
}

fn validate_event_record(record: &EventRecord) -> Result<(), StoreError> {
    i64::try_from(record.seq)
        .map_err(|_| invalid_recovery_data(format!("seq {} não cabe no SQLite", record.seq)))?;
    i64::try_from(record.ts).map_err(|_| {
        invalid_recovery_data(format!("timestamp {} não cabe no SQLite", record.ts))
    })?;
    decode_validated_domain_event(record.seq, &record.kind, record.version, &record.payload)?;
    Ok(())
}

fn decode_validated_domain_event(
    seq: u64,
    kind: &str,
    version: u32,
    payload: &serde_json::Value,
) -> Result<DomainEvent, StoreError> {
    if seq == 0 {
        return Err(invalid_recovery_data(
            "evento não pode usar seq zero".to_owned(),
        ));
    }
    if version == 0 {
        return Err(invalid_recovery_data(format!(
            "evento seq {} tem versão zero",
            seq
        )));
    }
    let event = DomainEvent::from_record(kind, version, payload.clone()).map_err(|error| {
        invalid_recovery_data(format!(
            "evento seq {} não é um DomainEvent válido: {error}",
            seq
        ))
    })?;
    if event.kind() != kind {
        return Err(invalid_recovery_data(format!(
            "evento seq {} declara kind {}, mas o payload é {}",
            seq,
            kind,
            event.kind()
        )));
    }
    if version > event.current_version() {
        return Err(invalid_recovery_data(format!(
            "evento seq {} usa versão futura {} para {} (máxima conhecida {})",
            seq,
            version,
            kind,
            event.current_version()
        )));
    }
    Ok(event)
}

fn invalid_recovery_data(message: String) -> StoreError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

/// Sonda estrutural O(1) usada somente DEPOIS que o fast open falhou. Não executa
/// `integrity_check` nem varre eventos: corrupção interna é observada por `project()`.
fn db_is_corrupt(path: &Path) -> bool {
    let Ok(conn) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return true;
    };
    validate_existing_store_head(&conn).is_err()
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
        match std::fs::remove_file(PathBuf::from(side)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(corrupt)
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::io::BufRead as _;
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

    #[test]
    fn concurrent_writer_snapshot_labels_the_exact_projected_head() {
        let tmp = TempDir::new("snapshot-concurrent-writer");
        let sender = Uuid::now_v7();
        let hot = {
            let mut first = EventStore::open(tmp.path()).expect("abrir conexão A");
            first
                .append(&DomainEvent::BusMessageSent {
                    id: "msg-a".into(),
                    from: sender,
                    to: "node:b".into(),
                })
                .expect("A grava seq 1");

            let mut second = EventStore::open(tmp.path()).expect("abrir conexão B");
            second
                .append(&DomainEvent::BusMessageSent {
                    id: "msg-b".into(),
                    from: sender,
                    to: "node:c".into(),
                })
                .expect("B grava seq 2");

            let snapshot_seq = first.take_snapshot().expect("A materializa snapshot");
            assert_eq!(
                snapshot_seq, 2,
                "o rótulo deve vir da mesma visão SQLite que projetou seq 2"
            );
            let hot = first.project().expect("estado quente após snapshot");
            assert_eq!(hot.bus_messages, 2, "cada mensagem incrementa a projeção");
            assert_eq!(
                first
                    .latest_snapshot()
                    .expect("ler snapshot")
                    .map(|(seq, _)| seq),
                Some(2),
                "a tabela precisa persistir o head exato projetado"
            );
            hot
        };

        let reopened = EventStore::open(tmp.path()).expect("reabrir store");
        let restored = reopened.project().expect("projetar após reabertura");
        assert_eq!(
            restored, hot,
            "seq 2 não pode ser reaplicado sobre um snapshot que já o contém"
        );
        assert_eq!(
            restored.bus_messages, 2,
            "evento não idempotente não pode ser contado duas vezes"
        );
        assert_eq!(
            reopened.replayed_events_total(),
            0,
            "snapshot em seq 2 cobre integralmente o log reaberto"
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
        let checkpoint_batch: Vec<_> = (0..SNAPSHOT_EVERY)
            .map(|i| DomainEvent::TokenUsageReported {
                node: format!("nó-{}", i % 3),
                tokens: i,
            })
            .collect();
        store
            .append_batch(&checkpoint_batch)
            .expect("batch até o checkpoint");
        let tail: Vec<_> = (SNAPSHOT_EVERY..SNAPSHOT_EVERY + 10)
            .map(|i| DomainEvent::TokenUsageReported {
                node: format!("nó-{}", i % 3),
                tokens: i,
            })
            .collect();
        store.append_batch(&tail).expect("batch pós-checkpoint");
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

    /// `events_since(cursor)` devolve SÓ o delta `seq > cursor`, em ordem, e concatenar os deltas
    /// reconstitui `events()` — a leitura incremental que substitui o full-replay na thread de UI.
    #[test]
    fn events_since_returns_only_delta_in_order() {
        let tmp = TempDir::new("events-since");
        let mut store = EventStore::open(tmp.path()).expect("abrir");
        for i in 0..6u64 {
            store
                .append(&DomainEvent::TokenUsageReported {
                    node: format!("n{i}"),
                    tokens: i,
                })
                .expect("append");
        }
        let all = store.events().expect("events");
        assert_eq!(all.len(), 6);

        // delta a partir do 3º seq → só os de seq maior, em ordem ASC.
        let cursor = all[2].seq;
        let delta = store.events_since(cursor).expect("events_since");
        assert_eq!(delta.len(), 3, "três eventos após o cursor");
        assert!(delta.iter().all(|r| r.seq > cursor), "todos seq > cursor");
        assert!(delta.windows(2).all(|w| w[0].seq < w[1].seq), "ordem ASC");

        // cursor no topo → vazio (ocioso: nada novo, custo desprezível).
        assert!(store.events_since(all[5].seq).expect("vazio").is_empty());
        // cursor 0 → equivale ao log inteiro.
        assert_eq!(store.events_since(0).expect("desde 0").len(), all.len());
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

    /// LARGADA F4-0: os 5 eventos novos do substrato de canais/credenciais (a) serializam com a
    /// tag serde `event` IGUAL ao `kind()` (contrato `from_record`/projeções) e (b) round-trippam
    /// idêntico (replay reconstrói byte-a-byte). É o gate da largada — o contrato congelado que as
    /// 7 frentes consomem sem re-editar `events.rs`.
    #[test]
    fn f4_0_substrate_events_roundtrip_and_kind_matches_serde_tag() {
        let evs = vec![
            DomainEvent::CredentialStored {
                channel: "whatsapp".into(),
                scope: "channel:whatsapp".into(),
                key_ref: "session".into(),
            },
            DomainEvent::CredentialRevoked {
                channel: "whatsapp".into(),
                key_ref: "session".into(),
            },
            DomainEvent::ChannelRegistered {
                channel: "whatsapp".into(),
                manifest_ref: "channels/whatsapp/manifest.toml".into(),
                trust_tier: "core".into(),
                install_ref: "v1.2.3".into(),
            },
            DomainEvent::ToolScopeDeclared {
                project: "loja".into(),
                channel: "whatsapp".into(),
                scope: "grupo:Vendas".into(),
            },
            DomainEvent::ToolScopeRevoked {
                project: "loja".into(),
                channel: "whatsapp".into(),
                scope: "grupo:Vendas".into(),
            },
        ];
        for ev in evs {
            let val = serde_json::to_value(&ev).expect("serialize");
            assert_eq!(
                val.get("event").and_then(|v| v.as_str()),
                Some(ev.kind()),
                "a tag serde `event` deve casar `kind()` para {}",
                ev.kind()
            );
            let back: DomainEvent = serde_json::from_value(val).expect("deserialize");
            assert_eq!(back, ev, "round-trip idêntico para {}", ev.kind());
            // META: não afeta a projeção do canvas (apply é no-op para estes).
            let mut st = ProjectedState::default();
            apply(&mut st, &ev);
            assert_eq!(
                st,
                ProjectedState::default(),
                "{} é META (no-op no apply)",
                ev.kind()
            );
        }
    }

    /// LARGADA F4-WA: os eventos do webhook ativo (2 estendidos + 2 novos) (a) serializam com a tag
    /// serde `event` IGUAL ao `kind()`, (b) round-trippam idêntico, (c) são META (no-op no apply). É o
    /// contrato congelado que as 6 frentes consomem sem re-editar `events.rs`.
    #[test]
    fn f4_wa_events_roundtrip_and_kind_matches_serde_tag() {
        let evs = vec![
            DomainEvent::WebhookConfigured {
                hook_id: "h7x".into(),
                target_ref: "@Vendas".into(),
                target_node: "@Vendas".into(),
                instruction: "registre o pedido na planilha e me avise".into(),
                autonomy_level: "notify_before".into(),
                secret_ref: String::new(),
            },
            DomainEvent::WebhookReceived {
                hook_id: "h7x".into(),
                ts: 1_718_600_000_000,
                target_ref: "@Vendas".into(),
                method: "POST".into(),
                payload_size: 482,
                payload_sha256: "9f86d0818884".into(),
            },
            DomainEvent::WebhookDispatched {
                webhook_id: "h7x".into(),
                target_node: "@Vendas".into(),
                method: "POST".into(),
                delivery_id: "msg_abc".into(),
                payload_sha256: "9f86d0818884".into(),
                payload_size: 482,
            },
            DomainEvent::WebhookActionGated {
                webhook_id: "h7x".into(),
                target_node: "@Vendas".into(),
                action_class: "gated-hard-external".into(),
                effective_level: "notify_before".into(),
            },
        ];
        for ev in evs {
            let val = serde_json::to_value(&ev).expect("serialize");
            assert_eq!(
                val.get("event").and_then(|v| v.as_str()),
                Some(ev.kind()),
                "a tag serde `event` deve casar `kind()` para {}",
                ev.kind()
            );
            let back: DomainEvent = serde_json::from_value(val).expect("deserialize");
            assert_eq!(back, ev, "round-trip idêntico para {}", ev.kind());
            let mut st = ProjectedState::default();
            apply(&mut st, &ev);
            assert_eq!(
                st,
                ProjectedState::default(),
                "{} é META (no-op no apply)",
                ev.kind()
            );
        }
    }

    /// LARGADA F4-1: os 4 eventos da vida do canal WhatsApp/Waha (conectar/ler/enviar/desconectar)
    /// (a) serializam com a tag serde `event` IGUAL ao `kind()`, (b) round-trippam idêntico,
    /// (c) são META (no-op no apply). É o contrato congelado que as frentes da onda F4-1 consomem
    /// sem re-editar `events.rs`.
    #[test]
    fn f4_1_channel_events_roundtrip_and_kind_matches_serde_tag() {
        let evs = vec![
            DomainEvent::ChannelConnected {
                channel: "whatsapp".into(),
                session_ref: "session".into(),
                scope: "waha:default".into(),
            },
            DomainEvent::ChannelDisconnected {
                channel: "whatsapp".into(),
                session_ref: "session".into(),
            },
            DomainEvent::ChannelMessageRead {
                channel: "whatsapp".into(),
                conversation_ref: "grupo:Vendas".into(),
                count: 12,
                read_by_node: "@Dev".into(),
            },
            DomainEvent::ChannelMessageSent {
                channel: "whatsapp".into(),
                conversation_ref: "grupo:Vendas".into(),
                broker_exec_ref: "exec_abc".into(),
                approved_by: "@Fundador".into(),
            },
        ];
        for ev in evs {
            let val = serde_json::to_value(&ev).expect("serialize");
            assert_eq!(
                val.get("event").and_then(|v| v.as_str()),
                Some(ev.kind()),
                "a tag serde `event` deve casar `kind()` para {}",
                ev.kind()
            );
            let back: DomainEvent = serde_json::from_value(val).expect("deserialize");
            assert_eq!(back, ev, "round-trip idêntico para {}", ev.kind());
            // META: a vida do canal não afeta a projeção do canvas (apply é no-op).
            let mut st = ProjectedState::default();
            apply(&mut st, &ev);
            assert_eq!(
                st,
                ProjectedState::default(),
                "{} é META (no-op no apply)",
                ev.kind()
            );
        }
    }

    /// SEGURANÇA F4-1 (épico 41 §IV): conteúdo de mensagem e token de sessão NUNCA aparecem no
    /// payload serializado dos eventos de canal — só refs/metadados. Prova por varredura do JSON:
    /// nenhum texto de conversa, nenhum segredo. (O texto/segredo entram via `execute(&secret)` do
    /// broker e do transporte, jamais no log.)
    #[test]
    fn f4_1_channel_events_carry_no_content_or_secret() {
        let segredo = "wa-session-token-SUPERSECRETO";
        let texto = "oi, qual o preço do bolo de cenoura?";
        let evs = vec![
            DomainEvent::ChannelConnected {
                channel: "whatsapp".into(),
                session_ref: "session".into(),
                scope: "waha:default".into(),
            },
            DomainEvent::ChannelMessageRead {
                channel: "whatsapp".into(),
                conversation_ref: "grupo:Vendas".into(),
                count: 3,
                read_by_node: "@Dev".into(),
            },
            DomainEvent::ChannelMessageSent {
                channel: "whatsapp".into(),
                conversation_ref: "grupo:Vendas".into(),
                broker_exec_ref: "exec_abc".into(),
                approved_by: "@Fundador".into(),
            },
        ];
        for ev in evs {
            let s = serde_json::to_string(&ev).expect("serialize");
            assert!(
                !s.contains(segredo),
                "evento {} jamais carrega o token de sessão",
                ev.kind()
            );
            assert!(
                !s.contains(texto),
                "evento {} jamais carrega o conteúdo da mensagem",
                ev.kind()
            );
        }
    }

    /// ADITIVIDADE F4-WA (ADR 0001 §2): um `WebhookConfigured` no payload ANTIGO `{hook_id,target_ref}`
    /// (sem os campos da F4-WA) deserializa sem erro e os defaults entram — `autonomy_level` degrada para
    /// o nível CONSERVADOR `notify_before` (replay NUNCA fabrica autonomia); os demais para vazio. Prova
    /// que replay de log pré-F4-WA não quebra e não escala privilégio.
    #[test]
    fn f4_wa_webhook_configured_old_payload_degrades_conservative() {
        let old = serde_json::json!({
            "event": "WebhookConfigured",
            "hook_id": "legacy",
            "target_ref": "@Time"
        });
        let ev: DomainEvent = serde_json::from_value(old).expect("deserialize payload antigo");
        match ev {
            DomainEvent::WebhookConfigured {
                hook_id,
                target_ref,
                target_node,
                instruction,
                autonomy_level,
                secret_ref,
            } => {
                assert_eq!(hook_id, "legacy");
                assert_eq!(target_ref, "@Time");
                assert_eq!(target_node, "", "campo aditivo ausente → vazio");
                assert_eq!(
                    instruction, "",
                    "sem instrução no legado → sem autoridade de ação"
                );
                assert_eq!(
                    autonomy_level, "notify_before",
                    "default CONSERVADOR — replay antigo nunca fabrica autonomia"
                );
                assert_eq!(secret_ref, "");
            }
            other => panic!("esperava WebhookConfigured, veio {}", other.kind()),
        }
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

    /// Regressão do fast path: reabrir um store já certificado não pode ler uma linha
    /// sequer do histórico SQLite nem um byte do JSONL completo. O contador é local à
    /// thread, portanto a prova não depende de timing nem da carga da máquina.
    #[test]
    #[serial]
    fn healthy_reopen_performs_zero_full_history_reads() {
        let tmp = TempDir::new("healthy-zero-full-read");
        let events = (0..2_048)
            .map(|index| DomainEvent::NoteCreated {
                name: format!("nota-{index}.md"),
            })
            .collect::<Vec<_>>();
        {
            let mut store = EventStore::open(tmp.path()).expect("criar store");
            store.append_batch(&events).expect("popular histórico");
        }

        reset_full_read_probe();
        let mut ui = RecordingUi::default();
        let reopened = EventStore::open_or_recover(tmp.path(), &mut ui)
            .expect("reabrir resiliente pelo fast path");
        assert_eq!(reopened.event_count().expect("head"), 2_048);
        let probe = full_read_probe();
        assert_eq!(probe.sqlite_history_scans, 0, "open varreu events");
        assert_eq!(probe.jsonl_history_reads, 0, "open releu/hashou log.jsonl");
        assert_eq!(
            probe.head_queries, 2,
            "open + event_count consultam só o head"
        );
        assert!(ui.events.is_empty(), "fast path não anuncia recuperação");
    }

    /// Benchmark determinístico por trabalho SQLite, não por cronômetro: 16k eventos
    /// exigem exatamente os mesmos VM steps e zero full reads que 32 eventos. Tempos são
    /// impressos apenas como diagnóstico; a asserção estável mede o algoritmo executado.
    #[test]
    #[serial]
    fn benchmark_healthy_reopen_work_does_not_grow_with_history() {
        fn populated_store(tag: &str, event_count: usize) -> TempDir {
            let tmp = TempDir::new(tag);
            let events = (0..event_count)
                .map(|index| DomainEvent::NoteCreated {
                    name: format!("bench-{index}.md"),
                })
                .collect::<Vec<_>>();
            let mut store = EventStore::open(tmp.path()).expect("criar benchmark store");
            store
                .append_batch(&events)
                .expect("popular benchmark store");
            drop(store);
            tmp
        }

        fn measured_reopen(dir: &Path) -> (FullReadProbe, std::time::Duration) {
            reset_full_read_probe();
            let started = std::time::Instant::now();
            drop(EventStore::open(dir).expect("reabrir benchmark store"));
            (full_read_probe(), started.elapsed())
        }

        let small = populated_store("open-bench-small", 32);
        let large = populated_store("open-bench-large", 16_384);
        let (small_probe, small_elapsed) = measured_reopen(small.path());
        let (large_probe, large_elapsed) = measured_reopen(large.path());

        eprintln!(
            "healthy reopen: 32={small_elapsed:?}, 16384={large_elapsed:?}, work={large_probe:?}"
        );
        assert_eq!(small_probe.sqlite_history_scans, 0);
        assert_eq!(small_probe.jsonl_history_reads, 0);
        assert_eq!(large_probe.sqlite_history_scans, 0);
        assert_eq!(large_probe.jsonl_history_reads, 0);
        assert_eq!(small_probe.head_queries, 1);
        assert_eq!(large_probe.head_queries, 1);
        assert_eq!(
            large_probe.head_vm_steps, small_probe.head_vm_steps,
            "custo do head SQLite não pode depender do número de eventos"
        );
    }

    /// O open O(1) não lê payloads. Se a corrupção só aparece no replay, o helper de
    /// produção detecta o erro semântico, valida integralmente o JSONL e recupera.
    #[test]
    #[serial]
    fn projected_open_recovers_semantically_corrupt_sqlite_payload() {
        let tmp = TempDir::new("semantic-db-recovery");
        let expected = {
            let mut store = EventStore::open(tmp.path()).expect("criar store");
            seed(&mut store);
            store.project().expect("projeção original")
        };
        let conn = Connection::open(tmp.path().join("lina.db")).expect("abrir SQLite cru");
        conn.execute("UPDATE events SET payload = 'não-é-json' WHERE seq = 2", [])
            .expect("corromper payload SQLite");
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .expect("checkpoint da corrupção");
        drop(conn);

        let opened = EventStore::open(tmp.path()).expect("head estrutural segue válido");
        assert!(
            opened.project().is_err(),
            "a corrupção interna só deve aparecer no replay"
        );
        drop(opened);

        reset_full_read_probe();
        let mut ui = RecordingUi::default();
        let (_store, recovered) = EventStore::open_projected_or_recover(tmp.path(), &mut ui)
            .expect("recuperar no replay");
        assert_eq!(recovered, expected);
        assert!(matches!(
            ui.events.as_slice(),
            [HostEvent::Recovering, HostEvent::Recovered]
        ));
        let probe = full_read_probe();
        assert!(
            probe.jsonl_history_reads >= 2,
            "rebuild precisa parsear e verificar SHA do JSONL inteiro"
        );
        assert!(std::fs::read_dir(tmp.path())
            .expect("listar preservado")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")));
    }

    #[test]
    #[serial]
    fn replay_rejects_kind_mismatch_and_future_version_before_apply() {
        for corruption in ["kind_mismatch", "future_version"] {
            for certified in [true, false] {
                let tmp = TempDir::new(&format!("semantic-{corruption}-{certified}"));
                {
                    let mut store = EventStore::open(tmp.path()).expect("criar store");
                    store
                        .append(&DomainEvent::WorkspaceCreated {
                            name: "Estado íntegro".into(),
                            focus_preset: String::new(),
                        })
                        .expect("append íntegro");
                }

                let conn = Connection::open(tmp.path().join("lina.db")).expect("abrir SQLite cru");
                match corruption {
                    "kind_mismatch" => {
                        conn.execute("UPDATE events SET kind = 'NodeAdded' WHERE seq = 1", [])
                            .expect("divergir kind do payload");
                    }
                    "future_version" => {
                        conn.execute("UPDATE events SET version = 99 WHERE seq = 1", [])
                            .expect("injetar versão futura");
                    }
                    _ => unreachable!("caso de teste fechado"),
                }
                conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                    .expect("checkpoint da corrupção");
                drop(conn);

                let opened = EventStore::open(tmp.path()).expect("head estrutural continua válido");
                if !certified {
                    std::fs::remove_file(tmp.path().join(JSONL_RECOVERY_STATE_FILE))
                        .expect("remover certificado após o fast open");
                }
                assert!(
                    opened.project().is_err(),
                    "{corruption} não pode alcançar apply"
                );

                let mut ui = RecordingUi::default();
                let result = opened.project_or_recover(&mut ui);
                if certified {
                    let (_, recovered) = result.expect("recuperar do JSONL certificado");
                    assert_eq!(recovered.workspace_name.as_deref(), Some("Estado íntegro"));
                    assert!(matches!(
                        ui.events.as_slice(),
                        [HostEvent::Recovering, HostEvent::Recovered]
                    ));
                } else {
                    assert!(
                        result.is_err(),
                        "{corruption} sem certificado deve permanecer erro"
                    );
                    assert!(
                        ui.events.is_empty(),
                        "sem fonte recuperável não anuncia sucesso"
                    );
                }
            }
        }
    }

    /// W4-4 (T6): `WorkspaceFocusSet` projeta `focused_workspace` e sobrevive ao replay.
    #[test]
    #[serial]
    fn workspace_focus_set_projects_and_replays() {
        let tmp = TempDir::new("focus");
        let focus = DomainEvent::WorkspaceFocusSet {
            workspace: "App X".into(),
            focus_id: Some("focus-001".into()),
        };
        {
            let mut store = EventStore::open(tmp.path()).expect("open");
            store
                .append(&DomainEvent::WorkspaceCreated {
                    name: "App X".into(),
                    focus_preset: String::new(),
                })
                .unwrap();
            let encoded = serde_json::to_value(&focus).expect("serializar foco correlacionado");
            assert_eq!(encoded["focus_id"], "focus-001");
            assert_eq!(
                serde_json::from_value::<DomainEvent>(encoded).expect("roundtrip do foco"),
                focus
            );
            store.append(&focus).unwrap();
            let state = store.project().expect("project");
            assert_eq!(state.focused_workspace.as_deref(), Some("App X"));

            let record = store
                .events()
                .expect("ler registros")
                .into_iter()
                .find(|record| record.kind == "WorkspaceFocusSet")
                .expect("registro de foco");
            assert_eq!(
                record.payload["focus_id"], "focus-001",
                "o registro durável expõe a correlação para recuperação"
            );
            assert_eq!(
                DomainEvent::from_record(&record.kind, record.version, record.payload)
                    .expect("reconstruir evento pelo registro"),
                focus
            );
        }

        let reopened = EventStore::open(tmp.path()).expect("reabrir");
        assert_eq!(
            reopened
                .project()
                .expect("replay do foco")
                .focused_workspace
                .as_deref(),
            Some("App X")
        );
    }

    #[test]
    fn workspace_focus_set_legacy_json_defaults_missing_focus_id() {
        let legacy = serde_json::json!({
            "event": "WorkspaceFocusSet",
            "workspace": "Legado"
        });
        let decoded: DomainEvent =
            serde_json::from_value(legacy.clone()).expect("desserializar foco legado");
        assert!(matches!(
            &decoded,
            DomainEvent::WorkspaceFocusSet {
                workspace,
                focus_id: None,
            } if workspace == "Legado"
        ));
        assert!(
            serde_json::to_value(&decoded)
                .expect("serializar foco legado")
                .get("focus_id")
                .is_none(),
            "None preserva o formato anterior sem campo artificial"
        );

        let tmp = TempDir::new("focus-legacy");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .insert_raw("WorkspaceFocusSet", 1, legacy)
            .expect("persistir payload legado");
        assert_eq!(
            store
                .project()
                .expect("projetar foco legado")
                .focused_workspace
                .as_deref(),
            Some("Legado")
        );
    }

    #[test]
    fn event_at_uses_primary_key_and_preserves_focus_correlation() {
        let tmp = TempDir::new("event-at");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let first_seq = store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Busca pontual".into(),
                focus_preset: String::new(),
            })
            .expect("gravar primeiro evento");
        let focus_seq = store
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: "Busca pontual".into(),
                focus_id: Some("focus-journal-001".into()),
            })
            .expect("gravar foco");

        reset_full_read_probe();
        assert_eq!(
            store
                .event_at(first_seq)
                .expect("buscar primeiro")
                .expect("primeiro existe")
                .kind,
            "WorkspaceCreated"
        );
        let focus = store
            .event_at(focus_seq)
            .expect("buscar foco")
            .expect("foco existe");
        assert_eq!(focus.seq, focus_seq);
        assert_eq!(focus.kind, "WorkspaceFocusSet");
        assert_eq!(focus.payload["focus_id"], "focus-journal-001");
        assert!(store
            .event_at(focus_seq + 1)
            .expect("buscar ausente")
            .is_none());
        assert!(
            store.event_at(u64::MAX).is_err(),
            "seq fora do intervalo SQLite não pode sofrer conversão truncada"
        );
        assert_eq!(
            full_read_probe().sqlite_history_scans,
            0,
            "busca pontual não pode cair no leitor integral"
        );

        let query_plan: String = store
            .conn
            .query_row(
                &format!("EXPLAIN QUERY PLAN {EVENT_BY_SEQ_SQL}"),
                [i64::try_from(focus_seq).expect("seq cabe no SQLite")],
                |row| row.get(3),
            )
            .expect("explicar query pontual");
        assert!(
            query_plan.contains("INTEGER PRIMARY KEY"),
            "SQLite deve buscar pela PK, plano observado: {query_plan}"
        );
    }

    #[test]
    fn seq_zero_is_never_treated_as_an_empty_store() {
        let tmp = TempDir::new("seq-zero");
        {
            let mut store = EventStore::open(tmp.path()).expect("criar store vazio");
            let event = DomainEvent::WorkspaceCreated {
                name: "Inválido".into(),
                focus_preset: String::new(),
            };
            store
                .conn
                .execute(
                    "INSERT INTO events (seq, ts, kind, version, payload) \
                     VALUES (0, ?1, ?2, ?3, ?4)",
                    params![
                        i64::try_from(now_millis()).expect("timestamp cabe no SQLite"),
                        event.kind(),
                        i64::from(event.current_version()),
                        serde_json::to_string(&event).expect("serializar evento")
                    ],
                )
                .expect("injetar seq zero");

            assert!(
                store.event_count().is_err(),
                "head zero não representa vazio"
            );
            assert!(store.event_at(0).is_err(), "seq zero nunca é consultável");
            assert!(
                store
                    .append(&DomainEvent::NoteCreated {
                        name: "não-gravar.md".into(),
                    })
                    .is_err(),
                "o escritor não pode alocar seq 1 sobre um log iniciado em zero"
            );
        }
        assert!(
            EventStore::open(tmp.path()).is_err(),
            "reabertura deve recusar o banco com única seq zero"
        );
    }

    #[test]
    fn jsonl_marker_zero_is_rejected_instead_of_meaning_empty() {
        let tmp = TempDir::new("jsonl-marker-zero");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .append(&DomainEvent::NoteCreated {
                name: "marcador.md".into(),
            })
            .expect("append");
        store
            .conn
            .execute("DELETE FROM jsonl_mirror", [])
            .expect("limpar marcador");
        store
            .conn
            .execute("INSERT INTO jsonl_mirror (seq) VALUES (0)", [])
            .expect("injetar marcador zero");
        assert!(
            jsonl_state_matches(&store.conn, &store.jsonl_path, 1).is_err(),
            "marcador zero é corrupção, não ausência de confirmação"
        );
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

    fn append_with_legacy_post_commit_error_contract(
        store: &mut EventStore,
        event: &DomainEvent,
    ) -> Result<u64, StoreError> {
        let tx = store
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (_, last_seq) = event_log_position(&tx)?;
        let seq = last_seq.checked_add(1).ok_or_else(|| {
            invalid_recovery_data("sequência do controle legado excedeu u64".into())
        })?;
        let next_seq = seq.checked_add(1).ok_or_else(|| {
            invalid_recovery_data("próxima sequência do controle legado excedeu u64".into())
        })?;
        let record = EventRecord {
            seq,
            ts: now_millis(),
            kind: event.kind().to_owned(),
            version: event.current_version(),
            payload: serde_json::to_value(event)?,
        };
        tx.execute(
            "INSERT INTO events (seq, ts, kind, version, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                i64::try_from(record.seq).map_err(|_| {
                    invalid_recovery_data("sequência do controle legado excedeu i64".into())
                })?,
                i64::try_from(record.ts).map_err(|_| {
                    invalid_recovery_data("timestamp do controle legado excedeu i64".into())
                })?,
                &record.kind,
                i64::from(record.version),
                serde_json::to_string(&record.payload)?
            ],
        )?;
        tx.commit()?;
        store.next_seq = next_seq;

        // Este `?` é deliberadamente o contrato defeituoso anterior: comunica falha
        // ao chamador mesmo depois de o fato semântico já estar durável no SQLite.
        store.mirror_committed_records(std::slice::from_ref(&record))?;
        Ok(seq)
    }

    fn eventstore_post_commit_mirror_failure_child(dir: &Path) -> ! {
        let mut store = EventStore::open(dir).expect("abrir store no processo filho");
        let jsonl = dir.join(JSONL_FILE);
        std::fs::create_dir(&jsonl).expect("injetar falha real no caminho do espelho");

        let committed = store.append(&DomainEvent::WorkspaceCreated {
            name: "TomikOS".into(),
            focus_preset: "developer".into(),
        });

        let committed_seq =
            committed.expect("o contrato atual deve reportar Committed/Ok após o commit SQLite");
        assert_eq!(committed_seq, 1);
        assert_eq!(store.event_count().expect("contar SQLite no filho"), 1);
        assert_eq!(store.events().expect("ler evento no filho").len(), 1);
        assert!(jsonl.is_dir(), "a falha do espelho precisa ser efetiva");
        let mirrored: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM jsonl_mirror", [], |row| row.get(0))
            .expect("contar confirmações no filho");
        assert_eq!(mirrored, 0, "nenhum espelho foi confirmado antes do corte");
        let state = read_recovery_state(&dir.join(JSONL_RECOVERY_STATE_FILE))
            .expect("ler certificado pending no filho")
            .expect("certificado pending existe");
        assert!(matches!(
            state.status,
            JsonlRecoveryStatus::Pending {
                target_event_count: 1,
                target_last_seq: 1
            }
        ));
        std::process::exit(0);
    }

    /// Regressão de confiabilidade: uma falha do espelho acontece DEPOIS do commit SQLite e,
    /// portanto, jamais pode induzir o chamador a repetir a operação semântica. O escritor roda
    /// em um processo filho que termina com o espelho indisponível; uma conexão nova no processo
    /// pai reconcilia exatamente o único evento a partir da autoridade SQLite.
    #[test]
    #[serial]
    fn workspace_reliability_eventstore_never_reports_uncommitted_after_db_commit() {
        const CHILD_DIR_ENV: &str = "LINA_EVENTSTORE_POST_COMMIT_CHILD_DIR";
        if let Some(dir) = std::env::var_os(CHILD_DIR_ENV) {
            eventstore_post_commit_mirror_failure_child(Path::new(&dir));
        }

        let legacy = TempDir::new("legacy-post-commit-error");
        let legacy_jsonl = legacy.path().join(JSONL_FILE);
        let mut legacy_store = EventStore::open(legacy.path()).expect("abrir controle legado");
        std::fs::create_dir(&legacy_jsonl).expect("injetar falha no controle legado");
        let legacy_error = append_with_legacy_post_commit_error_contract(
            &mut legacy_store,
            &DomainEvent::WorkspaceCreated {
                name: "TomikOS legado".into(),
                focus_preset: "developer".into(),
            },
        )
        .expect_err("o contrato legado deve expor Err depois da falha do espelho");
        assert!(
            matches!(&legacy_error, StoreError::Io(_)),
            "o controle legado precisa falhar no I/O real do espelho: {legacy_error}"
        );
        assert_eq!(
            legacy_store
                .event_count()
                .expect("contar commit legado apesar do Err"),
            1,
            "controle não-vácuo: o Err legado esconderia do caller um commit real"
        );
        assert_eq!(legacy_store.events().expect("ler commit legado").len(), 1);
        assert!(legacy_jsonl.is_dir(), "o erro legado veio do espelho real");
        let legacy_mirrored: i64 = legacy_store
            .conn
            .query_row("SELECT COUNT(*) FROM jsonl_mirror", [], |row| row.get(0))
            .expect("contar confirmações do controle legado");
        assert_eq!(legacy_mirrored, 0);
        let legacy_state = read_recovery_state(&legacy.path().join(JSONL_RECOVERY_STATE_FILE))
            .expect("ler pending do controle legado")
            .expect("pending legado existe");
        assert!(matches!(
            legacy_state.status,
            JsonlRecoveryStatus::Pending {
                target_event_count: 1,
                target_last_seq: 1
            }
        ));
        drop(legacy_store);

        let tmp = TempDir::new("mirror-after-commit-process");
        let jsonl = tmp.path().join(JSONL_FILE);
        let child_status = std::process::Command::new(
            std::env::current_exe().expect("localizar binário do teste"),
        )
        .args([
            "--exact",
            "events::tests::workspace_reliability_eventstore_never_reports_uncommitted_after_db_commit",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD_DIR_ENV, tmp.path())
        .status()
        .expect("executar processo filho no corte pós-commit");
        assert!(
            child_status.success(),
            "processo filho falhou: {child_status}"
        );
        assert!(
            jsonl.is_dir(),
            "o processo terminou deixando o espelho realmente indisponível"
        );

        std::fs::remove_dir(&jsonl).expect("restaurar caminho do espelho no processo pai");
        let reopened = EventStore::open(tmp.path()).expect("reabrir e reconciliar no processo pai");
        assert_eq!(reopened.event_count().expect("contar após reabrir"), 1);
        let records = reopened.events().expect("ler SQLite reconciliado");
        assert_eq!(records.len(), 1, "o caller não repetiu o evento semântico");
        assert_eq!(records[0].seq, 1);
        assert_eq!(records[0].kind, "WorkspaceCreated");
        assert_eq!(records[0].payload["name"], "TomikOS");
        let mirrored_after_reopen: i64 = reopened
            .conn
            .query_row("SELECT COUNT(*) FROM jsonl_mirror", [], |row| row.get(0))
            .expect("contar confirmações reconciliadas");
        assert_eq!(mirrored_after_reopen, 1);
        let first_reconciliation = read_jsonl_records(&jsonl);
        assert_eq!(
            first_reconciliation.len(),
            1,
            "evento ausente foi reconciliado"
        );
        assert_eq!(first_reconciliation, records);
        let mirror_bytes = std::fs::read(&jsonl).expect("ler bytes do espelho reconciliado");
        let mut mirror_hasher = Sha256::new();
        mirror_hasher.update(&mirror_bytes);
        let recovery_state = read_recovery_state(&tmp.path().join(JSONL_RECOVERY_STATE_FILE))
            .expect("ler certificado completo")
            .expect("certificado completo existe");
        assert_eq!(recovery_state.version, JSONL_RECOVERY_STATE_VERSION);
        assert!(matches!(
            recovery_state.status,
            JsonlRecoveryStatus::Complete {
                event_count: 1,
                last_seq: 1,
                ref digest_sha256,
            } if digest_sha256 == &sha256_hex(&mirror_hasher)
        ));
        drop(reopened);

        let healthy_reopen = EventStore::open(tmp.path()).expect("reabrir idempotente");
        assert_eq!(
            healthy_reopen.conn.total_changes(),
            0,
            "espelho saudável usa o fast path sem regravar confirmações"
        );
        drop(healthy_reopen);
        let second_reconciliation = read_jsonl_records(&jsonl);
        assert_eq!(
            second_reconciliation.len(),
            1,
            "reconciliar novamente não pode duplicar o evento"
        );
        assert_eq!(second_reconciliation[0].seq, 1);
    }

    #[test]
    #[serial]
    fn workspace_reliability_eventstore_repairs_truncated_corrupt_and_duplicate_mirror() {
        let tmp = TempDir::new("mirror-canonical-repair");
        let expected = {
            let mut store = EventStore::open(tmp.path()).expect("store");
            for index in 0..3 {
                store
                    .insert_raw("MirrorRepair", 1, serde_json::json!({ "index": index }))
                    .expect("append");
            }
            store.events().expect("eventos SQLite")
        };
        let jsonl = tmp.path().join(JSONL_FILE);

        let original_len = std::fs::metadata(&jsonl).expect("metadata").len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&jsonl)
            .expect("abrir para truncar")
            .set_len(original_len / 2)
            .expect("truncar");
        drop(EventStore::open(tmp.path()).expect("reparar truncamento"));
        assert_eq!(read_jsonl_records(&jsonl), expected);

        std::thread::sleep(std::time::Duration::from_millis(2));
        corrupt_middle(&jsonl);
        drop(EventStore::open(tmp.path()).expect("reparar corrupção in-place"));
        assert_eq!(read_jsonl_records(&jsonl), expected);

        let duplicate = serde_json::to_string(&expected[0]).expect("serializar duplicata");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&jsonl)
            .expect("abrir para duplicar");
        writeln!(file, "{duplicate}").expect("duplicar linha");
        file.sync_data().expect("sincronizar duplicata");
        drop(file);
        drop(EventStore::open(tmp.path()).expect("deduplicar canonicamente"));
        assert_eq!(read_jsonl_records(&jsonl), expected);
    }

    #[test]
    #[serial]
    fn workspace_reliability_zero_sqlite_recovers_only_from_certified_jsonl() {
        let tmp = TempDir::new("zero-db-certified-recovery");
        let workspace_id = Uuid::now_v7().to_string();
        let expected = {
            let mut store = EventStore::open(tmp.path()).expect("store");
            store
                .append_batch(&[
                    DomainEvent::WorkspaceCreated {
                        name: "Certificado".into(),
                        focus_preset: "blank".into(),
                    },
                    DomainEvent::WorkspaceIdAssigned {
                        workspace_id: workspace_id.clone(),
                    },
                ])
                .expect("batch inicial");
            store.events().expect("eventos")
        };
        let jsonl = tmp.path().join(JSONL_FILE);
        let mirror_before = std::fs::read(&jsonl).expect("espelho certificado");
        remove_sqlite_store_files(tmp.path());
        let zero_db = tmp.path().join("lina.db");
        std::fs::File::create(&zero_db)
            .and_then(|file| file.sync_all())
            .expect("criar SQLite zero bytes");

        assert!(
            db_is_corrupt(&zero_db),
            "DB existente sem schema é corrupto"
        );
        assert!(
            EventStore::open(tmp.path()).is_err(),
            "open comum não pode inicializar silenciosamente um DB já existente"
        );
        assert_eq!(
            std::fs::read(&jsonl).expect("JSONL após open recusado"),
            mirror_before,
            "o JSONL válido nunca é canonizado contra o DB vazio"
        );

        let mut ui = RecordingUi::default();
        let recovered = EventStore::open_or_recover(tmp.path(), &mut ui)
            .expect("recuperar DB zero pelo JSONL certificado");
        assert_eq!(recovered.events().expect("eventos recuperados"), expected);
        let projected = recovered.project().expect("projeção recuperada");
        assert_eq!(projected.workspace_name.as_deref(), Some("Certificado"));
        assert_eq!(
            projected.workspace_id.as_deref(),
            Some(workspace_id.as_str())
        );
        drop(recovered);
        assert_eq!(
            std::fs::read(&jsonl).expect("espelho após recovery"),
            mirror_before,
            "recovery não reescreve a única cópia íntegra"
        );
        assert!(matches!(
            ui.events.as_slice(),
            [HostEvent::Recovering, HostEvent::Recovered]
        ));
        assert!(
            std::fs::read_dir(tmp.path())
                .expect("listar artefatos")
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")),
            "o DB zero original foi preservado"
        );
    }

    #[test]
    #[serial]
    fn workspace_reliability_pending_certificate_rejects_committed_prefix() {
        let tmp = TempDir::new("pending-prefix");
        let mut store = EventStore::open(tmp.path()).expect("store");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Prefixo".into(),
                focus_preset: "blank".into(),
            })
            .expect("primeiro evento");
        let jsonl = tmp.path().join(JSONL_FILE);
        let saved_prefix = tmp.path().join("saved-prefix.jsonl");
        std::fs::rename(&jsonl, &saved_prefix).expect("isolar prefixo");
        std::fs::create_dir(&jsonl).expect("injetar falha no caminho do espelho");

        let second_seq = store
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: Uuid::now_v7().to_string(),
            })
            .expect("commit SQLite não vira falso erro após falha do espelho");
        assert_eq!(second_seq, 2);
        assert_eq!(store.event_count().expect("contagem SQLite"), 2);
        let state = read_recovery_state(&tmp.path().join(JSONL_RECOVERY_STATE_FILE))
            .expect("ler certificado")
            .expect("certificado existente");
        assert!(matches!(
            state.status,
            JsonlRecoveryStatus::Pending {
                target_event_count: 2,
                target_last_seq: 2
            }
        ));
        drop(store);

        std::fs::remove_dir(&jsonl).expect("remover falha injetada");
        std::fs::rename(&saved_prefix, &jsonl).expect("restaurar apenas prefixo seq=1");
        let prefix_before = std::fs::read(&jsonl).expect("prefixo");
        remove_sqlite_store_files(tmp.path());
        let mut ui = RecordingUi::default();
        assert!(
            EventStore::open_or_recover(tmp.path(), &mut ui).is_err(),
            "pending precisa recusar o prefixo mesmo que cada linha seja válida"
        );
        assert!(
            !tmp.path().join("lina.db").exists(),
            "prefixo incompleto não publica DB parcial"
        );
        assert_eq!(
            std::fs::read(&jsonl).expect("prefixo preservado"),
            prefix_before
        );
        assert!(ui.events.is_empty(), "recusa não anuncia recuperação");
    }

    #[test]
    #[serial]
    fn workspace_reliability_semantically_invalid_jsonl_never_publishes_db() {
        let tmp = TempDir::new("semantic-invalid-recovery");
        let jsonl = tmp.path().join(JSONL_FILE);
        {
            let mut store = EventStore::open(tmp.path()).expect("store");
            store
                .insert_raw(
                    "WorkspaceCreated",
                    1,
                    serde_json::to_value(DomainEvent::WorkspaceIdAssigned {
                        workspace_id: Uuid::now_v7().to_string(),
                    })
                    .expect("payload"),
                )
                .expect("registro estruturalmente persistível");
            let state = read_recovery_state(&tmp.path().join(JSONL_RECOVERY_STATE_FILE))
                .expect("ler certificado")
                .expect("certificado existente");
            assert!(matches!(
                state.status,
                JsonlRecoveryStatus::Complete {
                    event_count: 1,
                    last_seq: 1,
                    ..
                }
            ));
        }
        let mirror_before = std::fs::read(&jsonl).expect("espelho semanticamente inválido");
        remove_sqlite_store_files(tmp.path());

        let mut ui = RecordingUi::default();
        assert!(
            EventStore::open_or_recover(tmp.path(), &mut ui).is_err(),
            "kind persistido divergente do DomainEvent precisa abortar recovery"
        );
        assert!(!tmp.path().join("lina.db").exists());
        assert_eq!(
            std::fs::read(&jsonl).expect("espelho preservado"),
            mirror_before
        );
        assert!(ui.events.is_empty());
    }

    #[test]
    #[serial]
    fn workspace_reliability_append_batch_rolls_back_every_event_before_commit() {
        let tmp = TempDir::new("batch-rollback");
        let mut store = EventStore::open(tmp.path()).expect("store");
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER reject_workspace_id \
                 BEFORE INSERT ON events \
                 WHEN NEW.kind = 'WorkspaceIdAssigned' \
                 BEGIN SELECT RAISE(ABORT, 'falha precommit injetada'); END;",
            )
            .expect("trigger de falha");
        let events = [
            DomainEvent::WorkspaceCreated {
                name: "Nunca parcial".into(),
                focus_preset: "blank".into(),
            },
            DomainEvent::WorkspaceIdAssigned {
                workspace_id: Uuid::now_v7().to_string(),
            },
        ];

        assert!(store.append_batch(&events).is_err());
        assert_eq!(store.event_count().expect("contagem"), 0);
        assert!(store.events().expect("eventos").is_empty());
        assert_eq!(
            store.append_batch(&[]).expect("batch vazio"),
            0,
            "batch vazio devolve a contagem confirmada atual"
        );
        assert!(
            !tmp.path().join(JSONL_FILE).exists(),
            "falha precommit não expõe subconjunto no JSONL"
        );
        let state = read_recovery_state(&tmp.path().join(JSONL_RECOVERY_STATE_FILE))
            .expect("ler pending")
            .expect("pending existe");
        assert!(matches!(
            state.status,
            JsonlRecoveryStatus::Pending {
                target_event_count: 2,
                target_last_seq: 2
            }
        ));
    }

    #[test]
    #[serial]
    fn workspace_reliability_append_batch_commits_orders_and_certifies_once() {
        let tmp = TempDir::new("batch-success");
        let workspace_id = Uuid::now_v7().to_string();
        let events = [
            DomainEvent::WorkspaceCreated {
                name: "Batch íntegro".into(),
                focus_preset: "developer".into(),
            },
            DomainEvent::WorkspaceIdAssigned {
                workspace_id: workspace_id.clone(),
            },
        ];
        let mut store = EventStore::open(tmp.path()).expect("store");
        assert_eq!(
            store.append_batch(&events).expect("batch atômico"),
            2,
            "retorno é a contagem SQLite confirmada"
        );
        assert_eq!(store.append_batch(&[]).expect("batch vazio"), 2);
        let expected_records = store.events().expect("eventos");
        assert_eq!(
            expected_records
                .iter()
                .map(|record| record.seq)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        for (record, expected_event) in expected_records.iter().zip(&events) {
            assert_eq!(
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
                    .expect("roundtrip"),
                *expected_event
            );
        }
        let projected = store.project().expect("projetar batch");
        assert_eq!(projected.workspace_name.as_deref(), Some("Batch íntegro"));
        assert_eq!(
            projected.workspace_id.as_deref(),
            Some(workspace_id.as_str())
        );
        let jsonl = tmp.path().join(JSONL_FILE);
        assert_eq!(read_jsonl_records(&jsonl), expected_records);
        let state = read_recovery_state(&tmp.path().join(JSONL_RECOVERY_STATE_FILE))
            .expect("ler complete")
            .expect("complete existe");
        assert!(matches!(
            state.status,
            JsonlRecoveryStatus::Complete {
                event_count: 2,
                last_seq: 2,
                ..
            }
        ));
        drop(store);

        remove_sqlite_store_files(tmp.path());
        let mut ui = RecordingUi::default();
        let recovered = EventStore::open_or_recover(tmp.path(), &mut ui)
            .expect("certificado do batch recupera o DB inteiro");
        assert_eq!(
            recovered.events().expect("eventos recuperados"),
            expected_records
        );
    }

    #[test]
    #[serial]
    fn workspace_reliability_missing_or_incompatible_certificate_refuses_recovery() {
        let tmp = TempDir::new("certificate-required");
        {
            let mut store = EventStore::open(tmp.path()).expect("store");
            store
                .append(&DomainEvent::WorkspaceCreated {
                    name: "Sem certificado".into(),
                    focus_preset: "blank".into(),
                })
                .expect("evento");
        }
        let state_path = tmp.path().join(JSONL_RECOVERY_STATE_FILE);
        let mut state = read_recovery_state(&state_path)
            .expect("ler certificado")
            .expect("certificado");
        std::fs::remove_file(&state_path).expect("remover certificado");
        remove_sqlite_store_files(tmp.path());
        let mut ui = RecordingUi::default();
        assert!(EventStore::open_or_recover(tmp.path(), &mut ui).is_err());
        assert!(!tmp.path().join("lina.db").exists());

        state.version = JSONL_RECOVERY_STATE_VERSION + 1;
        write_recovery_state_atomic(&state_path, &state).expect("certificado incompatível");
        assert!(EventStore::open_or_recover(tmp.path(), &mut ui).is_err());
        assert!(!tmp.path().join("lina.db").exists());
        assert!(ui.events.is_empty());
    }

    #[test]
    #[serial]
    fn workspace_reliability_missing_db_rebuilds_from_unique_jsonl() {
        let tmp = TempDir::new("missing-db-recovery");
        let expected = {
            let mut store = EventStore::open(tmp.path()).expect("store");
            store
                .append(&DomainEvent::WorkspaceCreated {
                    name: "Recuperável".into(),
                    focus_preset: "blank".into(),
                })
                .expect("workspace");
            store
                .append(&DomainEvent::WorkspaceIdAssigned {
                    workspace_id: Uuid::now_v7().to_string(),
                })
                .expect("id");
            store.events().expect("eventos")
        };
        let jsonl = tmp.path().join(JSONL_FILE);
        remove_sqlite_store_files(tmp.path());

        let mirror_before = std::fs::read(&jsonl).expect("espelho antes");
        let injected = EventStore::rebuild_database_atomic(tmp.path(), &expected, |inserted| {
            if inserted == 1 {
                Err(invalid_recovery_data(
                    "falha injetada no meio do rebuild".to_owned(),
                ))
            } else {
                Ok(())
            }
        });
        assert!(injected.is_err(), "controle negativo interrompe o rebuild");
        assert!(
            !tmp.path().join("lina.db").exists(),
            "falha intermediária não publica DB parcial"
        );
        assert_eq!(
            std::fs::read(&jsonl).expect("espelho após corte"),
            mirror_before,
            "a única cópia recuperável fica byte-idêntica"
        );
        assert!(
            std::fs::read_dir(tmp.path())
                .expect("listar staging")
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".lina-db-recovery-")),
            "staging falho foi removido"
        );
        assert!(
            EventStore::open(tmp.path()).is_err(),
            "abertura sem recovery não pode tratar JSONL existente como store novo"
        );
        assert_eq!(
            std::fs::read(&jsonl).expect("espelho preservado"),
            mirror_before,
            "open comum nunca apaga a única cópia recuperável"
        );

        let mut ui = RecordingUi::default();
        let recovered =
            EventStore::open_or_recover(tmp.path(), &mut ui).expect("reconstruir DB ausente");
        assert_eq!(recovered.events().expect("eventos recuperados"), expected);
        assert!(matches!(
            ui.events.as_slice(),
            [HostEvent::Recovering, HostEvent::Recovered]
        ));
        drop(recovered);
        assert_eq!(
            read_jsonl_records(&jsonl),
            expected,
            "a recuperação preserva o espelho certificado"
        );
    }

    #[test]
    #[serial]
    fn workspace_reliability_ambiguous_or_absent_mirror_never_claims_recovery() {
        let conflicting = TempDir::new("conflicting-recovery");
        let jsonl = conflicting.path().join(JSONL_FILE);
        let original = {
            let mut store = EventStore::open(conflicting.path()).expect("store");
            store
                .insert_raw("Original", 1, serde_json::json!({ "value": 1 }))
                .expect("evento");
            store.events().expect("eventos")[0].clone()
        };
        let mut forged = original.clone();
        forged.payload = serde_json::json!({ "value": "divergente" });
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&jsonl)
            .expect("jsonl");
        writeln!(file, "{}", serde_json::to_string(&forged).expect("forjado")).expect("conflito");
        file.sync_data().expect("sync");
        drop(file);
        remove_sqlite_store_files(conflicting.path());
        let mut ui = RecordingUi::default();
        assert!(EventStore::open_or_recover(conflicting.path(), &mut ui).is_err());
        assert!(ui.events.is_empty(), "conflito não pode anunciar Recovered");

        let absent = TempDir::new("absent-recovery");
        let db = absent.path().join("lina.db");
        std::fs::write(&db, b"banco corrompido sem espelho").expect("corromper DB");
        let mut ui = RecordingUi::default();
        assert!(EventStore::open_or_recover(absent.path(), &mut ui).is_err());
        assert!(db.exists(), "sem espelho, o banco original fica intocado");
        assert!(
            ui.events.is_empty(),
            "sem fonte não existe falsa recuperação"
        );
    }

    fn remove_sqlite_store_files(dir: &Path) {
        for name in ["lina.db", "lina.db-wal", "lina.db-shm"] {
            match std::fs::remove_file(dir.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("remover {name}: {error}"),
            }
        }
    }

    fn read_jsonl_records(path: &Path) -> Vec<EventRecord> {
        let file = std::fs::File::open(path).expect("abrir log.jsonl");
        std::io::BufReader::new(file)
            .lines()
            .filter_map(|line| {
                let line = line.expect("ler linha do log.jsonl");
                (!line.trim().is_empty()).then_some(line)
            })
            .map(|line| serde_json::from_str(&line).expect("desserializar evento do log.jsonl"))
            .collect()
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

        // (b) seqs ÚNICOS e CONTÍGUOS — sem buraco nem duplicata (alocação sob BEGIN IMMEDIATE).
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

    /// ADR 0037: editar a Pasta de Trabalho por-nó (`NodeCwdSet`) projeta o `cwd` novo
    /// (último vence sobre o `TerminalSpawned.cwd` original) — é o caminho "só na próxima
    /// abertura": o restore lê `ProjectedNode.cwd`, então a pasta editada sobrevive ao
    /// fechar/reabrir mesmo sem re-spawn. Evento ADITIVO: log antigo sem ele replaya intacto.
    #[test]
    #[serial]
    fn node_cwd_set_projeta_cwd_editado_ultimo_vence() {
        let node = Uuid::now_v7();
        let tmp = TempDir::new("node-cwd-set");
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
                cwd: Some("/Users/founder/pasta-original".into()),
            })
            .expect("append TerminalSpawned");
        // A edição da pasta: vira o cwd projetado (o que o restore re-aplica).
        store
            .append(&DomainEvent::NodeCwdSet {
                node,
                cwd: "/Users/founder/pasta-nova".into(),
            })
            .expect("append NodeCwdSet");
        let state = store.project().expect("project");
        let n = state.nodes.get(&node).expect("nó projetado");
        assert_eq!(
            n.cwd.as_deref(),
            Some("/Users/founder/pasta-nova"),
            "NodeCwdSet (último) vence o cwd do spawn original — o restore re-entra na pasta editada"
        );

        // Aditivo: um payload sem a variante (log de geração anterior) nunca foi `NodeCwdSet`;
        // a desserialização de um `NodeCwdSet` real round-trips pelo store sem upcast.
        let rec = DomainEvent::from_record(
            "NodeCwdSet",
            1,
            serde_json::json!({ "event": "NodeCwdSet", "node": node, "cwd": "/x" }),
        )
        .expect("NodeCwdSet desserializa");
        assert_eq!(
            rec,
            DomainEvent::NodeCwdSet {
                node,
                cwd: "/x".into()
            }
        );
    }

    /// F2-3-2: `TerminalRedimensionado` projeta a grade do PTY (cols×rows) + o tamanho
    /// EXATO do card em px (w×h) que o canvas restaura ao reabrir.
    #[test]
    #[serial]
    fn terminal_redimensionado_projeta_dimensoes() {
        let node = Uuid::now_v7();
        let tmp = TempDir::new("resize-projecao");
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
            .append(&DomainEvent::TerminalRedimensionado {
                node,
                cols: 120,
                rows: 40,
                w: 960.0,
                h: 700.0,
            })
            .expect("append TerminalRedimensionado");
        let state = store.project().expect("project");
        let n = state.nodes.get(&node).expect("nó projetado");
        assert_eq!(n.cols, Some(120), "cols restauradas");
        assert_eq!(n.rows, Some(40), "rows restauradas");
        assert_eq!(n.w, Some(960.0), "largura px exata restaurada");
        assert_eq!(n.h, Some(700.0), "altura px exata restaurada");
    }

    /// F2-3-2: o último redimensionamento vence — `apply` muta o nó in-place (como
    /// `NodeMoved`), então reabrir mostra o tamanho FINAL, não o primeiro.
    #[test]
    #[serial]
    fn terminal_redimensionado_ultima_escrita_vence() {
        let node = Uuid::now_v7();
        let tmp = TempDir::new("resize-ultima");
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
        for (cols, rows, w, h) in [(80u16, 24u16, 640.0, 420.0), (132, 50, 1056.0, 875.0)] {
            store
                .append(&DomainEvent::TerminalRedimensionado {
                    node,
                    cols,
                    rows,
                    w,
                    h,
                })
                .expect("append resize");
        }
        let state = store.project().expect("project");
        let n = state.nodes.get(&node).expect("nó");
        assert_eq!(
            (n.cols, n.rows),
            (Some(132), Some(50)),
            "último resize vence"
        );
        assert_eq!((n.w, n.h), (Some(1056.0), Some(875.0)));
    }

    /// F2-3-2: as dimensões são ADITIVAS — (a) um snapshot antigo de `ProjectedNode`
    /// (sem os campos novos) desserializa com `None` (replay de snapshot antigo nunca
    /// quebra); (b) o `kind` persistido casa o nome da variante.
    #[test]
    fn terminal_redimensionado_dimensoes_aditivas_e_kind() {
        // (a) Shape byte-fiel de um ProjectedNode pré-F2-3-2 (todos os campos antigos,
        // NENHUM dos novos cols/rows/w/h).
        let old_json = serde_json::json!({
            "kind": "Terminal",
            "name": null, "role": null, "status": null,
            "x": 12.0, "y": 34.0,
            "cli": null, "cwd": null, "stalled": false
        });
        let n: ProjectedNode =
            serde_json::from_value(old_json).expect("snapshot antigo desserializa");
        assert_eq!(
            (n.cols, n.rows, n.w, n.h),
            (None, None, None, None),
            "snapshot antigo sem dims → None (aditivo)"
        );

        // (b) kind persistido casa o nome da variante.
        let ev = DomainEvent::TerminalRedimensionado {
            node: Uuid::now_v7(),
            cols: 100,
            rows: 30,
            w: 800.0,
            h: 525.0,
        };
        assert_eq!(ev.kind(), "TerminalRedimensionado");
    }

    /// F2-3-2: replay ROBUSTO — um `TerminalRedimensionado` ÓRFÃO (sem `NodeAdded` prévio;
    /// ex.: nó removido) é no-op no `apply` (o nó não existe no estado) e o `project()`
    /// não entra em pânico. Garante que um log com evento órfão sempre replaya.
    #[test]
    #[serial]
    fn terminal_redimensionado_orfao_nao_quebra_replay() {
        let node = Uuid::now_v7();
        let tmp = TempDir::new("resize-orfao");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .append(&DomainEvent::TerminalRedimensionado {
                node,
                cols: 100,
                rows: 30,
                w: 800.0,
                h: 525.0,
            })
            .expect("append resize órfão");
        let state = store
            .project()
            .expect("project não entra em pânico com evento órfão");
        assert!(!state.nodes.contains_key(&node), "evento órfão não cria nó");
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
                    model: Some("opus".into()),
                    effort: Some(Effort::High),
                    goal_id: None,
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

    /// F3-0-3 (ADR 0031) / F3-1 (spec 52 §4): `SpawnRequested` ganhou `model`/`effort` (preferência de
    /// execução) e `goal_id` (a Goal que justifica o recurso) — TODOS ADITIVOS. Roundtrip preserva; e
    /// um log F1-3-6 SEM as chaves replaya com `None` (serde default) — compat total com spawns já no
    /// log.
    #[test]
    fn spawn_requested_model_effort_are_additive() {
        let ev = DomainEvent::SpawnRequested {
            id: "msg_x".into(),
            requested_by: Uuid::now_v7(),
            name: "@QA".into(),
            role: "qa".into(),
            root_cause_id: "msg_root".into(),
            hops: 0,
            prompt: "valide".into(),
            model: Some("opus".into()),
            effort: Some(Effort::High),
            goal_id: Some("g_01".into()),
        };
        let back: DomainEvent = serde_json::from_value(serde_json::to_value(&ev).unwrap()).unwrap();
        assert_eq!(back, ev, "roundtrip preserva model/effort/goal_id");

        // Log ANTIGO (sem as chaves) → replaya com None.
        let mut v = serde_json::to_value(&ev).unwrap();
        let obj = v.as_object_mut().expect("objeto");
        obj.remove("model");
        obj.remove("effort");
        obj.remove("goal_id");
        match serde_json::from_value::<DomainEvent>(v).expect("desserializa log antigo") {
            DomainEvent::SpawnRequested {
                model,
                effort,
                goal_id,
                ..
            } => {
                assert_eq!(model, None, "log antigo sem model → None");
                assert_eq!(effort, None, "log antigo sem effort → None");
                assert_eq!(goal_id, None, "log antigo sem goal_id → None");
            }
            other => panic!("variante errada: {other:?}"),
        }
    }

    /// F3-0-4 (ADR 0031): `EffortAssigned` é META, roundtripa, serializa as strings que a F3-0-7
    /// audita (`effort:"high"`, `origin:"assigned"`), e seus campos opcionais são ADITIVOS — um log
    /// parcial (só `node`+`effort`) replaya com `origin` no default CONSERVADOR (`observed`, nunca
    /// `assigned`) e o resto `None`.
    #[test]
    fn effort_assigned_roundtrips_meta_and_additive() {
        let ev = DomainEvent::EffortAssigned {
            node: "@QA".into(),
            effort: Effort::High,
            model: Some("opus".into()),
            origin: EffortOrigin::Assigned,
            task_difficulty: Some("hard".into()),
            by: Some(Uuid::now_v7()),
        };
        assert_eq!(ev.kind(), "EffortAssigned");
        let back: DomainEvent = serde_json::from_value(serde_json::to_value(&ev).unwrap()).unwrap();
        assert_eq!(back, ev, "roundtrip preserva todos os campos");

        // META: apply não toca a projeção.
        let mut state = ProjectedState::default();
        let before = state.clone();
        apply(&mut state, &ev);
        assert_eq!(state, before, "EffortAssigned é META");

        // A superfície serializada casa o que a F3-0-7 audita (`ev["origin"].as_str()` etc.).
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["effort"].as_str(), Some("high"));
        assert_eq!(v["origin"].as_str(), Some("assigned"));

        // Log PARCIAL (só node+effort) → origin default conservador (observed), resto None.
        let legacy = serde_json::json!({"event":"EffortAssigned","node":"@X","effort":"low"});
        match serde_json::from_value::<DomainEvent>(legacy).expect("parcial") {
            DomainEvent::EffortAssigned {
                effort,
                origin,
                by,
                model,
                task_difficulty,
                ..
            } => {
                assert_eq!(effort, Effort::Low);
                assert_eq!(
                    origin,
                    EffortOrigin::Observed,
                    "default conservador ≠ assigned"
                );
                assert_eq!(by, None);
                assert_eq!(model, None);
                assert_eq!(task_difficulty, None);
            }
            other => panic!("variante errada: {other:?}"),
        }
    }

    /// `Effort::label()` é o vocabulário neutro low/medium/high do `LINA_EFFORT` e dos eventos.
    #[test]
    fn effort_label_is_lowercase_neutral() {
        assert_eq!(Effort::Low.label(), "low");
        assert_eq!(Effort::Medium.label(), "medium");
        assert_eq!(Effort::High.label(), "high");
    }

    /// F3-5 (FUNDAÇÃO/largada, spec 53 §11): as 10 variantes novas são META, fazem round-trip
    /// serde (incl. `ReclaimCandidate` aninhado) e são ADITIVAS — um payload PARCIAL de
    /// `SessionPersisted` carrega com defaults seguros (`after_first_prompt=true`, `*_at_ms=0`,
    /// `label=None`), e um log ANTIGO (variante pré-F3-5) replaya intacto. Prova que a largada
    /// não quebra o replay F0/F1/F2/F3.
    #[test]
    fn f3_5_contract_is_additive_meta_and_roundtrips() {
        let session = DomainEvent::SessionPersisted {
            node: "n-abc".into(),
            cli: "claude".into(),
            session_id: "sess-1".into(),
            after_first_prompt: true,
            first_prompt_at_ms: 100,
            label: Some("revisão da landing".into()),
            persisted_at_ms: 200,
        };
        assert_eq!(session.kind(), "SessionPersisted");
        let reclaim = DomainEvent::DiskReclaimProposed {
            candidates: vec![ReclaimCandidate {
                path: "target/".into(),
                bytes: 30_000_000_000,
                kind: "cargo_target".into(),
            }],
            reclaimable_bytes: 30_000_000_000,
            proposed_at_ms: 0,
        };
        assert_eq!(reclaim.kind(), "DiskReclaimProposed");
        for ev in [&session, &reclaim] {
            // Round-trip serde preserva tudo (inclui o `ReclaimCandidate` aninhado).
            let back: DomainEvent =
                serde_json::from_value(serde_json::to_value(ev).unwrap()).unwrap();
            assert_eq!(&back, ev, "round-trip preserva os campos");
            // META: o apply do canvas não toca a projeção (reconstrução é por replay externo).
            let mut state = ProjectedState::default();
            let before = state.clone();
            apply(&mut state, ev);
            assert_eq!(state, before, "F3-5 é META (no-op no canvas)");
        }

        // ADITIVO — payload PARCIAL de `SessionPersisted` (só os obrigatórios) → defaults seguros.
        let partial = serde_json::json!({"event":"SessionPersisted","node":"n-x","cli":"codex","session_id":"s9"});
        match serde_json::from_value::<DomainEvent>(partial).expect("parcial carrega") {
            DomainEvent::SessionPersisted {
                after_first_prompt,
                first_prompt_at_ms,
                label,
                persisted_at_ms,
                ..
            } => {
                assert!(
                    after_first_prompt,
                    "default_true: invariante de admissão ≥1 prompt"
                );
                assert_eq!(first_prompt_at_ms, 0);
                assert_eq!(label, None);
                assert_eq!(persisted_at_ms, 0);
            }
            other => panic!("variante errada: {other:?}"),
        }

        // ADITIVO — um log ANTIGO (variante pré-F3-5) replaya intacto: a largada não o quebra.
        let legacy = serde_json::json!({"event":"WorkspaceArchived"});
        assert_eq!(
            serde_json::from_value::<DomainEvent>(legacy).expect("evento antigo carrega"),
            DomainEvent::WorkspaceArchived,
            "replay F0..F3-4 intacto após a largada F3-5"
        );
    }

    /// Rito de paradigma (épico 39 §IV, ADR 0044): `ParadigmReviewed` é META, faz round-trip serde
    /// e é ADITIVO — payload sem `adr_ref` carrega (`None`). O rito que audita o #4 vira fato no log.
    #[test]
    fn paradigm_reviewed_is_additive_meta_and_roundtrips() {
        let ev = DomainEvent::ParadigmReviewed {
            phase: "F3".into(),
            invariant_challenged: "7-invariantes".into(),
            verdict: "sem-dogma; 3 impostos nomeados".into(),
            adr_ref: Some("0044".into()),
        };
        assert_eq!(ev.kind(), "ParadigmReviewed");
        let back: DomainEvent = serde_json::from_value(serde_json::to_value(&ev).unwrap()).unwrap();
        assert_eq!(back, ev, "round-trip preserva os campos");
        // META: não toca o canvas (auditoria por replay externo).
        let mut state = ProjectedState::default();
        let before = state.clone();
        apply(&mut state, &ev);
        assert_eq!(state, before, "ParadigmReviewed é META");
        // ADITIVO: payload sem `adr_ref` → None.
        let partial = serde_json::json!({
            "event":"ParadigmReviewed","phase":"F4","invariant_challenged":"x","verdict":"y"
        });
        match serde_json::from_value::<DomainEvent>(partial).expect("parcial carrega") {
            DomainEvent::ParadigmReviewed { adr_ref, .. } => assert_eq!(adr_ref, None),
            other => panic!("variante errada: {other:?}"),
        }
    }
}
