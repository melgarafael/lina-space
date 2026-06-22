//! **W3-4 · Roteador do supervisor (pipeline `route_message` com guardrails 0-4).** Drena a
//! [`Mailbox`], resolve os NOMES da mensagem para `NodeId` no roster do [`Supervisor`], aplica os
//! guardrails determinísticos (NUNCA LLM), persiste no event log e ENTREGA ao PTY do alvo via a
//! injeção faseada ([`crate::deliver_a2a`]). É o **escritor único** do estado de roteamento — vive
//! numa thread só (o `pump` do app, ou o teste headless), então o estado de dedupe/orçamento é
//! local (sem locks).
//!
//! Pipeline (design §3.5 + §4 tabela W3-7), em ordem:
//! `dedupe(id, 60s)` → `anti-loop(hops)` → `remetente existe` → `alvo existe` → `autonomia` →
//! `gate de fan-out` → `orçamento(root_cause_id)` → `anti-deadlock(wait-for)` → **rota + entrega**.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use thiserror::Error;

use crate::code::{CodeIntegration, INTEGRATOR_ROLE, INTEGRATOR_SETTLE_MS};
use crate::events::{
    AcceptanceCriterion, AwaitReason, BlockReason, CheckKind, DomainEvent, Effort, EffortOrigin,
    EventRecord, EventStore, StoreError, Verdict as GoalVerdict,
};
use crate::guard::{classify, decide, ActionClass, Decision};
use crate::mailbox::{
    parse_correction, parse_target, render_message_block_v2, validate_envelope_v2,
    EnvelopeViolation, MailMessage, Mailbox, TargetSpec,
};
use crate::plan::{ItemState, Plan, PlanError, PlanItem};
use crate::{A2aEnvelope, DeliveryOutcome, NodeId, NodeStatus, Recipient, RolePolicy, Supervisor};

/// Janela de dedupe por `id` (design §3: dedupe em 60s).
pub const DEDUPE_WINDOW_MS: u64 = 60_000;
/// Profundidade máxima de cadeia (= `hops`/`--await`); corta o anti-loop.
pub const MAX_DEPTH: u8 = 4;
/// Teto de delegações por `root_cause_id` por agente (orçamento/turno).
pub const DELEGATION_BUDGET: u32 = 8;
/// F1-3-6 (ADR 0019 §6): teto de spawns (`lina spawn`) por `(root_cause_id, agente)` por turno.
/// Mora AO LADO de [`DELEGATION_BUDGET`] (Maestro decisão 1: "onde `max_delegations` vive", sem
/// parser de `workspace.json`). Espelhado em `lina_bootstrap::Autonomy::max_spawns` p/ a doutrina.
pub const MAX_SPAWNS_PER_TURN: u32 = 2;
/// Acima de N alvos, um `broadcast` exige confirmação humana (gate de fan-out).
pub const FANOUT_GATE: usize = 3;
/// W3-7b (ADR 0002): teto de tempo (ms) de um `await` sem reply antes de fechar por Timeout (sweep
/// no `pump`). Default generoso — o reply é o caminho comum; o timeout só evita ticket órfão eterno.
pub const AWAIT_TIMEOUT_MS: u64 = 300_000;
/// W3-7 (§2.1, ajuste de COOPERAÇÃO): quantas vezes o MESMO salto `sender→target` pode reaparecer
/// DENTRO de um ciclo do mesmo `root_cause_id` antes de ser cortado como loop APERTADO. O diálogo
/// legítimo (A→B→A e algumas voltas) PASSA; só o ping-pong REPETIDO (A→B→A→B→A…) é barrado — antes
/// o grafo bloqueava já na 1ª volta (`LoopDetected`), quebrando a resposta (cooperação só num sentido).
/// Os backstops FINAIS do loop infinito continuam sendo `hops` (`MAX_DEPTH`) e o orçamento
/// (`DELEGATION_BUDGET`); este corta o ciclo apertado um pouco antes. `2` = o salto pode repetir 2×
/// num ciclo; a 3ª repetição que AINDA fecharia ciclo é `LoopDetected`.
pub const MAX_CYCLE_REVISITS: u32 = 2;
/// F3-1 (spec 52 §3, ponto [9]): teto DURO de voltas do OUTER loop de uma Goal por item. Esgotou
/// sem `Pass` → `GoalEscalated{turn_budget_exhausted}` (PAUSA resumível, NUNCA mais um turno). É a
/// defasagem-com-cap do ponto [9] e o ponto de escape do feedback negativo do ponto [8]. Faixa
/// canônica 1..=20 (a validação/configurabilidade via `lina params` é fatia de params, §1 tabela).
pub const GOAL_MAX_ITERATIONS: u32 = 3;
/// F3-1 (spec 52 §3, parse-failure breaker do Hermes D3): vereditos do juiz NÃO-PARSEÁVEIS seguidos
/// até `GoalEscalated{judge_unreliable}`. Distingue "instável por transporte" (reseta o contador,
/// fail-open) de "incapaz do contrato" (pausa). Default conservador 3.
pub const JUDGE_MAX_PARSE_FAILURES: u32 = 3;
/// F3-1 (spec 52 §3/§4, doc-fonte linha 12 "novo desenvolvedor com effort maior"): a ESCADA de
/// escalonamento de effort no `Fail`. Sobe um degrau por re-spawn; `Effort: Ord` torna "estritamente
/// maior" uma comparação, não string solta. Default `["normal","high"]` da spec = `[Medium, High]`.
/// `&'static` (não `Vec`) para [`RouterConfig`] seguir `Copy`; a configurabilidade dinâmica é porta
/// aberta (vira `Cow`/param quando `lina params` cobrir a escada).
pub const EFFORT_LADDER: &[Effort] = &[Effort::Medium, Effort::High];
/// F3-1 (spec 52 §3, gate d): `NodeId` SENTINELA do JUIZ ESTRUTURAL — o supervisor rodando
/// `CheckKind` determinístico (ZERO LLM). Nós reais nascem com `Uuid::now_v7()` (JAMAIS nil), então
/// `reviewer = STRUCTURAL_JUDGE` nunca colide com um `target` real → `reviewer != target` por
/// construção para o juiz automático (o executor nunca se auto-avalia).
const STRUCTURAL_JUDGE: NodeId = NodeId::nil();

/// ADR 0036: `NodeId` SENTINELA do GESTO HUMANO DIRETO via UI (o `by` de um confirm/ajuste vindo do
/// BOTÃO do card, não de um agente). NÃO é um nó do roster: a autenticação é a NATUREZA do canal
/// ([`Router::human_intent`], chamada LOCAL in-process — processo único, GPU-first; nenhum agente, que
/// entra por `route_message`→`node_by_name`, o produz). Determinístico (replay reconstrói idêntico),
/// distinto de [`STRUCTURAL_JUDGE`] (`nil`); como os nós reais nascem `Uuid::now_v7()`, este valor fixo
/// (os bytes soletram "HUMAN") nunca colide com um nó real. `by` = carimbo server-side, jamais do cliente.
const HUMAN_GESTURE: NodeId = NodeId::from_u128(0x4855_4d41_4e00_0000_0000_0000_0000_0000);

/// F3-CONF-2 (gate a): prefixo dos `SpawnRequested` de RE-DESPACHO anti-engolimento — DISTINTO de
/// `goal-respawn:`/`goal-newdev:` (escalonamento por `Fail`, [`count_respawns_with_effort`]), então as
/// duas escadas nunca se misturam na contagem. O id completo é `stall-respawn:{node}:{delivered_ts}`:
/// o `delivered_ts` (epoch da entrega) torna o re-despacho idempotente POR ENTREGA (uma re-injeção
/// abre um epoch novo; o gap entre o pedido e a re-injeção não re-escala).
const STALL_RESPAWN_PREFIX: &str = "stall-respawn";

/// F3-4 (pendência fechada na F3-5): `NodeId` SENTINELA do gatilho de AUTO-INTEGRAÇÃO. O auto-spawn do
/// DevOps integrador NÃO tem agente remetente — a decisão é do CORE (gatilho determinístico
/// [`crate::code::CodeIntegration`]), não de um terminal. Bytes soletram "AUTOINT"; distinto de
/// [`STRUCTURAL_JUDGE`] (`nil`) / [`HUMAN_GESTURE`]; como nós reais nascem `Uuid::now_v7()`, nunca
/// colide com um nó real. `requested_by` = carimbo server-side, JAMAIS forjável por agente (ADR 0007).
const INTEGRATOR_TRIGGER: NodeId = NodeId::from_u128(0x4155_544f_494e_5400_0000_0000_0000_0000);
/// Prefixo determinístico do `id` do `SpawnRequested` de auto-integração — sufixado pelo `seq` do
/// último `CodeChanged` (único por episódio; replay reconstrói idêntico). DISTINTO de `goal-respawn:`/
/// `stall-respawn:` → as escadas de spawn nunca se misturam na contagem.
const AUTO_INTEGRATE_PREFIX: &str = "auto-integrate";
/// Motivo do `SpawnGated` do auto-spawn do integrador: auto-organização que SUGERE (banner), nunca
/// aplica — o nascimento do integrador passa por aval humano (ADR 0007/0042), mesmo em `autonomo`
/// (criar um terminal sozinho é "aplicar"). O bin degrada `reason` desconhecido graciosamente
/// (`explain_spawn_gate` → "motivo: …"); a tradução pt-br fina do banner é costura do bin/UI.
const AUTO_INTEGRATE_REASON: &str = "auto_integrate";
/// Nome exibido do terminal integrador auto-spawnado (a unicidade no roster é do funil `admit_node`).
const INTEGRATOR_NAME: &str = "Integrador";

/// F3-CONF-2 (gate c): `reason` do `NodeStatusChanged{Idle}` emitido pelo RESET HUMANO do breaker.
/// Duplo papel: (1) registra durável o desbloqueio (a tela honesta volta de "pausado por segurança"
/// a ativo); (2) é o MARCADOR que ZERA a contagem de re-despachos de stall do nó — após o gesto
/// humano o protocolo recomeça do "1ª tentativa" (re-despacha), em vez de re-escalar de imediato a
/// entrega antiga ainda engolida. A AUTORIDADE do gesto é o CANAL ([`Router::reset_breaker`] é
/// chamado LOCAL in-process pela UI, como [`Router::human_intent`]; nenhum agente o alcança).
const BREAKER_RESET_REASON: &str = "breaker_reset";

/// Nível de autonomia do workspace (espelho leve do `lina_bootstrap::Autonomy` — o core NÃO
/// depende do bootstrap; o app traduz). Só `Manual` muda o roteamento (recusa delegação).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomyLevel {
    /// Não delega sozinho: recusa `ask`/`handoff`/`broadcast`.
    Manual,
    /// Propõe/dispara delegações (o gate humano vive no ponto de execução, não aqui).
    Assisted,
    /// Idem assistido para fins de roteamento.
    Autonomous,
}

impl AutonomyLevel {
    /// `true` se este nível BLOQUEIA delegações automáticas (só `Manual`).
    #[must_use]
    pub fn blocks_delegation(self) -> bool {
        matches!(self, AutonomyLevel::Manual)
    }
}

/// Configuração dos guardrails (defaults canônicos do design §4).
#[derive(Debug, Clone, Copy)]
pub struct RouterConfig {
    pub dedupe_window_ms: u64,
    pub max_depth: u8,
    pub delegation_budget: u32,
    /// F1-3-6 (ADR 0019 §6): teto de spawns por `(root_cause_id, agente)`/turno. Plugado AO LADO de
    /// `delegation_budget` (Maestro decisão 1). Enforçado em [`Router::handle_spawn`] (origem, pós-
    /// cascata). O nível por-workspace nasce de `Autonomy::max_spawns` — a fiação do `autonomy`
    /// REAL no `RouterConfig` (hoje `..default()` em `bridge.rs`) é o GAP escalado (decisão 2).
    pub max_spawns_per_turn: u32,
    pub fanout_gate: usize,
    pub autonomy: AutonomyLevel,
    /// W3-7b: teto de tempo de um `await` sem reply (fecha por `Timeout` no sweep do `pump`).
    pub await_timeout_ms: u64,
    /// W3-7c (§2.2): teto de tokens por workspace/dia. `0` = DESLIGADO (sem teto). Ao a soma da
    /// janela contábil atingir este valor, o workspace entra em `Paused` (gate humano via `lina
    /// resume`, não kill — invariante #6).
    pub token_budget_day: u64,
    /// F1-0-3 (ADR 0019 §1): cadência de amostragem do heartbeat de progresso, em ms (2 min).
    pub heartbeat_sample_ms: u64,
    /// F1-0-3 (ADR 0019 §3): amostras consecutivas sem progresso em `Busy` → WARN `NodeStalled`.
    pub stall_warn_samples: u32,
    /// F1-0-3 (ADR 0019 §3): amostras até o circuit breaker (pausa-com-gate; a AÇÃO é F1-0-7 —
    /// compare com `LifecycleEngine::stall_samples`).
    pub stall_breaker_samples: u32,
    /// F1-0-4 (P1): teto da RETENÇÃO por alvo `Busy`, em ms. F1-0-7 (ADR 0020): estourou
    /// → a mensagem vai à **DLQ** com motivo (antes ia "entrega mesmo assim"; o desvio
    /// prometido aterrissou). 13.11 §P0: timeout explícito, nunca espera infinita — é
    /// também o que garante que retenção jamais cria espera circular.
    pub retention_timeout_ms: u64,
    /// F1-0-7: tentativas de ENTREGA (PTY) por mensagem antes da DLQ. 1-based.
    pub delivery_max_attempts: u32,
    /// F1-0-7: base do backoff exponencial entre tentativas (ms): base·2^(attempt-1) +
    /// jitter determinístico por id (anti thundering-herd, reprodutível em teste).
    pub delivery_backoff_base_ms: u64,
    /// F1-0-7: falhas CONSECUTIVAS de entrega num nó até abrir o circuit breaker
    /// (`CircuitOpened` + `Blocked`). Anti retry-storm (13.11: "MANDATÓRIO").
    pub breaker_threshold: u32,
    /// F3-1 (spec 52 §3): teto duro de voltas do OUTER loop de uma Goal por item (ver
    /// [`GOAL_MAX_ITERATIONS`]).
    pub goal_max_iterations: u32,
    /// F3-1 (spec 52 §3): vereditos não-parseáveis seguidos até `judge_unreliable` (ver
    /// [`JUDGE_MAX_PARSE_FAILURES`]).
    pub judge_max_parse_failures: u32,
    /// F3-1 (spec 52 §3/§4): escada de effort do escalonamento no `Fail` (ver [`EFFORT_LADDER`]).
    pub effort_ladder: &'static [Effort],
}

/// Default do teto de retenção por alvo ocupado: **10 min** (CALIBRADO — ADR 0020
/// §Calibração). Reter atrás de um turno real é o comportamento DESEJADO ("entrega
/// quando ficar livre"), e um turno de claude leva MINUTOS: a observação de 14h mediu
/// retornos espaçados em **200–600 s** (DIRECIONAMENTO §P1); probes triviais, 3–16 s.
/// O default original (30 s) dead-letterava espera LEGÍTIMA sob rajada (achado MÉDIO do
/// medidor) — matava o propósito da retenção. 600 s cobre o teto observado dos turnos
/// reais e MANTÉM o papel anti-deadlock (finito; estourou → DLQ visível, retry manual).
/// Tunável por workspace via `RouterConfig::retention_timeout_ms`. Revisitar quando o
/// dashboard (F1-1-5) der tempos REAIS de resposta por agente.
pub const RETENTION_TIMEOUT_MS: u64 = 600_000;
/// F1-0-7: default de tentativas de entrega antes da DLQ.
pub const DELIVERY_MAX_ATTEMPTS: u32 = 5;
/// F1-0-7: default da base do backoff exponencial (1 s → 1,2,4,8,16 s + jitter).
pub const DELIVERY_BACKOFF_BASE_MS: u64 = 1_000;
/// F1-0-7: default do limiar do circuit breaker (falhas consecutivas por nó).
pub const BREAKER_THRESHOLD: u32 = 5;
/// **ACHADO-2:** backoff entre re-tentativas de uma entrega RETIDA por prompt-não-pronto (alvo
/// ocupado). Throttla o re-`wait_ready` (cada re-tentativa custa ~`ready_timeout`): sem ele, o
/// re-drain re-rodaria `wait_ready` a cada tick (busy-loop + head-of-line blocking). 2 s ≈ um
/// orçamento de prontidão — re-checa com folga, sem martelar. Backstop final: `retention_timeout`
/// (600 s) → DLQ (o alvo nunca ficou pronto = mensagem morta de verdade).
pub const NOT_READY_BACKOFF_MS: u64 = 2_000;

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            dedupe_window_ms: DEDUPE_WINDOW_MS,
            max_depth: MAX_DEPTH,
            delegation_budget: DELEGATION_BUDGET,
            max_spawns_per_turn: MAX_SPAWNS_PER_TURN,
            fanout_gate: FANOUT_GATE,
            autonomy: AutonomyLevel::Assisted,
            await_timeout_ms: AWAIT_TIMEOUT_MS,
            token_budget_day: 0, // desligado por padrão (opt-in por workspace)
            heartbeat_sample_ms: crate::lifecycle::HEARTBEAT_SAMPLE_MS,
            stall_warn_samples: crate::lifecycle::STALL_WARN_SAMPLES,
            stall_breaker_samples: crate::lifecycle::STALL_BREAKER_SAMPLES,
            retention_timeout_ms: RETENTION_TIMEOUT_MS,
            delivery_max_attempts: DELIVERY_MAX_ATTEMPTS,
            delivery_backoff_base_ms: DELIVERY_BACKOFF_BASE_MS,
            breaker_threshold: BREAKER_THRESHOLD,
            goal_max_iterations: GOAL_MAX_ITERATIONS,
            judge_max_parse_failures: JUDGE_MAX_PARSE_FAILURES,
            effort_ladder: EFFORT_LADDER,
        }
    }
}

/// Resultado de rotear UMA mensagem — explícito (sem silenciar): cada recusa diz o porquê.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    /// Roteada e entregue aos alvos.
    Delivered { targets: Vec<NodeId> },
    /// `id` já visto na janela de dedupe — ignorada.
    Duplicate,
    /// `hops` excedeu `max_depth` (anti-loop por profundidade).
    HopLimit,
    /// O remetente (`from`) não está no roster vivo.
    UnknownSender(String),
    /// Nenhum alvo vivo resolveu (`to` inexistente/morto, ou só o próprio remetente).
    NoTarget,
    /// Autonomia `manual`: delegação recusada.
    BlockedByAutonomy,
    /// `broadcast` para mais de `fanout_gate` alvos — pede confirmação humana (não entregue).
    FanoutGated { count: usize },
    /// Orçamento de delegação por `root_cause_id`/agente estourado.
    BudgetExceeded,
    /// `--await` fecharia um ciclo no wait-for-graph (deadlock).
    Deadlock,
    /// W3-7 (§2.1): adicionar `sender→target` fecharia um CICLO no grafo de handoffs do mesmo
    /// `root_cause_id` (A→B→A por mensagens novas) — ortogonal a `hops`/`trace`.
    LoopDetected,
    /// W3-7c (§2.2): o teto de custo (`token_budget_day`) foi atingido — o workspace está `Paused`.
    /// A delegação NÃO é entregue; é um GATE (humano confirma com `lina resume --confirm`), não kill.
    /// DISTINTO de `BudgetExceeded` (orçamento de DELEGAÇÃO por `root_cause_id`); este é o teto de TOKENS.
    CostCeiling,
    /// A entrega faseada falhou (PTY do alvo).
    DeliveryFailed(String),
    /// Falha ao persistir o roteamento no event log (não entregamos o que não logamos).
    PersistFailed(String),
    /// W3-5: intent de plano (`plan.claim`/`plan.check`) aplicado ao `plan.md` + logado — NÃO
    /// entregue a nenhuma PTY (não é delegação A2A).
    PlanApplied { intent: String, item: String },
    /// W3-5: intent de plano recusado pela regra do plano (item inexistente, conflito de owner,
    /// não-owner, ou `ref` ausente) — sem corromper o `plan.md`.
    PlanRejected(String),
    /// F3-0-5: `params.set`/`params.reset` aplicado — `SystemParamsChanged` logado (a config é
    /// PROJEÇÃO do log; ver `resolve_from_store`). NÃO entregue a PTY (verbo de configuração).
    ParamsApplied { intent: String, key: String },
    /// F3-0-5: comando de params recusado (payload inválido, valor fora da faixa, intent ou chave
    /// desconhecida) — nada logado, sem efeito.
    ParamsRejected(String),
    /// F3-0-7 §C: `params set` que ENFRAQUECE um laço balanceador (reduz abaixo do default) — NÃO
    /// aplica; loga `ActionGated{gated-hard, ask}` e sobe à fila de atenção p/ o humano confirmar.
    ParamsGated { key: String },
    /// F3-0-5: `effort.assign` aplicado — `EffortAssigned{origin:assigned}` logado para o nó alvo.
    EffortApplied { node: NodeId, effort: Effort },
    /// F3-0-5: `effort.assign` recusado (payload inválido, alvo inexistente, effort fora do
    /// vocabulário) — nada logado, sem efeito.
    EffortRejected(String),
    /// F3-1 (spec 52 §Superfície): verbo de Goal aplicado (`goal.define`/`goal.interpret`/
    /// `goal.confirm`) — evento da Goal logado. NÃO entregue a PTY (verbo estruturado, como plan/params).
    GoalApplied { intent: String, goal_id: String },
    /// F3-1: verbo de Goal recusado (payload inválido, campo obrigatório ausente, intent desconhecido)
    /// — nada logado, sem efeito. A `String` é o erro LEGÍVEL para o agente corrigir.
    GoalRejected(String),
    /// F3-3 (M-DETECTOR, spec 35 §4.1): sentinela `[LINA::CORRECTION]` no payload captada —
    /// `CorrectionObserved` logado com o `role` do SENDER autenticado (server-side, JAMAIS do
    /// payload — regra-mãe ADR 0007). NÃO entregue a PTY: é captação de aprendizado, não delegação.
    /// O `role` no outcome é o carimbado (observabilidade/teste prova que veio do binding, não do texto).
    CorrectionObserved { role: String },
    /// W4-3 (freio do rodapé): a orquestração está PAUSADA → esta delegação foi ENFILEIRADA (retida
    /// no `.inflight`, durável), NÃO injetada. Drena ao [`Router::resume`]. É GATE humano, não kill
    /// (inv #6): nada se perde, o estado fica salvo e visível.
    Queued,
    /// FIX #22 (backlog): PING DE PRESENÇA (`lina handshake`, `intent=handshake`, `to=*`) reconhecido e
    /// absorvido. Fire-and-forget: o roster vivo é do Supervisor (events.rs:141), então esta msg NÃO
    /// delega, NÃO entrega a ninguém (0 broadcast) e — crucialmente — NÃO loga `RouteBlocked`. DISTINTO
    /// de `UnknownSender("")`: o drain FLAT do handshake anonimiza `from → ""`, e antes deste desfecho a
    /// presença morria como erro espúrio no log (ruído que PARECIA falha, mas é o caminho esperado).
    Presence,
    /// F1-0-5 (`lina/msg@2`): envelope recusado pela validação DE FORMA — intent fora do
    /// enum canônico, `reply` sem `reply_to`, schema desconhecido, ou handoff sem o
    /// contrato completo. A `String` é o erro LEGÍVEL que nomeia o campo, para o agente
    /// corrigir e reenviar (a recusa também vai ao log: `RouteBlocked{invalid_intent|
    /// invalid_contract}` — ADR 0003, o log é o livro-razão das recusas).
    ContractRejected(String),
    /// F1-0-4 (P1 — entrega ciente de estado): o alvo está `Busy` → a mensagem foi RETIDA
    /// no `.inflight` (durável; o pump NÃO ackeia) e será re-tentada a cada tick até o
    /// alvo voltar a Idle/Ready, até o teto `retention_timeout_ms` (F1-0-7: estourou →
    /// DLQ). NÃO é erro: é o fix do atropelamento (baseline F1-0: 15/15 retornos <2s ao
    /// mesmo alvo).
    Retained { to: NodeId },
    /// F1-0-7 (ADR 0020): a tentativa de entrega FALHOU e a mensagem está agendada para
    /// retry com backoff exponencial+jitter (`MessageDeliveryFailed{next_retry_ms}` no
    /// log). Segue durável no `.inflight` (o pump NÃO ackeia); esgotadas
    /// `delivery_max_attempts` → DLQ.
    RetryBackoff { to: NodeId, attempt: u32 },
    /// F1-0-7 (ADR 0020): a mensagem foi movida à **DLQ** (`<.lina>/dead-letter/`) com o
    /// motivo legível — desfecho DEFINITIVO (o pump ackeia o `.inflight`; o arquivo da
    /// DLQ persiste para retry MANUAL). Nada some em silêncio.
    DeadLettered { reason: String },
    /// F1-3-6 (ADR 0019 §6): `lina spawn` de ORIGEM APROVADO pelo gate (sob cap, custo ok, não
    /// manual). O CORE só DECIDE — a criação física (PTY + `Supervisor::register` + bootstrap +
    /// `NodeAdded`) é do APP via o funil `admit_node` (ADR 0022), seam SEPARADO casado à tela
    /// (Maestro decisão 4). Nesta rodada é verificado como VALOR DE RETORNO (o nó NÃO é criado).
    SpawnApproved {
        name: String,
        role: String,
        prompt: String,
        requested_by: NodeId,
        root_cause_id: String,
        /// F3-0-3 (ADR 0031): modelo/effort PEDIDOS para o novo terminal (preferência de execução,
        /// não autoridade). O app os carimba no PTY (ENV) + `EffortAssigned` em F3-0-4; o caminho de
        /// spawn-via-agente (`handle_spawn`) ainda os deixa `None` (o verbo CLI que os passa é F3-0-5).
        model: Option<String>,
        effort: Option<Effort>,
    },
    /// F1-3-6 (ADR 0019 §6): o spawn precisa de CONFIRMAÇÃO HUMANA — `reason` ∈
    /// {`cascade`,`over_cap`,`cost`} (cascata SEMPRE gateia; cap estourado; workspace cost-paused).
    /// O app mostra um banner pt-br; NADA é criado sem o gesto humano. DISTINTO de `SpawnBlocked`
    /// (recusa terminal por autonomia `manual`).
    SpawnGated { reason: String },
    /// F1-3-6 (ADR 0019 §6): spawn RECUSADO pela autonomia `manual` (paridade com
    /// `BlockedByAutonomy` do handoff) — sem banner de confirmação (o humano deve subir a
    /// autonomia). O EVENTO `SpawnGated{reason:"manual"}` registra o fato no log.
    SpawnBlocked,
}

/// Erro de uma operação de plano feita DIRETO pelo supervisor (semeadura via `seed_plan_*`):
/// validação ([`PlanError`]) + persistência ([`StoreError`]) + escrita do arquivo (I/O).
#[derive(Debug, Error)]
pub enum PlanOpError {
    #[error("plano: {0}")]
    Plan(#[from] PlanError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// `true` se o `intent` é uma delegação (sujeita ao gate de autonomia). `handshake`/`status` não
/// delegam trabalho.
fn is_delegation(intent: &str) -> bool {
    matches!(intent, "ask" | "handoff" | "broadcast" | "review" | "")
}

/// Entrada do orçamento de delegação: contagem + carimbo (ms) do último toque. O carimbo habilita a
/// poda temporal (A5), simétrica à de `seen`, para o mapa não crescer sem limite numa sessão longa.
#[derive(Clone, Copy, Default)]
struct BudgetEntry {
    count: u32,
    /// F1-3-6 (ADR 0019 §6): contagem de SPAWNS (`lina spawn`) desta cadeia/agente — SEPARADA de
    /// `count` (delegação budget 8 vs spawn cap 2: semânticas distintas). Reusa a MESMA chave
    /// `(root_cause_id, sender)` e a poda temporal A5 de `prune_expired` (evita um 2º mapa).
    spawn_count: u32,
    last_ms: u64,
}

/// W3-7b (ADR 0002): ticket em memória de um `await` aberto. Guarda o necessário para FECHAR
/// (`waiter` → `end_await`; `opened_ms` → timeout) E para AUTENTICAR o fechamento: `target` é o nó
/// que o waiter aguarda — só um reply VINDO desse `target` (sender já resolvido pelo supervisor)
/// pode fechar o ticket. Sem `target` aqui, o `reply_to` (campo do outbox, público no log) decidiria
/// o await de outro nó (Round 4: confused-deputy). O contexto completo segue durável no `AwaitOpened`.
#[derive(Clone, Copy)]
struct AwaitTicket {
    waiter: NodeId,
    target: NodeId,
    opened_ms: u64,
}

/// **Roteador do supervisor.** Detém o `Arc<Supervisor>` (roster + filas serial + pub/sub) e o
/// estado de guardrail (dedupe/orçamento). Single-thread por design (escritor único de `.lina/`).
pub struct Router {
    sup: Arc<Supervisor>,
    mailbox: Mailbox,
    config: RouterConfig,
    /// `id` → carimbo (ms) da primeira vez que vimos — dedupe por janela.
    seen: HashMap<String, u64>,
    /// `(root_cause_id, remetente)` → orçamento por turno (contagem + carimbo p/ poda temporal).
    budget: HashMap<(String, NodeId), BudgetEntry>,
    /// W3-7 (A2): nó → `(root_cause_id, hops, last_ms)` da última mensagem ENTREGUE a ele. Quando
    /// esse nó depois submete uma msg com `root=None`, o supervisor DERIVA o root (herdado) e
    /// `hops = +1` daqui — o enforcement de cadeia vem do binding do supervisor, não do campo que a
    /// CLI não preenche. Round 5 #8: `last_ms` habilita MIGRAÇÃO/PODA — um binding OCIOSO (stale >
    /// janela, sem await pendente) deixa de congelar a vítima no 1º root e passa a seguir a cadeia
    /// ATIVA (anti-loop não fragmenta por sessão longa), SEM reabrir a forja (um binding fresco de
    /// root diferente NUNCA é sobrescrito — defesa P1).
    delivered_root: HashMap<NodeId, (String, u8, u64)>,
    /// W3-7b (ADR 0002): `id` da pergunta → ticket do `await` aberto. Fechado pelo `reply_to` casado
    /// (`Replied`) ou pelo sweep de timeout (`Timeout`) — em ambos, `end_await` libera o wait-for-graph.
    pending: HashMap<String, AwaitTicket>,
    /// W4-3 (freio do rodapé): `true` = orquestração pausada. Delegações novas ENFILEIRAM (retidas no
    /// `.inflight`, duráveis) em vez de injetar, drenando ao [`Router::resume`] — GATE humano, não
    /// kill (inv #6). Escritor único (o Router é single-thread, dono do estado de roteamento).
    paused: bool,
    /// **RETRY de bloqueio TRANSIENTE** (race de inicialização): `id` → nº de ticks que a msg ficou
    /// retida por `UnknownSender`/`NoTarget`. Na criação de terminais, um agente pode mandar `lina ask`
    /// ANTES de o remetente/destino entrar no roster vivo (o PTY sobe e roda o hook antes/junto do
    /// `register`); a msg era ACKEADA e PERDIDA. Agora é RETIDA no `.inflight` e re-tentada por até
    /// [`MAX_TRANSIENT_RETRIES`] ticks — quando o nó aparece, ENTREGA. Esgotado o teto, descarta (não
    /// fica para sempre se o destino realmente não existe). NÃO afrouxa segurança: o `from` segue
    /// autenticado pela origem (subdir) e o `route_message` é idêntico — só a decisão de ack/reter muda.
    transient_retries: HashMap<String, u32>,
    /// F1-0-4 (P1): `id` → carimbo (ms) da PRIMEIRA retenção por alvo ocupado. Dupla função:
    /// emite o `MessageRetained` só 1× (anti-amplificação A4) e cronometra o teto
    /// (`retention_timeout_ms`) — estourou, F1-0-7 manda à DLQ.
    retained_since: HashMap<String, u64>,
    /// F1-0-7: `id` → estado de retry de ENTREGA (pós-falha de PTY). Hot-path em memória;
    /// a DURABILIDADE vem dos eventos `MessageDeliveryFailed` no log (um restart re-deriva
    /// attempt/next_retry de lá — ver o ledger no `pump`).
    retries: HashMap<String, RetryState>,
    /// F1-0-7: falhas CONSECUTIVAS de entrega por nó (zera no sucesso). Ao cruzar
    /// `breaker_threshold` → `CircuitOpened` + `Blocked` + entra em `breaker_open`.
    node_failures: HashMap<NodeId, u32>,
    /// F1-0-7: nós com circuit breaker ABERTO (distinto de `Blocked` por custódia/W3-6 —
    /// não conflamos as semânticas). Reset: ator externo tira o nó de `Blocked`
    /// (lifecycle/humano) → detectado e limpo no veredito; ou o gesto humano explícito
    /// ([`Router::reset_breaker`]). O breaker de ENGOLIMENTO (F3-CONF-2) reusa este mesmo conjunto.
    breaker_open: HashSet<NodeId>,
    /// **F3-CONF-2: o vigia de despachos ENGOLIDOS, no relógio do router (`now_ms`).** `node →
    /// (delivered_at_ms, arm_seq)`: o instante LÓGICO da entrega mais nova ao nó (re-armado a cada
    /// entrega) + o `seq` do `MessageDelivered` que a armou. O `now_ms` (não o `ts` wall-clock do
    /// evento) é a base da janela — alinhado ao relógio que o app dirige o `pump`/`route_message`,
    /// e DETERMINÍSTICO em teste (o store carimba wall-clock; o protocolo não pode depender disso).
    /// O `arm_seq` GUARDA o disarm: só progresso APÓS a arma (`seq > arm_seq`) desarma — uma re-arma
    /// não é desfeita por um sinal de progresso ANTERIOR a ela (anti-corrida de ordenação). Disarm
    /// em [`Router::sync_dispatch_progress`] (token/output/delegou/morte). Hot-path O(delta)/tick.
    dispatched_at: HashMap<NodeId, (u64, u64)>,
    /// F3-CONF-2: maior `seq` já varrido por [`Router::sync_dispatch_progress`] — disarm incremental
    /// (delta only; NUNCA full-replay — o `O(N)` por tick foi a causa-raiz documentada do freeze).
    dispatch_progress_seq: u64,
    /// F3-4 (auto-spawn do integrador): `now_ms` do último check do gatilho de auto-integração. O
    /// gatilho ([`CodeIntegration`]) exige um `from_records` (replay) — caro por tick. Como ele só
    /// dispara após ≥ [`INTEGRATOR_SETTLE_MS`] de silêncio da equipe, throttlar o check a esse mesmo
    /// intervalo NÃO perde disparos (a condição persiste) e mantém o pump fora do `O(N)`/tick (a
    /// causa-raiz do freeze). `0` = nunca checado (o 1º tick com `now_ms` real cruza o intervalo).
    last_integrator_check_ms: u64,
}

/// **F1-0-7 — backoff exponencial com jitter DETERMINÍSTICO** por (id, attempt):
/// `base·2^(attempt-1)` (cap em 2^6) + `hash(id,attempt) % (exp/4 + 1)`. Determinístico =
/// reprodutível em teste E ainda anti-thundering-herd (mensagens distintas têm fases
/// distintas). Sem relógio/aleatoriedade de SO — coerente com replay (invariante #4).
fn backoff_with_jitter(base_ms: u64, attempt: u32, id: &str) -> u64 {
    let exp = base_ms.saturating_mul(1u64 << attempt.saturating_sub(1).min(6));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h ^= u64::from(attempt);
    h = h.wrapping_mul(0x0100_0000_01b3);
    exp + (h % (exp / 4 + 1))
}

/// F1-0-7: fallback p/ DLQ quando o arquivo original do `.inflight` não está legível
/// (nunca perder o REGISTRO do descarte por falta do corpo).
fn minimal_msg_for_dlq(id: &str, to: &str) -> MailMessage {
    let mut msg = MailMessage::new("", to, "ask", "(corpo original ilegivel no .inflight)");
    msg.id = id.to_string();
    msg
}

/// F1-0-4/F1-0-7: decisão do veredito de retenção para um destino single-node.
enum Verdict {
    /// Sem retenção — o pipeline segue para os guardrails/entrega.
    Pass,
    /// NÃO injete agora: alvo `Busy` (`target_busy`) ou breaker aberto (`circuit_open`).
    Hold(NodeId, &'static str),
    /// A retenção estourou o teto → mover à DLQ (F1-0-7/ADR 0020).
    DeadLetter(NodeId),
}

/// F1-0-7: estado de uma mensagem aguardando retry de entrega (backoff).
struct RetryState {
    from: NodeId,
    target: NodeId,
    /// Bloco `[LINA::MSG]` já renderizado na rota original (root/hops idênticos).
    block: String,
    /// Para o binding A2 na entrega bem-sucedida (mesma semântica da rota original).
    root: String,
    hops: u8,
    attempts: u32,
    next_ms: u64,
}

/// Teto de re-tentativas de um bloqueio TRANSIENTE (`UnknownSender`/`NoTarget`) antes de desistir.
/// Cobre a janela de registro na criação de nós; após isso, um destino ausente é descartado (anti-loop).
const MAX_TRANSIENT_RETRIES: u32 = 50;

impl Router {
    /// Roteador com a configuração padrão.
    #[must_use]
    pub fn new(sup: Arc<Supervisor>, mailbox: Mailbox) -> Self {
        Self::with_config(sup, mailbox, RouterConfig::default())
    }

    /// Roteador com configuração customizada.
    #[must_use]
    pub fn with_config(sup: Arc<Supervisor>, mailbox: Mailbox, config: RouterConfig) -> Self {
        Self {
            sup,
            mailbox,
            config,
            seen: HashMap::new(),
            budget: HashMap::new(),
            delivered_root: HashMap::new(),
            pending: HashMap::new(),
            paused: false,
            transient_retries: HashMap::new(),
            retained_since: HashMap::new(),
            retries: HashMap::new(),
            node_failures: HashMap::new(),
            breaker_open: HashSet::new(),
            dispatched_at: HashMap::new(),
            dispatch_progress_seq: 0,
            last_integrator_check_ms: 0,
        }
    }

    /// A mailbox observada.
    #[must_use]
    pub fn mailbox(&self) -> &Mailbox {
        &self.mailbox
    }

    /// `true` se a orquestração está pausada pelo freio (W4-3).
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// **W4-3 (freio do rodapé): pausa a auto-orquestração.** Enquanto pausada, delegações novas
    /// ENFILEIRAM (ficam retidas no `.inflight`, duráveis) em vez de injetar — é GATE humano, não
    /// kill (inv #6): nada se perde, o estado fica salvo e visível. Apenda `OrchestrationPaused` SÓ
    /// na transição (idempotente: re-pausar não re-loga — anti-amplificação A4). Devolve `true` se
    /// transicionou (estava ativo); `false` se já pausado.
    ///
    /// # Errors
    /// Falha ao persistir `OrchestrationPaused` no event log.
    pub fn pause(&mut self, store: &mut EventStore) -> Result<bool, StoreError> {
        if self.paused {
            return Ok(false);
        }
        store.append(&DomainEvent::OrchestrationPaused)?;
        self.paused = true;
        Ok(true)
    }

    /// **W4-3 (freio do rodapé): retoma a auto-orquestração e DRENA a fila.** Apenda
    /// `OrchestrationResumed` (só na transição) e re-roda o [`Router::pump`], entregando as
    /// delegações que ficaram represadas no `.inflight` sob o freio. `deliver` é a entrega faseada
    /// (como em [`Router::pump`]). Devolve os resultados do dreno (observabilidade); vazio se não
    /// estava pausado.
    ///
    /// # Errors
    /// Falha ao persistir `OrchestrationResumed` no event log.
    pub fn resume<D>(
        &mut self,
        store: &mut EventStore,
        now_ms: u64,
        deliver: D,
    ) -> Result<Vec<(String, RouteOutcome)>, StoreError>
    where
        D: FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String>,
    {
        if !self.paused {
            return Ok(Vec::new());
        }
        store.append(&DomainEvent::OrchestrationResumed)?;
        self.paused = false;
        Ok(self.pump(store, now_ms, deliver))
    }

    /// **W4-3: reconstrói o estado do freio (`paused`) a partir do event log** — o log é a fonte da
    /// verdade (inv #4). Chame na RECUPERAÇÃO pós-crash, antes do 1º `pump`: sem isto, um `Router`
    /// novo nasce despausado e drenaria a fila que o humano havia represado (inv #6: estado sempre
    /// salvo e visível). `paused` segue o ÚLTIMO evento `OrchestrationPaused`/`OrchestrationResumed`
    /// do log (mesmo padrão event-sourced do [`CostLedger`]).
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn restore_orchestration_state(&mut self, store: &EventStore) -> Result<(), StoreError> {
        let mut paused = false;
        for rec in store.events()? {
            match rec.kind.as_str() {
                "OrchestrationPaused" => paused = true,
                "OrchestrationResumed" => paused = false,
                _ => {}
            }
        }
        self.paused = paused;
        Ok(())
    }

    /// **F3-4 (pendência fechada na F3-5): auto-spawn do DevOps integrador.** Fia o gatilho
    /// determinístico ÓRFÃO ([`CodeIntegration::integrator_dispatch`]) ao caminho de spawn governado.
    /// Verificação time-based chamada no tick do [`Self::pump`]: quando a equipe ASSENTOU (todos
    /// Idle/Dead há ≥ [`INTEGRATOR_SETTLE_MS`]) E há branch não-integrada E nenhum pedido de integrador
    /// está EM VOO (idempotência event-sourced, [`integrator_request_in_flight`]), apenda
    /// `SpawnRequested{role:DEVOPS}` (intent-vs-action: o pedido no log SEMPRE, antes da decisão) +
    /// `SpawnGated{auto_integrate}`. O nascimento passa por aval humano — a fila de atenção projeta o
    /// banner do `SpawnGated` no log (não do retorno); auto-organização SUGERE, nunca aplica.
    ///
    /// `Ok(Some(SpawnGated))` quando propôs; `Ok(None)` quando nada a fazer. A DECISÃO é PURA de
    /// `now_ms` ([`plan_integrator_spawn`]) → o replay reproduz o disparo idêntico (gate f/h).
    ///
    /// # Errors
    /// Falha ao ler ou apendar no event log.
    pub fn dispatch_integrator_if_due(
        &mut self,
        store: &mut EventStore,
        now_ms: u64,
    ) -> Result<Option<RouteOutcome>, StoreError> {
        let events = plan_integrator_spawn(&store.events()?, now_ms);
        if events.is_empty() {
            return Ok(None);
        }
        for event in &events {
            store.append(event)?;
        }
        Ok(Some(RouteOutcome::SpawnGated {
            reason: AUTO_INTEGRATE_REASON.to_string(),
        }))
    }

    /// **Drena a mailbox e roteia cada mensagem.** `deliver(target, from, text)` faz a entrega
    /// faseada de fato (em produção: [`crate::deliver_a2a`] com o perfil/grid do alvo). Devolve
    /// `(id, resultado)` por mensagem (observabilidade).
    pub fn pump<D>(
        &mut self,
        store: &mut EventStore,
        now_ms: u64,
        mut deliver: D,
    ) -> Vec<(String, RouteOutcome)>
    where
        D: FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String>,
    {
        // W3-7b (ADR 0002): fecha tickets de `await` vencidos (Timeout) antes de processar o tick.
        self.sweep_await_timeouts(store, now_ms);
        // F3-CONF-2 (gate a): o protocolo ANTI-ENGOLIMENTO, manutenção temporal como o sweep acima.
        // Desarma os vigias por PROGRESSO (delta do log — NUNCA full-replay) e CONSOME o engolido:
        // re-despacho informado 1×, depois breaker sticky + escala ao humano. Falha de I/O é VISÍVEL
        // (stderr), nunca silenciosa; o tick segue (um log lento não derruba a auto-orquestração).
        if let Err(e) = self.sync_dispatch_progress(store) {
            eprintln!("lina-core: pump não desarmou vigias por progresso (retentará): {e}");
        }
        if let Err(e) = self.enforce_dispatch_protocol(store, now_ms) {
            eprintln!("lina-core: pump falhou no protocolo anti-engolimento (retentará): {e}");
        }
        // F3-4 (pendência fechada na F3-5): auto-spawn do DevOps integrador. Verificação time-based como
        // os sweeps acima — quando a equipe assentou com branch pendente, PROPÕE o integrador (gated,
        // aval humano; o banner vem da projeção do `SpawnGated` no log). THROTTLADA ao intervalo de
        // assentamento: o check exige um replay (`CodeIntegration::from_records`), e rodá-lo por tick
        // reintroduziria o `O(N)`/tick que causou o freeze — como o gatilho só dispara após ≥ SETTLE de
        // silêncio, checar a esse passo não perde disparos. Falha de I/O é VISÍVEL (stderr), nunca
        // silenciosa; o tick segue (um log lento não derruba a auto-organização).
        if now_ms.saturating_sub(self.last_integrator_check_ms) >= INTEGRATOR_SETTLE_MS {
            self.last_integrator_check_ms = now_ms;
            if let Err(e) = self.dispatch_integrator_if_due(store, now_ms) {
                eprintln!(
                    "lina-core: pump não avaliou o auto-spawn do integrador (retentará): {e}"
                );
            }
        }
        // A6: drena de forma RECUPERÁVEL — move para `.inflight` e reprocessa órfãos de um crash.
        let msgs = self.mailbox.drain_to_inflight().unwrap_or_else(|e| {
            // Falha de I/O ao listar o outbox (disco/permissão): as mensagens NÃO são perdidas
            // (ficam no outbox/.inflight para o próximo tick), mas a falha é VISÍVEL, nunca silenciosa.
            eprintln!("lina-core: pump não conseguiu drenar a mailbox (será retentado): {e}");
            Vec::new()
        });
        // D1: dedupe DURÁVEL (à prova de restart E de ack-falho). O `seen` em memória zera no restart
        // e é podado por tempo; sem isso, um órfão em `.inflight` (crash entre `MessageRouted`+entrega
        // ao PTY e o `ack`) seria RE-ENTREGUE (2ª injeção irreversível). Antes de (re)processar,
        // F1-0-7: retries de ENTREGA devidos rodam ANTES do drain (não re-rodam o
        // pipeline — guardrails/persist já aconteceram na rota original).
        self.process_due_retries(store, now_ms, &mut deliver);

        // F1-0-7 (ADR 0020): o dedupe DURÁVEL virou um LEDGER de entrega derivado do log —
        // distingue "roteada+entregue/DLQ" (no-op com resultado conhecido) de "roteada com
        // entrega FALHA pendente" (retoma o retry do agendamento DURÁVEL — um restart não
        // re-injeta nem dropa). Uma varredura por tick, só quando há o que processar.
        let ledger: HashMap<String, LedgerEntry> = if msgs.is_empty() {
            HashMap::new()
        } else {
            delivery_ledger(store)
        };
        msgs.into_iter()
            .map(|m| {
                // F1-0-7: msg com retry em curso NESTE processo → segura (o motor de retry/retenção
                // é quem entrega; o arquivo segue durável no `.inflight`). Este check é PRÉ-LEDGER:
                // intercepta ANTES do "roteada-sem-entrega ⇒ assume entregue" — é o que torna a
                // retenção por not-ready (ACHADO-2) durável sem ser dropada pelo ledger.
                if let Some(r) = self.retries.get(&m.id) {
                    // ACHADO-2: `attempts==0` = entrada de RETENÇÃO (prompt não-pronto), não falha →
                    // reporta `Retained` (não `RetryBackoff`); ambos são `hold` (não ackeiam).
                    let out = if r.attempts == 0 {
                        RouteOutcome::Retained { to: r.target }
                    } else {
                        RouteOutcome::RetryBackoff {
                            to: r.target,
                            attempt: r.attempts,
                        }
                    };
                    return (m.id, out);
                }
                match ledger.get(&m.id) {
                    // Entregue ou já na DLQ → no-op com resultado conhecido (ack).
                    Some(l) if l.delivered || l.dead => {
                        if let Err(e) = self.mailbox.ack_inflight(&m.id) {
                            eprintln!("lina-core: pump falhou ao confirmar inflight {}: {e}", m.id);
                        }
                        return (m.id, RouteOutcome::Duplicate);
                    }
                    // Roteada com FALHA de entrega pendente (restart no meio do backoff):
                    // re-deriva o retry do log — attempt e agendamento DURÁVEIS.
                    Some(l) if l.failed.is_some() => {
                        let (attempt, next_ms) = l.failed.unwrap_or((0, now_ms));
                        let Some(target) = l.to_node else {
                            // Sem alvo single-node reconstruível → conservador (D1): ack.
                            let _ = self.mailbox.ack_inflight(&m.id);
                            return (m.id, RouteOutcome::Duplicate);
                        };
                        let Some(from) = self.sup.node_by_name(&m.from) else {
                            // Remetente saiu do roster — não dá para re-entregar com
                            // atribuição correta: DLQ com motivo legível (nada some).
                            let out = self.dead_letter_now(
                                &m,
                                store,
                                "remetente fora do roster ao retomar o retry pos-restart"
                                    .to_string(),
                            );
                            if matches!(out, RouteOutcome::DeadLettered { .. }) {
                                let _ = self.mailbox.ack_inflight(&m.id);
                            }
                            return (m.id, out);
                        };
                        let block = render_message_block_v2(
                            &m.id,
                            &m.from,
                            &m.to,
                            &m.intent,
                            &m.payload,
                            &l.root,
                            l.hops,
                            m.contract.as_ref(),
                        );
                        self.retries.insert(
                            m.id.clone(),
                            RetryState {
                                from,
                                target,
                                block,
                                root: l.root.clone(),
                                hops: l.hops,
                                attempts: attempt,
                                next_ms,
                            },
                        );
                        return (
                            m.id,
                            RouteOutcome::RetryBackoff {
                                to: target,
                                attempt,
                            },
                        );
                    }
                    // SEAM-2 (R1): roteada + RETIDA por not-ready, sem entrega/DLQ/falha → RE-RETÉM
                    // (inv#4: mensagem viva não evapora num restart). Re-semeia a entrada de RETENÇÃO
                    // (`attempts:0`) no motor de re-entrega e RESTAURA o relógio de retenção do LOG
                    // (`retained_at`) → o backstop de 600 s conta o tempo TOTAL (não reinicia). Em
                    // produção o `ts` do log e o `now_ms` do pump são o MESMO relógio (wall-clock), então
                    // a contagem é correta. `process_due_retries` re-checa a prontidão e entrega 1×
                    // (dedupe via `MessageDelivered`) ou DLQ no teto. (Vem ANTES do `Some(_)`
                    // assume-entregue — é o que fecha o R1.)
                    Some(l) if l.retained_at.is_some() => {
                        let Some(target) = l.to_node else {
                            // Sem alvo single-node reconstruível → conservador (D1): ack.
                            let _ = self.mailbox.ack_inflight(&m.id);
                            return (m.id, RouteOutcome::Duplicate);
                        };
                        let Some(from) = self.sup.node_by_name(&m.from) else {
                            // Remetente saiu do roster — sem atribuição correta: DLQ legível (nada some).
                            let out = self.dead_letter_now(
                                &m,
                                store,
                                "remetente fora do roster ao retomar a retencao pos-restart"
                                    .to_string(),
                            );
                            if matches!(out, RouteOutcome::DeadLettered { .. }) {
                                let _ = self.mailbox.ack_inflight(&m.id);
                            }
                            return (m.id, out);
                        };
                        let block = render_message_block_v2(
                            &m.id,
                            &m.from,
                            &m.to,
                            &m.intent,
                            &m.payload,
                            &l.root,
                            l.hops,
                            m.contract.as_ref(),
                        );
                        // Relógio de retenção DURÁVEL: do log (instante original), não de `now_ms` —
                        // assim o teto conta o tempo TOTAL e não reinicia a cada restart.
                        self.retained_since
                            .entry(m.id.clone())
                            .or_insert_with(|| l.retained_at.unwrap_or(now_ms));
                        self.retries.insert(
                            m.id.clone(),
                            RetryState {
                                from,
                                target,
                                block,
                                root: l.root.clone(),
                                hops: l.hops,
                                attempts: 0, // RETENÇÃO (não falha) — jamais strike
                                next_ms: now_ms.saturating_add(NOT_READY_BACKOFF_MS),
                            },
                        );
                        return (m.id, RouteOutcome::Retained { to: target });
                    }
                    // Roteada SEM evento de entrega/falha (a janela A6 original: crash
                    // entre rotear e o desfecho da injeção) → CONSERVADOR como o D1
                    // histórico: assume entregue (re-injetar é irreversível; documentado).
                    Some(_) => {
                        if let Err(e) = self.mailbox.ack_inflight(&m.id) {
                            eprintln!("lina-core: pump falhou ao confirmar inflight {}: {e}", m.id);
                        }
                        return (m.id, RouteOutcome::Duplicate);
                    }
                    None => {}
                }
                let outcome = self.route_message(&m, store, now_ms, &mut deliver);
                // A6: só CONFIRMA (remove de `.inflight`) quando o evento durável foi persistido
                // (`MessageRouted`/`RouteBlocked`/plan). Em `PersistFailed` o registro PERMANECE em
                // `.inflight` para o próximo tick — nada se perde num crash entre drenar e logar.
                // W4-3: `Queued` (freio) também NÃO dá ack — a delegação fica retida no `.inflight`
                // (durável) até o `resume` a drenar; ackear aqui a perderia.
                //
                // RETRY TRANSIENTE: `UnknownSender`/`NoTarget` na CRIAÇÃO costumam ser uma RACE (o nó
                // ainda não entrou no roster vivo). Em vez de ackear+perder, RETÉM a msg no `.inflight`
                // por até `MAX_TRANSIENT_RETRIES` ticks — quando o nó aparece, o próximo tick ENTREGA.
                // Esgotado o teto, ackeia (destino realmente ausente → não retém para sempre).
                let transient = matches!(
                    outcome,
                    RouteOutcome::UnknownSender(_) | RouteOutcome::NoTarget
                );
                let retain_transient = transient && {
                    let n = self.transient_retries.entry(m.id.clone()).or_insert(0);
                    *n += 1;
                    *n < MAX_TRANSIENT_RETRIES
                };
                if retain_transient {
                    // O `route_message` já marcou esta msg em `seen` (dedupe). Desmarca para que o
                    // PRÓXIMO tick a RE-PROCESSE (senão viraria `Duplicate` e seria ackeada+perdida). O
                    // dedupe DURÁVEL (ledger no log) segue protegendo contra re-ENTREGA — só a
                    // re-ROTA da msg ainda-não-entregue é reaberta. Seguro: a origem (`from`) continua
                    // autenticada pelo subdir; nada de roteamento muda, só a re-tentativa.
                    self.seen.remove(&m.id);
                }
                // F1-0-7 (ADR 0020): transiente ESGOTADO → DLQ com motivo legível (antes:
                // ack+drop silencioso — "destino realmente ausente" agora é VISÍVEL e
                // recuperável manualmente; critério 2 da story, metade alvo-Dead).
                let outcome = if transient && !retain_transient {
                    self.dead_letter_now(
                        &m,
                        store,
                        format!(
                            "destino indisponivel ({outcome:?}) apos {MAX_TRANSIENT_RETRIES} \
                             re-tentativas"
                        ),
                    )
                } else {
                    outcome
                };
                // F1-0-4: `Retained` NÃO dá ack (re-avaliada no próximo tick; relógio por
                // TEMPO). F1-0-7: `RetryBackoff` idem — o motor de retry é quem fecha.
                let hold = matches!(
                    outcome,
                    RouteOutcome::PersistFailed(_)
                        | RouteOutcome::Queued
                        | RouteOutcome::Retained { .. }
                        | RouteOutcome::RetryBackoff { .. }
                );
                if !hold && !retain_transient {
                    // Desfecho definitivo (entregue, duplicado, hop-limit, dead-lettered):
                    // confirma e limpa os contadores (anti-vazamento dos mapas).
                    self.transient_retries.remove(&m.id);
                    self.retained_since.remove(&m.id);
                    if let Err(e) = self.mailbox.ack_inflight(&m.id) {
                        eprintln!("lina-core: pump falhou ao confirmar inflight {}: {e}", m.id);
                    }
                }
                (m.id, outcome)
            })
            .collect()
    }

    /// **Roteia UMA mensagem pelo pipeline de guardrails** e, se passar, entrega + persiste.
    pub fn route_message<D>(
        &mut self,
        msg: &MailMessage,
        store: &mut EventStore,
        now_ms: u64,
        deliver: &mut D,
    ) -> RouteOutcome
    where
        D: FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String>,
    {
        // ── W4-3 (freio do rodapé): orquestração PAUSADA → uma delegação NOVA fica ENFILEIRADA, não
        //    injetada (inv #6: GATE, não kill; nada se perde). A msg permanece no `.inflight`
        //    (durável); o `pump` NÃO dá ack em `Queued`, então ela é reavaliada a cada tick e DRENA
        //    no `resume`. Checado ANTES do dedupe — marcá-la `seen` a tornaria `Duplicate` no resume
        //    (acked sem entregar = perdida). Replies (`reply_to`) e intents de plano PASSAM: o freio
        //    trava a auto-orquestração (delegações), não o fechamento de loops/await já abertos.
        //    r4 achado #12: o freio cobre TAMBÉM `lina spawn` — deixar o spawn passar criava o
        //    terminal mas represava o 1º prompt dele (meia-orquestração: nó vivo sem tarefa).
        if self.paused
            && msg.reply_to.is_none()
            && (is_delegation(&msg.intent) || is_spawn_intent(&msg.intent))
        {
            return RouteOutcome::Queued;
        }

        // ── Guardrail: dedupe por `id` na janela (design §3). Poda `seen` E `budget` (A5).
        self.prune_expired(now_ms);
        if self.seen.contains_key(&msg.id) {
            // ADR 0003: `duplicate` NÃO é logado (anti-amplificação A4 — sob flood viraria um
            // amplificador de escrita). A duplicata é benigna por definição.
            return RouteOutcome::Duplicate;
        }
        // Marca como visto JÁ — uma 2ª cópia do mesmo id (re-drenada/reenviada) é duplicada.
        self.seen.insert(msg.id.clone(), now_ms);

        // ── FIX #22 (backlog): PING DE PRESENÇA (`lina handshake`: intent=handshake, to=*) é
        //    fire-and-forget — o roster vivo é do Supervisor (events.rs:141), não desta rota. Desviado
        //    AQUI, ANTES da resolução de `from`: o drain FLAT do handshake ANONIMIZA `from → ""`, então
        //    sem este desvio a presença morreria em `UnknownSender("")` + `RouteBlocked` ESPÚRIO — ruído
        //    que PARECE erro no stderr do app mas é o caminho esperado. Não delega, não entrega, e NÃO
        //    loga erro. Erros REAIS (ask/handoff com alvo inválido, ou qualquer intent ≠ handshake)
        //    seguem o pipeline normal e AINDA logam `RouteBlocked` — não mascaramos nada além da presença.
        if is_presence(msg) {
            return RouteOutcome::Presence;
        }

        // ── Guardrail 0a (Round 6 — ELEVADO p/ ANTES do desvio de plano): remetente existe no roster
        //    vivo. O `NodeId` resolvido é a ORIGEM autenticada — TANTO a delegação QUANTO o intent de
        //    plano passam por aqui, então um `from` forjado/desconhecido (ex.: msg do outbox FLAT, cujo
        //    `from` o drain anonimiza) é RECUSADO como `UnknownSender` ANTES de tocar o plano (fecha F1).
        //    Resolvido ANTES do anti-loop por profundidade — derivamos root/hops do binding (A2).
        let Some(sender) = self.sup.node_by_name(&msg.from) else {
            log_block(store, msg, BlockReason::UnknownSender);
            return RouteOutcome::UnknownSender(msg.from.clone());
        };

        // ── F1-0-5: validação DE FORMA do envelope (`lina/msg@2`) — intent canônico +
        //    contrato de handoff completo. DEPOIS da autenticação do remetente (identidade
        //    decide ANTES de qualquer campo do contrato — red-team da story) e ANTES do
        //    desvio de plano/entrega. `@1` passa intacto (compat). Validação NO ROUTER,
        //    nunca no LLM (13.11 §P2); a recusa é estruturada e LEGÍVEL (o agente corrige).
        if let Err(violation) = validate_envelope_v2(msg) {
            let reason = match &violation {
                EnvelopeViolation::Intent(_) => BlockReason::InvalidIntent,
                EnvelopeViolation::Contract(_) => BlockReason::InvalidContract,
            };
            log_block(store, msg, reason);
            return RouteOutcome::ContractRejected(violation.why().to_string());
        }

        // ── W3-5: intents de plano NÃO são delegação A2A. O supervisor os APLICA ao `plan.md`
        //    (escritor único) e LOGA o evento, sem entregar a nenhuma PTY. Desviam aqui — depois do
        //    dedupe (reenvios são absorvidos) E da resolução de origem (Round 6: `by` = origem
        //    AUTENTICADA, nunca um `from` forjado), antes do pipeline de delegação/resolução de alvo.
        //    (Round 4: intent de plano nunca fecha await — o desvio precede o lifecycle do reply.)
        if is_plan_intent(&msg.intent) {
            return self.handle_plan(msg, store);
        }

        // ── F1-3-6 (ADR 0019 §6): `lina spawn` é verbo ESTRUTURADO interceptado AQUI (molde do
        //    plano, IMEDIATAMENTE depois dele) e DECIDIDO pelo gate inforjável — NÃO entregue a
        //    PTY. Precede os guardrails de alvo/hop/retenção de propósito: o `@Nome` do spawn NÃO
        //    existe no roster (cairia em `NoTarget`) e spawn não injeta turno (não passa por
        //    retenção/close_await/hop-limit). `sender` já está resolvido/autenticado (acima) e o
        //    envelope já passou no `validate_envelope_v2`. O gate lê origem×cascata REUSANDO
        //    `derive_root_hops` (binding inforjável — jamais `msg.hops`/`msg.root_cause_id`).
        if is_spawn_intent(&msg.intent) {
            return self.handle_spawn(msg, sender, store, now_ms);
        }

        // ── F3-0-5: `lina params set/reset` é verbo ESTRUTURADO interceptado AQUI (molde do plano):
        //    aplica ao log (`SystemParamsChanged`), NUNCA entregue a PTY (o `@workspace`/escopo não é
        //    um nó). `by` é carimbado pela origem×cascata (`derive_root_hops`, binding inforjável),
        //    JAMAIS lido do payload (ADR 0007). Precede o pipeline de entrega/alvo, como plan/spawn.
        if is_params_intent(&msg.intent) {
            return self.handle_params(msg, sender, store);
        }

        // ── F3-0-5: `lina effort @T <low|medium|high>` — verbo estruturado (mirror de handle_params):
        //    resolve o alvo, valida o effort e emite `EffortAssigned{origin:assigned}`. `by`/`origin`
        //    carimbados server-side (jamais do payload — ADR 0007). NÃO entregue a PTY.
        if is_effort_intent(&msg.intent) {
            return self.handle_effort(msg, sender, store);
        }

        // ── F3-1: verbos da Goal (`goal.define`/`goal.interpret`/`goal.confirm`) — verbos
        //    ESTRUTURADOS (molde de plan/params): emitem eventos da Goal, NUNCA entregues a PTY.
        //    `goal.confirm.by` é carimbado com o `sender` autenticado (server-side, jamais payload —
        //    ADR 0007). Precedem o pipeline de entrega/alvo, como os demais verbos.
        if is_goal_intent(&msg.intent) {
            return self.handle_goal(msg, sender, store);
        }

        // ── F3-4-3 (spec 36 §2): `lina code-changed` — SINAL DE MUDANÇA de um commit numa worktree de
        //    agente. Verbo ESTRUTURADO (molde de plan/goal): apenda `CodeChanged` com `author_node`
        //    CARIMBADO SERVER-SIDE (= `sender` autenticado, JAMAIS o payload — regra-mãe ADR 0007), e
        //    faz o BROADCAST POR PERTENCIMENTO: compara `paths`×claims e avisa SÓ os donos afetados, que
        //    PARAM. NÃO é delegação A2A entregue a um alvo `to`; a entrega é a notificação de
        //    pertencimento. Precede o pipeline de entrega/alvo, como os demais verbos.
        if is_code_changed_intent(&msg.intent) {
            return self.handle_code_changed(msg, sender, store, deliver);
        }

        // ── F3-4-5 (spec 36 §3): `lina branch-integrated` — PROVA DE FECHAMENTO de uma branch
        //    integrada na linha de develop (DevOps integrador, após um merge PROVADO). Verbo
        //    ESTRUTURADO: apenda `BranchIntegrated` — a ÚNICA prova de "branch fechada" (a projeção
        //    `code::branches_nao_integradas` só a remove do pendente com este evento, nunca por
        //    suposição). NÃO é delegação A2A entregue a um `to`; é registro no log, como code.changed.
        if is_branch_integrated_intent(&msg.intent) {
            return self.handle_branch_integrated(msg, store);
        }

        // ── F3-3 (M-DETECTOR, spec 35 §4.1): sentinela `[LINA::CORRECTION]` no payload — verbo
        //    ESTRUTURADO de CAPTAÇÃO de aprendizado (molde de plan/params/goal). Detectado pela
        //    SENTINELA (não por intent canônico — qualquer mensagem cujo payload ABRE com o marcador
        //    é a auto-declaração de correção do papel). Emite `CorrectionObserved` com o `role` do
        //    SENDER autenticado server-side (JAMAIS do payload — regra-mãe ADR 0007). NÃO entregue a
        //    PTY: é observação, não delegação. Barato/determinístico; o Refletor consome o evento
        //    DEPOIS, assíncrono fora do caminho crítico (inv #1: zero LLM aqui).
        if let Some(summary) = parse_correction(&msg.payload) {
            return self.handle_correction(sender, &summary, &msg.id, store);
        }

        // ── F1-0-4 (P1 — o fix do atropelamento): ENTREGA CIENTE DE ESTADO. Alvo single-node
        //    `Busy` (state-machine F1-0-3, consultada no roster — inclui o Busy carimbado
        //    pelo próprio router na entrega anterior) → NÃO injeta: a msg fica RETIDA no
        //    `.inflight` (durável) e é re-avaliada INTEIRA a cada tick, até o alvo voltar
        //    a Idle/Ready ou estourar `retention_timeout_ms` (→ entrega mesmo assim; DLQ é
        //    F1-0-7).
        //
        //    BRAINSTORM REGISTRADO (opções do DIRECIONAMENTO §P1): **(a) adiar ✔ escolhida**
        //    (estende o mecanismo de retenção do `.inflight` que o retry transiente já usa —
        //    é o fix real do atropelamento); (b) agregar N respostas em 1 bloco — anotada
        //    como melhoria futura (possível over-engineering agora, nas palavras do próprio
        //    doc); (c) lock — o `lock_pty` já serializa BYTES, mas o problema é injetar em
        //    cima do TURNO do Claude (semântico), não interleave de bytes.
        //
        //    POSIÇÃO NO PIPELINE (deliberada): ANTES do `close_await_on_reply` — uma reply
        //    retida NÃO pode fechar o ticket antes de ser entregue (na re-avaliação ela
        //    perderia a isenção anti-loop e viraria `LoopDetected`); e antes de qualquer
        //    efeito (await/budget/persist), para a re-avaliação ser limpa. Trade-off
        //    consciente: rejeições permanentes (autonomia/fanout/budget) ficam adiadas
        //    enquanto o alvo está Busy — bounded pelo timeout. Broadcast NÃO retém (tem
        //    gate próprio — ADR 0007). FIFO por alvo: o `.inflight` re-drena em ordem de
        //    id (UUID v7 temporal); como CADA entrega marca o alvo `Busy`
        //    (`mark_busy_after_delivery`), a próxima da fila retém até o alvo completar o
        //    turno (Idle pelo lifecycle/EndDetector) — serialização "uma aguardando a outra".
        match self.retention_verdict(msg, sender, now_ms) {
            Verdict::Pass => {}
            Verdict::Hold(target, reason) => {
                if !self.retained_since.contains_key(&msg.id) {
                    self.retained_since.insert(msg.id.clone(), now_ms);
                    // Livro-razão da espera (1× por msg — A4). Falha de append NÃO solta a
                    // injeção (reter é o estado seguro; a msg segue durável no `.inflight`).
                    if let Err(e) = store.append(&DomainEvent::MessageRetained {
                        id: msg.id.clone(),
                        to: target,
                        reason: reason.to_string(),
                    }) {
                        eprintln!("lina-core: falha ao logar MessageRetained({}): {e}", msg.id);
                    }
                }
                // Espelho do retry transiente: desmarca o dedupe para o PRÓXIMO tick
                // re-processar (senão viraria `Duplicate` e seria ackeada+perdida). O
                // dedupe DURÁVEL (ledger no log) segue protegendo contra re-ENTREGA.
                self.seen.remove(&msg.id);
                return RouteOutcome::Retained { to: target };
            }
            // F1-0-7 (ADR 0020): retenção estourou o teto → DLQ com motivo legível (nada
            // some, nada espera para sempre; retry MANUAL re-enfileirando o arquivo).
            Verdict::DeadLetter(_target) => {
                return self.dead_letter_now(
                    msg,
                    store,
                    format!(
                        "retencao estourou o teto ({} ms) com o alvo ainda ocupado/bloqueado",
                        self.config.retention_timeout_ms
                    ),
                );
            }
        }

        // ── W3-7b (ADR 0002) + Round 4 (confused-deputy): se ESTA msg é um REPLY que CASA um await
        //    pendente E vem do `target` aguardado, FECHA o ticket (end_await + AwaitClosed{Replied}).
        //    Capturado AQUI — DEPOIS da resolução do `sender` (o fechamento só pode ocorrer com
        //    remetente legítimo/resolvido) e do desvio de plano, mas ANTES do guard de loop (que vem
        //    depois e veria o `pending` já esvaziado). `is_legit_reply` = casou um ticket REAL cujo
        //    `target == sender`. Só um reply legítimo pula o anti-loop (back-edge da pergunta); um
        //    `reply_to` forjado/de-não-participante → `false` → passa pelo anti-loop normal.
        let is_legit_reply = self.close_await_on_reply(msg, sender, store);

        // ── A2 + W1: deriva `root_cause_id`/`hops` EFETIVOS do BINDING interno (jamais dos campos
        //    forjáveis da mailbox): herda do `delivered_root[sender]` (root + hops+1); sem binding,
        //    o nó é a raiz a profundidade 0. É o que torna orçamento/grafo/hop-limit à prova de forja.
        let (root, hops) = self.derive_root_hops(msg, sender);

        // ── Guardrail: anti-loop por PROFUNDIDADE sobre os `hops` EFETIVOS (antes `msg.hops` ficava
        //    sempre 0 na cadeia real → o guard era inerte; agora cresce 0→1→2…).
        if hops > self.config.max_depth {
            log_block(store, msg, BlockReason::HopLimit);
            return RouteOutcome::HopLimit;
        }

        // ── Guardrail 0b: alvo existe (resolve SEM efeito colateral — não publica ainda).
        let (recipient, policy) = match parse_target(&msg.to) {
            TargetSpec::Broadcast => (Recipient::Broadcast, RolePolicy::All),
            TargetSpec::Role(r) => (Recipient::Role(r), RolePolicy::FirstIdle),
            TargetSpec::Name(n) => match self.sup.node_by_name(&n) {
                Some(id) => (Recipient::Node(id), RolePolicy::FirstIdle),
                None => {
                    log_block(store, msg, BlockReason::NoTarget);
                    return RouteOutcome::NoTarget;
                }
            },
        };
        let preview = self.sup.resolve(&recipient, policy, Some(sender));
        if preview.is_empty() {
            log_block(store, msg, BlockReason::NoTarget);
            return RouteOutcome::NoTarget;
        }

        // ── Guardrail 1: autonomia (manual recusa delegação).
        if self.config.autonomy.blocks_delegation() && is_delegation(&msg.intent) {
            log_block(store, msg, BlockReason::BlockedByAutonomy);
            return RouteOutcome::BlockedByAutonomy;
        }

        // ── Guardrail: gate de fan-out — SÓ na CASCATA (ADR 0007). `hops` é EFETIVO (do binding, não
        //    forjável — `derive_root_hops`/W1): `hops==0` é a ORIGEM (o agente foi pedido pelo humano e
        //    NÃO recebeu A2A → sem binding) e a 1ª onda dela entrega a TODOS (igual o Maestri); `hops>=1`
        //    é o sender RE-ESPALHANDO algo que recebeu — onde mora a tempestade — e segue gateado (pede
        //    confirmação humana). Um agente NÃO compra fan-out livre forjando `hops`: o valor vem do
        //    binding carimbado numa entrega real. Backstops finais intactos (MAX_DEPTH/anti-loop/custódia).
        if hops >= 1
            && matches!(recipient, Recipient::Broadcast)
            && preview.len() > self.config.fanout_gate
        {
            log_block(store, msg, BlockReason::FanoutGated);
            return RouteOutcome::FanoutGated {
                count: preview.len(),
            };
        }

        let is_deleg = is_delegation(&msg.intent);

        // ── Guardrail (§2.2): TETO DE CUSTO por workspace/dia. Workspace-level (precede o orçamento
        //    por-cadeia): se a soma de uso da janela contábil atingiu `token_budget_day`, o workspace
        //    está `Paused` e nenhuma delegação sai — é GATE humano (`lina resume`), não kill (inv #6).
        //    DISTINTO de `BudgetExceeded` (delegação/root). `CostCeilingHit` é apendado SÓ na TRANSIÇÃO
        //    para Paused (anti-amplificação A4): se já pausado, bloqueia sem re-logar. Event-sourced:
        //    o ledger é reconstruído do log (invariante #4), então o teste pode semear o uso direto.
        if is_deleg && self.config.token_budget_day > 0 {
            let ledger = match CostLedger::replay(store, now_ms) {
                Ok(l) => l,
                // Não conseguimos LER o log para somar o uso → falha segura (não roteia).
                Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
            };
            if ledger.paused {
                return RouteOutcome::CostCeiling; // já pausado: bloqueia sem re-apender (A4)
            }
            if ledger.tokens >= self.config.token_budget_day {
                if let Err(e) = store.append(&DomainEvent::CostCeilingHit {
                    day: ledger.day.clone(),
                    tokens: ledger.tokens,
                }) {
                    return RouteOutcome::PersistFailed(e.to_string());
                }
                return RouteOutcome::CostCeiling; // transição → Paused
            }
        }

        // ── Guardrail 3: orçamento por `(root_cause_id, agente)` — SÓ na CASCATA (`hops>=1`, ADR 0007).
        //    Com a propagação do root (A2), delegações da mesma cadeia acumulam contra o mesmo root. A
        //    ORIGEM (`hops==0`) JÁ escapava por construção — cada msg de origem ganha um root FRESCO
        //    (`msg.id`, W1), então `(root, sender)` nunca acumula; o `hops>=1` torna isso EXPLÍCITO no
        //    ponto de decisão (defesa em profundidade: dois motivos independentes p/ a origem não ser
        //    limitada). Só CHECA aqui; incrementa após o persist (mesma condição).
        if is_deleg && hops >= 1 {
            let count = self
                .budget
                .get(&(root.clone(), sender))
                .map(|e| e.count)
                .unwrap_or(0);
            if count >= self.config.delegation_budget {
                log_block(store, msg, BlockReason::BudgetExceeded);
                return RouteOutcome::BudgetExceeded;
            }
        }

        // ── Guardrail (§2.1): anti-loop por GRAFO DE EVENTOS handoff `from→to` filtrado pelo
        //    `root_cause_id`. Pega o ping-pong A→B→A→B… formado por mensagens NOVAS sob o mesmo root —
        //    que `hops`/`trace` NÃO pegam (cada `MailMessage::new` zera ambos). Só ask/handoff a 1 alvo
        //    (broadcast é gateado à parte). Topologia de eventos: independe do texto.
        //    COOPERAÇÃO: o ida-e-volta do DIÁLOGO (A→B→A e algumas voltas) é legítimo e PASSA — o guard
        //    só corta quando o MESMO salto se repete >= `MAX_CYCLE_REVISITS` vezes DENTRO de um ciclo
        //    (ping-pong apertado/repetido). Antes bloqueava já na 1ª volta → a resposta nunca chegava.
        //    Backstops finais do loop infinito: `hops` (`MAX_DEPTH`) + orçamento (`DELEGATION_BUDGET`).
        //    Um REPLY LEGÍTIMO (`is_legit_reply`: casou um ticket pendente) é a back-edge esperada
        //    da pergunta → pula o guard. `reply_to` forjado (sem ticket) NÃO pula (passa pelo guard).
        if is_deleg && !is_legit_reply && !matches!(recipient, Recipient::Broadcast) {
            if let Some(&target) = preview.first() {
                match handoff_would_tight_loop(store, &root, sender, target, MAX_CYCLE_REVISITS) {
                    Ok(true) => {
                        // F3-CONF-2 (gate b): o anti-loop é CEGO ao progresso (#22c) — barraria o
                        // Maestro de RE-DESPACHAR um worker ENGOLIDO (correção legítima de um
                        // despacho que não virou trabalho). ISENTA se o alvo tem entrega PENDENTE
                        // sem progresso desde então (vigia armado); a tempestade real (saltos COM
                        // atividade no meio) DESARMA o vigia → sem entrega pendente → SEGUE barrada.
                        // O vigia é o discriminador estrutural (zero LLM); backstops `hops`/budget
                        // continuam cobrindo o loop infinito mesmo na isenção.
                        if let Err(e) = self.sync_dispatch_progress(store) {
                            eprintln!(
                                "lina-core: route_message não desarmou vigias (barra o loop): {e}"
                            );
                        }
                        if !self.dispatched_at.contains_key(&target) {
                            log_block(store, msg, BlockReason::LoopDetected);
                            return RouteOutcome::LoopDetected;
                        }
                        // engolido → re-despacho de correção ISENTO (cai através do anti-loop).
                    }
                    Ok(false) => {}
                    // Não conseguimos LER o log para verificar o ciclo → falha segura (não roteia).
                    Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
                }
            }
        }

        // ── Guardrail 2 / ABERTURA do await (ADR 0002): para `--await`, a checagem de deadlock e a
        //    abertura. ask/handoff (1 alvo): `begin_await_ticket` abre a aresta (liberada só no
        //    reply/timeout) — `await_target` lembra o alvo p/ rollback se a rota/persist falhar.
        let await_target: Option<NodeId> = if msg.await_reply {
            if matches!(recipient, Recipient::Broadcast) {
                // Round 5 #6: broadcast-await faz a checagem de deadlock PURA (`would_await_deadlock`,
                // não muta o grafo) — JAMAIS apaga uma aresta single-target viva do sender. Sem
                // lifecycle (multi-alvo não cabe no wait-for-graph por-waiter; foge ao ADR).
                for &target in &preview {
                    if self.sup.would_await_deadlock(sender, target) {
                        log_block(store, msg, BlockReason::Deadlock);
                        return RouteOutcome::Deadlock;
                    }
                }
                None
            } else {
                match preview.first().copied() {
                    // Round 5 #7: aresta chaveada pelo `id` do ticket (multi-aresta) — abrir este
                    // await NÃO sobrescreve outros awaits abertos do mesmo sender.
                    Some(target) => match self.sup.begin_await_ticket(sender, &msg.id, target) {
                        Ok(()) => Some(target), // aresta ABERTA (fica até reply/timeout)
                        Err(_) => {
                            log_block(store, msg, BlockReason::Deadlock);
                            return RouteOutcome::Deadlock;
                        }
                    },
                    None => None,
                }
            }
        } else {
            None
        };

        // ── Passou. Carimba o envelope com root/hops EFETIVOS e ROTEIA (publica `BusEvent::Message`
        //    + devolve os alvos frescos, sem o remetente nem nós já no trace).
        let mut env = A2aEnvelope::new(sender, recipient.clone(), Some(intent_or_ask(&msg.intent)));
        env.root_cause_id = Some(root.clone());
        env.hops = hops;
        env.await_reply = msg.await_reply;
        let targets = self.sup.route(&env, policy);
        if targets.is_empty() {
            if let Some(target) = await_target {
                // rollback da aresta aberta acima (rota falhou) — só este ticket (Round 5 #7).
                self.sup.end_await_ticket(sender, &msg.id, target);
            }
            log_block(store, msg, BlockReason::NoTarget);
            return RouteOutcome::NoTarget;
        }

        // Persiste o ROTEAMENTO (com root/hops/to_node p/ o grafo anti-loop) antes de entregar.
        let to_node = if matches!(recipient, Recipient::Broadcast) {
            None
        } else {
            targets.first().copied()
        };
        if let Err(e) = store.append(&DomainEvent::MessageRouted {
            id: msg.id.clone(),
            from: sender,
            to: msg.to.clone(),
            intent: intent_or_ask(&msg.intent),
            root_cause_id: root.clone(),
            hops,
            to_node,
        }) {
            if let Some(target) = await_target {
                // rollback: não abrimos await sem roteamento durável (só este ticket — Round 5 #7).
                self.sup.end_await_ticket(sender, &msg.id, target);
            }
            // Exceção do ADR 0003: não há como logar a falha de logar (recai em stderr/PersistFailed).
            return RouteOutcome::PersistFailed(e.to_string());
        }

        // ── ADR 0002: roteamento durável → ABRE o lifecycle do await (a aresta já está segurada).
        //    O `AwaitOpened` registra o contexto completo no log (fonte da verdade); o `pending`
        //    guarda só o necessário p/ fechar (reply/timeout).
        if let Some(target) = await_target {
            if let Err(e) = store.append(&DomainEvent::AwaitOpened {
                id: msg.id.clone(),
                waiter: sender,
                target,
                root_cause_id: root.clone(),
            }) {
                // rollback: não abrimos o que não conseguimos logar (só este ticket — Round 5 #7).
                self.sup.end_await_ticket(sender, &msg.id, target);
                return RouteOutcome::PersistFailed(e.to_string());
            }
            self.pending.insert(
                msg.id.clone(),
                AwaitTicket {
                    waiter: sender,
                    target,
                    opened_ms: now_ms,
                },
            );
        }
        // Roteamento logado → AGORA contabiliza o orçamento (delegação efetivada). Conta TODA delegação
        // (bookkeeping/observabilidade + poda A5): a entrada de ORIGEM é inofensiva (root FRESCO por msg
        // → nunca re-checada) e é PODADA pela janela. O ENFORCEMENT cascata-only vive no CHECK acima
        // (`hops>=1`), não aqui. Carimba o tempo para a poda temporal (A5).
        if is_deleg {
            let entry = self
                .budget
                .entry((root.clone(), sender))
                .or_insert(BudgetEntry {
                    count: 0,
                    spawn_count: 0,
                    last_ms: now_ms,
                });
            entry.count += 1;
            entry.last_ms = now_ms;
        }

        // Renderiza o bloco [LINA::MSG] (com root_cause_id/hops achatados — auditoria/UX) e entrega.
        // F1-0-5: o contrato/constraints (quando presentes) entram LEGÍVEIS no bloco — é o
        // constraint-transfer: nada implícito atravessa o handoff. São dados transportados;
        // nenhuma decisão do router (acima) os leu.
        let to_label = recipient_label(&recipient, &msg.to);
        let block = render_message_block_v2(
            &msg.id,
            &msg.from,
            &to_label,
            &msg.intent,
            &msg.payload,
            &root,
            hops,
            msg.contract.as_ref(),
        );
        let mut delivered = Vec::with_capacity(targets.len());
        let single_target = targets.len() == 1; // F1-0-4: gate pós-entrega só em rota 1-a-1
        for target in targets {
            match deliver(target, sender, &block) {
                // ACHADO-2: `injected()` distingue entrega FEITA (texto colado / sessão retomada)
                // de OCUPADO (`Injected{ready:false}` = prompt não-pronto, NADA escrito). Só a
                // entrega de fato segue o caminho de "entregue"; o ocupado RETÉM (arm abaixo).
                Ok(outcome) if outcome.injected() => {
                    delivered.push(target);
                    // F1-0-4: o alvo recebeu um turno → está `Busy` AGORA (transição feita
                    // pelo router, determinística — é o que serializa a próxima entrega ao
                    // mesmo alvo). E encerra o relógio de retenção desta msg.
                    if single_target {
                        self.mark_busy_after_delivery(store, target);
                    }
                    self.retained_since.remove(&msg.id);
                    // ACHADO-2: entrega real limpa qualquer retenção pendente desta msg.
                    self.retries.remove(&msg.id);
                    // A2: registra que `target` recebeu esta cadeia → futuras submissões dele herdam
                    //     o root e ganham hops+1 (binding do supervisor, não confiança no campo).
                    // FRONTEIRA DE SEGURANÇA (P1 vs Round 5 #8 migração):
                    //  • A defesa anti-FORJA é que o root vem SEMPRE do binding carimbado numa ENTREGA
                    //    real (ou de `msg.id` na origem) — JAMAIS do campo forjável `msg.root_cause_id`.
                    //    A migração NÃO reabre essa forja: só uma entrega legítima muda o binding.
                    //  • P1 (anti-envenenamento) protege um binding VIVO: `keep_existing` recusa
                    //    sobrescrever um binding de root DIFERENTE — então um root fresco de entrada não
                    //    faz a vítima "esquecer" a cadeia em curso nem escapar do orçamento/loop.
                    //  • Round 5 #8 (migração) só vale para nó OCIOSO: bindings stale (> janela, SEM
                    //    await pendente) já foram PODADOS no `prune_expired` no topo do tick. Aqui resta
                    //    binding FRESCO ou protegido-por-await (P1 o mantém) OU nenhum (nó ocioso →
                    //    re-set abaixo, migrando p/ a cadeia que o entrega). MESMO root → atualiza
                    //    hops/last_ms (mantém vivo). (Decisão ANTES do insert p/ não segurar o `get`.)
                    let keep_existing = matches!(
                        self.delivered_root.get(&target),
                        Some((existing, _, _)) if *existing != root
                    );
                    if !keep_existing {
                        self.delivered_root
                            .insert(target, (root.clone(), hops, now_ms));
                    }
                    match store.append(&DomainEvent::MessageDelivered {
                        id: msg.id.clone(),
                        to: target,
                    }) {
                        // F3-CONF-2: ARMA o vigia de engolimento (só rota 1-a-1 = handoff/ask, não
                        // broadcast best-effort) no relógio do router (`now_ms`) + o `seq` da entrega.
                        Ok(seq) => {
                            if single_target {
                                self.dispatched_at.insert(target, (now_ms, seq));
                            }
                        }
                        // A entrega ao PTY já ocorreu (irreversível) mas não foi logada — o event
                        // log é a fonte da verdade, então a inconsistência é ALTA e VISÍVEL (stderr),
                        // nunca um warn silencioso de tracing.
                        Err(e) => eprintln!(
                            "lina-core: CRÍTICO — entregue mas NÃO logado (MessageDelivered {} → {target}): {e}",
                            msg.id
                        ),
                    }
                }
                // ACHADO-2: prompt-NÃO-pronto (`Ok(Injected{ready:false})`) = alvo OCUPADO, NÃO
                // falha. O lifecycle pode julgá-lo Idle (turno longo cuja saída a state-machine
                // ainda não fechou), mas o grid é a verdade no instante da entrega. RETÉM (sinal de
                // ocupado), JAMAIS strike/DLQ — mensagem viva não morre por o colega trabalhar. Só
                // rota 1-a-1 retém (broadcast é best-effort, ADR 0007 — alvo não-pronto é pulado).
                Ok(_) => {
                    if single_target {
                        return self.retain_not_ready(
                            msg, target, sender, &block, &root, hops, store, now_ms,
                        );
                    }
                }
                Err(e) => {
                    eprintln!("lina-core: entrega faseada falhou (alvo {target}): {e}");
                    // F1-0-7 (ADR 0020): rota 1-a-1 → entra no motor de retry (backoff →
                    // DLQ) em vez do antigo ack+drop silencioso. Broadcast mantém a
                    // semântica existente (re-entrega parcial re-injetaria nos que já
                    // receberam; tem gate próprio — ADR 0007).
                    if single_target {
                        return self.on_delivery_failure(
                            msg, store, target, sender, &block, &root, hops, now_ms, &e,
                        );
                    }
                }
            }
        }

        if delivered.is_empty() {
            RouteOutcome::DeliveryFailed("nenhum alvo recebeu".into())
        } else {
            RouteOutcome::Delivered { targets: delivered }
        }
    }

    /// **F1-0-4/F1-0-7 — o veredito de retenção** (P1 + DLQ + breaker).
    /// Resolução do destino SEM efeito colateral (mesma do guardrail 0b); só retém destino
    /// que resolve a EXATAMENTE 1 nó vivo (broadcast/multi tem gate próprio — ADR 0007).
    /// `Hold(target_busy)` = estado `Busy` na state-machine F1-0-3 (inclui o `Busy` que a
    /// entrega anterior carimbou — serialização); `Hold(circuit_open)` = breaker do nó
    /// aberto (novas msgs NEM tentam — anti retry-storm); `DeadLetter` = a retenção
    /// estourou `retention_timeout_ms` (nunca espera infinita/circular — 13.11 §P0; o
    /// destino prometido da F1-0-4 aterrissou aqui, ADR 0020).
    fn retention_verdict(&mut self, msg: &MailMessage, sender: NodeId, now_ms: u64) -> Verdict {
        // Resolução sem efeito colateral; só single-node.
        let (recipient, policy) = match parse_target(&msg.to) {
            TargetSpec::Broadcast => return Verdict::Pass,
            TargetSpec::Role(r) => (Recipient::Role(r), RolePolicy::FirstIdle),
            TargetSpec::Name(n) => match self.sup.node_by_name(&n) {
                Some(id) => (Recipient::Node(id), RolePolicy::FirstIdle),
                None => return Verdict::Pass, // NoTarget → pipeline normal (transiente cobre)
            },
        };
        let preview = self.sup.resolve(&recipient, policy, Some(sender));
        let [target] = preview[..] else {
            return Verdict::Pass;
        };

        // Teto da retenção: estourou → DLQ (F1-0-7; nunca espera infinita/circular).
        if let Some(&first) = self.retained_since.get(&msg.id) {
            if now_ms.saturating_sub(first) >= self.config.retention_timeout_ms {
                return Verdict::DeadLetter(target);
            }
        }

        // F1-0-7: breaker do nó. Reset EXTERNO detectado aqui: se alguém (lifecycle/
        // humano) tirou o nó de `Blocked`, o breaker fecha e as falhas zeram.
        if self.breaker_open.contains(&target) {
            let status = self.sup.get(target).map(|i| i.status);
            if status == Some(NodeStatus::Blocked) {
                return Verdict::Hold(target, "circuit_open");
            }
            self.breaker_open.remove(&target);
            self.node_failures.remove(&target);
        }

        // Retenção PURAMENTE por estado: a serialização "uma aguardando a outra" emerge
        // da state-machine (cada entrega marca o alvo `Busy`; só o ciclo real — lifecycle/
        // EndDetector → Idle — libera).
        match self.sup.get(target) {
            Some(info) if info.status == NodeStatus::Busy => Verdict::Hold(target, "target_busy"),
            _ => Verdict::Pass,
        }
    }

    /// **F1-0-4 — o alvo que acabou de receber um turno injetado ESTÁ `Busy`.** O router
    /// (escritor único, com o store em mãos) registra a transição no padrão fato-antes-
    /// do-efeito do `LifecycleEngine::transition` (evento → roster), com
    /// `reason: "a2a_delivery"`. É o que torna a serialização determinística: a PRÓXIMA
    /// mensagem ao mesmo alvo encontra `Busy` na hora, sem janela de corrida com a
    /// detecção por output. A volta a `Idle` continua sendo do lifecycle (fim-de-resposta).
    fn mark_busy_after_delivery(&self, store: &mut EventStore, target: NodeId) {
        let Some(info) = self.sup.get(target) else {
            return;
        };
        if info.status == NodeStatus::Busy || info.status == NodeStatus::Dead {
            return; // no-op sem evento (anti-amplificação, padrão ADR 0005)
        }
        if let Err(e) = store.append(&DomainEvent::NodeStatusChanged {
            node: target,
            status: NodeStatus::Busy.as_str().to_string(),
            from: info.status.as_str().to_string(),
            reason: "a2a_delivery".to_string(),
        }) {
            eprintln!("lina-core: falha ao logar NodeStatusChanged(a2a_delivery): {e}");
        }
        if let Err(e) = self.sup.set_status(target, NodeStatus::Busy) {
            eprintln!("lina-core: falha ao marcar {target} Busy pós-entrega: {e}");
        }
    }

    /// **F1-0-7 (ADR 0020) — move a mensagem à DLQ AGORA.** Ordem durável: escreve o
    /// arquivo em `<.lina>/dead-letter/` ANTES de devolver o desfecho (o pump só ackeia o
    /// `.inflight` depois — a mensagem nunca fica sem casa). Falha de I/O na DLQ → NÃO
    /// descarta: devolve `PersistFailed` e a msg segue retida no `.inflight`.
    fn dead_letter_now(
        &mut self,
        msg: &MailMessage,
        store: &mut EventStore,
        reason: String,
    ) -> RouteOutcome {
        if let Err(e) = self.mailbox.dead_letter(msg) {
            eprintln!(
                "lina-core: falha ao escrever DLQ de {} (retida): {e}",
                msg.id
            );
            return RouteOutcome::PersistFailed(format!("dlq: {e}"));
        }
        if let Err(e) = store.append(&DomainEvent::MessageDeadLettered {
            id: msg.id.clone(),
            to: msg.to.clone(),
            reason: reason.clone(),
        }) {
            // Arquivo da DLQ já existe (durável); o evento falhou — visível, nunca silencioso.
            eprintln!(
                "lina-core: CRÍTICO — DLQ gravada mas não logada ({}): {e}",
                msg.id
            );
        }
        // NOTIFICAÇÃO ao Maestro (critério da story): stderr legível; a projeção visível
        // é o próprio diretório `dead-letter/` (+ o evento no log para a UI/F1-1).
        eprintln!(
            "lina-core: ☠ DLQ ← {} (para {}): {reason} — retry MANUAL: mova \
             dead-letter/{}.json de volta ao outbox",
            msg.id, msg.to, msg.id
        );
        self.retained_since.remove(&msg.id);
        self.retries.remove(&msg.id);
        self.transient_retries.remove(&msg.id);
        RouteOutcome::DeadLettered { reason }
    }

    /// **F1-0-7 — uma tentativa de entrega FALHOU.** Registra `MessageDeliveryFailed`
    /// (timestamps no log = prova do backoff), agenda o retry com backoff exponencial +
    /// jitter determinístico, alimenta o circuit breaker do nó, e — esgotadas as
    /// tentativas — manda à DLQ.
    #[allow(clippy::too_many_arguments)]
    /// **ACHADO-2 — a entrega descobriu o alvo OCUPADO** (`deliver` devolveu
    /// `Ok(Injected{ready:false})`: prompt não-pronto, NADA escrito). NÃO é falha — é o mesmo
    /// destino que a retenção por estado (`MessageRetained`), só que descoberto NA entrega (o
    /// lifecycle julgava Idle). Registra `MessageRetained{prompt_not_ready}` 1× (via
    /// `retained_since`, anti-amplificação A4) e enfileira a msg no MOTOR de re-entrega
    /// (`self.retries`) com `attempts:0` (= RETENÇÃO, não falha) e backoff `NOT_READY_BACKOFF_MS`.
    ///
    /// **Por que o motor de re-entrega (e não só re-drenar):** `MessageRouted` JÁ foi logado (antes
    /// do loop de entrega), então re-drenar cairia no ledger "roteada-sem-entrega ⇒ assume entregue
    /// ⇒ ack" (perderia a msg). O check pré-ledger do pump (`self.retries`) intercepta ANTES disso e
    /// `process_due_retries` re-checa a prontidão REAL com backoff — SEM re-rotear (sem
    /// `MessageRouted` spam) e SEM strike. Backstop: `retention_timeout` (600 s) → DLQ.
    fn retain_not_ready(
        &mut self,
        msg: &MailMessage,
        target: NodeId,
        from: NodeId,
        block: &str,
        root: &str,
        hops: u8,
        store: &mut EventStore,
        now_ms: u64,
    ) -> RouteOutcome {
        if !self.retained_since.contains_key(&msg.id) {
            self.retained_since.insert(msg.id.clone(), now_ms);
            if let Err(e) = store.append(&DomainEvent::MessageRetained {
                id: msg.id.clone(),
                to: target,
                reason: "prompt_not_ready".to_string(),
            }) {
                eprintln!(
                    "lina-core: falha ao logar MessageRetained(prompt_not_ready {}): {e}",
                    msg.id
                );
            }
        }
        // Entrada de RETENÇÃO no motor de re-entrega (`attempts:0` = ocupado, JAMAIS strike). O pump
        // a intercepta no check pré-ledger; `process_due_retries` re-tenta a prontidão com backoff.
        self.retries.insert(
            msg.id.clone(),
            RetryState {
                from,
                target,
                block: block.to_string(),
                root: root.to_string(),
                hops,
                attempts: 0,
                next_ms: now_ms.saturating_add(NOT_READY_BACKOFF_MS),
            },
        );
        // Espelho do retry transiente/Hold: desmarca o dedupe (o check pré-ledger é a guarda real).
        self.seen.remove(&msg.id);
        RouteOutcome::Retained { to: target }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_delivery_failure(
        &mut self,
        msg: &MailMessage,
        store: &mut EventStore,
        target: NodeId,
        sender: NodeId,
        block: &str,
        root: &str,
        hops: u8,
        now_ms: u64,
        error: &str,
    ) -> RouteOutcome {
        // Breaker: falhas CONSECUTIVAS por nó (zeradas em qualquer sucesso).
        let fails = self.node_failures.entry(target).or_insert(0);
        *fails += 1;
        let fails = *fails;
        if fails >= self.config.breaker_threshold && !self.breaker_open.contains(&target) {
            self.breaker_open.insert(target);
            if let Err(e) = store.append(&DomainEvent::CircuitOpened {
                node: target,
                failures: fails,
            }) {
                eprintln!("lina-core: falha ao logar CircuitOpened({target}): {e}");
            }
            // Estado canônico: Blocked com motivo no log (consultável via roster/
            // agents.json/`lina list`; o verbo `lina check` é F1-0-6).
            if let Some(info) = self.sup.get(target) {
                if info.status != NodeStatus::Blocked && info.status != NodeStatus::Dead {
                    if let Err(e) = store.append(&DomainEvent::NodeStatusChanged {
                        node: target,
                        status: NodeStatus::Blocked.as_str().to_string(),
                        from: info.status.as_str().to_string(),
                        reason: "circuit_breaker".to_string(),
                    }) {
                        eprintln!("lina-core: falha ao logar Blocked(circuit_breaker): {e}");
                    }
                    // F3-CONF-2 (#21): o reason "circuit_breaker" viaja no Bus (não só no log),
                    // para a tela honesta surfacar "pausado por segurança" em vez de âmbar "trabalhando".
                    if let Err(e) = self.sup.set_status_with_reason(
                        target,
                        NodeStatus::Blocked,
                        Some("circuit_breaker".to_string()),
                    ) {
                        eprintln!("lina-core: falha ao bloquear {target} (breaker): {e}");
                    }
                }
            }
            eprintln!(
                "lina-core: ⛔ circuit breaker ABERTO para {target} ({fails} falhas \
                 consecutivas) — novas mensagens retêm/DLQ até reset externo"
            );
        }

        let attempt = self.retries.get(&msg.id).map_or(0, |r| r.attempts) + 1;
        if attempt >= self.config.delivery_max_attempts {
            return self.dead_letter_now(
                msg,
                store,
                format!("entrega falhou {attempt}x (ultimo erro: {error}) — tentativas esgotadas"),
            );
        }
        let next_ms =
            now_ms + backoff_with_jitter(self.config.delivery_backoff_base_ms, attempt, &msg.id);
        if let Err(e) = store.append(&DomainEvent::MessageDeliveryFailed {
            id: msg.id.clone(),
            to: target,
            attempt,
            error: error.to_string(),
            next_retry_ms: next_ms,
        }) {
            eprintln!(
                "lina-core: falha ao logar MessageDeliveryFailed({}): {e}",
                msg.id
            );
        }
        self.retries.insert(
            msg.id.clone(),
            RetryState {
                from: sender,
                target,
                block: block.to_string(),
                root: root.to_string(),
                hops,
                attempts: attempt,
                next_ms,
            },
        );
        // Como na retenção: o próximo processamento é do motor de retry (pump), não do
        // pipeline — mas desmarcamos o dedupe por simetria/segurança de re-apresentação.
        self.seen.remove(&msg.id);
        RouteOutcome::RetryBackoff {
            to: target,
            attempt,
        }
    }

    /// **F1-0-7 — processa os retries DEVIDOS** (chamado no topo de cada `pump` tick).
    /// Não re-roda o pipeline (guardrails/persist já aconteceram na rota original): tenta
    /// a ENTREGA direto com o bloco original. Sucesso → `MessageDelivered` + binding A2 +
    /// busy-mark + ack. Falha → próxima tentativa/DLQ. Breaker aberto → segura sem queimar
    /// tentativa.
    fn process_due_retries<D>(&mut self, store: &mut EventStore, now_ms: u64, deliver: &mut D)
    where
        D: FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String>,
    {
        let due: Vec<String> = self
            .retries
            .iter()
            .filter(|(_, r)| r.next_ms <= now_ms)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            let Some(state) = self.retries.get(&id) else {
                continue;
            };
            let (target, from, attempts) = (state.target, state.from, state.attempts);
            // Breaker aberto (ainda Blocked) → não queima tentativa; reset externo
            // detectado libera (mesma regra do veredito).
            if self.breaker_open.contains(&target) {
                if self.sup.get(target).map(|i| i.status) == Some(NodeStatus::Blocked) {
                    continue;
                }
                self.breaker_open.remove(&target);
                self.node_failures.remove(&target);
            }
            // Alvo ocupado no momento do retry → espera o próximo tick (sem queimar
            // tentativa; a retenção por estado vale também aqui).
            if self.sup.get(target).map(|i| i.status) == Some(NodeStatus::Busy) {
                continue;
            }
            let (block, root, hops) = {
                let s = &self.retries[&id];
                (s.block.clone(), s.root.clone(), s.hops)
            };
            match deliver(target, from, &block) {
                // ACHADO-2: só conta como ENTREGUE se de fato injetou (`injected()`) — uma
                // re-tentativa que reencontra o alvo ocupado (`Ok(Injected{ready:false})`) NÃO é
                // entrega (cairia em "entregue mas nada escrito"): cai no arm de retenção abaixo.
                Ok(o) if o.injected() => {
                    self.node_failures.insert(target, 0);
                    self.retries.remove(&id);
                    match store.append(&DomainEvent::MessageDelivered {
                        id: id.clone(),
                        to: target,
                    }) {
                        // F3-CONF-2: re-arma o vigia de engolimento (entrega via retry conta igual).
                        Ok(seq) => {
                            self.dispatched_at.insert(target, (now_ms, seq));
                        }
                        Err(e) => {
                            eprintln!(
                                "lina-core: CRÍTICO — retry entregue mas NÃO logado ({id}): {e}"
                            )
                        }
                    }
                    // Mesmos efeitos da entrega original: binding A2 + busy-mark.
                    let keep_existing = matches!(
                        self.delivered_root.get(&target),
                        Some((existing, _, _)) if *existing != root
                    );
                    if !keep_existing {
                        self.delivered_root.insert(target, (root, hops, now_ms));
                    }
                    self.mark_busy_after_delivery(store, target);
                    if let Err(e) = self.mailbox.ack_inflight(&id) {
                        eprintln!("lina-core: retry entregue mas ack falhou ({id}): {e}");
                    }
                }
                // ACHADO-2: a re-tentativa achou o alvo AINDA ocupado (prompt não-pronto) → segue
                // RETIDO (re-agenda backoff), SEM strike nem incremento de `attempts`. DLQ só pelo
                // teto de retenção (alvo nunca ficou pronto = mensagem morta de verdade).
                Ok(_) => {
                    let first = self.retained_since.get(&id).copied().unwrap_or(now_ms);
                    if now_ms.saturating_sub(first) >= self.config.retention_timeout_ms {
                        let msg = self
                            .mailbox
                            .inflight_message(&id)
                            .unwrap_or_else(|| minimal_msg_for_dlq(&id, &target.to_string()));
                        let out = self.dead_letter_now(
                            &msg,
                            store,
                            format!(
                                "retencao por prompt-nao-pronto estourou o teto ({} ms) — alvo nunca ficou pronto",
                                self.config.retention_timeout_ms
                            ),
                        );
                        if matches!(out, RouteOutcome::DeadLettered { .. }) {
                            self.retries.remove(&id);
                            if let Err(e) = self.mailbox.ack_inflight(&id) {
                                eprintln!(
                                    "lina-core: DLQ (retencao) ok mas ack falhou ({id}): {e}"
                                );
                            }
                        }
                    } else if let Some(s) = self.retries.get_mut(&id) {
                        // Re-agenda: o backoff throttla o re-`wait_ready` (sem busy-loop); `attempts`
                        // permanece 0 (retenção). `MessageRetained` já foi logado 1× (retained_since).
                        s.next_ms = now_ms.saturating_add(NOT_READY_BACKOFF_MS);
                    }
                }
                Err(e) => {
                    // Reconstrói uma MailMessage mínima p/ o caminho de falha/DLQ (o
                    // arquivo ORIGINAL segue no `.inflight` — a DLQ relê de lá se preciso).
                    let msg = self
                        .mailbox
                        .inflight_message(&id)
                        .unwrap_or_else(|| minimal_msg_for_dlq(&id, &target.to_string()));
                    let _ = attempts; // attempts re-lido em on_delivery_failure via self.retries
                    let out = self.on_delivery_failure(
                        &msg, store, target, from, &block, &root, hops, now_ms, &e,
                    );
                    if let RouteOutcome::DeadLettered { .. } = out {
                        if let Err(e) = self.mailbox.ack_inflight(&id) {
                            eprintln!("lina-core: DLQ ok mas ack do inflight falhou ({id}): {e}");
                        }
                    }
                }
            }
        }
    }

    /// Remove entradas de dedupe (`seen`) E de orçamento (`budget`) mais velhas que a janela. As duas
    /// crescem com o tráfego; sem podar `budget` junto, ele vazaria memória numa sessão longa (A5),
    /// ao contrário de `seen` que já era podado. F1-0-4: também poda os relógios de retenção.
    fn prune_expired(&mut self, now_ms: u64) {
        let window = self.config.dedupe_window_ms;
        self.seen
            .retain(|_, &mut t| now_ms.saturating_sub(t) < window);
        self.budget
            .retain(|_, e| now_ms.saturating_sub(e.last_ms) < window);
        // F1-0-4: cinto anti-vazamento (normalmente limpo na entrega/ack): relógios de
        // retenção muito além do teto.
        let retention_cap = self
            .config
            .retention_timeout_ms
            .saturating_mul(4)
            .max(window);
        self.retained_since
            .retain(|_, &mut t| now_ms.saturating_sub(t) < retention_cap);
        // Round 5 #8: poda bindings OCIOSOS (stale > janela), EXCETO de nós com await PENDENTE — podar
        // mid-await deixaria um root fresco roubar a cadeia ainda viva (o `await_timeout` pode exceder
        // a janela de dedupe). A poda + re-set na entrega é o que MIGRA o binding p/ a cadeia ativa,
        // sem fragmentar o grafo anti-loop numa sessão longa (lição A5: estado que cresce sem poda).
        let awaiting: HashSet<NodeId> = self
            .pending
            .values()
            .flat_map(|t| [t.waiter, t.target])
            .collect();
        self.delivered_root.retain(|node, (_, _, last)| {
            now_ms.saturating_sub(*last) < window || awaiting.contains(node)
        });
    }

    /// **A2 + W1: deriva o `(root_cause_id, hops)` EFETIVOS** de uma mensagem de `sender`.
    /// **SEGURANÇA (W1):** o outbox é canal NÃO-autenticado → NUNCA confiar em `msg.root_cause_id`/
    /// `msg.hops` para DECISÃO DE CADEIA (são forjáveis; um root fresco por msg zeraria orçamento,
    /// esvaziaria a partição do grafo anti-loop e manteria hops=0 — burlando os 3 guardrails de uma
    /// vez). O root/hops vêm SEMPRE do binding interno que o supervisor carimba na ENTREGA
    /// (`delivered_root[sender]`); sem binding, o nó é a própria raiz a profundidade 0. (O campo no
    /// bloco renderizado segue informativo p/ o agente; o ENFORCEMENT não o usa.)
    fn derive_root_hops(&self, msg: &MailMessage, sender: NodeId) -> (String, u8) {
        match self.delivered_root.get(&sender) {
            Some((root, h, _)) => (root.clone(), h.saturating_add(1)),
            None => (msg.id.clone(), 0),
        }
    }

    /// **SEAM-1 (M4) — semeia o binding `delivered_root` de um nó RECÉM-ADMITIDO** (o filho de um
    /// `lina spawn`), para o anti-fork-bomb valer END-TO-END. Sem isto, `admit_node` (que NÃO passa
    /// por `route_message`) cria o filho SEM binding → seu próprio `lina spawn @neto` seria ORIGEM
    /// (`derive_root_hops` devolve `(msg.id, 0)`) e escaparia do gate de cascata. Com o binding
    /// semeado pelo `(root, hops)` EFETIVOS do spawn (de `SpawnApproved.root_cause_id`/`SpawnRequested.hops`
    /// — INFORJÁVEIS, do gate), `derive_root_hops(filho)` devolve `hops+1 >= 1` → CASCATA → gate humano.
    ///
    /// **Segurança:** o app passa `root`/`hops` que NASCERAM no binding inforjável do gate (jamais
    /// campo de agente) — não reabre forja. Respeita a defesa P1 (anti-envenenamento, igual
    /// `route_message`): NÃO sobrescreve um binding VIVO de root DIFERENTE. Para um nó recém-criado
    /// não há binding → insere; o guard é defesa em profundidade (re-seed idempotente).
    pub fn seed_delivered_root(&mut self, node: NodeId, root: String, hops: u8, now_ms: u64) {
        let keep_existing = matches!(
            self.delivered_root.get(&node),
            Some((existing, _, _)) if *existing != root
        );
        if !keep_existing {
            self.delivered_root.insert(node, (root, hops, now_ms));
        }
    }

    /// **W3-7b (ADR 0002) + Round 4: fecha o `await` se `msg` é o REPLY AUTENTICADO casado.** Devolve
    /// `true` SE e só se: (1) `msg.reply_to` casou um ticket REAL em `pending`, (2) o `sender` (já
    /// resolvido pelo supervisor) é o `target` que o waiter aguardava, e (3) o `to` da resposta é o
    /// próprio waiter. Só então: `end_await(waiter)` (libera o wait-for-graph) + `AwaitClosed{Replied}`
    /// + remove de `pending`.
    ///
    /// **SEGURANÇA (Round 4, confused-deputy):** o `id` da pergunta é PÚBLICO (vaza no `log.jsonl` e no
    /// bloco `[LINA::MSG]`). Sem a checagem `sender == ticket.target`, qualquer nó C (identidade real,
    /// sem forjar `from`) fecharia o await de B só escrevendo `{from:@C, reply_to:q.id}` — apagando a
    /// aresta `B→A` do wait-for-graph, injetando um `AwaitClosed{Replied}` espúrio e pulando o
    /// anti-loop. O fechamento NUNCA é decidido pelo campo do outbox: só o PARTICIPANTE autenticado
    /// (o `target` aguardado) vence. Sem casar → `false`: a msg de C segue como mensagem COMUM, sujeita
    /// a todos os guardrails. PEEK antes de remover — um reply ilegítimo NÃO consome o ticket.
    fn close_await_on_reply(
        &mut self,
        msg: &MailMessage,
        sender: NodeId,
        store: &mut EventStore,
    ) -> bool {
        let Some(reply_to) = msg.reply_to.as_deref() else {
            return false;
        };
        let Some(&ticket) = self.pending.get(reply_to) else {
            return false; // reply_to forjado / sem ticket → não autoriza pular o anti-loop
        };
        // O remetente da resposta TEM de ser o `target` que o waiter aguardava (não um terceiro que
        // só leu o `id` no log). E a resposta tem de voltar PARA o waiter (defesa em profundidade).
        if sender != ticket.target {
            return false;
        }
        let to_is_waiter = matches!(
            parse_target(&msg.to),
            TargetSpec::Name(n) if self.sup.node_by_name(&n) == Some(ticket.waiter)
        );
        if !to_is_waiter {
            return false;
        }
        // Autenticado → AGORA consome o ticket e fecha SÓ esta aresta (Round 5 #7: outros awaits do
        // mesmo waiter seguem vivos).
        self.pending.remove(reply_to);
        self.sup
            .end_await_ticket(ticket.waiter, reply_to, ticket.target);
        if let Err(e) = store.append(&DomainEvent::AwaitClosed {
            id: reply_to.to_string(),
            reason: AwaitReason::Replied,
        }) {
            eprintln!("lina-core: falha ao logar AwaitClosed(replied) p/ {reply_to}: {e}");
        }
        true
    }

    /// **W3-7b (ADR 0002): fecha tickets de `await` vencidos** (`now - opened_ms > await_timeout_ms`)
    /// com `AwaitClosed{Timeout}` + `end_await`. Chamado no `pump` — evita ticket/aresta órfã eterna
    /// (lição A5: estado que cresce sem poda). Sem reply nem timeout, o ticket persistiria.
    fn sweep_await_timeouts(&mut self, store: &mut EventStore, now_ms: u64) {
        let timeout = self.config.await_timeout_ms;
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, t)| now_ms.saturating_sub(t.opened_ms) > timeout)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(ticket) = self.pending.remove(&id) {
                // Round 5 #7: fecha SÓ a aresta deste ticket vencido.
                self.sup.end_await_ticket(ticket.waiter, &id, ticket.target);
                if let Err(e) = store.append(&DomainEvent::AwaitClosed {
                    id: id.clone(),
                    reason: AwaitReason::Timeout,
                }) {
                    eprintln!("lina-core: falha ao logar AwaitClosed(timeout) p/ {id}: {e}");
                }
            }
        }
    }

    /// **W3-5: aplica um intent de plano** (`plan.claim`/`plan.check`). Valida o comando contra a
    /// projeção atual (fonte da verdade), loga o evento ANTES de tocar o disco, e reescreve o
    /// `.lina/plan.md` (escritor único). NÃO entrega a nenhuma PTY.
    fn handle_plan(&mut self, msg: &MailMessage, store: &mut EventStore) -> RouteOutcome {
        // `plan.add`/`plan.seed` carregam os dados no PAYLOAD (não no `--ref` de claim/check) — ramifica
        // ANTES do `plan_ref()` obrigatório, senão seriam recusados por "sem ref" (o caminho de produção
        // que faltava: até aqui só os testes os exercitavam, montando os eventos à mão).
        match msg.intent.as_str() {
            "plan.add" => return self.handle_plan_add(msg, store),
            "plan.seed" => return self.handle_plan_seed(msg, store),
            _ => {}
        }
        let Some(item) = msg.plan_ref() else {
            return RouteOutcome::PlanRejected("mensagem de plano sem campo 'ref'".into());
        };
        let item = item.to_string();
        let by = msg.from.clone();

        // Projeção atual = base da VALIDAÇÃO. `plan` é mutado pelo comando e, se aceito, vira a
        // projeção pós-evento (apply_* infalível == a mutação do try_*), poupando reprojeção.
        let mut plan = match store.project() {
            Ok(state) => state.plan,
            Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
        };

        let event = match msg.intent.as_str() {
            "plan.claim" => match plan.try_claim(&item, &by) {
                Ok(()) => DomainEvent::PlanClaimed {
                    id: msg.id.clone(),
                    item: item.clone(),
                    by: by.clone(),
                },
                Err(e) => return RouteOutcome::PlanRejected(e.to_string()),
            },
            "plan.check" => match plan.try_check(&item, &by) {
                Ok(()) => DomainEvent::PlanChecked {
                    id: msg.id.clone(),
                    item: item.clone(),
                    by: by.clone(),
                },
                Err(e) => return RouteOutcome::PlanRejected(e.to_string()),
            },
            other => {
                return RouteOutcome::PlanRejected(format!("intent de plano desconhecido: {other}"))
            }
        };

        // Log = fonte da verdade ANTES de escrever a projeção (não escrevemos o que não logamos).
        if let Err(e) = store.append(&event) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        if let Err(e) = self.mailbox.write_plan(&plan.render()) {
            // O evento já está no log → o replay reconstrói. A projeção em disco divergiu: ALTO e
            // VISÍVEL (stderr), nunca silencioso (invariante #4 — o log é a fonte da verdade).
            eprintln!(
                "lina-core: CRÍTICO — plan.{} logado mas plan.md não reescrito: {e}",
                msg.intent
            );
        }
        // F3-1-4 (spec 52 §3): o `plan.check` é o GATILHO do OUTER loop. Se o item serve a uma Goal,
        // o juiz estrutural roda os critérios de aceite e avança o laço (ReviewVerdict → GoalAchieved
        // / GoalEscalated / escala de effort). `plan.claim` não dispara o juiz (o item nem terminou).
        if msg.intent == "plan.check" {
            if let Some(checked) = plan.itens.iter().find(|i| i.id == item).cloned() {
                if let Some(goal_id) = checked.goal_id.clone() {
                    if let Err(e) = self.review_and_advance(&goal_id, &checked, store) {
                        eprintln!("lina-core: CRÍTICO — OUTER loop da Goal falhou em {item}: {e}");
                    }
                }
            }
        }
        RouteOutcome::PlanApplied {
            intent: msg.intent.clone(),
            item,
        }
    }

    /// **F3-4-3 (spec 36 §2): handler do sinal de mudança `code.changed`.** Apenda `CodeChanged` com
    /// `author_node` CARIMBADO SERVER-SIDE (= `msg.from`, o remetente JÁ autenticado por
    /// [`Supervisor::node_by_name`] no topo de `route_message`), IGNORANDO qualquer `author_node` no
    /// payload (regra-mãe ADR 0007: campo escrito por agente nunca decide identidade). Depois faz o
    /// **broadcast por pertencimento**: cruza `paths`×claims ATIVOS (`crate::code::intersecting_paths`,
    /// ZERO LLM) e entrega a notificação de PARADA SÓ aos donos afetados (≠ autor) que estão vivos — quem
    /// não tem interseção é ignorado em silêncio. O item de atenção `CodeConflict` é reconstruído à parte
    /// pela projeção da fila (do mesmo `CodeChanged` no log). A comparação só ALERTA, nunca concede posse.
    fn handle_code_changed<D>(
        &mut self,
        msg: &MailMessage,
        sender: NodeId,
        store: &mut EventStore,
        deliver: &mut D,
    ) -> RouteOutcome
    where
        D: FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String>,
    {
        let payload: serde_json::Value =
            serde_json::from_str(&msg.payload).unwrap_or(serde_json::Value::Null);
        let branch = payload
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if branch.is_empty() {
            return RouteOutcome::ContractRejected("code.changed exige 'branch' no payload".into());
        }
        let commit = payload
            .get("commit")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let paths: Vec<String> = payload
            .get("paths")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // `author_node` = remetente AUTENTICADO (NOME), JAMAIS o payload (ADR 0007). O `sender` (NodeId)
        // já é o binding inforjável; o NOME (`msg.from`) é o mesmo espaço dos `owner` dos claims, então a
        // comparação autor×dono é direta e a projeção da fila reconstrói o conflito do mesmo carimbo.
        let author = msg.from.clone();
        if let Err(e) = store.append(&DomainEvent::CodeChanged {
            branch: branch.clone(),
            paths: paths.clone(),
            author_node: author.clone(),
            commit,
        }) {
            return RouteOutcome::PersistFailed(e.to_string());
        }

        // Broadcast por PERTENCIMENTO: quem tem claim ATIVO (Doing) que intersecta os paths, e NÃO é o
        // autor, PARA. Notifica só os vivos; sem interseção → silêncio (nó não muda de estado).
        let plan = match store.project() {
            Ok(state) => state.plan,
            Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
        };
        let mut targets: Vec<NodeId> = Vec::new();
        for item in &plan.itens {
            let Some(owner) = item.owner.as_deref() else {
                continue;
            };
            if owner == author || item.status != ItemState::Doing {
                continue;
            }
            if crate::code::intersecting_paths(&paths, &item.paths).is_empty() {
                continue;
            }
            if let Some(owner_node) = self.sup.node_by_name(owner) {
                if targets.contains(&owner_node) {
                    continue; // um dono com vários itens afetados é avisado uma vez
                }
                let note = format!(
                    "o colega ({author}) mexeu na branch {branch} em arquivo que você reservou — pare e avalie (rebase, renegociar ou desconflitar)"
                );
                // Entrega best-effort: a notificação é o "PARA" do pertencimento; a direção humana
                // PERSISTE no `CodeConflict` da fila (do log), então uma falha de injeção não perde o
                // sinal. Convenção `deliver(target, from, texto)`: alvo = o dono que PARA; from = o autor.
                let _ = deliver(owner_node, sender, &note);
                targets.push(owner_node);
            }
        }
        RouteOutcome::Delivered { targets }
    }

    /// **F3-4-5 (spec 36 §3): handler da prova de integração `branch.integrated`.** Apenda
    /// `BranchIntegrated{branch, into, commit}` — a ÚNICA prova de que uma branch foi fechada (a
    /// projeção [`crate::code::branches_nao_integradas`] só a remove do pendente com este evento,
    /// nunca por suposição). `branch`/`into`/`commit` são DADO descritivo do git (os fatos do merge),
    /// não identidade: diferente de `code.changed`, aqui não há autor a carimbar nem pertencimento a
    /// cruzar — a AUTORIDADE da integração é o gate humano das salvaguardas git (ADR 0042: nunca
    /// push --force/reset --hard/apagar não-mergeada), não este registro. `branch` vazio é rejeitado.
    fn handle_branch_integrated(
        &mut self,
        msg: &MailMessage,
        store: &mut EventStore,
    ) -> RouteOutcome {
        let payload: serde_json::Value =
            serde_json::from_str(&msg.payload).unwrap_or(serde_json::Value::Null);
        let branch = payload
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if branch.is_empty() {
            return RouteOutcome::ContractRejected(
                "branch.integrated exige 'branch' no payload".into(),
            );
        }
        let into = payload
            .get("into")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let commit = payload
            .get("commit")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Err(e) = store.append(&DomainEvent::BranchIntegrated {
            branch,
            into,
            commit,
        }) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        // Registro puro: ninguém é notificado (a prova vive no log; a projeção de pendências a lê).
        RouteOutcome::Delivered {
            targets: Vec::new(),
        }
    }

    /// Núcleo compartilhado de `plan.add`/`plan.seed`: semeia o item (`PlanItemAdded`) e o ATRIBUI à
    /// Goal/parents/DoD (`PlanItemAttributed`), mutando o `plan` que o caller projetou (e que ele depois
    /// reescreve no `plan.md` — o `plan.seed` emite o `GoalDecomposed` no meio). `Some(_)` = rejeição
    /// (id duplicado) ou persistência, pronto para o caller `return`; `None` = item criado e logado
    /// (COD-5: sem duplicar a sequência log-antes-de-projetar do invariante #4).
    fn attribute_item(
        &mut self,
        store: &mut EventStore,
        plan: &mut Plan,
        attr: ItemAttribution,
    ) -> Option<RouteOutcome> {
        let ItemAttribution {
            item,
            desc,
            goal_id,
            parents,
            acceptance,
            budget_tokens,
            paths,
        } = attr;
        if let Err(e) = plan.add_item(item.clone(), desc.clone()) {
            return Some(RouteOutcome::PlanRejected(e.to_string()));
        }
        if let Err(e) = store.append(&DomainEvent::PlanItemAdded {
            item: item.clone(),
            desc,
        }) {
            return Some(RouteOutcome::PersistFailed(e.to_string()));
        }
        plan.apply_item_attributed(
            &item,
            goal_id.clone(),
            parents.clone(),
            acceptance.clone(),
            budget_tokens,
            paths.clone(),
        );
        if let Err(e) = store.append(&DomainEvent::PlanItemAttributed {
            item,
            goal_id,
            parents,
            acceptance,
            budget_tokens,
            paths,
        }) {
            return Some(RouteOutcome::PersistFailed(e.to_string()));
        }
        None
    }

    /// **F3-1-6 (spec 52 §2): `plan.add`** — promove um item ao plano e o ATRIBUI a uma Goal (DoD por
    /// item). Ao contrário do nível-Goal (HumanReview forçado em `handle_goal`), aqui o `check_kind` é
    /// RESPEITADO: este é o critério verificável por máquina — quando o item é `check`ado, o juiz o roda
    /// sob `guard::decide` (um `Command` `GatedHard` continua pedindo `Ask`; o critério verifica, não autoriza).
    fn handle_plan_add(&mut self, msg: &MailMessage, store: &mut EventStore) -> RouteOutcome {
        let payload: serde_json::Value =
            serde_json::from_str(&msg.payload).unwrap_or(serde_json::Value::Null);
        let item = payload
            .get("item")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if item.is_empty() {
            return RouteOutcome::PlanRejected("plan.add exige o campo 'item'".into());
        }
        let desc = payload
            .get("desc")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let goal_id = payload
            .get("goal_id")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let parents = payload
            .get("parents")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let acceptance: Vec<AcceptanceCriterion> = payload
            .get("acceptance")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let budget_tokens = payload
            .get("budget_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        // F3-4-3 (ADR 0041): `paths` reservados — mesmo parse opcional de `parents` (ausente → []);
        // assim `lina plan add` sem `--paths` segue idêntico, e com paths o item declara o que reserva.
        let paths = payload
            .get("paths")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut plan = match store.project() {
            Ok(state) => state.plan,
            Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
        };
        if let Some(outcome) = self.attribute_item(
            store,
            &mut plan,
            ItemAttribution {
                item: item.clone(),
                desc,
                goal_id,
                parents,
                acceptance,
                budget_tokens,
                paths,
            },
        ) {
            return outcome;
        }
        if let Err(e) = self.mailbox.write_plan(&plan.render()) {
            eprintln!("lina-core: CRÍTICO — plan.add logado mas plan.md não reescrito: {e}");
        }
        RouteOutcome::PlanApplied {
            intent: msg.intent.clone(),
            item,
        }
    }

    /// **F3-1-6 (spec 52 §2, gate de saída (a)): `plan.seed`** — DECOMPÕE uma Goal CONFIRMADA em ≥1 item
    /// do plano. A decomposição é DETERMINÍSTICA (invariante #1: ZERO LLM no core): 1 item espelha a meta
    /// e herda seus critérios de aceite. A decomposição RICA em N sub-tarefas é do Maestro-agente via
    /// `plan add` (porta aberta da F3-2), JAMAIS de cognição no core. Emite `PlanItemAdded` +
    /// `PlanItemAttributed` + `GoalDecomposed`. Recusa meta fora de `Confirmed` (a fase é o trinco
    /// idempotente — re-seed de uma meta já `Decomposed` é recusado pelo gate de ciclo, não pelo bin).
    fn handle_plan_seed(&mut self, msg: &MailMessage, store: &mut EventStore) -> RouteOutcome {
        let payload: serde_json::Value =
            serde_json::from_str(&msg.payload).unwrap_or(serde_json::Value::Null);
        let goal_id = payload
            .get("goal_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if goal_id.is_empty() {
            return RouteOutcome::GoalRejected("plan.seed exige o campo 'goal_id'".into());
        }
        let events = match store.events() {
            Ok(events) => events,
            Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
        };
        let goals = crate::project_goals(&events);
        let Some(goal) = goals.iter().find(|g| g.goal_id == goal_id) else {
            return RouteOutcome::GoalRejected(format!("meta {goal_id} nao existe no log"));
        };
        // GATE DE CICLO (a recusa é do supervisor, não do bin): só decompõe a meta CONFIRMADA — antes do
        // gate humano não há o que decompor; depois (`Decomposed`/`InLoop`/...) a fase barra o re-seed.
        if goal.phase != crate::GoalPhase::Confirmed {
            return RouteOutcome::GoalRejected(format!(
                "meta {goal_id} precisa estar confirmada para decompor (fase atual nao permite)"
            ));
        }
        // Item id DETERMINÍSTICO derivado do goal_id (`chars().take(8)` — nunca panica, COD-4): único
        // entre metas e idempotente no replay (o evento carrega o id literal). 1 item herda o statement
        // + os critérios da Goal (vazio → o juiz defere ao humano via turn-budget).
        let short: String = goal_id.chars().take(8).collect();
        let item = format!("g{short}");
        let desc = goal.statement.clone();
        let acceptance = goal.acceptance.clone();

        let mut plan = match store.project() {
            Ok(state) => state.plan,
            Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
        };
        if let Some(outcome) = self.attribute_item(
            store,
            &mut plan,
            ItemAttribution {
                item: item.clone(),
                desc,
                goal_id: Some(goal_id.clone()),
                parents: Vec::new(),
                acceptance,
                budget_tokens: 0,
                // Item de meta decomposta não declara paths (a decomposição rica em sub-tarefas com
                // arquivos reservados é do Maestro-agente via `plan add --paths`, porta da F3-2).
                paths: Vec::new(),
            },
        ) {
            return outcome;
        }
        if let Err(e) = store.append(&DomainEvent::GoalDecomposed {
            goal_id: goal_id.clone(),
            plan_items: vec![item],
        }) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        if let Err(e) = self.mailbox.write_plan(&plan.render()) {
            eprintln!("lina-core: CRÍTICO — plan.seed logado mas plan.md não reescrito: {e}");
        }
        RouteOutcome::GoalApplied {
            intent: msg.intent.clone(),
            goal_id,
        }
    }

    /// **F3-1-4/5 (spec 52 §3): o OUTER loop da Goal** — disparado por um `plan.check` de um item que
    /// serve a uma Goal. O Lina roda SÓ o OUTER loop (invariante #1: o INNER é o CLI de terceiro):
    /// observa o item declarado pronto, roda o JUIZ ESTRUTURAL (`run_acceptance`, ZERO LLM), emite o
    /// `ReviewVerdict` (juiz ≠ executor — gate d) e decide a próxima ação:
    /// - **Pass** + todos os itens da Goal com último veredito `Pass` → exatamente um `GoalAchieved`.
    /// - **Fail** + `iteration >= goal_max_iterations` → `GoalEscalated{turn_budget_exhausted}` (PAUSA
    ///   resumível, nunca mais um turno — o backstop do ponto [9]).
    /// - **Fail** abaixo do teto → escala de effort + re-despacho informado (doc-fonte linha 12).
    ///
    /// `None` de [`run_acceptance`] (HumanReview / fail-open) DEFERE — sem veredito automático.
    fn review_and_advance(
        &mut self,
        goal_id: &str,
        item: &PlanItem,
        store: &mut EventStore,
    ) -> Result<(), StoreError> {
        // 1) Juiz estrutural. DEFERE (None) → o gate humano/QA decide; o turn-budget é o backstop.
        let Some((verdict, evidence, defect_class)) =
            run_acceptance(&item.acceptance, self.config.autonomy)
        else {
            return Ok(());
        };
        // 2) target = o owner do item (o "desenvolvedor" revisado). Sem owner vivo → nada a revisar.
        let Some(target) = item.owner.as_deref().and_then(|o| self.sup.node_by_name(o)) else {
            return Ok(());
        };
        // 3) iteração corrente = falhas anteriores + 1 (a tentativa que acabou de ser julgada).
        let iteration = count_item_fails(store, goal_id, &item.id)? + 1;
        // 4) emite o ReviewVerdict com o juiz ESTRUTURAL (reviewer != target — ENFORÇADO).
        let verdict_seq = match emit_review_verdict(
            store,
            goal_id,
            &item.id,
            target,
            STRUCTURAL_JUDGE,
            verdict,
            &evidence,
            &defect_class,
            iteration,
        )? {
            Some(seq) => seq,
            None => return Ok(()), // recusado (reviewer==target) — não avança.
        };
        // 5) avança o laço.
        match verdict {
            GoalVerdict::Pass => {
                let plan = store.project()?.plan;
                if all_items_pass(store, &plan, goal_id)? {
                    let iterations = max_goal_iteration(store, goal_id)?;
                    store.append(&DomainEvent::GoalAchieved {
                        goal_id: goal_id.to_string(),
                        iterations,
                        verdict_event_id: format!("seq:{verdict_seq}"),
                    })?;
                }
            }
            GoalVerdict::Fail => {
                if iteration >= self.config.goal_max_iterations {
                    store.append(&DomainEvent::GoalEscalated {
                        goal_id: goal_id.to_string(),
                        reason: "turn_budget_exhausted".to_string(),
                    })?;
                } else {
                    self.escalate_on_fail(
                        store,
                        goal_id,
                        item,
                        target,
                        iteration,
                        &evidence,
                        &defect_class,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// **F3-1-4/F3-2-5 (spec 52 §3/§4, gate d/e): escala no `Fail`** abaixo do turn-budget. O degrau de
    /// `effort_ladder` sobe com a iteração (capado no topo; `Effort: Ord` torna "estritamente maior"
    /// uma comparação). Distingue, como o doc-fonte (linha 12):
    /// - **re-despacho ao MESMO** dev — o degrau NÃO subiu desde a iteração anterior: re-injeta no
    ///   `target` (mesmo `name`) com evidência no prompt ("Corrija e re-submeta").
    /// - **NOVO desenvolvedor** — o degrau SUBIU (effort estritamente maior): `name` distinto e prompt
    ///   carregando as "tentativas anteriores" (o "novo desenvolvedor com effort maior").
    ///
    /// **Breaker sticky:** nunca emite um re-spawn IDÊNTICO (mesma tarefa+effort — [`count_respawns_with_effort`]).
    /// Quando a escada satura no topo, o re-spawn repetiria o mesmo effort → SUPRIME e defere ao
    /// turn-budget/gate humano (com escada de 1 degrau, morde já na 2ª falha: "pare de auto-escalar").
    ///
    /// A admissão física (LINA_EFFORT no PTY filho; criar o terminal do novo dev) é do app
    /// (`handle_spawn`/`admit_node`); aqui só o LIVRO-RAZÃO do pedido, com `goal_id` que o justifica.
    #[allow(clippy::too_many_arguments)]
    fn escalate_on_fail(
        &mut self,
        store: &mut EventStore,
        goal_id: &str,
        item: &PlanItem,
        target: NodeId,
        iteration: u32,
        evidence: &str,
        defect_class: &str,
    ) -> Result<(), StoreError> {
        let ladder = self.config.effort_ladder;
        if ladder.is_empty() {
            return Ok(()); // sem escada configurada → nada a escalar (turn-budget ainda freia).
        }
        // degrau pela iteração, capado no topo da escada (1ª falha → degrau 0; sobe a cada volta).
        let degrau = (iteration as usize).min(ladder.len()).saturating_sub(1);
        let effort = ladder[degrau];
        // BREAKER STICKY: repetir um re-spawn idêntico (mesma tarefa+effort) não adianta — defere ao
        // turn-budget/gate humano. Capa a auto-escalada quando a escada satura (gate d, doc-fonte 12).
        if count_respawns_with_effort(store, goal_id, &item.id, effort)? >= 1 {
            return Ok(());
        }
        // re-mesmo vs novo-dev: o degrau SUBIU em relação à iteração anterior?
        let prev_degrau = (iteration.saturating_sub(1) as usize)
            .min(ladder.len())
            .saturating_sub(1);
        let novo_dev = degrau > prev_degrau;
        // name/role do alvo via projeção (a admissão física resolve os finais — fronteira core/shell).
        let (target_name, role) = match store.project()?.nodes.get(&target) {
            Some(n) => (
                n.name
                    .clone()
                    .unwrap_or_else(|| item.owner.clone().unwrap_or_default()),
                n.role.clone().unwrap_or_default(),
            ),
            None => (item.owner.clone().unwrap_or_default(), String::new()),
        };
        let (id, name, prompt) = if novo_dev {
            (
                format!("goal-newdev:{goal_id}:{}:{degrau}", item.id),
                // `name` distinto e determinístico (replay idempotente) → o app cria um NOVO terminal.
                format!("{target_name}#reforco{degrau}"),
                format!(
                    "NOVO desenvolvedor (effort reforçado) para o item {}. As tentativas anteriores \
                     não fecharam — última evidência: {evidence}. Defeito: {defect_class}. Assuma \
                     com mais cuidado.",
                    item.id
                ),
            )
        } else {
            (
                format!("goal-respawn:{goal_id}:{}:{iteration}", item.id),
                target_name,
                format!(
                    "Revisão do item {} FALHOU (iteração {iteration}). Evidência: {evidence}. \
                     Defeito: {defect_class}. Corrija e re-submeta.",
                    item.id
                ),
            )
        };
        store.append(&DomainEvent::SpawnRequested {
            id,
            requested_by: STRUCTURAL_JUDGE,
            name,
            role,
            root_cause_id: goal_id.to_string(),
            hops: 1,
            prompt,
            model: None,
            effort: Some(effort),
            goal_id: Some(goal_id.to_string()),
        })?;
        Ok(())
    }

    /// **F3-CONF-2: desarma os vigias de engolimento por PROGRESSO** lendo o DELTA do log
    /// (`seq > dispatch_progress_seq`) — incremental, JAMAIS full-replay (o `O(N)` por tick foi a
    /// causa-raiz documentada do freeze). Progresso atribuível ao nó (mesma noção do `attention.rs`):
    /// `TokenUsageReported{node}` (todo turno real consome tokens), `MessageRouted{from}` (delegou
    /// adiante), `NodeStatusChanged{Busy,pty_output}` (output novo) — ou morte (`Dead`/removido). O
    /// `seq` GUARDA contra corrida: só desarma se o progresso veio APÓS a arma (`P > arm_seq`); um
    /// sinal ANTERIOR a uma re-arma não a desfaz. Idempotente: chamar no `pump` E no `route_message`
    /// (caminho do loop) avança o cursor uma vez por evento.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    fn sync_dispatch_progress(&mut self, store: &EventStore) -> Result<(), StoreError> {
        let delta = store.events_since(self.dispatch_progress_seq)?;
        let Some(max_seq) = delta.last().map(|r| r.seq) else {
            return Ok(()); // ocioso: nada novo
        };
        for rec in &delta {
            let p = rec.seq;
            let str_field = |k: &str| rec.payload.get(k).and_then(|v| v.as_str());
            // O nó cujo progresso o evento atesta (None = evento sem relevância p/ o vigia).
            let node: Option<NodeId> = match rec.kind.as_str() {
                "TokenUsageReported" => str_field("node").and_then(|s| s.parse().ok()),
                "MessageRouted" => str_field("from").and_then(|s| s.parse().ok()),
                "NodeRemoved" | "TerminalExited" => str_field("node").and_then(|s| s.parse().ok()),
                "NodeStatusChanged" => {
                    // Output novo (Busy+pty_output) = progresso; Dead = morte. O carimbo `a2a_delivery`
                    // da própria entrega NÃO é trabalho (filtrado, espelha attention.rs).
                    let busy_output = str_field("status") == Some(NodeStatus::Busy.as_str())
                        && str_field("reason") == Some(crate::lifecycle::reason::PTY_OUTPUT);
                    let dead = str_field("status") == Some(NodeStatus::Dead.as_str());
                    if busy_output || dead {
                        str_field("node").and_then(|s| s.parse::<NodeId>().ok())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(n) = node {
                // GUARDA de ordenação: só desarma se o progresso veio DEPOIS da arma corrente.
                if self
                    .dispatched_at
                    .get(&n)
                    .is_some_and(|(_, arm_seq)| p > *arm_seq)
                {
                    self.dispatched_at.remove(&n);
                }
            }
        }
        self.dispatch_progress_seq = max_seq;
        Ok(())
    }

    /// **F3-CONF-2: os nós ENGOLIDOS em `now_ms`** — entrega pendente (sem progresso, `dispatched_at`)
    /// há ≥ [`crate::attention::DELIVERED_NO_PROGRESS_WINDOW_MS`] E nó de volta a `Idle` (um nó `Busy`
    /// é turf do stall detector, zero sobreposição). A janela conta desde a ENTREGA no relógio do
    /// router (`now_ms`); um turno real consome tokens → o disarm o tira daqui antes de alarmar.
    fn swallowed_nodes(&self, now_ms: u64) -> Vec<(NodeId, u64)> {
        let window = crate::attention::DELIVERED_NO_PROGRESS_WINDOW_MS;
        self.dispatched_at
            .iter()
            .filter(|(_, (delivered_at, _))| now_ms.saturating_sub(*delivered_at) >= window)
            .filter(|(node, _)| {
                matches!(self.sup.get(**node), Some(info) if info.status == NodeStatus::Idle)
            })
            .map(|(node, (delivered_at, _))| (*node, *delivered_at))
            .collect()
    }

    /// **F3-CONF-2 (gate a): o protocolo ANTI-ENGOLIMENTO — o OUTER loop CONSOME o alarme W3.**
    /// Mecaniza no core o que hoje só vive em skill (`lina-orchestration` 6-7): um despacho ENTREGUE
    /// que voltou a `Idle` e ficou ≥ janela sem progresso (`DeliveredNoProgress`) é re-despachado **1×
    /// informado**; se a re-tentativa TAMBÉM engole, abre o breaker (sticky, impede a 3ª tentativa
    /// automática) e ESCALA ao humano (`GoalEscalated{stalled}`). O Maestro deixa de descobrir o
    /// despacho engolido só "olhando o disco vazio" (#22c, reunião real com cliente).
    ///
    /// **ZERO LLM (inv #1):** o juiz é ESTRUTURAL (ausência de evento de progresso na janela), nunca
    /// um modelo. **ADR 0007:** o re-despacho/escala é DADO (reusa `SpawnRequested`/`GoalEscalated`,
    /// carimbados server-side), JAMAIS autoridade. **Idempotente por replay:** a decisão deriva do log
    /// (re-despachos por epoch de entrega + reset humano), nunca de estado efêmero — replay reconstrói.
    /// **Escopo v1:** goal-driven (o nó precisa possuir um item de uma Goal — `GoalEscalated` exige
    /// `goal_id` auditável); handoff fora de Goal mantém só a DETECÇÃO (alarme da UI).
    ///
    /// Custo: O(1) na tick comum (sem engolido); só paga a leitura do log/projeção quando HÁ engolido
    /// (raro — 90s sem progresso).
    ///
    /// # Errors
    /// Falha ao ler/persistir no event log.
    fn enforce_dispatch_protocol(
        &mut self,
        store: &mut EventStore,
        now_ms: u64,
    ) -> Result<(), StoreError> {
        let swallowed = self.swallowed_nodes(now_ms);
        if swallowed.is_empty() {
            return Ok(()); // caminho comum: O(1), nada lido do log
        }
        // Só AQUI (raro) pagamos a leitura do log + a projeção do plano/roster.
        let records = store.events()?;
        let proj = store.project()?;
        for (node, delivered_at) in swallowed {
            // Nó já pausado pelo breaker → não re-despacha NEM re-escala (idempotência da escala: uma
            // vez aberto/Blocked, o nó sai daqui até o reset humano).
            if self.breaker_open.contains(&node) {
                continue;
            }
            // O protocolo é do OUTER loop, PLANO-cêntrico: re-despacho/escala precisam da Goal que
            // justifica o recurso. Sem item de Goal → só a detecção (UI) cobre (v1 goal-driven).
            let Some(item) = proj.plan.itens.iter().find(|it| {
                it.owner.as_deref().and_then(|o| self.sup.node_by_name(o)) == Some(node)
            }) else {
                continue;
            };
            let Some(goal_id) = item.goal_id.clone() else {
                continue;
            };
            let item_id = item.id.clone();
            // Idempotência POR EPOCH: o `now_ms` da entrega na chave. Contagem escopada ao pós-reset.
            let respawn_id = format!("{STALL_RESPAWN_PREFIX}:{node}:{delivered_at}");
            let prior = stall_respawn_ids_since_reset(&records, node);
            if prior.contains(&respawn_id) {
                continue; // ESTE epoch já foi re-despachado — aguarda a re-injeção (não escala no gap)
            }
            if prior.is_empty() {
                // 1ª tentativa: RE-DESPACHO informado ao MESMO dev (reusa `SpawnRequested`).
                let (name, role) = match proj.nodes.get(&node) {
                    Some(n) => (
                        n.name
                            .clone()
                            .unwrap_or_else(|| item.owner.clone().unwrap_or_default()),
                        n.role.clone().unwrap_or_default(),
                    ),
                    None => (item.owner.clone().unwrap_or_default(), String::new()),
                };
                store.append(&DomainEvent::SpawnRequested {
                    id: respawn_id,
                    requested_by: STRUCTURAL_JUDGE,
                    name,
                    role,
                    root_cause_id: goal_id.clone(),
                    hops: 1,
                    prompt: format!(
                        "Não recebi nenhum sinal de progresso do item {item_id} desde a entrega — o \
                         turno não começou. Reassuma a tarefa e comece a trabalhar agora."
                    ),
                    model: None,
                    effort: None,
                    goal_id: Some(goal_id),
                })?;
            } else {
                // A re-tentativa TAMBÉM engoliu → breaker sticky (3ª tentativa NÃO sai sozinha) +
                // escala ao humano. O reset do breaker exige gesto humano (`reset_breaker`).
                self.open_stall_breaker(store, node)?;
                store.append(&DomainEvent::GoalEscalated {
                    goal_id,
                    reason: "stalled".to_string(),
                })?;
            }
        }
        Ok(())
    }

    /// **F3-CONF-2: abre o circuit breaker por ENGOLIMENTO** — reusa o MESMO mecanismo do breaker de
    /// entrega (conjunto `breaker_open` + `Blocked` com `reason:"circuit_breaker"`), para a tela
    /// honesta pintar "pausado por segurança" e o [`Router::reset_breaker`] liberar por igual. NÃO
    /// emite `CircuitOpened` (esse é específico de falhas de ENTREGA por PTY): o registro durável do
    /// stall é o `GoalEscalated{stalled}` que o chamador apenda em seguida. `reason` server-side.
    ///
    /// # Errors
    /// Falha ao persistir o `NodeStatusChanged{Blocked}` no event log.
    fn open_stall_breaker(
        &mut self,
        store: &mut EventStore,
        node: NodeId,
    ) -> Result<(), StoreError> {
        self.breaker_open.insert(node);
        if let Some(info) = self.sup.get(node) {
            if info.status != NodeStatus::Blocked && info.status != NodeStatus::Dead {
                store.append(&DomainEvent::NodeStatusChanged {
                    node,
                    status: NodeStatus::Blocked.as_str().to_string(),
                    from: info.status.as_str().to_string(),
                    reason: "circuit_breaker".to_string(),
                })?;
                if let Err(e) = self.sup.set_status_with_reason(
                    node,
                    NodeStatus::Blocked,
                    Some("circuit_breaker".to_string()),
                ) {
                    eprintln!("lina-core: falha ao bloquear {node} (breaker de engolimento): {e}");
                }
            }
        }
        Ok(())
    }

    /// **F3-CONF-2 (gate c): RESET do circuit breaker sob GESTO HUMANO.** O "clique para liberar" do
    /// card (UI/G) chama isto LOCAL in-process (como [`Router::human_intent`]): tira o nó de `Blocked`,
    /// fecha o breaker e zera as falhas, para a auto-orquestração voltar a despachá-lo; e marca o
    /// reset (`reason:"breaker_reset"`) — que REINICIA o protocolo anti-engolimento do nó (volta ao
    /// "1ª tentativa", em vez de re-escalar a entrega antiga). Devolve `true` se liberou, `false` se
    /// não havia nada a resetar (idempotente — clique duplo não re-loga).
    ///
    /// **Segurança (ADR 0007):** um AGENTE NUNCA reabre o próprio breaker — este caminho não passa por
    /// `route_message`/`node_by_name` (nenhum sender de roster o produz); a AUTORIDADE é a NATUREZA do
    /// canal (chamada in-process; processo único, GPU-first). O `by` é, por construção, o gesto humano
    /// ([`HUMAN_GESTURE`]); o `reason` é carimbado server-side, jamais lido do cliente.
    ///
    /// # Errors
    /// Falha ao persistir o `NodeStatusChanged{Idle}` no event log.
    pub fn reset_breaker(
        &mut self,
        node: NodeId,
        store: &mut EventStore,
    ) -> Result<bool, StoreError> {
        let was_open = self.breaker_open.remove(&node);
        let blocked = matches!(self.sup.get(node), Some(i) if i.status == NodeStatus::Blocked);
        if !was_open && !blocked {
            return Ok(false); // idempotente: nada pausado a liberar
        }
        self.node_failures.remove(&node);
        if blocked {
            store.append(&DomainEvent::NodeStatusChanged {
                node,
                status: NodeStatus::Idle.as_str().to_string(),
                from: NodeStatus::Blocked.as_str().to_string(),
                reason: BREAKER_RESET_REASON.to_string(),
            })?;
            if let Err(e) = self
                .sup
                .set_status_with_reason(node, NodeStatus::Idle, None)
            {
                eprintln!("lina-core: falha ao liberar {node} (reset humano do breaker): {e}");
            }
        }
        Ok(true)
    }

    /// **F3-3 (gate h) — aposentar uma crença por gesto humano (spec 35 §6.7).** O clique "esquecer
    /// isso" no painel "Como o [papel] pensa" emite `BeliefRetired{reason:"retired_by_human"}`: a
    /// crença some da PRÓXIMA injeção, mas NÃO é deletada (rebaixada, reconstruível por replay, inv #4).
    /// `belief_id` é DADO (endereçamento de QUAL crença), jamais autoridade — o humano é o árbitro.
    ///
    /// **Segurança (ADR 0007):** um AGENTE NUNCA aposenta uma crença — este caminho não passa por
    /// `route_message`/`node_by_name` (nenhum sender de roster o produz); a AUTORIDADE é a NATUREZA do
    /// canal (gesto humano in-process, `push_human_intent`, igual ao [`Router::reset_breaker`]). O
    /// `reason` é carimbado server-side aqui, jamais lido do cliente. Devolve `false` se `belief_id`
    /// vazio (gesto malformado — não loga evento órfão).
    ///
    /// # Errors
    /// Falha ao persistir o `BeliefRetired` no event log.
    pub fn retire_belief(
        &mut self,
        belief_id: &str,
        store: &mut EventStore,
    ) -> Result<bool, StoreError> {
        if belief_id.trim().is_empty() {
            return Ok(false); // gesto malformado: não emite evento órfão
        }
        store.append(&DomainEvent::BeliefRetired {
            belief_id: belief_id.to_string(),
            reason: "retired_by_human".to_string(),
        })?;
        Ok(true)
    }

    /// **F3-0-5: `lina params set/reset`** (molde de [`Router::handle_plan`]). Parseia o contrato
    /// `{key, scope, value, target?}` do payload (DADO do envelope), valida a faixa pelo core
    /// ([`crate::params::validate_range`] — o bin não duplica as faixas), e emite um
    /// `SystemParamsChanged`. **Segurança (ADR 0007):** `by` é CARIMBO server-side da origem×cascata
    /// (`derive_root_hops`: origem humana → `"human"`; cascata de agente → `"maestro"`), JAMAIS o
    /// `by` que um agente escreva no payload; `old` também é server-side (`""` aqui — o histórico do
    /// log é a fonte do valor anterior, nunca o payload). O parâmetro muda magnitude, nunca autoridade.
    fn handle_params(
        &mut self,
        msg: &MailMessage,
        sender: NodeId,
        store: &mut EventStore,
    ) -> RouteOutcome {
        let payload: serde_json::Value = match serde_json::from_str(&msg.payload) {
            Ok(v) => v,
            Err(e) => {
                return RouteOutcome::ParamsRejected(format!("payload de params inválido: {e}"))
            }
        };
        let field = |k: &str| {
            payload
                .get(k)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let (key, scope, value) = (field("key"), field("scope"), field("value"));
        let target = payload
            .get("target")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if key.is_empty() || scope.is_empty() {
            return RouteOutcome::ParamsRejected("params exige 'key' e 'scope'".into());
        }

        match msg.intent.as_str() {
            // O `set` RECUSA fora-da-faixa (≠ load, que clampa) — feedback explícito ao expert.
            "params.set" => {
                if let Err(e) = crate::params::validate_range(&key, &value) {
                    return RouteOutcome::ParamsRejected(e);
                }
                // F3-0-7 §C: REDUZIR um laço balanceador abaixo do default ENFRAQUECE uma defesa →
                // GatedHard: NÃO aplica direto; loga `ActionGated{ask}` (a fila de atenção projeta um
                // GuardAsk p/ o humano confirmar — guard.rs §1.1, gated-hard é Ask em TODO nível). O
                // valor já passou na faixa → o parse p/ a classificação é infalível. (`reset` não
                // reduz — volta ao default; não gateia.)
                if let Ok(n) = value.parse::<u64>() {
                    if crate::params::param_change_action_class(&key, n) == ActionClass::GatedHard {
                        let decision = decide(ActionClass::GatedHard, self.config.autonomy);
                        let gated = DomainEvent::ActionGated {
                            cmd: format!("params set {key} {n}"),
                            class: ActionClass::GatedHard.as_str().to_string(),
                            decision: decision.as_str().to_string(),
                            node: Some(msg.from.clone()),
                        };
                        if let Err(e) = store.append(&gated) {
                            return RouteOutcome::PersistFailed(e.to_string());
                        }
                        return RouteOutcome::ParamsGated { key };
                    }
                }
            }
            "params.reset" => {}
            other => {
                return RouteOutcome::ParamsRejected(format!(
                    "intent de params desconhecido: {other}"
                ))
            }
        }

        // `by` carimbado SERVER-SIDE pela origem×cascata (inforjável) — nunca o do payload.
        let (_root, hops) = self.derive_root_hops(msg, sender);
        let by = if hops == 0 { "human" } else { "maestro" };
        // `reset` volta a chave para "não-setado nesta camada" (herda) → `new` vazio.
        let new = if msg.intent == "params.reset" {
            String::new()
        } else {
            value
        };
        let event = DomainEvent::SystemParamsChanged {
            key: key.clone(),
            scope,
            new,
            target,
            old: String::new(),
            by: Some(by.to_string()),
        };
        if let Err(e) = store.append(&event) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        RouteOutcome::ParamsApplied {
            intent: msg.intent.clone(),
            key,
        }
    }

    /// **F3-0-5: `lina effort @T <low|medium|high>`** (mirror de [`Router::handle_params`]). Resolve
    /// o alvo (`target`, DADO do payload) via [`Supervisor::node_by_name`], valida o `effort` contra
    /// o vocabulário do enum [`Effort`], e emite `EffortAssigned`. **Segurança (ADR 0007):**
    /// `origin` é SEMPRE `Assigned` (o verbo É uma atribuição explícita) e `by` é CARIMBO server-side
    /// da origem×cascata (`derive_root_hops`: origem humana → `None`; cascata de agente →
    /// `Some(sender)`), JAMAIS lidos do payload. `target`/`effort` são dado; um agente não forja
    /// `origin`/`by`. (Atribuir effort a um nó vivo só registra/projeta o badge — a APLICAÇÃO no PTY
    /// é por-spawn, F3-0-4 — então não há auto-promoção de gasto aqui.)
    fn handle_effort(
        &mut self,
        msg: &MailMessage,
        sender: NodeId,
        store: &mut EventStore,
    ) -> RouteOutcome {
        let payload: serde_json::Value = match serde_json::from_str(&msg.payload) {
            Ok(v) => v,
            Err(e) => {
                return RouteOutcome::EffortRejected(format!("payload de effort inválido: {e}"))
            }
        };
        let target = payload
            .get("target")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if target.is_empty() {
            return RouteOutcome::EffortRejected("effort.assign exige 'target'".into());
        }
        // (1) resolve o alvo (DADO do payload) → NodeId; alvo inexistente recusa.
        let Some(node) = self.sup.node_by_name(target) else {
            return RouteOutcome::EffortRejected(format!("terminal desconhecido: {target}"));
        };
        // `by`/`origin` carimbados SERVER-SIDE (inforjável); `hops` (origem×cascata) decide ambos.
        let (_root, hops) = self.derive_root_hops(msg, sender);
        // (2) GUARDA anti-auto-promoção (F3-0-7 §B2, ADR 0007): um AGENTE em cascata (`hops>=1`) NÃO
        //     pode se AUTO-ATRIBUIR effort (`node == sender`) — `assigned` exige carimbo de
        //     Maestro/humano. Agente atribuindo a OUTRO segue permitido (`by=Some(sender)`); o humano
        //     na origem (`hops==0`) é livre. Defesa em profundidade: barra já o REGISTRO de uma
        //     auto-atribuição (que envelheceria mal quando a aplicação dinâmica do effort evoluir).
        if hops >= 1 && node == sender {
            return RouteOutcome::EffortRejected(
                "auto-atribuição de effort exige Maestro/humano".into(),
            );
        }
        // (3) valida o effort contra o vocabulário do enum (low|medium|high).
        let effort = match payload.get("effort").and_then(serde_json::Value::as_str) {
            Some("low") => Effort::Low,
            Some("medium") => Effort::Medium,
            Some("high") => Effort::High,
            other => {
                return RouteOutcome::EffortRejected(format!(
                    "effort inválido {other:?}: use low, medium ou high"
                ))
            }
        };
        // `origin` é sempre `Assigned` (o verbo é uma atribuição); `by` segue a cascata (None = humano).
        let by = (hops >= 1).then_some(sender);
        let event = DomainEvent::EffortAssigned {
            node: node.to_string(),
            effort,
            model: None,
            origin: EffortOrigin::Assigned,
            task_difficulty: None,
            by,
        };
        if let Err(e) = store.append(&event) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        RouteOutcome::EffortApplied { node, effort }
    }

    /// **F3-1 (spec 52 §Superfície): verbos da Goal** (molde de [`Router::handle_params`]/
    /// [`Router::handle_effort`]). `goal.define` cunha o `goal_id` (UUIDv7, SERVER-SIDE — jamais do
    /// payload) e o `root_cause_id` (binding inforjável), emitindo `GoalDefined`. `goal.interpret`
    /// emite `GoalInterpreted` (SUGESTÃO — não autoriza). `goal.confirm` é o GATE HUMANO da meta:
    /// **`by` é CARIMBADO com o `sender` AUTENTICADO** (server-side), JAMAIS lido do payload (ADR 0007
    /// / spec 52 §Segurança 2 — um `GoalConfirmed` forjado com `by` de outro nó é impossível aqui).
    /// NUNCA entregue a PTY (verbo estruturado, como plan/params/effort).
    /// **ADR 0036: dispara um GESTO HUMANO DIRETO da UI** (confirmar/ajustar uma Goal pelo botão do
    /// card). Diferente do A2A: o humano não é um nó nem escreve no outbox — a autenticação é a NATUREZA
    /// do canal (chamada LOCAL in-process; processo único, GPU-first — nenhum remoto alcança). Carimba
    /// `by` = [`HUMAN_GESTURE`] server-side, JAMAIS uma identidade escolhida pelo cliente (spec 52 §Seg2):
    /// um agente entra por `route_message`→`node_by_name` e NUNCA produz este `by`. Só aceita os intents
    /// de Goal que são gesto humano legítimo (`goal.confirm`/`goal.interpret`); recusa o resto.
    pub fn human_intent(
        &mut self,
        intent: &str,
        payload: &str,
        store: &mut EventStore,
    ) -> RouteOutcome {
        if !matches!(intent, "goal.confirm" | "goal.interpret") {
            return RouteOutcome::GoalRejected(format!(
                "gesto humano direto nao suporta o intent {intent}"
            ));
        }
        // `from`/`to` são RÓTULOS de proveniência (o canal autentica, não a string). O `by` real é o
        // sentinel [`HUMAN_GESTURE`] passado como sender — `handle_goal` o carimba no `GoalConfirmed`,
        // e `goal.confirm`/`goal.interpret` não derivam root/hops (só `goal.define` o faz), então o
        // sentinel sem binding de roster é seguro aqui.
        let msg = MailMessage::new("human", "goal", intent, payload);
        self.handle_goal(&msg, HUMAN_GESTURE, store)
    }

    /// **F3-3 (M-DETECTOR, spec 35 §4.1) — captação de correção.** Emite `CorrectionObserved` com o
    /// `role` do SENDER autenticado (server-side, da projeção do roster — JAMAIS do payload do agente,
    /// regra-mãe ADR 0007). `summary` é o "o quê" (DADO comportamental); `refs` aponta a mensagem-fonte
    /// (auditoria/proveniência do Refletor). NÃO entrega a PTY — molde dos verbos estruturados
    /// (plan/params/goal). O Refletor (async, fora do caminho crítico, inv #1) consome este evento
    /// DEPOIS; aqui é só captação barata e determinística. A projeção só roda quando a sentinela casa
    /// (raro), nunca no caminho-quente de toda mensagem.
    fn handle_correction(
        &mut self,
        sender: NodeId,
        summary: &str,
        msg_id: &str,
        store: &mut EventStore,
    ) -> RouteOutcome {
        // `role` = binding server-side do nó (projeção do log), NUNCA o texto do agente. `""` se o nó
        // ainda não tem papel projetado — degradação honesta (o evento registra; o Refletor decide).
        let role = store
            .project()
            .ok()
            .and_then(|p| p.nodes.get(&sender).and_then(|n| n.role.clone()))
            .unwrap_or_default();
        match store.append(&DomainEvent::CorrectionObserved {
            role: role.clone(),
            summary: summary.to_string(),
            refs: vec![msg_id.to_string()],
        }) {
            Ok(_) => RouteOutcome::CorrectionObserved { role },
            Err(e) => RouteOutcome::PersistFailed(e.to_string()),
        }
    }

    fn handle_goal(
        &mut self,
        msg: &MailMessage,
        sender: NodeId,
        store: &mut EventStore,
    ) -> RouteOutcome {
        let payload: serde_json::Value =
            serde_json::from_str(&msg.payload).unwrap_or(serde_json::Value::Null);
        let text = |k: &str| {
            payload
                .get(k)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let event = match msg.intent.as_str() {
            "goal.define" => {
                let statement = text("statement");
                if statement.is_empty() {
                    return RouteOutcome::GoalRejected("goal.define exige 'statement'".into());
                }
                // `root_cause_id` do binding inforjável (`derive_root_hops`), nunca `msg.root_cause_id`.
                let (root, _hops) = self.derive_root_hops(msg, sender);
                // `goal_id` cunhado SERVER-SIDE (UUIDv7 estável), nunca do payload do agente.
                let goal_id = uuid::Uuid::now_v7().to_string();
                let budget_tokens = payload
                    .get("budget_tokens")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                // `origin` é PROVENIÊNCIA = o CANAL de entrada do Espaço (não quem enviou — isso é o
                // `by`/root): @Tradutor se há um Tradutor no roster (a porta do leigo), senão @Maestro.
                // Rótulo, JAMAIS credencial. Carimbado SERVER-SIDE (do roster projetado), nunca do payload.
                let roles: Vec<String> = store
                    .project()
                    .map(|p| p.nodes.values().filter_map(|n| n.role.clone()).collect())
                    .unwrap_or_default();
                let origin = lina_role_discovery::entry_origin(&roles).to_string();
                let goal_event = DomainEvent::GoalDefined {
                    goal_id: goal_id.clone(),
                    statement,
                    root_cause_id: root,
                    origin,
                    budget_tokens,
                };
                (goal_event, goal_id)
            }
            "goal.interpret" => {
                let goal_id = text("goal_id");
                if goal_id.is_empty() {
                    return RouteOutcome::GoalRejected("goal.interpret exige 'goal_id'".into());
                }
                // Chave `proposed_team` — a MESMA que a CLI serializa (`lina.rs`); o `get("team")`
                // anterior era um descasamento CLI↔core (o time nunca era persistido pelo caminho
                // real), no molde do bug de `accept`↔`acceptance` que a GL-2 já corrigiu.
                let proposed_team = payload
                    .get("proposed_team")
                    .and_then(serde_json::Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                // A CLI serializa os critérios como `acceptance:[AcceptanceCriterion]` (mesma chave/forma
                // de `plan.add`). Aqui extraímos só a `desc` e FORÇAMOS `HumanReview`: na interpretação da
                // Goal todo critério nasce confirmado por humano (gate, spec 52 §1) — o DoD verificável por
                // máquina (`Command`/`TestPass`) entra por ITEM via `PlanItemAttributed`, NUNCA pela Goal
                // (um agente não injeta um `Command` no nível da meta).
                let acceptance_criteria = payload
                    .get("acceptance")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Vec<AcceptanceCriterion>>(v).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| AcceptanceCriterion {
                        desc: c.desc,
                        check_kind: CheckKind::HumanReview,
                        check_arg: None,
                    })
                    .collect();
                let goal_event = DomainEvent::GoalInterpreted {
                    goal_id: goal_id.clone(),
                    interpretation: text("interpretation"),
                    strategy: text("strategy"),
                    proposed_team,
                    acceptance_criteria,
                };
                (goal_event, goal_id)
            }
            "goal.confirm" => {
                let goal_id = text("goal_id");
                if goal_id.is_empty() {
                    return RouteOutcome::GoalRejected("goal.confirm exige 'goal_id'".into());
                }
                // GATE HUMANO: `by` = o `sender` AUTENTICADO (server-side), JAMAIS do payload — um
                // agente não forja a identidade de quem confirma a meta.
                let amended_statement = payload
                    .get("amended_statement")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from);
                let goal_event = DomainEvent::GoalConfirmed {
                    goal_id: goal_id.clone(),
                    by: sender,
                    amended_statement,
                };
                (goal_event, goal_id)
            }
            other => {
                return RouteOutcome::GoalRejected(format!("intent de goal desconhecido: {other}"))
            }
        };
        let (goal_event, goal_id) = event;
        if let Err(e) = store.append(&goal_event) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        // F3-2-3 (spec 52 §3/§4, gate c): a confirmação é o GATILHO da MONTAGEM do time. §327: a
        // EXECUÇÃO (gastar recurso com spawns) respeita a autonomia — dispara automaticamente SÓ com
        // o GESTO HUMANO (`HUMAN_GESTURE`, qualquer nível) ou em `autonomo`. Em manual/assistido, um
        // agente que confirma REGISTRA o fato (`GoalConfirmed`, `by` server-side) mas a montagem
        // espera o toque humano (a tabela §327: "PROPÕE → humano confirma"). O `GoalConfirmed` é o
        // gate (auditável); a montagem é a AÇÃO gated — separá-los preserva o `by` inforjável sem
        // bloquear o registro do fato. A admissão FÍSICA dos spawns (PTY/custo/cap) é do app.
        if msg.intent == "goal.confirm"
            && (sender == HUMAN_GESTURE || self.config.autonomy == AutonomyLevel::Autonomous)
        {
            if let Err(e) = self.assemble_team(&goal_id, sender, store) {
                eprintln!("lina-core: CRÍTICO — montagem do time da Goal {goal_id} falhou: {e}");
            }
        }
        RouteOutcome::GoalApplied {
            intent: msg.intent.clone(),
            goal_id,
        }
    }

    /// **F3-2-3 (spec 52 §3/§4, gate c): monta o time INICIAL de uma Goal confirmada.** Consome o
    /// `proposed_team` da ÚLTIMA `GoalInterpreted` ([`proposed_team_of`] — a correção via re-`interpret`
    /// faz a última vencer) e emite UM `SpawnRequested` por papel. O `effort` é o DEGRAU BASE da escada
    /// (`effort_ladder.first()`) — o classificador de dificuldade POR PAPEL é porta aberta F3+ (spec 52
    /// §4: "esta spec define o CANAL, o classificador fica plugável"); o que importa aqui é o `effort`
    /// no envelope. `goal_id` JUSTIFICA o recurso (auditoria). `requested_by` = quem confirmou (carimbo
    /// server-side: `HUMAN_GESTURE` no toque da UI, ou o agente em `autonomo`), JAMAIS do payload.
    /// `hops:0` — é a 1ª leva AUTORIZADA pelo gate, NÃO cascata (cascata = spawn-de-spawn de um MEMBRO,
    /// `hops>=1`, que segue exigindo aval humano por [`Router::handle_spawn`]/admit_node). A admissão
    /// FÍSICA (PTY + `LINA_EFFORT` + cost/cap) é do app; aqui só o LIVRO-RAZÃO. Idempotente: re-confirm
    /// não duplica spawns já montados ([`spawn_id_exists`] sobre o `id` determinístico).
    fn assemble_team(
        &mut self,
        goal_id: &str,
        by: NodeId,
        store: &mut EventStore,
    ) -> Result<(), StoreError> {
        let effort = self.config.effort_ladder.first().copied();
        for role in proposed_team_of(store, goal_id)? {
            let id = format!("goal-team:{goal_id}:{role}");
            if spawn_id_exists(store, &id)? {
                continue; // já montado — re-confirm é idempotente (não duplica o time).
            }
            store.append(&DomainEvent::SpawnRequested {
                id,
                requested_by: by,
                name: format!("@{role}"),
                role: role.clone(),
                root_cause_id: goal_id.to_string(),
                hops: 0,
                prompt: format!(
                    "Você é {role} nesta meta. Assuma os itens do plano atribuídos ao seu papel \
                     e entregue conforme os critérios de aceite."
                ),
                model: None,
                effort,
                goal_id: Some(goal_id.to_string()),
            })?;
        }
        Ok(())
    }

    /// **F1-3-6 (ADR 0019 §6): gate INFORJÁVEL do `lina spawn`** (molde de [`Router::handle_plan`]).
    /// NÃO cunha NodeId nem abre PTY (autoridade única do Supervisor/app; fronteira core/shell
    /// inv#7): **DECIDE** (forma → autonomia → cascata → cap → custo) e devolve o `RouteOutcome` que
    /// o APP executa via o funil `admit_node` (ADR 0022, seam da tela — Maestro decisão 4). Lê
    /// origem×cascata REUSANDO [`Router::derive_root_hops`] — o binding `delivered_root` (inforjável),
    /// JAMAIS `msg.hops`/`msg.root_cause_id` (campos PÚBLICOS/forjáveis do outbox). É o coração da
    /// não-forjabilidade (ADR 0007): nenhum campo escrito pelo agente decide origem/cap/autorização.
    ///
    /// **Livro-razão (igual `handle_plan`):** loga ANTES do efeito; `SpawnRequested` SEMPRE
    /// (pós-validação de forma); `SpawnGated` em cada ramo barrado; falha de `append` →
    /// `PersistFailed` (não decidir o que não logamos — invariante #4). `NodeAdded`/`requested_by`
    /// só na EXECUÇÃO física (app).
    fn handle_spawn(
        &mut self,
        msg: &MailMessage,
        sender: NodeId,
        store: &mut EventStore,
        now_ms: u64,
    ) -> RouteOutcome {
        // ── 0) Args do envelope: name = `to` (`@Nome`), role = `ref` (`role:<r>`), prompt = payload.
        //    Validação de FORMA (name/role não-vazios) → `SpawnBlocked`. O bin recusa antes de
        //    enfileirar (UX local); este é o backstop durável. Pedido MALFORMADO NÃO vira
        //    `SpawnRequested` (não é um spawn real, é comando inválido — não polui o livro-razão).
        let name = msg.to.trim().to_string();
        let role = msg
            .ref_id
            .as_deref()
            .map(|r| r.strip_prefix("role:").unwrap_or(r).trim())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || name == "@" || role.is_empty() {
            return RouteOutcome::SpawnBlocked;
        }
        let prompt = msg.payload.clone();

        // ── 1) BINDING INFORJÁVEL (reuso puro): root/hops EFETIVOS — a ÚNICA autoridade de
        //    origem×cascata. Sem binding (origem) → `(msg.id, 0)`; com binding (recebeu A2A) →
        //    `(root_herdado, hops+1)`. Ignora por construção `msg.hops`/`msg.root_cause_id`.
        let (root, hops) = self.derive_root_hops(msg, sender);

        // ── 2) intent-vs-action (13.14 P1): loga o PEDIDO SEMPRE, ANTES da decisão, com a cadeia
        //    EFETIVA (não a forjada) e o `requested_by` AUTENTICADO (sender, jamais `from`).
        if let Err(e) = store.append(&DomainEvent::SpawnRequested {
            id: msg.id.clone(),
            requested_by: sender,
            name: name.clone(),
            role: role.clone(),
            root_cause_id: root.clone(),
            hops,
            // SEAM-1: o 1º prompt entra no log para o banner de um spawn GATED ser reconstruível.
            prompt: prompt.clone(),
            // F3-0-3: o caminho spawn-via-agente ainda não transporta model/effort (o `MailMessage`
            // não tem esses campos; o verbo `lina spawn --model/--effort` é F3-0-5). `None` por ora;
            // o caminho HUMANO os escolhe no modal e os aplica via admit_node/F3-0-4.
            model: None,
            effort: None,
            // F3-1 (spec 52 §4): o caminho spawn-via-agente ainda não atrela a Goal (o ciclo de Goal
            // é a fatia CLI/lógica seguinte). `None` por ora — call-site conciliado ao novo contrato.
            goal_id: None,
        }) {
            return RouteOutcome::PersistFailed(e.to_string());
        }

        // ── 3) Autonomia `manual` → RECUSA terminal (paridade com `BlockedByAutonomy` do handoff).
        //    Registra o fato (`SpawnGated{manual}`) e devolve `SpawnBlocked` (UX distinta do banner).
        //    GOVERNADO pela matriz de autonomia do `RouterConfig` (Maestro decisão 2 — GAP de fiação
        //    em `bridge.rs` escalado na entrega; sem ele este ramo seria fake em produção).
        if self.config.autonomy.blocks_delegation() {
            if let Err(e) = append_spawn_gated(store, &msg.id, sender, "manual") {
                return RouteOutcome::PersistFailed(e.to_string());
            }
            return RouteOutcome::SpawnBlocked;
        }

        // ── 4) CASCATA (`hops>=1`): spawn-de-spawn pede gesto humano SEMPRE (ADR 0007 — a defesa
        //    central anti-fork-bomb). Forjar `hops=0` no payload NÃO ajuda: `hops` veio do binding.
        if hops >= 1 {
            if let Err(e) = append_spawn_gated(store, &msg.id, sender, "cascade") {
                return RouteOutcome::PersistFailed(e.to_string());
            }
            return RouteOutcome::SpawnGated {
                reason: "cascade".into(),
            };
        }

        // ── 5) CAP por `(root, sender)` (origem sob teto). Reusa o mapa `budget` + a poda A5. Acima
        //    do cap → banner humano. (Ver entrega: para ORIGEM o root é FRESCO por `lina spawn`, então
        //    este cap só morde dentro de um root COMPARTILHADO — escalado como gap de origem-burst.)
        let used = self
            .budget
            .get(&(root.clone(), sender))
            .map(|e| e.spawn_count)
            .unwrap_or(0);
        if used >= self.config.max_spawns_per_turn {
            if let Err(e) = append_spawn_gated(store, &msg.id, sender, "over_cap") {
                return RouteOutcome::PersistFailed(e.to_string());
            }
            return RouteOutcome::SpawnGated {
                reason: "over_cap".into(),
            };
        }

        // ── 6) CUSTO (ADR 0005 / W3-7c): workspace cost-`Paused` → banner humano (o novo terminal
        //    gastaria sob teto estourado). Reusa o `CostLedger` event-sourced; só quando há teto
        //    configurado. NÃO transiciona p/ `Paused` (isso é do caminho de delegação) — só barra se
        //    JÁ pausado. Falha ao LER o log → falha segura (não aprova o que não conseguimos checar).
        if self.config.token_budget_day > 0 {
            match CostLedger::replay(store, now_ms) {
                Ok(ledger) if ledger.paused => {
                    if let Err(e) = append_spawn_gated(store, &msg.id, sender, "cost") {
                        return RouteOutcome::PersistFailed(e.to_string());
                    }
                    return RouteOutcome::SpawnGated {
                        reason: "cost".into(),
                    };
                }
                Ok(_) => {}
                Err(e) => return RouteOutcome::PersistFailed(e.to_string()),
            }
        }

        // ── 7) ALLOW: contabiliza o spawn em `(root, sender)` (carimba `last_ms` p/ a poda A5) e
        //    devolve `SpawnApproved` — o APP cria o nó (admit_node), enfileira o 1º prompt e narra.
        let entry = self
            .budget
            .entry((root.clone(), sender))
            .or_insert(BudgetEntry {
                count: 0,
                spawn_count: 0,
                last_ms: now_ms,
            });
        entry.spawn_count += 1;
        entry.last_ms = now_ms;
        RouteOutcome::SpawnApproved {
            name,
            role,
            prompt,
            requested_by: sender,
            root_cause_id: root,
            // F3-0-3: idem ao `SpawnRequested` acima — o spawn-via-agente não especifica model/effort
            // nesta rodada (F3-0-5 fia o verbo CLI); o caminho humano os aplica fora do `handle_spawn`.
            model: None,
            effort: None,
        }
    }

    /// **W3-5: semeia um item no plano** (chamado pelo supervisor/app, não pela mailbox): loga
    /// `PlanItemAdded` e reescreve o `plan.md`. O item nasce `status:todo` / `@owner:?`.
    ///
    /// # Errors
    /// [`PlanOpError::Plan`] se o id já existe; [`PlanOpError::Store`]/[`PlanOpError::Io`] em falha
    /// de persistência/escrita.
    pub fn seed_plan_item(
        &mut self,
        store: &mut EventStore,
        item: impl Into<String>,
        desc: impl Into<String>,
    ) -> Result<(), PlanOpError> {
        let item = item.into();
        let desc = desc.into();
        let mut plan = store.project()?.plan;
        plan.add_item(item.clone(), desc.clone())?;
        store.append(&DomainEvent::PlanItemAdded { item, desc })?;
        self.mailbox.write_plan(&plan.render())?;
        Ok(())
    }

    /// **W3-5: adiciona uma decisão viva ao plano** — loga `PlanDecisionAdded` e reescreve o `plan.md`.
    ///
    /// # Errors
    /// [`PlanOpError::Store`]/[`PlanOpError::Io`] em falha de persistência/escrita.
    pub fn seed_plan_decision(
        &mut self,
        store: &mut EventStore,
        text: impl Into<String>,
    ) -> Result<(), PlanOpError> {
        let text = text.into();
        let mut plan = store.project()?.plan;
        plan.add_decision(text.clone());
        store.append(&DomainEvent::PlanDecisionAdded { text })?;
        self.mailbox.write_plan(&plan.render())?;
        Ok(())
    }
}

/// F3-1-6 (spec 52 §2): os dados de um item a semear E atribuir (`plan.add`/`plan.seed`) — agrupa o
/// que vira `PlanItemAdded` + `PlanItemAttributed` para [`Router::attribute_item`] (sem assinatura de
/// muitos args). `goal_id: None` = item avulso (legado); `acceptance` vazio defere ao humano.
struct ItemAttribution {
    item: String,
    desc: String,
    goal_id: Option<String>,
    parents: Vec<String>,
    acceptance: Vec<AcceptanceCriterion>,
    budget_tokens: u64,
    /// F3-4-3 (ADR 0041): arquivos que o item reserva (cruzados com `CodeChanged.paths`).
    paths: Vec<String>,
}

/// `true` se o intent é uma operação de plano (aplicada ao `plan.md`, nunca entregue a PTY).
/// `claim`/`check` mutam um item via `--ref`; `add`/`seed` (F3-1-6) carregam os dados no payload e
/// nascem do `lina plan add`/`lina plan seed` — todos interceptados por INTENT (não por alvo).
fn is_plan_intent(intent: &str) -> bool {
    matches!(
        intent,
        "plan.claim" | "plan.check" | "plan.add" | "plan.seed"
    )
}

/// F1-3-6 (ADR 0019 §6): `true` se o intent é um pedido de SPAWN (`lina spawn`). Interceptado no
/// router como verbo estruturado (molde de [`is_plan_intent`]) — DECIDIDO pelo gate
/// [`Router::handle_spawn`], NUNCA entregue a PTY (o alvo `@Nome` nem existe no roster ainda).
fn is_spawn_intent(intent: &str) -> bool {
    intent == "spawn"
}

/// F3-0-5: `true` se o intent é um verbo de PARÂMETROS (`lina params set/reset`). Interceptado no
/// router como verbo estruturado (molde de [`is_plan_intent`]) — aplicado ao log
/// (`SystemParamsChanged`), NUNCA entregue a PTY.
fn is_params_intent(intent: &str) -> bool {
    matches!(intent, "params.set" | "params.reset")
}

/// F3-0-5: `true` se o intent é o verbo de atribuição de effort (`lina effort @T`). Interceptado no
/// router (mirror de [`is_params_intent`]) — emite `EffortAssigned`, NUNCA entregue a PTY.
fn is_effort_intent(intent: &str) -> bool {
    intent == "effort.assign"
}

/// F3-1: `true` se o intent é um verbo da Goal (`goal.define`/`goal.interpret`/`goal.confirm`).
/// Interceptado no router (molde de [`is_params_intent`]) — emite eventos da Goal, NUNCA entregue a
/// PTY (o alvo `goal`/`@workspace` não é um nó endereçável).
fn is_goal_intent(intent: &str) -> bool {
    matches!(intent, "goal.define" | "goal.interpret" | "goal.confirm")
}

/// F3-4-3 (spec 36 §2): `true` se o intent é o SINAL DE MUDANÇA de código (`lina code-changed`).
/// Interceptado no router (molde de [`is_goal_intent`]) — apenda `CodeChanged` (author_node
/// server-side) + broadcast por pertencimento, NUNCA entregue como delegação a um `to`.
fn is_code_changed_intent(intent: &str) -> bool {
    intent == "code.changed"
}

/// F3-4-5 (spec 36 §3): `true` se o intent é a PROVA DE FECHAMENTO de uma branch
/// (`lina branch-integrated`). Interceptado no router — apenda `BranchIntegrated` (registro puro;
/// a projeção `code::branches_nao_integradas` o consome). Molde de [`is_code_changed_intent`].
fn is_branch_integrated_intent(intent: &str) -> bool {
    intent == "branch.integrated"
}

/// F3-1-2 (spec 52 §1/§3): o JUIZ ESTRUTURAL — roda os critérios de aceite de um item, ZERO LLM.
/// `None` = DEFERE ao humano/QA (algum critério é `HumanReview`, ou o juiz não pôde avaliar →
/// fail-open; o turn-budget é o backstop). `Some(Pass)` sse TODOS os critérios automáticos passam;
/// `Some(Fail, evidence, defect_class)` no primeiro que falha (evidência OBSERVADA, não "ok").
///
/// **Segurança (spec 52 §Segurança 3):** um critério `Command`/`TestPass` cujo comando seja ação
/// `GatedHard`/`GatedSoft`-gated (deploy/pay/`rm -rf`/push em main) NÃO é executado — o juiz não roda
/// ação irreversível para "verificar" um critério. Degrada a `Fail` (exige confirmação humana); o
/// critério verifica, não autoriza.
fn run_acceptance(
    criteria: &[AcceptanceCriterion],
    autonomy: AutonomyLevel,
) -> Option<(GoalVerdict, String, String)> {
    // HumanReview em QUALQUER critério defere o item inteiro ao humano/QA (degrada honesto — o juiz
    // estrutural não fabrica um Pass que exige olho humano).
    if criteria
        .iter()
        .any(|c| matches!(c.check_kind, CheckKind::HumanReview))
    {
        return None;
    }
    for c in criteria {
        let arg = c.check_arg.as_deref().unwrap_or("");
        let passed = match c.check_kind {
            CheckKind::Command | CheckKind::TestPass => {
                // §Seg 3: o juiz NÃO roda ação não-rotineira para "verificar" (GatedHard → Ask em todo
                // nível; GatedSoft → Ask em manual/assistido). Degrada a Fail — verifica, não autoriza.
                if decide(classify(arg), autonomy) != Decision::Allow {
                    return Some((
                        GoalVerdict::Fail,
                        format!(
                            "critério '{}' exige confirmação humana (ação não-rotineira): {arg}",
                            c.desc
                        ),
                        "criterio_nao_cumprido".to_string(),
                    ));
                }
                // Erro de transporte do juiz (nem rodou) → fail-open: `?` propaga `None` (DEFERE; o
                // turn-budget é o backstop).
                run_check_command(arg)?
            }
            CheckKind::FileExists => std::path::Path::new(arg).exists(),
            CheckKind::HumanReview => return None, // defensivo — já filtrado acima.
        };
        if !passed {
            return Some((
                GoalVerdict::Fail,
                format!("critério '{}' não cumprido (check: {arg})", c.desc),
                "criterio_nao_cumprido".to_string(),
            ));
        }
    }
    Some((GoalVerdict::Pass, String::new(), String::new()))
}

/// Roda um comando de check via shell (`sh -c`). `Some(true)` = exit 0, `Some(false)` = exit != 0,
/// `None` = não pôde nem rodar (erro de transporte → fail-open no chamador, spec 52 §3). Sem captura
/// de stdout — só o exit code decide (determinístico, ZERO LLM).
fn run_check_command(cmd: &str) -> Option<bool> {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .ok()
        .map(|status| status.success())
}

/// F3-1-4 (spec 52 §3, gate d): emite um `ReviewVerdict` SÓ depois de enforçar `reviewer != target` —
/// o executor NUNCA se auto-avalia (anti-viés "dizer pronto"). `reviewer == target` → RECUSA (não
/// loga; `None`). Sucesso → `Some(seq)` do evento. `reviewer`/`target` são carimbados pelo supervisor
/// (server-side), nunca lidos do payload de um agente (regra-mãe ADR 0007 / spec 52 §Segurança).
#[allow(clippy::too_many_arguments)]
fn emit_review_verdict(
    store: &mut EventStore,
    goal_id: &str,
    plan_item: &str,
    target: NodeId,
    reviewer: NodeId,
    verdict: GoalVerdict,
    evidence: &str,
    defect_class: &str,
    iteration: u32,
) -> Result<Option<u64>, StoreError> {
    if reviewer == target {
        eprintln!(
            "lina-core: ReviewVerdict RECUSADO — reviewer == target (auto-avaliacao proibida) \
             goal={goal_id} item={plan_item}"
        );
        return Ok(None);
    }
    let seq = store.append(&DomainEvent::ReviewVerdict {
        goal_id: goal_id.to_string(),
        plan_item: plan_item.to_string(),
        target,
        reviewer,
        verdict,
        evidence: evidence.to_string(),
        defect_class: defect_class.to_string(),
        iteration,
    })?;
    Ok(Some(seq))
}

/// F3-1-4: conta os `ReviewVerdict::Fail` já no log para `(goal_id, plan_item)` — o nº de voltas
/// falhas ANTERIORES (a iteração corrente = este valor + 1). Varre o log (padrão CostLedger).
fn count_item_fails(store: &EventStore, goal_id: &str, item: &str) -> Result<u32, StoreError> {
    let mut fails = 0u32;
    for rec in store.events()? {
        if rec.kind != "ReviewVerdict" {
            continue;
        }
        let same = rec.payload.get("goal_id").and_then(|v| v.as_str()) == Some(goal_id)
            && rec.payload.get("plan_item").and_then(|v| v.as_str()) == Some(item)
            && rec.payload.get("verdict").and_then(|v| v.as_str()) == Some("Fail");
        if same {
            fails += 1;
        }
    }
    Ok(fails)
}

/// F3-1-4 (gate b): `true` se TODO item da `goal_id` tem como ÚLTIMO veredito um `Pass` (o laço pode
/// fechar). Goal sem itens → `false` (nada a fechar). Item sem veredito ou último `Fail` → `false`.
fn all_items_pass(store: &EventStore, plan: &Plan, goal_id: &str) -> Result<bool, StoreError> {
    let items: Vec<&str> = plan
        .itens
        .iter()
        .filter(|i| i.goal_id.as_deref() == Some(goal_id))
        .map(|i| i.id.as_str())
        .collect();
    if items.is_empty() {
        return Ok(false);
    }
    let events = store.events()?;
    for item in items {
        let last_verdict = events
            .iter()
            .rev()
            .find(|rec| {
                rec.kind == "ReviewVerdict"
                    && rec.payload.get("goal_id").and_then(|v| v.as_str()) == Some(goal_id)
                    && rec.payload.get("plan_item").and_then(|v| v.as_str()) == Some(item)
            })
            .and_then(|rec| rec.payload.get("verdict").and_then(|v| v.as_str()));
        if last_verdict != Some("Pass") {
            return Ok(false);
        }
    }
    Ok(true)
}

/// F3-1-4: o maior `iteration` entre os `ReviewVerdict` da goal — o total de voltas do OUTER loop
/// (vai em `GoalAchieved.iterations`).
fn max_goal_iteration(store: &EventStore, goal_id: &str) -> Result<u32, StoreError> {
    let mut max = 0u32;
    for rec in store.events()? {
        if rec.kind != "ReviewVerdict"
            || rec.payload.get("goal_id").and_then(|v| v.as_str()) != Some(goal_id)
        {
            continue;
        }
        let it = u32::try_from(
            rec.payload
                .get("iteration")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX);
        max = max.max(it);
    }
    Ok(max)
}

/// F3-2-3: o `proposed_team` da ÚLTIMA `GoalInterpreted` da `goal_id` — a correção via re-`interpret`
/// faz a última vencer (replay em ordem sobrescreve). Vazio se não houve interpretação ou time
/// proposto. É DADO transportado: o time é uma SUGESTÃO, não autoridade (a montagem é gated por §327).
fn proposed_team_of(store: &EventStore, goal_id: &str) -> Result<Vec<String>, StoreError> {
    let mut team: Vec<String> = Vec::new();
    for rec in store.events()? {
        if rec.kind != "GoalInterpreted"
            || rec.payload.get("goal_id").and_then(|v| v.as_str()) != Some(goal_id)
        {
            continue;
        }
        if let Some(arr) = rec.payload.get("proposed_team").and_then(|v| v.as_array()) {
            team = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
    }
    Ok(team)
}

/// F3-2-3: `true` se já existe um `SpawnRequested` com este `id` — idempotência da montagem do time
/// (re-confirm não duplica os spawns do papel; o `id` `goal-team:{goal}:{role}` é determinístico).
fn spawn_id_exists(store: &EventStore, id: &str) -> Result<bool, StoreError> {
    for rec in store.events()? {
        if rec.kind == "SpawnRequested"
            && rec.payload.get("id").and_then(|v| v.as_str()) == Some(id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// F3-2-5 (breaker sticky): conta os `SpawnRequested` de RE-ESCALONAMENTO já emitidos para
/// `(goal_id, item)` com este `effort` — o anti-re-spawn-idêntico (mesma tarefa+effort). Filtra pelo
/// prefixo do `id` (`goal-respawn:`/`goal-newdev:`) para NÃO contar a montagem inicial (`goal-team:`).
fn count_respawns_with_effort(
    store: &EventStore,
    goal_id: &str,
    item: &str,
    effort: Effort,
) -> Result<u32, StoreError> {
    let respawn = format!("goal-respawn:{goal_id}:{item}:");
    let newdev = format!("goal-newdev:{goal_id}:{item}:");
    let want = serde_json::to_value(effort).ok();
    let mut n = 0u32;
    for rec in store.events()? {
        if rec.kind != "SpawnRequested" {
            continue;
        }
        let id = rec.payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if (id.starts_with(&respawn) || id.starts_with(&newdev))
            && rec.payload.get("effort") == want.as_ref()
        {
            n += 1;
        }
    }
    Ok(n)
}

/// **F3-CONF-2: os ids de RE-DESPACHO de stall (`stall-respawn:{node}:…`) emitidos para `node` APÓS o
/// último RESET HUMANO do breaker dele** (`NodeStatusChanged{reason:"breaker_reset"}`). Escopar ao
/// pós-reset faz o gesto humano REINICIAR o protocolo (volta ao "1ª tentativa": lista vazia ⇒
/// re-despacha; já contém este epoch ⇒ aguarda; contém OUTRO epoch ⇒ a re-tentativa engoliu ⇒
/// escala). Derivável do log (invariante #4) — varre uma vez (custo só pago quando HÁ engolido).
fn stall_respawn_ids_since_reset(records: &[EventRecord], node: NodeId) -> Vec<String> {
    let node_str = node.to_string();
    let prefix = format!("{STALL_RESPAWN_PREFIX}:{node_str}:");
    let mut last_reset_seq = 0u64;
    for rec in records {
        if rec.kind == "NodeStatusChanged"
            && rec.payload.get("node").and_then(|v| v.as_str()) == Some(node_str.as_str())
            && rec.payload.get("reason").and_then(|v| v.as_str()) == Some(BREAKER_RESET_REASON)
        {
            last_reset_seq = rec.seq;
        }
    }
    records
        .iter()
        .filter(|rec| rec.seq > last_reset_seq && rec.kind == "SpawnRequested")
        .filter_map(|rec| rec.payload.get("id").and_then(|v| v.as_str()))
        .filter(|id| id.starts_with(&prefix))
        .map(String::from)
        .collect()
}

/// F1-3-6: loga `SpawnGated{reason}` (livro-razão da decisão do gate, par do `SpawnRequested`).
/// `reason` ∈ {`cascade`,`over_cap`,`manual`,`cost`}. Free fn (não toca o estado do Router) —
/// mesmo padrão de [`log_block`].
fn append_spawn_gated(
    store: &mut EventStore,
    id: &str,
    requested_by: NodeId,
    reason: &str,
) -> Result<(), StoreError> {
    store.append(&DomainEvent::SpawnGated {
        id: id.to_string(),
        requested_by,
        reason: reason.to_string(),
    })?;
    Ok(())
}

/// **A DECISÃO PURA do auto-spawn do integrador (F3-4, testável com `ts` controlado).** Dado os
/// records e `now_ms`, devolve os eventos a apendar (`[SpawnRequested, SpawnGated]`) quando o gatilho
/// determinístico dispara E não há pedido em voo; senão `[]`. PURO (sem I/O) — o `EventStore::append`
/// carimba `ts` com wall-clock, então a lógica time-based mora aqui (não no wrapper), reproduzível por
/// replay igual a [`CodeIntegration::from_records`]. O `id` leva o `seq` do último `CodeChanged`
/// (único por episódio; determinístico).
fn plan_integrator_spawn(records: &[EventRecord], now_ms: u64) -> Vec<DomainEvent> {
    let Some(dispatch) =
        CodeIntegration::from_records(records).integrator_dispatch(now_ms, INTEGRATOR_SETTLE_MS)
    else {
        return Vec::new();
    };
    if integrator_request_in_flight(records) {
        return Vec::new();
    }
    // Sufixo do id = seq do último `CodeChanged` (sempre ≥1 quando o gatilho dispara — há branch
    // pendente). Torna o `id` único por episódio: um novo lote de trabalho gera um id novo.
    let last_code_seq = records
        .iter()
        .filter(|r| r.kind == "CodeChanged")
        .map(|r| r.seq)
        .max()
        .unwrap_or(0);
    let id = format!("{AUTO_INTEGRATE_PREFIX}:{last_code_seq}");
    let prompt = format!(
        "Junte na develop as branches ainda não integradas: {}. Carregue a skill lina-integration e \
         valide cada merge (exit codes diretos) antes de fechar com `lina branch-integrated`.",
        dispatch.branches.join(", ")
    );
    vec![
        DomainEvent::SpawnRequested {
            id: id.clone(),
            requested_by: INTEGRATOR_TRIGGER,
            name: INTEGRATOR_NAME.to_string(),
            role: INTEGRATOR_ROLE.to_string(),
            root_cause_id: id.clone(),
            hops: 0,
            prompt,
            model: None,
            effort: None,
            goal_id: None,
        },
        DomainEvent::SpawnGated {
            id,
            requested_by: INTEGRATOR_TRIGGER,
            reason: AUTO_INTEGRATE_REASON.to_string(),
        },
    ]
}

/// Idempotência do auto-spawn do integrador (EDGE-triggered): `true` se já há um pedido de integrador
/// EM VOO — emitido DEPOIS do último trabalho pendente novo (`CodeChanged`). O gatilho é LEVEL (a
/// condição "equipe assentada + branch pendente" persiste até a integração); sem esta guarda, cada
/// tick spawnaria um integrador. Sem `CodeChanged` novo desde o último pedido ⇒ não re-disparar (o
/// banner segue pendente até o humano decidir); um `CodeChanged` posterior ⇒ re-arma. Só conta
/// `SpawnRequested` com `role == INTEGRATOR_ROLE` (as outras escadas de spawn não contam).
fn integrator_request_in_flight(records: &[EventRecord]) -> bool {
    let mut last_code_changed: Option<u64> = None;
    let mut last_integrator_req: Option<u64> = None;
    for rec in records {
        match rec.kind.as_str() {
            "CodeChanged" => last_code_changed = Some(rec.seq),
            "SpawnRequested"
                if rec.payload.get("role").and_then(|v| v.as_str()) == Some(INTEGRATOR_ROLE) =>
            {
                last_integrator_req = Some(rec.seq);
            }
            _ => {}
        }
    }
    match (last_integrator_req, last_code_changed) {
        // Pedi um integrador DEPOIS do último trabalho novo → ainda em voo (não re-disparar).
        (Some(req), Some(code)) => req > code,
        // Pedi sem nenhum `CodeChanged` registrado → conservador: em voo.
        (Some(_), None) => true,
        // Nunca pedi → livre para disparar.
        (None, _) => false,
    }
}

/// FIX #22: `true` se a mensagem é um PING DE PRESENÇA (`lina handshake`): `intent == "handshake"` E
/// alvo broadcast (`to == "*"`). Fire-and-forget — o roster vivo é do Supervisor (events.rs:141); a msg
/// não delega nem entrega a ninguém. Reconhecê-la cedo evita o `UnknownSender("")` + `RouteBlocked`
/// espúrio (o drain FLAT do handshake anonimiza `from → ""`). Exige AMBOS os predicados → um `ask`/
/// `handoff` real com alvo inválido (intent ≠ handshake) NÃO casa e segue logando como erro REAL.
fn is_presence(msg: &MailMessage) -> bool {
    msg.intent.as_str() == "handshake" && matches!(parse_target(&msg.to), TargetSpec::Broadcast)
}

/// `intent` ou `"ask"` se vazio (default canônico).
fn intent_or_ask(intent: &str) -> String {
    if intent.is_empty() {
        "ask".to_string()
    } else {
        intent.to_string()
    }
}

/// Rótulo legível do alvo para o bloco injetado (`@Nome` para nó nomeado, ou o `to` original).
fn recipient_label(recipient: &Recipient, raw_to: &str) -> String {
    match recipient {
        Recipient::Broadcast => "*".to_string(),
        _ => raw_to.to_string(),
    }
}

/// **ADR 0003: apenda `RouteBlocked` antes de retornar uma recusa** — o log é o livro-razão das
/// recusas (validável headless). Falha ao logar é VISÍVEL (stderr), nunca silenciosa. `duplicate`
/// é a exceção (não chama isto — anti-amplificação A4); `PersistFailed` também (não loga a falha de
/// logar). Reason é `Copy` → segue usável no `eprintln`.
fn log_block(store: &mut EventStore, msg: &MailMessage, reason: BlockReason) {
    if let Err(e) = store.append(&DomainEvent::RouteBlocked {
        id: msg.id.clone(),
        reason,
        from: msg.from.clone(),
        to: msg.to.clone(),
    }) {
        eprintln!(
            "lina-core: CRÍTICO — falha ao logar RouteBlocked({reason:?}) para {}: {e}",
            msg.id
        );
    }
}

/// **D1: ids já roteados (têm `MessageRouted` no log).** Dedupe DURÁVEL — sobrevive ao restart (o
/// `seen` em memória zera) e ao ack-falho. Best-effort: erro de leitura → conjunto vazio (a pior
/// hipótese recai no `seen` em memória + no `inspect_file`; nunca pior que antes). Custo da classe
/// W2 (varredura do log), diferido para o round de performance.
/// **F1-0-7 (ADR 0020) — ledger de ENTREGA derivado do log** (a idempotência durável da
/// story): por `id`, o estado consolidado de `MessageRouted`/`MessageDelivered`/
/// `MessageDeliveryFailed`/`MessageDeadLettered`. É o que permite ao pump, pós-restart,
/// distinguir "roteada e entregue" (no-op com resultado conhecido) de "roteada com
/// entrega FALHA pendente" (retoma o retry do attempt/agendamento corretos — nada é
/// re-injetado nem dropado).
struct LedgerEntry {
    to_node: Option<NodeId>,
    root: String,
    hops: u8,
    delivered: bool,
    dead: bool,
    /// Última falha: (attempt, next_retry_ms) — o agendamento sobrevive ao restart.
    failed: Option<(u32, u64)>,
    /// **SEAM-2 (R1):** `ts` do 1º `MessageRetained{prompt_not_ready}` desta msg (instante REAL
    /// da retenção, do log) — `Some` ⇒ roteada+retida-por-not-ready. Torna a retenção DURÁVEL: o
    /// pump re-semeia o motor de re-entrega (em vez de "roteada-sem-entrega ⇒ assume entregue"),
    /// e o backstop de retenção conta o tempo TOTAL desde aqui (não reinicia no restart). A
    /// retenção por `target_busy` retorna ANTES do `MessageRouted` (sem entrada no ledger → não
    /// afetada). `Option` (não `bool`) porque o instante é necessário para restaurar o relógio.
    retained_at: Option<u64>,
}

fn delivery_ledger(store: &EventStore) -> HashMap<String, LedgerEntry> {
    let mut out: HashMap<String, LedgerEntry> = HashMap::new();
    let Ok(recs) = store.events() else {
        return out;
    };
    for rec in recs {
        let id = rec
            .payload
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }
        match rec.kind.as_str() {
            "MessageRouted" => {
                let to_node = rec
                    .payload
                    .get("to_node")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok());
                let root = rec
                    .payload
                    .get("root_cause_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let hops = rec
                    .payload
                    .get("hops")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as u8;
                out.entry(id).or_insert(LedgerEntry {
                    to_node,
                    root,
                    hops,
                    delivered: false,
                    dead: false,
                    failed: None,
                    retained_at: None,
                });
            }
            // SEAM-2 (R1): retenção por not-ready (logada DEPOIS do `MessageRouted`) marca a entrada
            // como PENDENTE-RETIDA com o instante REAL — sem isto o restart cairia no braço
            // "roteada-sem-entrega ⇒ assume entregue" e DROPARIA a msg viva. Só `prompt_not_ready`
            // (a retenção `target_busy` retorna antes do `MessageRouted` ⇒ sem entrada aqui). 1º vence
            // (anti-amplificação A4 já garante 1× no log; `or` defensivo se houver replays).
            "MessageRetained"
                if rec.payload.get("reason").and_then(|v| v.as_str())
                    == Some("prompt_not_ready") =>
            {
                if let Some(e) = out.get_mut(&id) {
                    e.retained_at.get_or_insert(rec.ts);
                }
            }
            "MessageDelivered" => {
                if let Some(e) = out.get_mut(&id) {
                    e.delivered = true;
                }
            }
            "MessageDeliveryFailed" => {
                if let Some(e) = out.get_mut(&id) {
                    let attempt = rec
                        .payload
                        .get("attempt")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32;
                    let next = rec
                        .payload
                        .get("next_retry_ms")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    e.failed = Some((attempt, next));
                }
            }
            "MessageDeadLettered" => {
                if let Some(e) = out.get_mut(&id) {
                    e.dead = true;
                }
            }
            _ => {}
        }
    }
    out
}

// F1-0-7: o antigo `routed_ids` (D1, "roteado ⇒ entregue") foi SUBSTITUÍDO pelo
// `delivery_ledger` acima — que distingue entregue/falha-pendente/DLQ e torna o estado
// de retry durável (ADR 0020). A semântica conservadora do D1 sobrevive como o braço
// "roteada sem desfecho de entrega" do pump.

/// **§2.1 (ajuste de cooperação): o grafo de handoffs fecharia um ciclo APERTADO/REPETIDO?**
/// Reconstrói as arestas `from→to_node` dos `MessageRouted` do log com o MESMO `root_cause_id` e
/// responde `true` SE E SÓ SE adicionar `sender→target` (a) FECHA um ciclo (`target` JÁ ALCANÇA
/// `sender`) **E** (b) esse MESMO salto `sender→target` já apareceu `>= max_revisits` vezes neste root
/// (ping-pong, não diálogo). Assim o ida-e-volta legítimo (A→B→A, 1ª volta = `revisits` 0) e as
/// cadeias que retornam UMA vez (A→B→C→A) PASSAM; só o loop repetido é cortado. O loop infinito segue
/// barrado pelos backstops finais `hops`/`DELEGATION_BUDGET`. Identidades em `NodeId`; independe do
/// texto. Derivável do log (invariante #4).
///
/// # Errors
/// Falha ao ler o event log.
fn handoff_would_tight_loop(
    store: &EventStore,
    root_cause_id: &str,
    sender: NodeId,
    target: NodeId,
    max_revisits: u32,
) -> Result<bool, StoreError> {
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    // Quantas vezes o salto `sender→target` já foi roteado neste root (repetição do MESMO handoff).
    let mut revisits: u32 = 0;
    for rec in store.events()? {
        if rec.kind != "MessageRouted" {
            continue;
        }
        let same_root = rec
            .payload
            .get("root_cause_id")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == root_cause_id);
        if !same_root {
            continue;
        }
        let from = rec
            .payload
            .get("from")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<NodeId>().ok());
        let to = rec
            .payload
            .get("to_node")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<NodeId>().ok());
        if let (Some(f), Some(t)) = (from, to) {
            if f == sender && t == target {
                revisits = revisits.saturating_add(1);
            }
            adj.entry(f).or_default().push(t);
        }
    }
    // Loop APERTADO: o salto já se repetiu o bastante (b) E ele fecharia um ciclo (a). O diálogo
    // (revisits < max) e o retorno único de uma cadeia mais longa passam; só o ping-pong é cortado.
    Ok(revisits >= max_revisits && reaches(&adj, target, sender))
}

/// `true` se há um caminho `start → … → goal` no grafo dirigido `adj` (DFS, à prova de ciclos).
fn reaches(adj: &HashMap<NodeId, Vec<NodeId>>, start: NodeId, goal: NodeId) -> bool {
    if start == goal {
        return true;
    }
    let mut stack = vec![start];
    let mut visited: HashSet<NodeId> = HashSet::new();
    while let Some(n) = stack.pop() {
        if !visited.insert(n) {
            continue;
        }
        if let Some(nexts) = adj.get(&n) {
            for &m in nexts {
                if m == goal {
                    return true;
                }
                stack.push(m);
            }
        }
    }
    false
}

/// **W3-7c (§2.2): teto de custo — agregador de uso por workspace/dia, EVENT-SOURCED.** Reconstrói
/// o estado do teto varrendo o event log (invariante #4: o log é a fonte da verdade; nada de campo
/// efêmero) — mesmo padrão do grafo anti-loop (`handoff_would_tight_loop`). Não toca o `ProjectedState`:
/// o uso/teto são META do canvas, mas observáveis e replayáveis.
///
/// **Janela contábil:** soma os `TokenUsageReported` apendados DEPOIS do último `CostCeilingResumed`
/// (que zera a janela). Sem o reset, retomar reabriria com a soma anterior ainda > teto e a próxima
/// delegação re-pausaria — `lina resume` não "reabriria" de fato. `paused` segue o último evento de
/// teto visto (`CostCeilingHit` → pausado; `CostCeilingResumed` → liberado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostLedger {
    /// Dia-UTC (`YYYY-MM-DD`) DERIVADO do relógio do roteador (`now_ms`) — rótulo do `CostCeilingHit`.
    pub day: String,
    /// Soma de tokens da janela contábil atual (desde o último `CostCeilingResumed`).
    pub tokens: u64,
    /// `true` se o workspace está em `Paused` (último evento de teto foi um `CostCeilingHit`).
    pub paused: bool,
}

impl CostLedger {
    /// Reconstrói o ledger do log. `now_ms` define o dia-UTC corrente (janela diária) E o rótulo
    /// `day` do teto eventual.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore, now_ms: u64) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?, now_ms))
    }

    /// **Round 5 #9: agrega uso/teto SÓ da janela do DIA-UTC corrente** (`utc_day(rec.ts) ==
    /// utc_day(now_ms)`). Registros de outros dias NÃO contam → a janela diária zera sozinha à
    /// meia-noite (reduz a dependência do `lina resume` humano). Puro (sem I/O) → testável com `ts`
    /// controlado, já que `EventStore::append` carimba o `ts` com wall-clock e não há injeção de relógio.
    fn from_records(records: &[EventRecord], now_ms: u64) -> Self {
        let day = utc_day(now_ms);
        let mut tokens: u64 = 0;
        let mut paused = false;
        for rec in records {
            // Fora do dia-UTC corrente NÃO conta (janela diária — Round 5 #9). Isto zera tanto a soma
            // quanto o estado `paused` à meia-noite: um `CostCeilingHit` de ontem não pausa hoje.
            if utc_day(rec.ts) != day {
                continue;
            }
            match rec.kind.as_str() {
                "TokenUsageReported" => {
                    let t = rec
                        .payload
                        .get("tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    tokens = tokens.saturating_add(t);
                }
                "CostCeilingHit" => paused = true,
                // Retomada humana: sai de Paused E zera a janela contábil (novo período no mesmo dia).
                "CostCeilingResumed" => {
                    paused = false;
                    tokens = 0;
                }
                _ => {}
            }
        }
        Self {
            day,
            tokens,
            paused,
        }
    }
}

/// Dia-UTC (`YYYY-MM-DD`) de um instante em ms desde a época. Algoritmo civil-from-days de Howard
/// Hinnant (puro, sem dependência de fuso/calendário externo) — determinístico para o rótulo do teto.
fn utc_day(now_ms: u64) -> String {
    let days = (now_ms / 86_400_000) as i64; // dias inteiros desde 1970-01-01 (UTC)
                                             // civil_from_days: converte a contagem de dias na época para (ano, mês, dia) do calendário civil.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DomainEvent, EventRecord, EventStore};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Store temporário (event log em disco) — descartado no Drop.
    struct TmpStore {
        dir: std::path::PathBuf,
        store: EventStore,
    }
    impl TmpStore {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lina-router-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let store = EventStore::open(&dir).expect("abrir store");
            Self { dir, store }
        }
    }
    impl Drop for TmpStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn sink() -> Box<dyn std::io::Write + Send> {
        Box::new(std::io::sink())
    }

    /// F1-0-4: o alvo "terminou o turno" (Idle). Desde a entrega ciente de estado, TODA
    /// entrega marca o alvo `Busy` (serialização P1) — testes que entregam de novo ao
    /// MESMO alvo simulam o fim do turno entre as entregas, como o lifecycle real faria
    /// (EndDetector → Idle). Sem isto a 2ª entrega é `Retained{target_busy}` — que é
    /// exatamente o comportamento de produto provado nos testes de retenção.
    fn turn_done(sup: &Supervisor, node: NodeId) {
        sup.set_status(node, NodeStatus::Idle).expect("set Idle");
    }

    /// `deliver` de teste que só REGISTRA (target, from, text) — sem PTY real.
    type Recorder = Rc<RefCell<Vec<(NodeId, NodeId, String)>>>;
    fn recorder() -> (
        Recorder,
        impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String>,
    ) {
        let rec: Recorder = Rc::new(RefCell::new(Vec::new()));
        let r2 = Rc::clone(&rec);
        let f = move |target: NodeId, from: NodeId, text: &str| {
            r2.borrow_mut().push((target, from, text.to_string()));
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        };
        (rec, f)
    }

    fn router_with(tag: &str) -> (Router, Arc<Supervisor>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lina-router-mb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sup = Arc::new(Supervisor::new());
        let mb = Mailbox::new(dir.join(".lina"));
        (Router::new(Arc::clone(&sup), mb), sup, dir)
    }

    /// Os `reason` (snake_case) de todos os `RouteBlocked` no log — base das asserções de bloqueio
    /// (ADR 0003 / AC-4.1: a recusa é assertada NO LOG, não só no enum de retorno).
    fn blocked_reasons(store: &EventStore) -> Vec<String> {
        store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "RouteBlocked")
            .filter_map(|r| {
                r.payload
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect()
    }

    #[test]
    fn ask_routes_resolves_names_and_delivers() {
        let (mut router, sup, dir) = router_with("happy");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("happy");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "@B", "ask", "oi");
        let out = router.route_message(&msg, &mut ts.store, 1000, &mut deliver);
        assert_eq!(out, RouteOutcome::Delivered { targets: vec![b] });

        let calls = rec.borrow();
        assert_eq!(calls.len(), 1, "uma entrega");
        assert_eq!(calls[0].0, b, "alvo é B");
        assert!(calls[0].2.contains("[LINA::MSG]") && calls[0].2.contains("payload: oi"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────── F3-4-3 (spec 36 §2): handler do sinal de mudança `code.changed` ─────────

    /// Plano sintético direto no log: item reserva `paths`, claimed por `owner` (vira Doing na projeção).
    fn seed_claim(store: &mut EventStore, item: &str, paths: &[&str], owner: &str) {
        store
            .append(&DomainEvent::PlanItemAdded {
                item: item.into(),
                desc: "x".into(),
            })
            .expect("PlanItemAdded");
        store
            .append(&DomainEvent::PlanItemAttributed {
                item: item.into(),
                goal_id: None,
                parents: vec![],
                acceptance: vec![],
                budget_tokens: 0,
                paths: paths.iter().map(|p| (*p).to_string()).collect(),
            })
            .expect("PlanItemAttributed");
        store
            .append(&DomainEvent::PlanClaimed {
                id: format!("claim-{item}"),
                item: item.into(),
                by: owner.into(),
            })
            .expect("PlanClaimed");
    }

    /// **Regra-mãe (ADR 0007):** `code.changed` carimba `author_node` SERVER-SIDE (= remetente
    /// AUTENTICADO), IGNORANDO um `author_node` FORJADO no payload. Mutação prova: o forjado some.
    #[test]
    fn code_changed_stamps_author_server_side_ignoring_payload() {
        let (mut router, sup, dir) = router_with("cc-auth");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("cc-auth");
        let (_rec, mut deliver) = recorder();
        // author_node FORJADO no payload = @EVIL — deve ser ignorado.
        let payload =
            r#"{"branch":"lina/a","commit":"c1","paths":["src/x.rs"],"author_node":"@EVIL"}"#;
        let msg = MailMessage::new("@A", "@workspace", "code.changed", payload);
        let out = router.route_message(&msg, &mut ts.store, 1000, &mut deliver);
        assert!(matches!(out, RouteOutcome::Delivered { .. }));
        let cc = ts
            .store
            .events()
            .expect("events")
            .into_iter()
            .find(|r| r.kind == "CodeChanged")
            .expect("CodeChanged logado");
        assert_eq!(
            cc.payload.get("author_node").and_then(|v| v.as_str()),
            Some("@A"),
            "author = remetente autenticado, NUNCA o @EVIL forjado no payload"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **F3-4-5 (costura intent→handler fechada pelo Maestro na integração):** `branch.integrated`
    /// percorre `route_message` → apenda `BranchIntegrated` (a ÚNICA prova de fechamento, spec 36 §3)
    /// → a projeção `code` deixa de listar a branch como pendente. End-to-end (NÃO `store.append` à
    /// mão): prova a COSTURA (o verbo de B chegava órfão sem este handler), não só o evento.
    #[test]
    fn branch_integrated_emits_event_and_closes_branch_end_to_end() {
        let (mut router, sup, dir) = router_with("bi-close");
        let _d = sup.register("@DevOps", None, sink());
        let mut ts = TmpStore::new("bi-close");
        let (_rec, mut deliver) = recorder();
        // 1) um commit deixa a branch PENDENTE (CodeChanged sem BranchIntegrated).
        let cc = r#"{"branch":"lina/a","commit":"c1","paths":["src/x.rs"]}"#;
        let _ = router.route_message(
            &MailMessage::new("@DevOps", "@workspace", "code.changed", cc),
            &mut ts.store,
            1000,
            &mut deliver,
        );
        assert!(
            crate::code::CodeIntegration::replay(&ts.store)
                .expect("proj")
                .is_branch_pending("lina/a"),
            "pendente após CodeChanged"
        );
        // 2) branch.integrated PELO CAMINHO REAL → BranchIntegrated no log (handler LIGADO).
        let bi = r#"{"branch":"lina/a","into":"develop","commit":"m1"}"#;
        let out = router.route_message(
            &MailMessage::new("@DevOps", "@workspace", "branch.integrated", bi),
            &mut ts.store,
            2000,
            &mut deliver,
        );
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "intent processado pelo handler (não fica órfão)"
        );
        assert!(
            ts.store
                .events()
                .expect("events")
                .into_iter()
                .any(|r| r.kind == "BranchIntegrated"),
            "BranchIntegrated emitido pelo handler (a costura está LIGADA)"
        );
        // 3) a projeção fecha a branch — prova de fechamento (spec 36 §3).
        assert!(
            !crate::code::CodeIntegration::replay(&ts.store)
                .expect("proj")
                .is_branch_pending("lina/a"),
            "branch fechada após BranchIntegrated"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── F3-4 (pendência fechada na F3-5): auto-spawn do DevOps integrador ──────────────────────

    /// `EventRecord` sintético a partir do `DomainEvent` REAL — payload com a tag `event`, para
    /// `CodeIntegration::from_record` decodificar. Fixture à-mão sem a tag seria engolido em silêncio.
    fn synth_rec(seq: u64, ts: u64, ev: &DomainEvent) -> EventRecord {
        EventRecord {
            seq,
            ts,
            kind: ev.kind().to_string(),
            version: ev.current_version(),
            payload: serde_json::to_value(ev).expect("serializar evento"),
        }
    }

    /// Um nó ASSENTADO (Idle) desde `since_ts` — fixture do gatilho "equipe assentada".
    fn node_idle(seq: u64, since_ts: u64, node: NodeId) -> EventRecord {
        synth_rec(
            seq,
            since_ts,
            &DomainEvent::NodeStatusChanged {
                node,
                status: NodeStatus::Idle.as_str().to_string(),
                from: String::new(),
                reason: String::new(),
            },
        )
    }

    /// Um commit que deixa `branch` pendente (sem `BranchIntegrated`).
    fn code_changed_rec(seq: u64, ts: u64, branch: &str) -> EventRecord {
        synth_rec(
            seq,
            ts,
            &DomainEvent::CodeChanged {
                branch: branch.to_string(),
                paths: vec!["src/x.rs".to_string()],
                author_node: "n-author".to_string(),
                commit: "c1".to_string(),
            },
        )
    }

    const SETTLE: u64 = INTEGRATOR_SETTLE_MS;

    /// **Gate (h) — o DevOps é PROPOSTO (auto-spawn) quando a equipe assenta com branch pendente.** A
    /// decisão pura emite `SpawnRequested{DEVOPS, hops:0}` + `SpawnGated{auto_integrate}` (aval
    /// humano); idempotente no mesmo lote (re-chamar com o pedido já no log NÃO re-propõe).
    #[test]
    fn auto_spawn_proposes_integrator_when_team_settled() {
        let n1 = NodeId::from_u128(1);
        let records = vec![
            code_changed_rec(1, 1_000, "lina/a"),
            node_idle(2, 1_000, n1),
        ];
        let now = 1_000 + SETTLE + 1;
        let events = plan_integrator_spawn(&records, now);
        assert_eq!(events.len(), 2, "SpawnRequested + SpawnGated");
        assert!(
            matches!(&events[0], DomainEvent::SpawnRequested { role, requested_by, hops, .. }
                if role == INTEGRATOR_ROLE && *requested_by == INTEGRATOR_TRIGGER && *hops == 0),
            "pedido do integrador, origem (hops 0), requested_by = sentinela do sistema"
        );
        assert!(matches!(
            &events[1],
            DomainEvent::SpawnGated { reason, requested_by, .. }
                if reason == AUTO_INTEGRATE_REASON && *requested_by == INTEGRATOR_TRIGGER
        ));

        // Idempotência: com o SpawnRequested já no log, a guarda edge-triggered barra a re-proposta.
        let mut after = records.clone();
        after.push(synth_rec(3, now, &events[0]));
        after.push(synth_rec(4, now, &events[1]));
        assert!(
            plan_integrator_spawn(&after, now + SETTLE).is_empty(),
            "pedido em voo ⇒ não re-propõe (sem tempestade)"
        );
    }

    /// Re-arma: um `CodeChanged` NOVO depois do último pedido solta a guarda → propõe de novo (a
    /// equipe terminou outro lote de trabalho).
    #[test]
    fn auto_spawn_rearms_on_new_work() {
        let n1 = NodeId::from_u128(1);
        let records = vec![
            code_changed_rec(1, 1_000, "lina/a"),
            node_idle(2, 1_000, n1),
            synth_rec(
                3,
                2_000,
                &DomainEvent::SpawnRequested {
                    id: "auto-integrate:1".into(),
                    requested_by: INTEGRATOR_TRIGGER,
                    name: INTEGRATOR_NAME.into(),
                    role: INTEGRATOR_ROLE.into(),
                    root_cause_id: "auto-integrate:1".into(),
                    hops: 0,
                    prompt: String::new(),
                    model: None,
                    effort: None,
                    goal_id: None,
                },
            ),
            code_changed_rec(4, 3_000, "lina/b"), // trabalho NOVO depois do pedido
        ];
        assert_eq!(
            plan_integrator_spawn(&records, 1_000 + SETTLE + 1).len(),
            2,
            "CodeChanged posterior ao pedido ⇒ re-propõe"
        );
    }

    /// Silencioso quando a equipe NÃO assentou (um nó ainda Busy), mesmo com branch pendente.
    #[test]
    fn auto_spawn_silent_when_team_busy() {
        let n1 = NodeId::from_u128(1);
        let n2 = NodeId::from_u128(2);
        let records = vec![
            code_changed_rec(1, 1_000, "lina/a"),
            node_idle(2, 1_000, n1),
            synth_rec(
                3,
                1_000,
                &DomainEvent::NodeStatusChanged {
                    node: n2,
                    status: NodeStatus::Busy.as_str().to_string(),
                    from: String::new(),
                    reason: String::new(),
                },
            ),
        ];
        assert!(
            plan_integrator_spawn(&records, 1_000 + SETTLE + 1).is_empty(),
            "um nó Busy ⇒ equipe não assentada ⇒ não dispara"
        );
    }

    /// Silencioso quando NÃO há branch pendente (equipe assentada, mas nada a juntar).
    #[test]
    fn auto_spawn_silent_when_nothing_pending() {
        let n1 = NodeId::from_u128(1);
        let records = vec![node_idle(1, 1_000, n1)];
        assert!(plan_integrator_spawn(&records, 1_000 + SETTLE + 1).is_empty());
    }

    /// **A COSTURA LIGADA (não órfã): o `pump` dispara o auto-spawn.** End-to-end pelo tick REAL — a
    /// branch pendente + equipe assentada faz o pump apendar `SpawnRequested{DEVOPS}` no log (prova que
    /// a fiação existe, não só a decisão pura). Mailbox vazia: só os sweeps + auto-spawn correm.
    #[test]
    fn pump_wires_auto_spawn_integrator() {
        let (mut router, sup, dir) = router_with("auto-spawn-pump");
        let mut ts = TmpStore::new("auto-spawn-pump");
        let (_rec, mut deliver) = recorder();
        let n1 = NodeId::from_u128(0xA1);
        ts.store
            .append(&DomainEvent::CodeChanged {
                branch: "lina/a".into(),
                paths: vec!["src/x.rs".into()],
                author_node: "n-a".into(),
                commit: "c1".into(),
            })
            .expect("cc");
        ts.store
            .append(&DomainEvent::NodeStatusChanged {
                node: n1,
                status: NodeStatus::Idle.as_str().to_string(),
                from: String::new(),
                reason: String::new(),
            })
            .expect("idle");
        // `append` carimba `ts` com wall-clock; o now_ms do tick fica além da janela de assentamento.
        let base = ts.store.events().expect("ev").last().expect("last").ts;
        router.pump(&mut ts.store, base + SETTLE + 1_000, &mut deliver);
        assert!(
            ts.store
                .events()
                .expect("ev")
                .iter()
                .any(|r| r.kind == "SpawnRequested"
                    && r.payload.get("role").and_then(|v| v.as_str()) == Some(INTEGRATOR_ROLE)),
            "o pump fiou o auto-spawn: SpawnRequested{{DEVOPS}} no log"
        );
        drop(sup);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Broadcast por PERTENCIMENTO: só o dono do claim que intersecta os paths (e ≠ autor) é
    /// notificado a PARAR; um claim disjunto fica intacto. `CodeChanged` logado (author = @A).
    #[test]
    fn code_changed_notifies_only_intersecting_claim_owner() {
        let (mut router, sup, dir) = router_with("cc-belong");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("cc-belong");
        let (rec, mut deliver) = recorder();
        seed_claim(&mut ts.store, "T1", &["src/leads.rs"], "@B");
        seed_claim(&mut ts.store, "T2", &["src/outro.rs"], "@C");
        // @A commita tocando SÓ src/leads.rs (reservado por @B).
        let payload = r#"{"branch":"lina/a","commit":"c1","paths":["src/leads.rs"]}"#;
        let msg = MailMessage::new("@A", "@workspace", "code.changed", payload);
        let out = router.route_message(&msg, &mut ts.store, 2000, &mut deliver);
        assert_eq!(out, RouteOutcome::Delivered { targets: vec![b] });
        let calls = rec.borrow();
        assert_eq!(calls.len(), 1, "só @B (pertencimento); @C intacto");
        assert_eq!(calls[0].0, b, "alvo é @B (o dono que PARA)");
        assert!(calls[0].2.contains("pare"), "a notificação manda PARAR");
        drop(calls);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Controle −: o autor NÃO é avisado do PRÓPRIO commit, e um commit disjunto não avisa ninguém.
    #[test]
    fn code_changed_skips_self_author_and_disjoint() {
        let (mut router, sup, dir) = router_with("cc-self");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("cc-self");
        let (rec, mut deliver) = recorder();
        // @A reserva src/leads.rs e é o PRÓPRIO autor do commit que o toca.
        seed_claim(&mut ts.store, "T1", &["src/leads.rs"], "@A");
        let payload = r#"{"branch":"lina/a","commit":"c1","paths":["src/leads.rs"]}"#;
        let msg = MailMessage::new("@A", "@workspace", "code.changed", payload);
        let out = router.route_message(&msg, &mut ts.store, 2000, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::Delivered { targets: vec![] },
            "autor==dono → ninguém avisado"
        );
        assert!(rec.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Payload sem `branch` → recusa LEGÍVEL, nada logado (sinal malformado não polui o log).
    #[test]
    fn code_changed_without_branch_is_rejected() {
        let (mut router, sup, dir) = router_with("cc-bad");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("cc-bad");
        let (_rec, mut deliver) = recorder();
        let msg = MailMessage::new("@A", "@workspace", "code.changed", r#"{"paths":["x"]}"#);
        let out = router.route_message(&msg, &mut ts.store, 1000, &mut deliver);
        assert!(matches!(out, RouteOutcome::ContractRejected(_)));
        assert!(
            ts.store
                .events()
                .expect("events")
                .iter()
                .all(|r| r.kind != "CodeChanged"),
            "branch ausente → nenhum CodeChanged logado"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **W3-6c A3 (AC):** uma msg escrita no outbox POR-NÓ de @X com `from` FORJADO=@Y é roteada como
    /// vinda de @X — a ORIGEM (diretório-dono, atribuída pelo `drain` do supervisor) vence o campo. A
    /// entrega registra `from` = NodeId de @X (nunca @Y) e o bloco diz `from: @X`. O supervisor é o
    /// `pump`; o `drain` autentica a origem; o router não precisou mudar (deriva sender de `msg.from`).
    #[test]
    fn per_node_outbox_authenticates_origin_over_forged_from() {
        let (mut router, sup, dir) = router_with("a3");
        let x = sup.register("@X", None, sink());
        let b = sup.register("@B", None, sink());
        let y = sup.register("@Y", None, sink()); // @Y existe: a diferença é a ORIGEM, não a inexistência
        let mut ts = TmpStore::new("a3");
        let (rec, mut deliver) = recorder();

        // Campo `from` FORJADO = @Y, mas o arquivo é escrito no outbox do nó @X.
        let m = MailMessage::new("@Y", "@B", "ask", "deploy prod");
        router
            .mailbox()
            .enqueue_as("@X", &m)
            .expect("enqueue_as @X");

        let outcomes = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0].1, RouteOutcome::Delivered { .. }));

        let calls = rec.borrow();
        assert_eq!(calls.len(), 1, "uma entrega");
        assert_eq!(calls[0].0, b, "alvo é @B");
        assert_eq!(calls[0].1, x, "a origem @X vence o campo forjado @Y");
        assert_ne!(calls[0].1, y, "jamais atribuído ao @Y forjado");
        assert!(
            calls[0].2.contains("from: @X"),
            "o bloco entregue atribui from: @X"
        );
        drop(calls);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **DIFERENCIAL #1 (o caso do fundador "manda oi pro Terminal B") — nome COM ESPAÇO, ponta a
    /// ponta.** Reproduz o caminho REAL do `lina ask`: o terminal escreve no SEU subdir por-nó
    /// (`enqueue_as("Terminal A", …)`) com o campo `from` que o drain IGNORA (até VAZIO, o exato
    /// cenário que o diagnóstico temia). O `pump` drena (`drain_to_inflight` carimba `from` =
    /// dir-dono), `node_by_name("Terminal A")` resolve e ENTREGA — JAMAIS `UnknownSender("")`. Cobre
    /// o gap dos testes per-nó existentes (que só usam `@X` SEM espaço); nomes de terminal reais têm
    /// espaço, e `valid_node_name`/`collect_node_outbox` os aceitam.
    #[test]
    fn per_node_ask_with_spaced_name_stamps_origin_and_delivers() {
        let (mut router, sup, dir) = router_with("spaced");
        let a = sup.register("Terminal A", None, sink());
        let b = sup.register("Terminal B", None, sink());
        let mut ts = TmpStore::new("spaced");
        let (rec, mut deliver) = recorder();

        // `from` VAZIO de propósito: a origem (dir-dono) decide, nunca o campo. É o cenário do
        // diagnóstico (from="" → UnknownSender("")); provamos que o carimbo per-nó o RESOLVE.
        let m = MailMessage::new("", "@Terminal B", "ask", "oi");
        router
            .mailbox()
            .enqueue_as("Terminal A", &m)
            .expect("enqueue_as 'Terminal A' (nome com espaço é subdir válido)");

        let outcomes = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].1,
            RouteOutcome::Delivered { targets: vec![b] },
            "entregue a 'Terminal B' — o drain carimbou from='Terminal A' (NÃO UnknownSender(\"\"))"
        );

        let calls = rec.borrow();
        assert_eq!(calls.len(), 1, "uma entrega");
        assert_eq!(
            calls[0].1, a,
            "origem autenticada = NodeId de 'Terminal A' (campo vazio vencido)"
        );
        assert!(
            calls[0].2.contains("from: Terminal A"),
            "o bloco entregue atribui from: Terminal A (a origem, não o campo vazio)"
        );
        drop(calls);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **CONTRA-PROVA — de ONDE vem o `UnknownSender("")` do log de uso real.** NÃO é o `lina ask`
    /// (por-nó, carimbado), mas o outbox FLAT (legado/anônimo — ex.: `lina handshake`, que usa
    /// `enqueue` flat): o drain ANONIMIZA o `from` (anti-impersonação), `node_by_name("")` não
    /// resolve → `UnknownSender("")`. Mesmos nós e router do teste acima: SÓ o caminho de enqueue
    /// muda (flat vs por-nó) e o desfecho inverte — a string VAZIA é a assinatura do FLAT, não do per-nó.
    #[test]
    fn flat_outbox_message_is_unknown_empty_sender_not_per_node() {
        let (mut router, sup, dir) = router_with("flatunknown");
        let _a = sup.register("Terminal A", None, sink());
        let _b = sup.register("Terminal B", None, sink());
        let mut ts = TmpStore::new("flatunknown");
        let (_rec, mut deliver) = recorder();

        // FLAT (como `lina handshake`, lina.rs): `from` nomeado, mas o drain flat o anonimiza → "".
        router
            .mailbox()
            .enqueue(&MailMessage::new("Terminal A", "@Terminal B", "ask", "oi"))
            .expect("enqueue flat");

        let outcomes = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].1,
            RouteOutcome::UnknownSender(String::new()),
            "FLAT anonimizado → from vazio → UnknownSender(\"\") (a assinatura do log de uso real)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FIX #22 (backlog) — o `lina handshake` (presença, `to=*`) é absorvido como `Presence` e NÃO
    /// loga `RouteBlocked`: o ruído "mensagem … não roteada: UnknownSender(\"\")" no stderr do app some
    /// na ORIGEM (sem desfecho de erro). Exatamente o caminho FLAT do `flat_outbox_…` acima, só que com
    /// `intent=handshake` + `to=*` — e o desfecho inverte de erro para presença benigna.
    #[test]
    fn handshake_presence_is_absorbed_not_route_blocked() {
        let (mut router, sup, dir) = router_with("presence");
        let _a = sup.register("Terminal A", None, sink());
        let mut ts = TmpStore::new("presence");
        let (rec, mut deliver) = recorder();

        // Como o `lina handshake` real (lina.rs): enqueue FLAT, intent=handshake, to=*.
        router
            .mailbox()
            .enqueue(&MailMessage::new(
                "Terminal A",
                "*",
                "handshake",
                "Terminal A entrou no workspace",
            ))
            .expect("enqueue handshake");

        let outcomes = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].1,
            RouteOutcome::Presence,
            "handshake é presença benigna, não UnknownSender(\"\")"
        );
        assert!(
            blocked_reasons(&ts.store).is_empty(),
            "presença NÃO loga RouteBlocked — sem ruído de erro no log (a fonte do stderr do app)"
        );
        assert!(
            rec.borrow().is_empty(),
            "0 broadcast: a presença não injeta em ninguém"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FIX #22 — NÃO mascaramos erros REAIS: uma msg de delegação (`ask`) com `from` resolvível mas
    /// alvo INEXISTENTE AINDA loga `RouteBlocked{no_target}`. Só o handshake/presença é silenciado.
    #[test]
    fn real_message_with_invalid_target_still_logs_block() {
        let (mut router, sup, dir) = router_with("realbad");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("realbad");
        let (_rec, mut deliver) = recorder();

        // from resolvível (@A), to inexistente (@Ghost), intent REAL (ask) → NoTarget + RouteBlocked.
        let msg = MailMessage::new("@A", "@Ghost", "ask", "oi");
        let outcome = router.route_message(&msg, &mut ts.store, 1000, &mut deliver);
        assert_eq!(outcome, RouteOutcome::NoTarget);
        assert!(
            blocked_reasons(&ts.store)
                .iter()
                .any(|r| r == "no_target"),
            "erro REAL (alvo inválido) AINDA loga RouteBlocked — a presença não afrouxou o pipeline"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_id_within_window_is_ignored() {
        let (mut router, sup, dir) = router_with("dedupe");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("dedupe");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "@B", "ask", "oi");
        assert!(matches!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        // Mesmo id, dentro da janela → duplicada (não entrega de novo).
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1500, &mut deliver),
            RouteOutcome::Duplicate
        );
        assert_eq!(rec.borrow().len(), 1, "só a 1ª entregou");
        // ADR 0003 (anti-amplificação A4): `duplicate` NÃO apenda RouteBlocked.
        assert!(
            !blocked_reasons(&ts.store).contains(&"duplicate".to_string()),
            "duplicate não pode virar evento (amplificaria escrita sob flood)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_id_after_window_is_not_duplicate() {
        let (mut router, sup, dir) = router_with("window");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("window");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "@B", "ask", "oi");
        router.route_message(&msg, &mut ts.store, 0, &mut deliver);
        turn_done(&sup, b); // F1-0-4: B terminou o turno entre as duas entregas
                            // Bem depois da janela de 60s → re-roteável (a entrada velha foi podada).
        let out = router.route_message(&msg, &mut ts.store, DEDUPE_WINDOW_MS + 1, &mut deliver);
        assert!(matches!(out, RouteOutcome::Delivered { .. }));
        assert_eq!(rec.borrow().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// W1: o `root_cause_id` da mailbox é IGNORADO — forjar um root fresco/distinto por mensagem NÃO
    /// reseta a contagem. O orçamento acumula sob o root DERIVADO do binding; um delegador repetido
    /// estoura o teto mesmo forjando um root distinto a cada msg.
    #[test]
    fn delegation_budget_is_enforced_per_root_cause() {
        let (mut router, sup, dir) = router_with("budget");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("budget");
        let (_rec, mut deliver) = recorder();

        // A→B estabelece o binding de B (root = id da msg de A — carimbado na entrega).
        let m0 = MailMessage::new("@A", "@B", "ask", "inicia");
        router.route_message(&m0, &mut ts.store, 1, &mut deliver);

        // B delega DELEGATION_BUDGET vezes, CADA uma com um root_cause_id FORJADO distinto → todas
        // ignoram o forjado e contam contra (root herdado, B); a (teto+1)-ésima estoura.
        for i in 0..DELEGATION_BUDGET {
            turn_done(&sup, c); // F1-0-4: C termina cada turno (testa o BUDGET, não a retenção)
            let mut msg = MailMessage::new("@B", "@C", "ask", format!("t{i}"));
            msg.root_cause_id = Some(format!("forjado-{i}")); // IGNORADO (W1)
            assert!(
                matches!(
                    router.route_message(&msg, &mut ts.store, 10 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "delegação {i} deveria passar (root forjado ignorado)"
            );
        }
        turn_done(&sup, c); // idem: o estouro abaixo deve vir do orçamento, não do estado
        let mut over = MailMessage::new("@B", "@C", "ask", "demais");
        over.root_cause_id = Some("forjado-final".into());
        assert_eq!(
            router.route_message(&over, &mut ts.store, 999, &mut deliver),
            RouteOutcome::BudgetExceeded
        );
        // AC-4.1: a recusa por orçamento está NO LOG.
        assert!(blocked_reasons(&ts.store).contains(&"budget_exceeded".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// W1: `msg.hops` é forjável (outbox não-autenticado) → IGNORADO. Uma msg de ORIGEM com hops=99
    /// é tratada como profundidade 0 (entregue), NÃO cortada. O hop-limit real vem da cadeia derivada
    /// do binding (ver `hop_limit_cuts_propagated_chain_at_max_depth`).
    #[test]
    fn forged_hops_are_ignored() {
        let (mut router, sup, dir) = router_with("forged-hops");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("forged-hops");
        let (_rec, mut deliver) = recorder();

        let mut msg = MailMessage::new("@A", "@B", "ask", "oi");
        msg.hops = 99; // forjado — deve ser ignorado
        assert!(
            matches!(
                router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
                RouteOutcome::Delivered { .. }
            ),
            "hops forjado é ignorado — origem é profundidade 0, sem HopLimit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_sender_and_target_are_rejected() {
        let (mut router, sup, dir) = router_with("unknown");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("unknown");
        let (_rec, mut deliver) = recorder();

        // Remetente fantasma.
        let ghost = MailMessage::new("@Ghost", "@A", "ask", "oi");
        assert_eq!(
            router.route_message(&ghost, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::UnknownSender("@Ghost".into())
        );
        // Alvo inexistente.
        let no_target = MailMessage::new("@A", "@Nobody", "ask", "oi");
        assert_eq!(
            router.route_message(&no_target, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::NoTarget
        );
        // AC-4.1: ambas as recusas estão NO LOG.
        let reasons = blocked_reasons(&ts.store);
        assert!(reasons.contains(&"unknown_sender".to_string()));
        assert!(reasons.contains(&"no_target".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **RETRY TRANSIENTE — a race de inicialização ENTREGA quando o nó chega.** O remetente manda
    /// `lina ask` para um colega que AINDA não entrou no roster (criação concorrente). Antes a msg era
    /// ackeada e PERDIDA (`NoTarget`); agora é RETIDA no `.inflight` e o tick seguinte — após o destino
    /// registrar — ENTREGA. Reproduz o cenário REAL do fundador (bloqueios só no início, depois flui).
    #[test]
    fn transient_no_target_is_retained_and_delivers_when_node_arrives() {
        let (mut router, sup, dir) = router_with("retry-arrives");
        sup.register("Terminal A", None, sink()); // só o REMETENTE existe no início
        let mut ts = TmpStore::new("retry-arrives");
        let (rec, mut deliver) = recorder();

        let m = MailMessage::new("", "@Terminal B", "ask", "oi");
        router
            .mailbox()
            .enqueue_as("Terminal A", &m)
            .expect("enqueue A");

        // 1º tick: B ainda não existe → NoTarget, mas a msg é RETIDA (não ackeada).
        let r1 = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(
            r1[0].1,
            RouteOutcome::NoTarget,
            "B ausente → NoTarget transiente"
        );
        assert!(rec.borrow().is_empty(), "nada entregue ainda");

        // B chega (registro tardio — a race).
        let b = sup.register("Terminal B", None, sink());

        // 2º tick: a msg RETIDA é re-processada e ENTREGA (sem o remetente re-enviar).
        let r2 = router.pump(&mut ts.store, 1001, &mut deliver);
        assert_eq!(
            r2[0].1,
            RouteOutcome::Delivered { targets: vec![b] },
            "o tick seguinte entrega à B recém-chegada — a msg não foi perdida"
        );
        assert_eq!(
            rec.borrow().len(),
            1,
            "entregue exatamente uma vez (sem duplicar)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **TETO do retry — destino REALMENTE inexistente NÃO retém para sempre.** Após
    /// `MAX_TRANSIENT_RETRIES` ticks sem o destino aparecer, a msg é finalmente ackeada (descartada);
    /// ticks posteriores não a re-processam (anti-loop). Sem isto, um typo de nome ficaria no `.inflight`
    /// e seria re-roteado eternamente.
    #[test]
    fn transient_retry_gives_up_after_cap() {
        let (mut router, sup, dir) = router_with("retry-cap");
        sup.register("Terminal A", None, sink());
        let mut ts = TmpStore::new("retry-cap");
        let (_rec, mut deliver) = recorder();

        let m = MailMessage::new("", "@Fantasma", "ask", "oi"); // destino que nunca existe
        router
            .mailbox()
            .enqueue_as("Terminal A", &m)
            .expect("enqueue A");

        // Bombeia além do teto: cada tick re-tenta enquanto < teto; no teto, ackeia.
        for t in 0..(MAX_TRANSIENT_RETRIES + 3) {
            router.pump(&mut ts.store, 1000 + u64::from(t), &mut deliver);
        }
        // A msg parou de ser re-processada: o nº de RouteBlocked{no_target} é LIMITADO pelo teto,
        // não cresce sem fim. (Sem o teto, seriam `MAX+3`.)
        let blocks = blocked_reasons(&ts.store)
            .into_iter()
            .filter(|r| r == "no_target")
            .count();
        assert!(
            blocks as u32 <= MAX_TRANSIENT_RETRIES,
            "as re-tentativas param no teto ({MAX_TRANSIENT_RETRIES}); vieram {blocks}"
        );
        // Após desistir, um novo tick não tem mais nada a processar (inflight vazio).
        let extra = router.pump(&mut ts.store, 9999, &mut deliver);
        assert!(extra.is_empty(), "msg já descartada — nada a re-rotear");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_autonomy_blocks_delegation() {
        let dir = std::env::temp_dir().join(format!("lina-router-man-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sup = Arc::new(Supervisor::new());
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let cfg = RouterConfig {
            autonomy: AutonomyLevel::Manual,
            ..RouterConfig::default()
        };
        let mut router =
            Router::with_config(Arc::clone(&sup), Mailbox::new(dir.join(".lina")), cfg);
        let mut ts = TmpStore::new("manual");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "@B", "ask", "oi");
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::BlockedByAutonomy
        );
        assert!(rec.borrow().is_empty());
        assert!(blocked_reasons(&ts.store).contains(&"blocked_by_autonomy".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn await_into_a_waiting_target_is_a_deadlock() {
        let (mut router, sup, dir) = router_with("deadlock");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("deadlock");
        let (_rec, mut deliver) = recorder();

        // B já espera A (B--await-->A). Agora A--await-->B fecharia o ciclo.
        sup.begin_await(b, a).expect("B espera A");
        let msg = MailMessage::new("@A", "@B", "ask", "oi").awaiting();
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Deadlock
        );
        assert!(blocked_reasons(&ts.store).contains(&"deadlock".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **fanout-fix (a) — o BUG do fundador:** um broadcast de ORIGEM-USUÁRIO (o agente foi pedido
    /// pelo humano e NÃO recebeu nenhuma A2A → sem binding → `hops==0`) para N > `FANOUT_GATE` nós
    /// vivos ENTREGA a TODOS, sem gate humano e com ZERO `RouteBlocked` (igual o Maestri faz). É a 1ª
    /// onda legítima; a tempestade mora na CASCATA (re-espalhamento), coberta pelo teste (b).
    #[test]
    fn user_origin_broadcast_fans_out_to_all_without_gate() {
        let (mut router, sup, dir) = router_with("fanout-origin");
        let _a = sup.register("@A", None, sink());
        // 5 colegas vivos (> FANOUT_GATE=3).
        for n in ["@B1", "@B2", "@B3", "@B4", "@B5"] {
            sup.register(n, None, sink());
        }
        let mut ts = TmpStore::new("fanout-origin");
        let (rec, mut deliver) = recorder();

        // @A nunca recebeu A2A → ORIGEM (hops==0): a 1ª onda pedida pelo humano.
        let msg = MailMessage::new("@A", "*", "broadcast", "oi a todos");
        match router.route_message(&msg, &mut ts.store, 1000, &mut deliver) {
            RouteOutcome::Delivered { targets } => {
                assert_eq!(
                    targets.len(),
                    5,
                    "fan-out inicial entrega a TODOS os 5 vivos"
                );
            }
            other => panic!("origem deve entregar a todos (sem gate), veio {other:?}"),
        }
        assert_eq!(rec.borrow().len(), 5, "5 entregas REAIS");
        assert!(
            blocked_reasons(&ts.store).is_empty(),
            "ZERO RouteBlocked: o fan-out inicial não é barrado pelo FANOUT_GATE"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **fanout-fix (b):** broadcast em CASCATA (o sender RECEBEU uma entrega → ganhou binding →
    /// `hops>=1`, está RE-ESPALHANDO) para > `FANOUT_GATE` alvos AINDA é GATEADO (pede confirmação
    /// humana, não entrega) — a tempestade mora aqui. (Antes, este teste cobria o broadcast de origem;
    /// o invariante mudou: origem LIBERA — teste (a) —, cascata MANTÉM o freio.)
    #[test]
    fn cascade_broadcast_over_fanout_gate_still_gated() {
        let (mut router, sup, dir) = router_with("fanout-cascade");
        let _a = sup.register("@A", None, sink());
        let _relay = sup.register("@Relay", None, sink());
        for n in ["@B1", "@B2", "@B3", "@B4"] {
            sup.register(n, None, sink());
        }
        let mut ts = TmpStore::new("fanout-cascade");
        let (rec, mut deliver) = recorder();

        // A→Relay: Relay RECEBE uma entrega → ganha binding → suas mensagens viram CASCATA (hops>=1).
        let seed = MailMessage::new("@A", "@Relay", "ask", "espalha isto");
        assert!(matches!(
            router.route_message(&seed, &mut ts.store, 1, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let delivered_before = rec.borrow().len(); // 1 (a entrega do seed)

        // Relay re-broadcasta (CASCATA) para > FANOUT_GATE alvos → GATEADO (não entrega).
        let bc = MailMessage::new("@Relay", "*", "broadcast", "todos!");
        let out = router.route_message(&bc, &mut ts.store, 2, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::FanoutGated { .. }),
            "broadcast em CASCATA > gate segue gateado: {out:?}"
        );
        assert_eq!(
            rec.borrow().len(),
            delivered_before,
            "broadcast cascata gateado NÃO entrega"
        );
        assert!(blocked_reasons(&ts.store).contains(&"fanout_gated".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **fanout-fix (d):** delegações de ORIGEM (cada `lina ask` do agente pedido pelo humano é sua
    /// própria raiz — `hops==0`, root fresco por msg via W1/A2) NÃO são limitadas pelo
    /// `DELEGATION_BUDGET` — o humano pode mandar > BUDGET avisos 1-a-1. (A CASCATA segue limitada —
    /// ver `delegation_budget_is_enforced_per_root_cause`.)
    #[test]
    fn origin_delegations_are_not_limited_by_budget() {
        let (mut router, sup, dir) = router_with("budget-origin");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("budget-origin");
        let (_rec, mut deliver) = recorder();

        // @A nunca recebe A2A → cada ask é ORIGEM (hops==0, root fresco). Manda BUDGET+5 asks 1-a-1.
        for i in 0..(DELEGATION_BUDGET + 5) {
            turn_done(&sup, b); // F1-0-4: B termina cada turno (testa o ORÇAMENTO, não a retenção)
            let m = MailMessage::new("@A", "@B", "ask", format!("aviso {i}"));
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, 1000 + u64::from(i), &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "aviso de ORIGEM {i} deve entregar — origem não é limitada por orçamento"
            );
        }
        assert!(
            !blocked_reasons(&ts.store).contains(&"budget_exceeded".to_string()),
            "nenhum budget_exceeded num fan-out 1-a-1 de origem"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pump_drains_mailbox_and_routes() {
        let (mut router, sup, dir) = router_with("pump");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("pump");
        let (rec, deliver) = recorder();

        // Round 6: injeção pelo outbox POR-NÓ (autenticado por origem), como o bin real faz — o flat
        // agora é anônimo (A3-flat) e seria recusado como UnknownSender.
        router
            .mailbox()
            .enqueue_as("@A", &MailMessage::new("@A", "@B", "ask", "oi"))
            .expect("enqueue por-no");
        let results = router.pump(&mut ts.store, 1000, deliver);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].1, RouteOutcome::Delivered { .. }));
        assert_eq!(rec.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────────── W3-5: plano compartilhado ─────────────────────────────

    /// Router + store novos com um plano semeado (workspace + T1/T2 + uma decisão), tudo via log.
    fn seeded_plan_router(tag: &str) -> (Router, std::path::PathBuf, TmpStore) {
        let (mut router, sup, dir) = router_with(tag);
        // Round 6 (F1): o plano agora exige origem AUTENTICADA — o guard de remetente (`node_by_name`)
        // foi elevado p/ ANTES do desvio de intent de plano. Registra os nós que os testes de plano
        // usam como remetentes legítimos (@A: claim/check; @B: conflito/não-owner). Sem isto seriam
        // `UnknownSender` — exatamente como uma origem forjada/flat passa a ser recusada.
        sup.register("@A", None, sink());
        sup.register("@B", None, sink());
        let mut ts = TmpStore::new(tag);
        ts.store
            .append(&DomainEvent::WorkspaceCreated {
                name: "App X".into(),
                focus_preset: String::new(),
            })
            .expect("workspace");
        router
            .seed_plan_item(&mut ts.store, "T1", "desenhar schema")
            .expect("seed T1");
        router
            .seed_plan_item(&mut ts.store, "T2", "implementar api")
            .expect("seed T2");
        router
            .seed_plan_decision(&mut ts.store, "Backend usa Postgres")
            .expect("seed decisao");
        (router, dir, ts)
    }

    /// Aceite W3-5 (b): `plan claim T1` por @A → o `plan.md` mostra T1 `@owner:@A status:doing` E o
    /// evento `plan.claim` no log carrega `id`/`item`/`by` no MESMO registro. NÃO entrega a PTY.
    #[test]
    fn plan_claim_applies_to_file_and_logs_event_fields() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-claim");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1");
        let out = router.route_message(&msg, &mut ts.store, 1000, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::PlanApplied {
                intent: "plan.claim".into(),
                item: "T1".into()
            }
        );
        assert!(rec.borrow().is_empty(), "intent de plano NAO entrega a PTY");

        // (i) projeção em disco.
        let live = router
            .mailbox()
            .read_plan()
            .expect("read")
            .expect("plan.md existe");
        assert!(
            live.contains("- [~] T1 :: desenhar schema :: @owner:@A :: status:doing"),
            "plan.md deveria mostrar T1 doing/@A; veio:\n{live}"
        );

        // (ii) campos no MESMO registro do log.
        let recs = ts.store.events().expect("events");
        let claim = recs
            .iter()
            .find(|r| r.kind == "PlanClaimed")
            .expect("evento PlanClaimed no log");
        assert_eq!(
            claim.payload["id"].as_str(),
            Some(msg.id.as_str()),
            "id da msg"
        );
        assert_eq!(claim.payload["item"].as_str(), Some("T1"), "ref do item");
        assert_eq!(claim.payload["by"].as_str(), Some("@A"), "quem reivindicou");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Aceite W3-5 (c): claim de item já ownereado por OUTRO → rejeitado, sem corromper o plano.
    #[test]
    fn plan_claim_conflict_is_rejected_without_corruption() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-conflict");
        let (_rec, mut deliver) = recorder();

        let a = MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1");
        assert!(matches!(
            router.route_message(&a, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::PlanApplied { .. }
        ));
        let b = MailMessage::new("@B", "plan", "plan.claim", "").with_ref("plan:T1");
        assert!(matches!(
            router.route_message(&b, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::PlanRejected(_)
        ));

        let live = router.mailbox().read_plan().unwrap().unwrap();
        assert!(
            live.contains("@owner:@A :: status:doing"),
            "T1 segue com @A"
        );
        assert!(!live.contains("@owner:@B"), "@B nao deve ter assumido nada");
        let claims = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "PlanClaimed")
            .count();
        assert_eq!(claims, 1, "o claim conflitante nao deve gerar evento");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────── F1-3-4 pendência (item 8 do conselho F1-3): anti-race do claim, EXERCITADO ─────────

    /// **Anti-race do claim pelo FUNIL REAL dos workers** (pendência do gate F1-3, item 8):
    /// dois nós com leitura VELHA do plano (ambos viram `T1` livre) depositam `plan.claim`
    /// do MESMO item nas suas outboxes ANTES de qualquer processamento — a janela de race
    /// genuína do `lina plan claim`. O `pump` drena os dois na mesma rodada. O mecanismo
    /// provado: a validação acontece na APLICAÇÃO (`try_claim` contra a projeção ATUAL do
    /// log), nunca na leitura — exatamente UM vence, o perdedor recebe rejeição estruturada
    /// nomeando o item, o `plan.md` fica com UM dono e o REPLAY do log reconstrói estado
    /// idêntico (inv#4 — zero corrupção). Auditável no log: o `PlanClaimed` do vencedor
    /// (o conflito em si não gera evento — design consagrado no W3-5 (c), teste acima).
    /// Não-vacuoso: remover a guarda de owner do `try_claim` faz `(applied, rejected)`
    /// virar `(2, 0)` e duplica `PlanClaimed` — este teste falha (mutação provada na entrega).
    #[test]
    fn claim_race_via_mailbox_one_winner_loser_rejected_replay_consistent() {
        let (mut router, dir, mut ts) = seeded_plan_router("race-claim");
        let (_rec, deliver) = recorder();

        // Leitura-velha genuína: as DUAS mensagens entram na fila antes do 1º processamento.
        router
            .mailbox()
            .enqueue_as(
                "@A",
                &MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1"),
            )
            .expect("enqueue claim de @A");
        router
            .mailbox()
            .enqueue_as(
                "@B",
                &MailMessage::new("@B", "plan", "plan.claim", "").with_ref("plan:T1"),
            )
            .expect("enqueue claim de @B");

        let results = router.pump(&mut ts.store, 1_000, deliver);
        assert_eq!(results.len(), 2, "o pump drenou os dois claims da janela");
        let applied = results
            .iter()
            .filter(|(_, o)| matches!(o, RouteOutcome::PlanApplied { .. }))
            .count();
        let rejected: Vec<&str> = results
            .iter()
            .filter_map(|(_, o)| match o {
                RouteOutcome::PlanRejected(r) => Some(r.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            (applied, rejected.len()),
            (1, 1),
            "exatamente UM vence e UM é rejeitado: {results:?}"
        );
        assert!(
            rejected[0].contains("T1"),
            "a rejeição é estruturada e nomeia o item disputado: {}",
            rejected[0]
        );

        // Auditável no log: exatamente 1 `PlanClaimed`; o vencedor é o `by` do evento.
        let claims: Vec<serde_json::Value> = ts
            .store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "PlanClaimed")
            .map(|r| r.payload)
            .collect();
        assert_eq!(claims.len(), 1, "um único PlanClaimed auditável");
        let winner = claims[0]["by"].as_str().expect("by no payload").to_string();

        // plan.md em disco: o item tem UM dono (o vencedor); o perdedor não aparece.
        let loser = if winner == "@A" { "@B" } else { "@A" };
        let live = router.mailbox().read_plan().expect("ler").expect("plan.md");
        assert!(
            live.contains(&format!("@owner:{winner} :: status:doing")),
            "T1 pertence ao vencedor {winner}: {live}"
        );
        assert!(
            !live.contains(&format!("@owner:{loser}")),
            "o perdedor {loser} não assumiu nada: {live}"
        );

        // Inv#4: o replay do log reconstrói o MESMO plano — o conflito não sujou o estado.
        let projected = ts.store.project().expect("project").plan.render();
        assert_eq!(projected, live, "replay ≡ plan.md em disco");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A posse é STICKY** (complemento da race acima): o perdedor re-tenta DEPOIS de ver a
    /// derrota (nova mensagem, novo id — não é dedupe) e segue rejeitado; o dono não muda e
    /// nenhum `PlanClaimed` novo entra no log. A rejeição do conflito não é transiente: não
    /// existe caminho em que insistência roube o item sem intervenção externa.
    #[test]
    fn claim_loser_retry_stays_rejected_and_owner_sticky() {
        let (mut router, dir, mut ts) = seeded_plan_router("race-sticky");
        let (_rec, mut deliver) = recorder();

        let first = MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1");
        assert!(matches!(
            router.route_message(&first, &mut ts.store, 1_000, &mut deliver),
            RouteOutcome::PlanApplied { .. }
        ));

        // Duas insistências do perdedor (ids novos a cada tentativa) — ambas rejeitadas.
        for ts_ms in [2_000, 3_000] {
            let retry = MailMessage::new("@B", "plan", "plan.claim", "").with_ref("plan:T1");
            assert!(
                matches!(
                    router.route_message(&retry, &mut ts.store, ts_ms, &mut deliver),
                    RouteOutcome::PlanRejected(_)
                ),
                "insistência do perdedor em t={ts_ms} segue rejeitada"
            );
        }

        let claims = ts
            .store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "PlanClaimed")
            .count();
        assert_eq!(claims, 1, "insistência não gera evento de claim");
        let live = router.mailbox().read_plan().expect("ler").expect("plan.md");
        assert!(
            live.contains("@owner:@A :: status:doing"),
            "o dono original segue intacto: {live}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Aceite W3-5 (d): `check` pelo owner → `done` + evento `plan.check`; `check` por não-owner →
    /// rejeitado (sem mudar o status).
    #[test]
    fn plan_check_requires_owner_and_logs() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-check");
        let (_rec, mut deliver) = recorder();

        let claim = MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1");
        router.route_message(&claim, &mut ts.store, 1000, &mut deliver);

        // não-owner @B não conclui.
        let bad = MailMessage::new("@B", "plan", "plan.check", "").with_ref("plan:T1");
        assert!(matches!(
            router.route_message(&bad, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::PlanRejected(_)
        ));
        assert!(
            router
                .mailbox()
                .read_plan()
                .unwrap()
                .unwrap()
                .contains("@owner:@A :: status:doing"),
            "check de nao-owner nao muda o status"
        );

        // owner @A conclui.
        let good = MailMessage::new("@A", "plan", "plan.check", "").with_ref("plan:T1");
        assert!(matches!(
            router.route_message(&good, &mut ts.store, 1002, &mut deliver),
            RouteOutcome::PlanApplied { .. }
        ));
        let live = router.mailbox().read_plan().unwrap().unwrap();
        assert!(
            live.contains("- [x] T1 :: desenhar schema :: @owner:@A :: status:done"),
            "{live}"
        );
        let checks = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "PlanChecked")
            .count();
        assert_eq!(checks, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan intent sem `ref` → rejeitado limpo (sem panic, sem corromper).
    #[test]
    fn plan_intent_without_ref_is_rejected() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-noref");
        let (_rec, mut deliver) = recorder();
        let msg = MailMessage::new("@A", "plan", "plan.claim", ""); // sem with_ref
        assert!(matches!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::PlanRejected(_)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Aceite W3-5 (e): replay do log reconstrói o `plan.md` IDÊNTICO (reabre o store do mesmo log).
    #[test]
    fn replay_reconstructs_plan_md_identically() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-replay");
        let (_rec, mut deliver) = recorder();

        for (from, intent, item, t) in [
            ("@A", "plan.claim", "plan:T1", 1000u64),
            ("@A", "plan.check", "plan:T1", 1001u64),
            ("@B", "plan.claim", "plan:T2", 1002u64),
        ] {
            let msg = MailMessage::new(from, "plan", intent, "").with_ref(item);
            assert!(
                matches!(
                    router.route_message(&msg, &mut ts.store, t, &mut deliver),
                    RouteOutcome::PlanApplied { .. }
                ),
                "{intent} {item} deveria aplicar"
            );
        }

        let live = router
            .mailbox()
            .read_plan()
            .expect("read")
            .expect("plan.md existe");

        // Reconstrói reabrindo o store do MESMO log em disco → projeta → renderiza.
        let reopened = EventStore::open(&ts.dir).expect("reabrir store");
        let reconstructed = reopened.project().expect("project").plan.render();

        assert_eq!(
            live, reconstructed,
            "replay do log deve reconstruir o plan.md identico"
        );
        assert!(live.contains("- [x] T1 :: desenhar schema :: @owner:@A :: status:done"));
        assert!(live.contains("- [~] T2 :: implementar api :: @owner:@B :: status:doing"));
        assert!(live.contains("- Backend usa Postgres"));
        assert!(
            live.starts_with("# Plano — App X\n"),
            "cabecalho do workspace"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 6 (F1): `plan.claim` com `from` de nó NÃO registrado (forjado) é RECUSADO como
    /// `UnknownSender` ANTES de tocar o plano — o desvio de intent de plano agora passa pelo guard de
    /// origem (elevado p/ antes do desvio). Antes do fix, aplicaria (impersonação de origem fantasma).
    #[test]
    fn plan_claim_from_unknown_sender_is_rejected() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-unknown-sender");
        let (_rec, mut deliver) = recorder();
        // @Vitima NÃO está no roster (só @A/@B registrados em seeded_plan_router).
        let forged = MailMessage::new("@Vitima", "plan", "plan.claim", "").with_ref("plan:T1");
        let out = router.route_message(&forged, &mut ts.store, 1000, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::UnknownSender(_)),
            "origem forjada/desconhecida deve ser recusada; veio {out:?}"
        );
        let live = router.mailbox().read_plan().unwrap().unwrap();
        assert!(
            live.contains("- [ ] T1 :: desenhar schema :: @owner:? :: status:todo"),
            "T1 deve seguir livre (nada aplicado); veio:\n{live}"
        );
        let claims = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "PlanClaimed")
            .count();
        assert_eq!(claims, 0, "origem forjada nao gera PlanClaimed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 6 (F1 + A3-flat, ponta-a-ponta): um `plan.claim` escrito no outbox FLAT com `from=@A`
    /// (peer REAL) NÃO impersona — o drain anonimiza o `from` e o router o recusa (UnknownSender). É o
    /// invariante central: nenhum campo do outbox NÃO-autenticado decide a cadeia do plano.
    #[test]
    fn flat_plan_claim_cannot_impersonate_real_peer() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-flat-impersonate");
        // @A É um peer real (registrado). Mesmo assim, pela via FLAT não pode reivindicar em nome dele.
        let forged = MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1");
        router.mailbox().enqueue(&forged).expect("enqueue flat");
        let (_rec, mut deliver) = recorder();
        let results = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(results.len(), 1, "uma msg drenada");
        assert!(
            matches!(results[0].1, RouteOutcome::UnknownSender(_)),
            "FLAT anonimo deve ser recusado; veio {:?}",
            results[0].1
        );
        let live = router.mailbox().read_plan().unwrap().unwrap();
        assert!(
            live.contains("- [ ] T1 :: desenhar schema :: @owner:? :: status:todo"),
            "T1 intacto (sem impersonacao); veio:\n{live}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 6 (contraponto): o MESMO `plan.claim`, pela via POR-NÓ (origem carimbada=@A), APLICA — e o
    /// campo `from` forjado do arquivo é IGNORADO em favor da origem autenticada (dir-dono).
    #[test]
    fn per_node_plan_claim_applies_with_authenticated_origin() {
        let (mut router, dir, mut ts) = seeded_plan_router("plan-pernode-apply");
        // O campo `from` da msg é uma isca forjada; o por-nó o sobrescreve por "@A" (dir-dono).
        let claim =
            MailMessage::new("@ForjadoIgnorado", "plan", "plan.claim", "").with_ref("plan:T1");
        router
            .mailbox()
            .enqueue_as("@A", &claim)
            .expect("enqueue por-no");
        let (_rec, mut deliver) = recorder();
        let results = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].1, RouteOutcome::PlanApplied { .. }),
            "por-no autenticado deve aplicar; veio {:?}",
            results[0].1
        );
        let live = router.mailbox().read_plan().unwrap().unwrap();
        assert!(
            live.contains("- [~] T1 :: desenhar schema :: @owner:@A :: status:doing"),
            "T1 reivindicado por @A (origem autenticada, nao @ForjadoIgnorado); veio:\n{live}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A5: o `budget` é podado na mesma janela do `seen` — entradas velhas somem (sem vazamento).
    #[test]
    fn budget_entries_are_pruned_after_window() {
        let (mut router, sup, dir) = router_with("budget-prune");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("budget-prune");
        let (_rec, mut deliver) = recorder();

        // Uma delegação cria uma entrada de budget (carimbada em t=1000).
        let msg = MailMessage::new("@A", "@B", "ask", "oi");
        assert!(matches!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(router.budget.len(), 1, "delegacao cria entrada de budget");

        // Bem depois da janela, qualquer route_message roda a poda no topo → a entrada velha some;
        // só a nova (do turno recente) permanece.
        turn_done(&sup, b); // F1-0-4: B terminou o turno da 1ª entrega
        let later = MailMessage::new("@A", "@B", "ask", "depois");
        assert!(matches!(
            router.route_message(&later, &mut ts.store, DEDUPE_WINDOW_MS + 2000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(
            router.budget.len(),
            1,
            "a entrada velha foi podada; não acumula sem limite"
        );
        assert!(
            router.budget.keys().all(|(root, _)| *root == later.id),
            "só a entrada do turno recente sobrevive (a de 1000 foi podada)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────── W3-7a: anti-loop de grafo / propagação / A6 ─────────────────────────

    /// W1 + §2.1: ping-pong A→B→A→B… por mensagens NOVAS, CADA uma com `root_cause_id` FORJADO DISTINTO
    /// e hops=0 (o vetor que ANTES burlava o anti-loop) → o forjado é IGNORADO, o root vem do BINDING,
    /// o grafo amarra a cadeia sob o MESMO root real e o loop apertado é cortado no limite. Se a forja
    /// FUNCIONASSE, cada msg viraria sua própria partição (root fresco) → nunca um ciclo → nunca
    /// bloqueio: o corte no limite PROVA que o root forjado não deu voltas extras. (Após o ajuste de
    /// cooperação, a 1ª volta A→B→A ENTREGA — diálogo; só a repetição além de `MAX_CYCLE_REVISITS` corta.)
    #[test]
    fn forged_root_does_not_bypass_loop_detection() {
        let (mut router, sup, dir) = router_with("loop");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("loop");
        let (_rec, mut deliver) = recorder();

        // As voltas dentro do limite entregam — cada uma com um root FORJADO DISTINTO (todos IGNORADOS;
        // o root real vem do binding, então o grafo NÃO fragmenta e o ciclo é amarrado).
        let pingpong = [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")];
        for (i, (from, to)) in pingpong.iter().enumerate() {
            // F1-0-4: cada perna só acontece após o turno anterior terminar (ping-pong real).
            turn_done(&sup, a);
            turn_done(&sup, b);
            let mut m = MailMessage::new(*from, *to, "ask", "volta");
            m.root_cause_id = Some(format!("forjado{i}")); // distinto e IGNORADO
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, 1000 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "volta {i} é diálogo (entrega) — forja de root não muda nada"
            );
        }
        // 3ª repetição do salto A→B (root forjado mais uma vez) → o loop apertado é cortado mesmo assim.
        turn_done(&sup, a);
        turn_done(&sup, b); // F1-0-4: o corte abaixo deve vir do ANTI-LOOP, não da retenção
        let mut over = MailMessage::new("@A", "@B", "ask", "volta demais");
        over.root_cause_id = Some("forjado_final".into());
        assert_eq!(
            router.route_message(&over, &mut ts.store, 1010, &mut deliver),
            RouteOutcome::LoopDetected,
            "root forjado não escapa do anti-loop: o limite ainda corta"
        );
        assert!(blocked_reasons(&ts.store).contains(&"loop_detected".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **DIFERENCIAL #1 (diálogo): a RESPOSTA chega.** A→B (root R) entrega a IDA; B→A (mesmo root R,
    /// `lina ask` COMUM — sem `--await`/`reply_to`) ENTREGA a VOLTA. A 1ª volta do ida-e-volta NÃO é
    /// mais tratada como loop (antes: `LoopDetected` já no 1º retorno → cooperação só num sentido). O
    /// anti-loop por grafo passou a cortar só o ping-pong REPETIDO (ver `tight_loop_blocked_…`).
    #[test]
    fn dialogue_reply_delivers_not_loop_detected() {
        let (mut router, sup, dir) = router_with("dialogue");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("dialogue");
        let (rec, mut deliver) = recorder();

        let ida = MailMessage::new("@A", "@B", "ask", "oi");
        assert_eq!(
            router.route_message(&ida, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { targets: vec![b] },
            "A→B entrega (ida)"
        );
        // B responde A — `lina ask` comum: herda o root R de A pelo binding (root=None, sem reply_to).
        let volta = MailMessage::new("@B", "@A", "ask", "oi de volta");
        assert_eq!(
            router.route_message(&volta, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::Delivered { targets: vec![a] },
            "B→A (mesmo root R, resposta) ENTREGA — NÃO LoopDetected na 1ª volta"
        );
        assert_eq!(
            rec.borrow().len(),
            2,
            "as DUAS pernas do diálogo entregaram"
        );
        assert!(
            !blocked_reasons(&ts.store).contains(&"loop_detected".to_string()),
            "nenhum loop_detected na 1ª volta do diálogo"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Loop APERTADO ainda barrado.** Ping-pong A→B→A→B… sob o mesmo root: as primeiras voltas são
    /// diálogo e entregam; quando o MESMO salto (A→B) se repete pela `MAX_CYCLE_REVISITS + 1`-ésima vez
    /// fechando o ciclo, é cortado (`LoopDetected` + `RouteBlocked{loop_detected}`). O loop infinito
    /// segue barrado (e, no limite, `hops`/orçamento são o backstop final).
    #[test]
    fn tight_loop_blocked_after_revisit_limit() {
        let (mut router, sup, dir) = router_with("tightloop");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("tightloop");
        let (_rec, mut deliver) = recorder();

        // As voltas dentro do limite entregam (diálogo). Com MAX_CYCLE_REVISITS=2, o salto A→B pode
        // ocorrer 2× e B→A 2× antes do corte.
        let pingpong = [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")];
        for (i, (from, to)) in pingpong.iter().enumerate() {
            // F1-0-4: cada perna só acontece após o turno anterior terminar (ping-pong real).
            turn_done(&sup, a);
            turn_done(&sup, b);
            let m = MailMessage::new(*from, *to, "ask", "ping");
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, 1000 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "volta {i} ({from}→{to}) ainda é diálogo (entrega)"
            );
        }
        // 3ª repetição do salto A→B fechando o ciclo → loop APERTADO cortado.
        let over = MailMessage::new("@A", "@B", "ask", "ping demais");
        assert_eq!(
            router.route_message(&over, &mut ts.store, 1010, &mut deliver),
            RouteOutcome::LoopDetected,
            "o ping-pong repetido além de MAX_CYCLE_REVISITS é cortado"
        );
        assert!(blocked_reasons(&ts.store).contains(&"loop_detected".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ═══════════════════ F3-CONF-2 · protocolo ANTI-ENGOLIMENTO (gates a/b/c) ═══════════════════

    const STALL_WINDOW: u64 = crate::attention::DELIVERED_NO_PROGRESS_WINDOW_MS;

    /// Cena base: @Maestro + @B vivos, uma Goal `g` com o item `T1` ATRIBUÍDO e reivindicado por @B
    /// (owner=@B). É o terreno goal-driven que o protocolo exige (a escala precisa de `goal_id`).
    fn seed_swallow_goal(
        tag: &str,
    ) -> (
        Router,
        Arc<Supervisor>,
        TmpStore,
        NodeId,
        std::path::PathBuf,
    ) {
        let (mut router, sup, dir) = router_with(tag);
        let _m = sup.register("@Maestro", None, sink());
        let b = sup.register("@B", None, sink());
        sup.set_status(b, NodeStatus::Idle).expect("B Idle");
        let mut ts = TmpStore::new(tag);
        ts.store
            .append(&DomainEvent::GoalDefined {
                goal_id: "g".into(),
                statement: "meta de teste".into(),
                root_cause_id: "r".into(),
                origin: "@Maestro".into(),
                budget_tokens: 0,
            })
            .unwrap();
        router
            .seed_plan_item(&mut ts.store, "T1", "tarefa")
            .unwrap();
        ts.store
            .append(&DomainEvent::PlanItemAttributed {
                item: "T1".into(),
                goal_id: Some("g".into()),
                parents: vec![],
                acceptance: vec![],
                budget_tokens: 0,
                paths: vec![],
            })
            .unwrap();
        let (_rec, mut deliver) = recorder();
        router.route_message(
            &MailMessage::new("@B", "plan", "plan.claim", "").with_ref("plan:T1"),
            &mut ts.store,
            1,
            &mut deliver,
        );
        (router, sup, ts, b, dir)
    }

    /// Caminho REAL: entrega um handoff @Maestro→@B em `now_ms` (arma o vigia no relógio do router)
    /// e marca o retorno a `Idle` SEM progresso (o lifecycle real faria — input ambiental do teste).
    /// O `now_ms` é o relógio LÓGICO do protocolo (o store carimba wall-clock, mas o vigia usa o
    /// `now_ms` da entrega — determinístico, sem `sleep`).
    fn deliver_and_idle(
        router: &mut Router,
        sup: &Supervisor,
        store: &mut EventStore,
        now_ms: u64,
    ) {
        let (_rec, mut deliver) = recorder();
        let b = sup.node_by_name("@B").expect("@B vivo");
        let m = MailMessage::new("@Maestro", "@B", "handoff", "assuma T1");
        let out = router.route_message(&m, store, now_ms, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "entrega real arma o vigia; veio {out:?}"
        );
        store
            .append(&DomainEvent::NodeStatusChanged {
                node: b,
                status: NodeStatus::Idle.as_str().to_string(),
                from: NodeStatus::Busy.as_str().to_string(),
                reason: "end_of_response".to_string(),
            })
            .unwrap();
        sup.set_status(b, NodeStatus::Idle).expect("idle roster");
    }
    /// Os PROMPTS dos `SpawnRequested` de re-despacho de stall (`stall-respawn:{node}:…`) no log.
    fn stall_respawns(store: &EventStore, node: NodeId) -> Vec<String> {
        let prefix = format!("{STALL_RESPAWN_PREFIX}:{node}:");
        store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "SpawnRequested")
            .filter(|r| field(r, "id").is_some_and(|id| id.starts_with(&prefix)))
            .filter_map(|r| field(&r, "prompt").map(String::from))
            .collect()
    }
    /// Os `reason` dos `GoalEscalated` no log.
    fn goal_escalations(store: &EventStore) -> Vec<String> {
        store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "GoalEscalated")
            .filter_map(|r| field(&r, "reason").map(String::from))
            .collect()
    }

    /// **Gate (a) WIRING — caminho real:** `route_message` entrega o handoff (arma o vigia no relógio
    /// do router), @B volta a `Idle` sem trabalhar e o `pump` (após a janela) emite UM `SpawnRequested`
    /// de re-despacho INFORMADO. Prova a costura `route_message`→arm→`pump`→enforce ponta-a-ponta.
    #[test]
    fn pump_redispatches_swallowed_dispatch_through_real_path() {
        let (mut router, sup, mut ts, b, dir) = seed_swallow_goal("pumpredisp");
        let (_rec, mut deliver) = recorder();

        deliver_and_idle(&mut router, &sup, &mut ts.store, 1_000);
        router.pump(&mut ts.store, 1_000 + STALL_WINDOW + 1, &mut deliver);

        let respawns = stall_respawns(&ts.store, b);
        assert_eq!(
            respawns.len(),
            1,
            "o pump CONSOME o engolido e re-despacha 1×"
        );
        assert!(
            respawns[0].contains("Não recebi nenhum sinal de progresso"),
            "re-despacho INFORMADO (prompt diz por que): {:?}",
            respawns[0]
        );
        assert!(
            goal_escalations(&ts.store).is_empty(),
            "1ª tentativa não escala"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Gate (a) — re-despacho 1× → escala + breaker sticky.** Epoch 1 engolido → re-despacho;
    /// epoch 2 (a re-entrega TAMBÉM engoliu) → `GoalEscalated{stalled}` + nó `Blocked`; a 3ª tentativa
    /// automática NÃO sai (sticky). Caminho real, epochs por `now_ms` LÓGICO distinto (determinístico).
    #[test]
    fn redispatch_then_escalate_with_sticky_breaker() {
        let (mut router, sup, mut ts, b, dir) = seed_swallow_goal("escalate");
        let (_rec, mut deliver) = recorder();

        // Epoch 1 (entrega @1_000): engolido → re-despacho, sem escala.
        deliver_and_idle(&mut router, &sup, &mut ts.store, 1_000);
        router.pump(&mut ts.store, 1_000 + STALL_WINDOW + 1, &mut deliver);
        assert_eq!(
            stall_respawns(&ts.store, b).len(),
            1,
            "epoch 1: re-despacho"
        );
        assert!(
            goal_escalations(&ts.store).is_empty(),
            "1ª tentativa NÃO escala"
        );

        // Epoch 2 (re-entrega @2_000+janela, epoch distinto) que TAMBÉM engole.
        let t2 = 2_000 + STALL_WINDOW;
        deliver_and_idle(&mut router, &sup, &mut ts.store, t2);
        router.pump(&mut ts.store, t2 + STALL_WINDOW + 1, &mut deliver);
        assert_eq!(
            goal_escalations(&ts.store),
            vec!["stalled".to_string()],
            "epoch 2 engoliu → escala ao humano com reason=stalled"
        );
        assert_eq!(
            stall_respawns(&ts.store, b).len(),
            1,
            "breaker sticky: NÃO há 2º re-despacho automático"
        );
        assert_eq!(
            sup.get(b).unwrap().status,
            NodeStatus::Blocked,
            "o breaker PAUSA o nó (tela honesta pinta 'pausado por segurança')"
        );

        // 3ª tentativa: com o nó Blocked pelo breaker, mais um tick NÃO produz novo re-despacho
        // (o nó pausado sai do vigia; só o reset humano o reabre). Sticky comprovado.
        router.pump(&mut ts.store, t2 + 3 * STALL_WINDOW, &mut deliver);
        assert_eq!(
            stall_respawns(&ts.store, b).len(),
            1,
            "breaker sticky impede a 3ª tentativa (só o humano libera)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Idempotência do GAP:** entre o re-despacho e a re-entrega, o MESMO epoch continua engolido.
    /// `pump` repetido NÃO re-despacha nem escala (espera a re-entrega) — sem isto, o protocolo
    /// escalaria prematuramente a cada tick durante o gap async. Caminho real (mesma entrega, 2 pumps).
    #[test]
    fn same_epoch_does_not_redispatch_or_escalate_in_gap() {
        let (mut router, sup, mut ts, b, dir) = seed_swallow_goal("gap");
        let (_rec, mut deliver) = recorder();

        deliver_and_idle(&mut router, &sup, &mut ts.store, 1_000);
        router.pump(&mut ts.store, 1_000 + STALL_WINDOW + 1, &mut deliver);
        assert_eq!(stall_respawns(&ts.store, b).len(), 1);
        // GAP: a re-entrega ainda não landou (MESMO epoch). Vários ticks depois → nada novo.
        router.pump(&mut ts.store, 1_000 + 5 * STALL_WINDOW, &mut deliver);
        assert_eq!(
            stall_respawns(&ts.store, b).len(),
            1,
            "mesmo epoch não re-despacha de novo (aguarda a re-entrega)"
        );
        assert!(
            goal_escalations(&ts.store).is_empty(),
            "NÃO escala no gap (a re-entrega do epoch 1 ainda não engoliu)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Gate (c) — reset sob gesto humano REINICIA o protocolo.** Após escalar (nó `Blocked`), o
    /// `reset_breaker` (chamado in-process pela UI) libera o nó e zera a contagem de stall: a entrega
    /// ainda engolida vira nova 1ª tentativa (re-despacho), NÃO re-escala de imediato. Idempotente.
    #[test]
    fn human_reset_unblocks_and_restarts_protocol() {
        let (mut router, sup, mut ts, b, dir) = seed_swallow_goal("reset");
        let (_rec, mut deliver) = recorder();

        deliver_and_idle(&mut router, &sup, &mut ts.store, 1_000);
        router.pump(&mut ts.store, 1_000 + STALL_WINDOW + 1, &mut deliver);
        let t2 = 2_000 + STALL_WINDOW;
        deliver_and_idle(&mut router, &sup, &mut ts.store, t2);
        router.pump(&mut ts.store, t2 + STALL_WINDOW + 1, &mut deliver);
        assert_eq!(sup.get(b).unwrap().status, NodeStatus::Blocked, "escalou");

        // RESET humano (gesto local in-process — nenhum agente o alcança).
        assert!(
            router.reset_breaker(b, &mut ts.store).unwrap(),
            "reset liberou o nó"
        );
        assert_eq!(
            sup.get(b).unwrap().status,
            NodeStatus::Idle,
            "o nó volta a ativo (tela honesta: deixa de 'pausado por segurança')"
        );
        assert!(
            !router.reset_breaker(b, &mut ts.store).unwrap(),
            "reset é idempotente (clique duplo não re-loga)"
        );

        // Pós-reset o protocolo RECOMEÇA: a entrega ainda engolida (epoch 2) vira RE-DESPACHO, não
        // re-escala. (`now_ms` ainda além da janela do epoch 2 → o vigia segue armado.)
        router.pump(&mut ts.store, t2 + 3 * STALL_WINDOW, &mut deliver);
        assert_eq!(
            goal_escalations(&ts.store).len(),
            1,
            "o gesto humano NÃO causa re-escala imediata da entrega antiga"
        );
        assert_eq!(
            stall_respawns(&ts.store, b).len(),
            2,
            "reset reinicia: o nó ganha uma nova 1ª tentativa"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Escopo v1 (goal-driven):** um handoff engolido a um nó SEM item de Goal não auto-re-despacha
    /// (a DETECÇÃO/alarme da UI cobre); o auto-protocolo exige a Goal que justifica o recurso (a escala
    /// precisa de `goal_id` auditável — ADR 0007). Trava a decisão de escopo (não é acidente).
    #[test]
    fn swallowed_non_goal_handoff_is_detection_only() {
        let (mut router, sup, dir) = router_with("nongoal");
        let _m = sup.register("@Maestro", None, sink());
        let b = sup.register("@B", None, sink());
        sup.set_status(b, NodeStatus::Idle).expect("B Idle");
        let mut ts = TmpStore::new("nongoal");
        let (_rec, mut deliver) = recorder();
        // SEM Goal/item: @B não possui nada no plano — mas a entrega é real e engole.
        deliver_and_idle(&mut router, &sup, &mut ts.store, 1_000);
        router.pump(&mut ts.store, 1_000 + STALL_WINDOW + 1, &mut deliver);
        assert!(
            stall_respawns(&ts.store, b).is_empty(),
            "handoff fora de Goal: sem auto-re-despacho (só a detecção cobre)"
        );
        assert!(goal_escalations(&ts.store).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Gate (b) — re-despacho a um worker ENGOLIDO é ISENTO do anti-loop** (#22c). MESMA topologia
    /// que BARRA a tempestade (`tight_loop_blocked_after_revisit_limit`: revisits≥2 + ciclo), mas o
    /// último despacho a @B foi ENGOLIDO (entregue, @B não respondeu) → @B tem vigia pendente → a
    /// repetição que FECHARIA o ciclo NÃO é `LoopDetected`: o Maestro corrige o worker preso.
    #[test]
    fn redispatch_to_swallowed_worker_is_exempt_from_loop_guard() {
        let (mut router, sup, dir) = router_with("loopexempt");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("loopexempt");
        let (_rec, mut deliver) = recorder();

        // A→B, B→A, A→B: estabelece revisits(A→B)=2 + a aresta B→A (fecha ciclo). O ÚLTIMO A→B é
        // entregue e ENGOLIDO (@B não responde) → o vigia de @B fica armado (entrega pendente).
        let legs = [("@A", "@B"), ("@B", "@A"), ("@A", "@B")];
        for (i, (from, to)) in legs.iter().enumerate() {
            turn_done(&sup, a);
            turn_done(&sup, b);
            let m = MailMessage::new(*from, *to, "ask", "ping");
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, 1000 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "perna {i} entrega (real)"
            );
        }
        // @B está ENGOLIDO. A repetição A→B fecharia o ciclo → o anti-loop CEGO barraria; a isenção
        // de engolido deixa passar (a correção legítima de um worker preso).
        turn_done(&sup, a);
        turn_done(&sup, b);
        let correction =
            MailMessage::new("@A", "@B", "handoff", "reassuma — sem sinal de progresso");
        let out = router.route_message(&correction, &mut ts.store, 1010, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "correção de worker engolido é ISENTA do anti-loop (não LoopDetected): {out:?}"
        );
        assert!(
            !blocked_reasons(&ts.store).contains(&"loop_detected".to_string()),
            "nenhum RouteBlocked{{loop_detected}} para a correção legítima"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A2: numa cadeia A→B→C com `root=None`, o supervisor PROPAGA o root (binding) e `hops` cresce
    /// 0→1 — observável nos `MessageRouted` do log (antes ambos nasciam root fresco / hops 0).
    #[test]
    fn root_and_hops_propagate_along_chain() {
        let (mut router, sup, dir) = router_with("propagate");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("propagate");
        let (_rec, mut deliver) = recorder();

        // A→B: A é origem (sem binding) → root = id da própria msg, hops 0.
        let m1 = MailMessage::new("@A", "@B", "ask", "faz X");
        assert!(matches!(
            router.route_message(&m1, &mut ts.store, 1, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        // B→C: B submete com root=None → herda o root de A (binding) e hops = 0+1 = 1.
        let m2 = MailMessage::new("@B", "@C", "ask", "faz Y");
        assert!(matches!(
            router.route_message(&m2, &mut ts.store, 2, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        let routed: Vec<_> = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "MessageRouted")
            .collect();
        assert_eq!(routed.len(), 2);
        let root0 = routed[0].payload["root_cause_id"]
            .as_str()
            .unwrap()
            .to_string();
        let root1 = routed[1].payload["root_cause_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(root0, m1.id, "a raiz é o turno de origem (id de A)");
        assert_eq!(root1, root0, "B herdou o root de A (propagação)");
        assert_eq!(routed[0].payload["hops"].as_u64(), Some(0));
        assert_eq!(
            routed[1].payload["hops"].as_u64(),
            Some(1),
            "hops cresceu 0→1 na cadeia propagada"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A2/AC: a propagação faz `hops` crescer até `max_depth` e o guard de profundidade CORTA a
    /// cadeia real (antes inerte, pois `msg.hops` ficava sempre 0).
    #[test]
    fn hop_limit_cuts_propagated_chain_at_max_depth() {
        let (mut router, sup, dir) = router_with("hopchain");
        for i in 0..=5 {
            sup.register(format!("@n{i}"), None, sink());
        }
        let mut ts = TmpStore::new("hopchain");
        let (_rec, mut deliver) = recorder();

        // Cadeia n0→n1→…→n5 (root=None, herdado): hops 0,1,2,3,4 — todas entregam.
        for i in 0..5u64 {
            let m = MailMessage::new(format!("@n{i}"), format!("@n{}", i + 1), "ask", "vai");
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, i + 1, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "hop {i} deveria entregar (hops <= max_depth)"
            );
        }
        // n5 está em hops=4; sua submissão deriva hops=5 > max_depth(4) → HopLimit (antes do alvo/loop).
        let over = MailMessage::new("@n5", "@n0", "ask", "estoura");
        assert_eq!(
            router.route_message(&over, &mut ts.store, 100, &mut deliver),
            RouteOutcome::HopLimit
        );
        assert!(blocked_reasons(&ts.store).contains(&"hop_limit".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A2: a propagação do root FAZ o orçamento acumular — um delegador repetido sob o root herdado
    /// estoura o teto (antes cada msg nova nascia com root fresco e a conta nunca batia).
    #[test]
    fn budget_accumulates_under_inherited_root() {
        let (mut router, sup, dir) = router_with("budget-inherit");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("budget-inherit");
        let (_rec, mut deliver) = recorder();

        // A→B estabelece o root do turno e o binding de B.
        let m0 = MailMessage::new("@A", "@B", "ask", "inicia");
        router.route_message(&m0, &mut ts.store, 1, &mut deliver);

        // B delega DELEGATION_BUDGET vezes (root herdado, root=None) → todas contam contra (root, B).
        for i in 0..DELEGATION_BUDGET {
            turn_done(&sup, c); // F1-0-4: C termina cada turno (testa o ORÇAMENTO, não a retenção)
            let m = MailMessage::new("@B", "@C", "ask", format!("d{i}"));
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, 10 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "delegação {i} de B (root herdado) deveria passar"
            );
        }
        // A (teto+1)-ésima de B, mesmo root herdado → estoura.
        turn_done(&sup, c);
        let over = MailMessage::new("@B", "@C", "ask", "demais");
        assert_eq!(
            router.route_message(&over, &mut ts.store, 999, &mut deliver),
            RouteOutcome::BudgetExceeded
        );
        assert!(blocked_reasons(&ts.store).contains(&"budget_exceeded".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-A6: `pump` é recuperável a crash — a mensagem drenada sobrevive em `.inflight` até o evento
    /// durável; um órfão (crash entre drenar e persistir) é REPROCESSADO; após persistir, `.inflight`
    /// fica vazio (confirmado).
    #[test]
    fn pump_recovers_inflight_after_crash() {
        let (mut router, sup, dir) = router_with("a6");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("a6");
        let (rec, deliver) = recorder();

        let m = MailMessage::new("@A", "@B", "ask", "oi");
        // Round 6: por-nó (autenticado) — o flat agora é anônimo e seria recusado no pump.
        router
            .mailbox()
            .enqueue_as("@A", &m)
            .expect("enqueue por-no");

        // "Crash": drena para .inflight mas NÃO roteia/persiste (não chamamos pump).
        let drained = router.mailbox().drain_to_inflight().expect("drain");
        assert_eq!(drained.len(), 1, "drenou (não-destrutivo) para .inflight");
        let inflight = router
            .mailbox()
            .inflight_dir()
            .join(format!("{}.json", m.id));
        assert!(
            inflight.exists(),
            "a mensagem sobrevive em .inflight (recuperável, não deletada)"
        );

        // Reabre: pump reprocessa o órfão, persiste MessageRouted e CONFIRMA (remove de .inflight).
        let results = router.pump(&mut ts.store, 1000, deliver);
        assert_eq!(results.len(), 1, "órfão reprocessado");
        assert!(matches!(results[0].1, RouteOutcome::Delivered { .. }));
        assert_eq!(rec.borrow().len(), 1, "entregue uma vez");
        assert!(
            !inflight.exists(),
            "após persistir, .inflight foi confirmado (vazio)"
        );
        assert!(
            ts.store
                .events()
                .unwrap()
                .iter()
                .any(|r| r.kind == "MessageRouted"),
            "o roteamento ficou durável no log"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────── F1-0-4: entrega ciente de estado (reter quando Busy) ─────────────────

    /// Eventos de um `kind` no log (helper das asserções de retenção).
    fn events_of(store: &EventStore, kind: &str) -> Vec<crate::events::EventRecord> {
        store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == kind)
            .collect()
    }

    /// **Calibração do teto de retenção (ADR 0020 §Calibração — achado MÉDIO do gate):**
    /// o default é da ordem de MINUTOS, nunca de segundos. Um turno real de claude leva
    /// minutos (retornos observados em 200–600 s na observação de 14h); um teto de
    /// segundos dead-letteraria espera LEGÍTIMA ("entrega quando ficar livre" é o
    /// produto). Trava anti-drift: cobrir o teto observado (600 s) e ficar MUITO acima
    /// dos turnos triviais (3–16 s medidos), mantendo-se finito (anti-deadlock).
    #[test]
    fn retention_default_e_da_ordem_de_minutos() {
        let d = RouterConfig::default().retention_timeout_ms;
        assert_eq!(d, RETENTION_TIMEOUT_MS);
        assert!(
            d >= 600_000,
            "default ({d} ms) deve cobrir o teto observado de turnos reais (600 s)"
        );
        assert!(
            d <= 3_600_000,
            "default ({d} ms) segue FINITO e na ordem de minutos (anti-deadlock + DLQ visível)"
        );
    }

    /// **Critério 1 da story:** alvo `Busy` → NÃO injeta (Retained + `MessageRetained` 1×);
    /// alvo volta a `Idle` → entrega EXATAMENTE 1× (sem perda, sem duplicata).
    #[test]
    fn retencao_alvo_busy_nao_injeta_e_entrega_no_idle() {
        let (mut router, sup, dir) = router_with("ret-busy");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("ret-busy");
        let (rec, mut deliver) = recorder();

        sup.set_status(b, NodeStatus::Busy).expect("B ocupado");
        let msg = MailMessage::new("@A", "@B", "ask", "espera tua vez");

        // Busy → Retained; nada injetado; MessageRetained{target_busy} no log (1×).
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1_000, &mut deliver),
            RouteOutcome::Retained { to: b }
        );
        assert!(rec.borrow().is_empty(), "nada injetado com o alvo Busy");
        let retained = events_of(&ts.store, "MessageRetained");
        assert_eq!(retained.len(), 1, "MessageRetained apendado na 1ª retenção");
        assert_eq!(
            retained[0].payload.get("reason").and_then(|v| v.as_str()),
            Some("target_busy")
        );

        // Re-avaliação (próximo tick) com B ainda Busy → segue Retained, SEM 2º evento (A4).
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1_200, &mut deliver),
            RouteOutcome::Retained { to: b }
        );
        assert_eq!(
            events_of(&ts.store, "MessageRetained").len(),
            1,
            "retenção repetida NÃO re-apenda (anti-amplificação)"
        );

        // B termina o turno → a entrega acontece EXATAMENTE 1×.
        turn_done(&sup, b);
        assert!(matches!(
            router.route_message(&msg, &mut ts.store, 1_400, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(rec.borrow().len(), 1, "entregue exatamente 1×");
        // Re-envio do mesmo id após a entrega → Duplicate (dedupe normal), nunca 2ª injeção.
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1_500, &mut deliver),
            RouteOutcome::Duplicate
        );
        assert_eq!(rec.borrow().len(), 1, "sem duplicata");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 2 da story:** 3 "respostas" simultâneas ao MESMO alvo são SERIALIZADAS —
    /// o log mostra entregas intercaladas com retenções, uma aguardando a outra (a entrega
    /// marca o alvo `Busy`; só o fim do turno libera a próxima).
    #[test]
    fn retencao_serializa_rajada_ao_mesmo_alvo() {
        let (mut router, sup, dir) = router_with("ret-rajada");
        for w in ["@W1", "@W2", "@W3"] {
            sup.register(w, None, sink());
        }
        let a = sup.register("@Maestro", None, sink());
        let mut ts = TmpStore::new("ret-rajada");
        let (rec, mut deliver) = recorder();
        sup.set_status(a, NodeStatus::Idle).expect("Maestro livre");

        // A rajada da baseline: 3 retornos depositados no mesmo instante (Δ=0).
        let m1 = MailMessage::new("@W1", "@Maestro", "ask", "RESULTADO 1");
        let m2 = MailMessage::new("@W2", "@Maestro", "ask", "RESULTADO 2");
        let m3 = MailMessage::new("@W3", "@Maestro", "ask", "RESULTADO 3");

        // Tick 1: m1 entrega (Maestro estava Idle) e o router o marca Busy → m2/m3 RETIDAS.
        assert!(matches!(
            router.route_message(&m1, &mut ts.store, 1_000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(
            router.route_message(&m2, &mut ts.store, 1_000, &mut deliver),
            RouteOutcome::Retained { to: a },
            "m2 aguarda m1 (alvo Busy pela entrega de m1)"
        );
        assert_eq!(
            router.route_message(&m3, &mut ts.store, 1_000, &mut deliver),
            RouteOutcome::Retained { to: a }
        );
        assert_eq!(rec.borrow().len(), 1, "só m1 injetada no tick da rajada");

        // Maestro terminou o turno de m1 → tick 2: m2 entrega, m3 segue aguardando.
        turn_done(&sup, a);
        assert!(matches!(
            router.route_message(&m2, &mut ts.store, 2_000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(
            router.route_message(&m3, &mut ts.store, 2_000, &mut deliver),
            RouteOutcome::Retained { to: a },
            "m3 aguarda m2 — uma aguardando a outra"
        );

        // Turno de m2 termina → tick 3: m3 entrega. Serialização completa, 0 colisão.
        turn_done(&sup, a);
        assert!(matches!(
            router.route_message(&m3, &mut ts.store, 3_000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(rec.borrow().len(), 3, "as 3 entregaram, uma por turno");

        // O LOG conta a história: 1 NodeStatusChanged{a2a_delivery} por entrega + 2 retenções.
        let delivered = events_of(&ts.store, "MessageDelivered");
        assert_eq!(delivered.len(), 3);
        assert_eq!(events_of(&ts.store, "MessageRetained").len(), 2);
        let busy_marks: Vec<_> = events_of(&ts.store, "NodeStatusChanged")
            .into_iter()
            .filter(|r| r.payload.get("reason").and_then(|v| v.as_str()) == Some("a2a_delivery"))
            .collect();
        assert_eq!(busy_marks.len(), 3, "cada entrega carimbou o alvo Busy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 4 da story (anti-deadlock):** o TIMEOUT de retenção dispara antes de
    /// qualquer espera circular — alvo PERPETUAMENTE Busy não retém para sempre: estourou
    /// o teto, ENTREGA mesmo assim (DLQ é F1-0-7).
    #[test]
    fn retencao_estoura_teto_e_vai_para_dlq() {
        let (mut router, sup, dir) = router_with("ret-teto");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("ret-teto");
        let (rec, mut deliver) = recorder();
        let cfg = RouterConfig {
            retention_timeout_ms: 1_000,
            ..RouterConfig::default()
        };
        router.config = cfg;

        sup.set_status(b, NodeStatus::Busy).expect("B preso");
        let msg = MailMessage::new("@A", "@B", "ask", "não espere para sempre");

        assert_eq!(
            router.route_message(&msg, &mut ts.store, 10_000, &mut deliver),
            RouteOutcome::Retained { to: b }
        );
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 10_500, &mut deliver),
            RouteOutcome::Retained { to: b },
            "dentro do teto segue retida"
        );
        // t=11_001 (>= 10_000 + 1_000): F1-0-7 (ADR 0020) — o teto corta a espera, e a
        // mensagem vai à DLQ com motivo (NUNCA injeta em alvo ocupado; nada some, nada
        // espera para sempre — 13.11 §P0; retry é manual).
        match router.route_message(&msg, &mut ts.store, 11_001, &mut deliver) {
            RouteOutcome::DeadLettered { reason } => {
                assert!(reason.contains("teto"), "motivo legível; veio: {reason}");
            }
            other => panic!("esperava DeadLettered no estouro do teto; veio {other:?}"),
        }
        assert!(rec.borrow().is_empty(), "NADA é injetado em alvo ocupado");
        assert!(
            dir.join(".lina/dead-letter")
                .join(format!("{}.json", msg.id))
                .exists(),
            "a mensagem está DURÁVEL na DLQ (projeção visível, retry manual)"
        );
        assert_eq!(
            events_of(&ts.store, "MessageDeadLettered").len(),
            1,
            "MessageDeadLettered no log (livro-razão)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Inv #4 + FIFO da story (via pump):** a fila retida VIVE no `.inflight` durável —
    /// um RESTART do router (novo processo) re-deriva a fila e entrega em ordem FIFO por
    /// alvo, exatamente 1× cada.
    #[test]
    fn retencao_via_pump_sobrevive_restart_fifo_exactly_once() {
        let (mut router, sup, dir) = router_with("ret-pump");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("ret-pump");
        let (rec, deliver) = recorder();
        sup.set_status(b, NodeStatus::Busy).expect("B ocupado");

        // 2 msgs (FIFO: id UUIDv7 temporal) depositadas pelo canal real (outbox por-nó).
        let m1 = MailMessage::new("@A", "@B", "ask", "primeira");
        let m2 = MailMessage::new("@A", "@B", "ask", "segunda");
        router.mailbox().enqueue_as("@A", &m1).expect("enq m1");
        router.mailbox().enqueue_as("@A", &m2).expect("enq m2");

        // Tick com B Busy: ambas RETIDAS, nada ackeado (seguem duráveis no .inflight).
        let r1 = router.pump(&mut ts.store, 1_000, deliver);
        assert!(r1
            .iter()
            .all(|(_, o)| matches!(o, RouteOutcome::Retained { .. })));
        let inflight = dir.join(".lina/outbox/.inflight");
        assert_eq!(
            std::fs::read_dir(&inflight).map(|d| d.count()).unwrap_or(0),
            2,
            "fila retida durável no .inflight"
        );

        // RESTART: router novo (estado em memória zerado) sobre a MESMA mailbox+store.
        drop(router);
        let mut router2 = Router::new(Arc::clone(&sup), Mailbox::new(dir.join(".lina")));
        let (rec2, deliver2) = recorder();
        // B ainda Busy → re-derivada e retida de novo (replay reconstrói a fila — inv #4).
        let r2 = router2.pump(&mut ts.store, 2_000, deliver2);
        assert!(r2
            .iter()
            .all(|(_, o)| matches!(o, RouteOutcome::Retained { .. })));

        // B livre → tick entrega m1 (FIFO) e re-retém m2 (B re-Busy pela entrega).
        turn_done(&sup, b);
        let (rec3, deliver3) = recorder();
        let r3 = router2.pump(&mut ts.store, 3_000, deliver3);
        let delivered_now: Vec<&String> = r3
            .iter()
            .filter(|(_, o)| matches!(o, RouteOutcome::Delivered { .. }))
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            delivered_now,
            vec![&m1.id],
            "FIFO: a 1ª da fila entrega primeiro"
        );
        assert_eq!(rec3.borrow().len(), 1);

        // Próximo turno → m2 entrega. Exactly-once de ponta a ponta.
        turn_done(&sup, b);
        let (rec4, deliver4) = recorder();
        let r4 = router2.pump(&mut ts.store, 4_000, deliver4);
        let delivered_last: Vec<&String> = r4
            .iter()
            .filter(|(_, o)| matches!(o, RouteOutcome::Delivered { .. }))
            .map(|(id, _)| id)
            .collect();
        assert_eq!(delivered_last, vec![&m2.id]);
        assert_eq!(rec4.borrow().len(), 1);
        assert_eq!(
            std::fs::read_dir(&inflight).map(|d| d.count()).unwrap_or(0),
            0,
            "tudo ackeado ao final (nada órfão)"
        );
        assert_eq!(
            events_of(&ts.store, "MessageDelivered").len(),
            2,
            "cada mensagem entregue EXATAMENTE 1× (sem perda, sem duplicata)"
        );
        let _ = (rec, rec2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Reply retida não vira loop:** a retenção acontece ANTES do fechamento do await —
    /// uma reply para um waiter `Busy` fica retida COM o ticket vivo; quando entrega, fecha
    /// o await normalmente (`AwaitClosed{replied}`) e NUNCA é cortada como loop.
    #[test]
    fn reply_retida_fecha_await_so_na_entrega() {
        let (mut router, sup, dir) = router_with("ret-reply");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("ret-reply");
        let (_rec, mut deliver) = recorder();

        // A --await--> B (abre o ticket; B fica Busy pela entrega da pergunta).
        let q = MailMessage::new("@A", "@B", "ask", "pergunta").awaiting();
        assert!(matches!(
            router.route_message(&q, &mut ts.store, 1_000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        // A está processando OUTRA coisa (Busy) quando B responde → a reply é RETIDA e o
        // ticket NÃO fecha (AwaitClosed ausente; aresta a→b viva).
        sup.set_status(a, NodeStatus::Busy).expect("A ocupado");
        let reply = MailMessage::new("@B", "@A", "ask", "resposta").replying_to(q.id.clone());
        assert_eq!(
            router.route_message(&reply, &mut ts.store, 1_100, &mut deliver),
            RouteOutcome::Retained { to: a }
        );
        assert!(
            events_of(&ts.store, "AwaitClosed").is_empty(),
            "reply retida NÃO fecha o await antes de ser entregue"
        );
        assert!(
            sup.begin_await(b, a).is_err(),
            "aresta a→b segue viva enquanto a reply espera"
        );

        // A termina o turno → a reply entrega, fecha o await e NÃO é loop.
        turn_done(&sup, a);
        assert!(matches!(
            router.route_message(&reply, &mut ts.store, 1_200, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let closed = events_of(&ts.store, "AwaitClosed");
        assert_eq!(closed.len(), 1, "await fechado na ENTREGA da reply");
        assert_eq!(
            closed[0].payload.get("reason").and_then(|v| v.as_str()),
            Some("replied")
        );
        assert!(
            !blocked_reasons(&ts.store).contains(&"loop_detected".to_string()),
            "reply retida nunca é confundida com loop"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────── W3-7b: await/reply lifecycle (ADR 0002) ─────────────────────────

    /// Acha o 1º registro de um `kind` no log (helper das asserções de await).
    fn first_event<'a>(
        recs: &'a [crate::events::EventRecord],
        kind: &str,
    ) -> &'a crate::events::EventRecord {
        recs.iter()
            .find(|r| r.kind == kind)
            .unwrap_or_else(|| panic!("evento {kind} ausente no log"))
    }

    /// AC-ADR (open→reply→release): `B --await--> A` apenda `AwaitOpened` e SEGURA a aresta `b→a` no
    /// wait-for-graph (enquanto aberta, `A --await--> B` é deadlock). O reply de A (`reply_to` casa o
    /// ticket) apenda `AwaitClosed{replied}` e LIBERA a aresta → `A --await--> B` volta a ser
    /// permitido. A liberação é verificada DIRETO no wait-for-graph (`sup.begin_await`) — isola do
    /// anti-loop por grafo, que sob W1 também amarraria A↔B pelo root do binding.
    #[test]
    fn await_lifecycle_open_reply_releases_waitgraph() {
        let (mut router, sup, dir) = router_with("await-lifecycle");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("await-lifecycle");
        let (_rec, mut deliver) = recorder();

        // (1) B --await--> A: abre o await (AwaitOpened + pending; aresta b→a viva no wait-for-graph).
        let q = MailMessage::new("@B", "@A", "ask", "pergunta").awaiting();
        assert!(matches!(
            router.route_message(&q, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let recs = ts.store.events().unwrap();
        let opened = first_event(&recs, "AwaitOpened");
        assert_eq!(opened.payload["id"].as_str(), Some(q.id.as_str()));
        assert_eq!(
            opened.payload["waiter"].as_str(),
            Some(b.to_string().as_str()),
            "waiter = B"
        );
        assert_eq!(
            opened.payload["target"].as_str(),
            Some(a.to_string().as_str()),
            "target = A"
        );

        // Enquanto aberto, b→a está vivo: a--await-->b fecharia o ciclo (deadlock no wait-for-graph).
        assert!(
            sup.begin_await(a, b).is_err(),
            "b→a vivo → a--await-->b seria deadlock"
        );

        // (2) Reply de A → B (`reply_to` = q.id CASA o ticket): fecha (AwaitClosed{replied}) e libera
        //     b→a. O guard de loop é pulado SÓ por ser reply legítimo (ticket real em pending).
        let reply = MailMessage::new("@A", "@B", "ask", "resposta").replying_to(q.id.clone());
        assert!(matches!(
            router.route_message(&reply, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let recs = ts.store.events().unwrap();
        let closed = first_event(&recs, "AwaitClosed");
        assert_eq!(closed.payload["id"].as_str(), Some(q.id.as_str()));
        assert_eq!(closed.payload["reason"].as_str(), Some("replied"));

        // (3) Liberado: a--await-->b agora é PERMITIDO (b→a foi removido pelo reply).
        assert!(
            sup.begin_await(a, b).is_ok(),
            "após o reply, o wait-for-graph foi liberado"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-ADR (timeout): um `await` sem reply até o teto fecha por `AwaitClosed{timeout}` no sweep
    /// do `pump`.
    #[test]
    fn await_timeout_closes_ticket() {
        let (mut router, sup, dir) = router_with("await-timeout");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("await-timeout");
        let (_rec, mut deliver) = recorder();

        let q = MailMessage::new("@B", "@A", "ask", "pergunta").awaiting();
        assert!(matches!(
            router.route_message(&q, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        // pump muito depois do teto (mailbox vazia) → o sweep fecha o ticket por Timeout.
        let results = router.pump(&mut ts.store, 1000 + AWAIT_TIMEOUT_MS + 1, deliver);
        assert!(results.is_empty(), "mailbox vazia — só o sweep roda");
        let recs = ts.store.events().unwrap();
        let closed = first_event(&recs, "AwaitClosed");
        assert_eq!(closed.payload["id"].as_str(), Some(q.id.as_str()));
        assert_eq!(closed.payload["reason"].as_str(), Some("timeout"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 4 (confused-deputy): um nó C de identidade REAL (sem forjar `from`) que apenas LEU o
    /// `id` da pergunta no log NÃO fecha o `await` de B só por escrever `reply_to = q.id`. O fechamento
    /// é decidido pelo PARTICIPANTE autenticado: só o `target` aguardado (A) fecha. Prova que (a) o
    /// reply de C (target ≠ sender) NÃO consome o ticket (sobrevive em `pending`; NENHUM `AwaitClosed`)
    /// e (b) o reply do A real fecha normalmente (`AwaitClosed{replied}` + aresta liberada).
    #[test]
    fn reply_from_non_target_does_not_close_await() {
        let (mut router, sup, dir) = router_with("await-intruder");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("await-intruder");
        let (_rec, mut deliver) = recorder();

        // (1) B --await--> A: abre o ticket (aresta b→a viva → a--await-->b seria deadlock).
        let q = MailMessage::new("@B", "@A", "ask", "pergunta").awaiting();
        assert!(matches!(
            router.route_message(&q, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert!(
            sup.begin_await(a, b).is_err(),
            "ticket aberto: b→a vivo no wait-for-graph"
        );

        // (2) C lê q.id no log e injeta {from:@C, reply_to:q.id}. Como sender(C) ≠ target(A), NÃO fecha.
        let intruder = MailMessage::new("@C", "@A", "ask", "intruso").replying_to(q.id.clone());
        let _ = router.route_message(&intruder, &mut ts.store, 1001, &mut deliver);
        let recs = ts.store.events().unwrap();
        assert!(
            !recs.iter().any(|r| r.kind == "AwaitClosed"),
            "reply de não-participante NÃO pode logar AwaitClosed"
        );
        assert!(
            sup.begin_await(a, b).is_err(),
            "o ticket de B SOBREVIVE ao reply forjado de C (aresta b→a intacta)"
        );

        // (3) O target REAL (A) responde → fecha normalmente (AwaitClosed{replied}, aresta liberada).
        let reply = MailMessage::new("@A", "@B", "ask", "resposta").replying_to(q.id.clone());
        assert!(matches!(
            router.route_message(&reply, &mut ts.store, 1002, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let recs = ts.store.events().unwrap();
        let closed = first_event(&recs, "AwaitClosed");
        assert_eq!(closed.payload["id"].as_str(), Some(q.id.as_str()));
        assert_eq!(closed.payload["reason"].as_str(), Some("replied"));
        assert!(
            sup.begin_await(a, b).is_ok(),
            "o reply do participante autenticado liberou o wait-for-graph"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 4 (defesa em profundidade): mesmo vindo do `target` legítimo (A), um reply cujo `to` NÃO
    /// é o waiter (B) não fecha o ticket — a resposta tem de voltar PARA quem perguntou. Só o reply
    /// bem-endereçado de A (`to = @B`) fecha. Verifica o participante autenticado E o destino corretos.
    #[test]
    fn reply_to_wrong_destination_does_not_close_await() {
        let (mut router, sup, dir) = router_with("await-misaddressed");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("await-misaddressed");
        let (_rec, mut deliver) = recorder();

        // B --await--> A.
        let q = MailMessage::new("@B", "@A", "ask", "pergunta").awaiting();
        assert!(matches!(
            router.route_message(&q, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        // A responde, MAS endereça a @C (não ao waiter B) → não fecha (to ≠ waiter).
        let misaddressed = MailMessage::new("@A", "@C", "ask", "errado").replying_to(q.id.clone());
        let _ = router.route_message(&misaddressed, &mut ts.store, 1001, &mut deliver);
        let recs = ts.store.events().unwrap();
        assert!(
            !recs.iter().any(|r| r.kind == "AwaitClosed"),
            "reply mal-endereçado (to ≠ waiter) NÃO fecha o ticket"
        );
        assert!(
            sup.begin_await(a, b).is_err(),
            "ticket de B sobrevive ao reply mal-endereçado de A"
        );

        // A responde bem-endereçado (to = @B) → fecha.
        let good = MailMessage::new("@A", "@B", "ask", "resposta").replying_to(q.id.clone());
        assert!(matches!(
            router.route_message(&good, &mut ts.store, 1002, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let recs = ts.store.events().unwrap();
        let closed = first_event(&recs, "AwaitClosed");
        assert_eq!(closed.payload["reason"].as_str(), Some("replied"));
        assert!(
            sup.begin_await(a, b).is_ok(),
            "reply bem-endereçado do target liberou o wait-for-graph"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────── Hardening Round 3: regressões do roteador ─────────────────────────

    /// reply_to-bypass: um `reply_to` FORJADO (sem ticket em `pending`) NÃO pula o anti-loop — então
    /// NÃO ganha voltas extras. Um `reply_to` que CASA um ticket real (reply legítimo) pula o guard
    /// (back-edge da pergunta — ver `await_lifecycle_open_reply_releases_waitgraph`). Após o ajuste de
    /// cooperação o ping-pong é cortado por REPETIÇÃO (`MAX_CYCLE_REVISITS`), não na 1ª volta: aqui o
    /// `reply_to` forjado em CADA salto NÃO o isenta do guard, então o loop apertado ainda fecha no limite.
    #[test]
    fn forged_reply_to_does_not_bypass_loop() {
        let (mut router, sup, dir) = router_with("replybypass");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("replybypass");
        let (_rec, mut deliver) = recorder();

        // Ping-pong, cada salto com `reply_to` FORJADO (não casa ticket → NÃO pula o guard). As voltas
        // dentro do limite entregam (diálogo); o forjado não compra rodadas extras.
        let pingpong = [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")];
        for (i, (from, to)) in pingpong.iter().enumerate() {
            // F1-0-4: cada perna só acontece após o turno anterior terminar (ping-pong real).
            turn_done(&sup, a);
            turn_done(&sup, b);
            let m = MailMessage::new(*from, *to, "ask", "volta").replying_to("msg_naoexiste");
            assert!(
                matches!(
                    router.route_message(&m, &mut ts.store, 1000 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "volta {i} é diálogo (entrega) — reply_to forjado não pula o guard nem muda o limite"
            );
        }
        // 3ª repetição do salto A→B com reply_to FORJADO → NÃO pula o guard → loop apertado cortado.
        turn_done(&sup, a);
        turn_done(&sup, b); // F1-0-4: o corte deve vir do anti-loop, não da retenção
        let forged =
            MailMessage::new("@A", "@B", "ask", "volta demais").replying_to("msg_naoexiste");
        assert_eq!(
            router.route_message(&forged, &mut ts.store, 1010, &mut deliver),
            RouteOutcome::LoopDetected,
            "reply_to forjado (sem ticket) não escapa do anti-loop nem ganha voltas extras"
        );
        assert!(blocked_reasons(&ts.store).contains(&"loop_detected".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1: uma mensagem de ENTRADA sob root fresco NÃO envenena o binding da vítima. A→B fixa B em
    /// R1; um atacante manda B sob outro root; B delega `root=None` → ainda herda R1 (não o do atacante).
    #[test]
    fn delivered_root_not_poisoned_by_foreign_root() {
        let (mut router, sup, dir) = router_with("poison");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let _x = sup.register("@X", None, sink());
        let mut ts = TmpStore::new("poison");
        let (_rec, mut deliver) = recorder();

        // A→B: fixa o binding de B em R1 = id da msg de A.
        let m_ab = MailMessage::new("@A", "@B", "ask", "cadeia R1");
        router.route_message(&m_ab, &mut ts.store, 1, &mut deliver);
        let r1 = m_ab.id.clone();

        // Atacante X→B sob root fresco (X é origem → root = id da msg de X, != R1).
        let m_xb = MailMessage::new("@X", "@B", "ask", "envenena");
        router.route_message(&m_xb, &mut ts.store, 2, &mut deliver);

        // B delega (root=None) → deriva do binding. P1: deve seguir R1 (não o root do atacante).
        let m_bc = MailMessage::new("@B", "@C", "ask", "delega");
        assert!(matches!(
            router.route_message(&m_bc, &mut ts.store, 3, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let routed = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "MessageRouted")
            .find(|r| r.payload["to"].as_str() == Some("@C"))
            .expect("MessageRouted de B→C");
        assert_eq!(
            routed.payload["root_cause_id"].as_str(),
            Some(r1.as_str()),
            "B herdou R1, não o root fresco do atacante (binding não foi envenenado)"
        );
        // sanidade: o root do atacante não é R1.
        assert_ne!(m_xb.id, r1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D1: após um RESTART (perda do `seen` em memória), um órfão em `.inflight` que JÁ tem
    /// `MessageRouted` no log NÃO é re-entregue — o pump consulta o log durável, confirma e pula.
    #[test]
    fn no_redelivery_after_restart_when_already_routed() {
        let (router, sup, dir) = router_with("d1");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("d1");

        // Enqueue + drena para .inflight (drenado, prestes a rotear).
        let m = MailMessage::new("@A", "@B", "ask", "oi");
        router.mailbox().enqueue(&m).expect("enqueue");
        let drained = router.mailbox().drain_to_inflight().expect("drain");
        assert_eq!(drained.len(), 1);
        let infl = router
            .mailbox()
            .inflight_dir()
            .join(format!("{}.json", m.id));
        assert!(infl.exists(), "órfão em .inflight");

        // Simula "roteado + entregue ao PTY" e CRASH antes do ack: persiste MessageRouted{id=m.id}.
        ts.store
            .append(&DomainEvent::MessageRouted {
                id: m.id.clone(),
                from: a,
                to: "@B".into(),
                intent: "ask".into(),
                root_cause_id: m.id.clone(),
                hops: 0,
                to_node: Some(b),
            })
            .expect("persist routed");

        // RESTART: descarta o Router (perde `seen`) e cria um novo sobre o MESMO .lina + store.
        drop(router);
        let mut router2 = Router::new(Arc::clone(&sup), Mailbox::new(dir.join(".lina")));
        let (rec2, deliver2) = recorder();
        let results = router2.pump(&mut ts.store, 5000, deliver2);

        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0].1, RouteOutcome::Duplicate),
            "órfão já roteado no log → Duplicate (sem 2ª entrega)"
        );
        assert!(rec2.borrow().is_empty(), "NENHUMA 2ª injeção no PTY");
        assert!(!infl.exists(), "órfão confirmado (.inflight limpo)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Registros de um `kind` no log (helper das asserções do teto de custo).
    fn records_of_kind(store: &EventStore, kind: &str) -> Vec<EventRecord> {
        store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == kind)
            .collect()
    }

    /// **AC-7.2 (W3-7c) — TETO DE CUSTO, headless via event log.** Semeando eventos de uso
    /// (`TokenUsageReported`) que SOMAM > `token_budget_day`, a próxima delegação retorna o outcome
    /// NOVO `CostCeiling` E apenda `CostCeilingHit{day, tokens}` (workspace `Paused`, observável no
    /// log). ANTES do teto, a delegação é normal (sem `CostCeilingHit`). `CostCeilingResumed` (o que
    /// `lina resume --confirm` apenda) REABRE a janela: a delegação volta a funcionar. Distinto do
    /// orçamento de delegação (`BudgetExceeded`).
    #[test]
    fn cost_ceiling_pauses_delegation_until_resume() {
        // Round 5 #9: a janela do teto agora é DIÁRIA (`utc_day(rec.ts)==utc_day(now_ms)`). Como o
        // `EventStore::append` carimba o `ts` com wall-clock real (não há injeção de relógio), o
        // `now_ms` do replay precisa cair no MESMO dia-UTC dos appends → usamos o relógio real. As
        // asserções de `day` ficam auto-consistentes via `utc_day(now)`.
        let now: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("lina-router-cost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sup = Arc::new(Supervisor::new());
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let cfg = RouterConfig {
            token_budget_day: 100,
            ..RouterConfig::default()
        };
        let mut router =
            Router::with_config(Arc::clone(&sup), Mailbox::new(dir.join(".lina")), cfg);
        let mut ts = TmpStore::new("cost");
        let (rec, mut deliver) = recorder();

        // ── (1) ANTES do teto (uso=0 < 100): delegação NORMAL, sem CostCeilingHit.
        let m1 = MailMessage::new("@A", "@B", "ask", "antes");
        assert!(
            matches!(
                router.route_message(&m1, &mut ts.store, now, &mut deliver),
                RouteOutcome::Delivered { .. }
            ),
            "antes do teto a delegação deve entregar normalmente"
        );
        assert!(
            records_of_kind(&ts.store, "CostCeilingHit").is_empty(),
            "sem uso reportado, nenhum teto pode ter sido batido"
        );

        // ── Semeia o USO direto no store (no app, viria do fim-de-resposta da CLI — W0-10): 70 + 80
        //    = 150 > 100. É a FONTE de uso event-sourced que o CostLedger soma por replay.
        ts.store
            .append(&DomainEvent::TokenUsageReported {
                node: "@B".into(),
                tokens: 70,
            })
            .expect("seed usage 70");
        ts.store
            .append(&DomainEvent::TokenUsageReported {
                node: "@B".into(),
                tokens: 80,
            })
            .expect("seed usage 80");

        // ── (2) Uso (150) ≥ teto (100): a próxima delegação retorna CostCeiling, NÃO entrega, e
        //    apenda CostCeilingHit{day, tokens}; o workspace fica Paused (observável no log).
        turn_done(&sup, b); // F1-0-4: o bloqueio abaixo deve vir do TETO, não da retenção
        let before = rec.borrow().len();
        let m2 = MailMessage::new("@A", "@B", "ask", "estoura");
        assert_eq!(
            router.route_message(&m2, &mut ts.store, now, &mut deliver),
            RouteOutcome::CostCeiling,
            "ao atingir o teto, a delegação é gateada (Paused), não entregue"
        );
        assert_eq!(
            rec.borrow().len(),
            before,
            "NADA foi entregue ao PTY no teto"
        );
        let hits = records_of_kind(&ts.store, "CostCeilingHit");
        assert_eq!(hits.len(), 1, "exatamente 1 CostCeilingHit na transição");
        assert_eq!(
            hits[0].payload.get("tokens").and_then(|v| v.as_u64()),
            Some(150),
            "o hit registra a soma da janela"
        );
        assert_eq!(
            hits[0].payload.get("day").and_then(|v| v.as_str()),
            Some(utc_day(now).as_str()),
            "o hit registra o dia-UTC derivado do relógio do roteador"
        );
        // Projeção observável do estado Paused, reconstruída do log (invariante #4).
        assert!(
            CostLedger::replay(&ts.store, now).expect("ledger").paused,
            "o ledger reconstruído do log deve indicar Paused"
        );

        // ── (2b) ANTI-AMPLIFICAÇÃO (A4): já pausado, uma nova delegação ainda bloqueia, mas NÃO
        //    apenda um 2º CostCeilingHit.
        let m3 = MailMessage::new("@A", "@B", "ask", "ainda pausado");
        assert_eq!(
            router.route_message(&m3, &mut ts.store, now, &mut deliver),
            RouteOutcome::CostCeiling
        );
        assert_eq!(
            records_of_kind(&ts.store, "CostCeilingHit").len(),
            1,
            "delegação sob Paused não re-apenda o teto (anti-amplificação)"
        );

        // ── (3) RETOMADA: `lina resume --confirm` apenda CostCeilingResumed → sai de Paused e ZERA a
        //    janela. A delegação volta a funcionar (sem re-pausar com a soma antiga).
        ts.store
            .append(&DomainEvent::CostCeilingResumed { day: utc_day(now) })
            .expect("resume");
        assert!(
            !CostLedger::replay(&ts.store, now).expect("ledger").paused,
            "após resume o workspace não está mais Paused"
        );
        turn_done(&sup, b); // F1-0-4: B livre — o que se testa é o pós-resume, não a retenção
        let m4 = MailMessage::new("@A", "@B", "ask", "depois do resume");
        assert!(
            matches!(
                router.route_message(&m4, &mut ts.store, now, &mut deliver),
                RouteOutcome::Delivered { .. }
            ),
            "após resume a delegação volta a entregar"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────── W4-3: freio do rodapé (orquestração pausada) ─────────────────────────

    /// **W4-3 (freio), headless.** Com a orquestração PAUSADA, uma delegação nova fica ENFILEIRADA
    /// (não injetada) e DURÁVEL no `.inflight` (não perde, não kill — inv #6); `OrchestrationPaused`
    /// consta no log. Ao `resume`, a fila DRENA: a delegação é roteada/entregue e `OrchestrationResumed`
    /// consta no log. (O freio trava AUTO-ORQUESTRAÇÃO; replies/plan seguem fechando loops abertos.)
    #[test]
    fn orchestration_pause_queues_delegation_until_resume() {
        let (mut router, sup, dir) = router_with("freio");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("freio");
        let (rec, mut deliver) = recorder();

        // ── (1) Freio ON → OrchestrationPaused no log (idempotente: 2º pause não re-loga — A4).
        assert!(
            router.pause(&mut ts.store).expect("pause"),
            "1º pause transiciona"
        );
        assert!(
            !router.pause(&mut ts.store).expect("pause idem"),
            "2º pause é no-op"
        );
        assert_eq!(
            records_of_kind(&ts.store, "OrchestrationPaused").len(),
            1,
            "exatamente 1 OrchestrationPaused (anti-amplificação)"
        );

        // ── (2) Delegação nova sob o freio: ENFILEIRA (não injeta). Fica no `.inflight` (durável).
        let m = MailMessage::new("@A", "@B", "ask", "espera o freio");
        // Round 6: por-nó (autenticado) — o flat agora é anônimo e seria recusado no pump.
        router
            .mailbox()
            .enqueue_as("@A", &m)
            .expect("enqueue por-no");
        let r1 = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(r1.len(), 1);
        assert!(
            matches!(r1[0].1, RouteOutcome::Queued),
            "pausado: a delegação é enfileirada, não entregue"
        );
        assert!(
            rec.borrow().is_empty(),
            "NADA foi injetado no PTY sob o freio"
        );
        assert!(
            records_of_kind(&ts.store, "MessageRouted").is_empty(),
            "delegação enfileirada não é roteada ainda (entrega adiada)"
        );
        let infl = router
            .mailbox()
            .inflight_dir()
            .join(format!("{}.json", m.id));
        assert!(
            infl.exists(),
            "a delegação permanece DURÁVEL no .inflight (não perde)"
        );

        // ── (3) Retomar → OrchestrationResumed + DRENA a fila: agora roteia e injeta.
        let r2 = router
            .resume(&mut ts.store, 1001, &mut deliver)
            .expect("resume");
        assert_eq!(r2.len(), 1, "o resume drenou a única delegação enfileirada");
        assert!(
            matches!(r2[0].1, RouteOutcome::Delivered { .. }),
            "ao retomar, a delegação enfileirada é entregue"
        );
        assert_eq!(rec.borrow().len(), 1, "agora SIM injetou no PTY");
        assert_eq!(
            records_of_kind(&ts.store, "OrchestrationResumed").len(),
            1,
            "OrchestrationResumed no log"
        );
        assert_eq!(
            records_of_kind(&ts.store, "MessageRouted").len(),
            1,
            "ao drenar, a delegação é finalmente roteada"
        );
        assert!(
            !infl.exists(),
            "drenada → .inflight limpo (ack após persistir)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **W4-3 (freio) — durabilidade do estado pós-crash (inv #4/#6).** O freio é event-sourced:
    /// `OrchestrationPaused` vai ao log. Um `Router` novo (recuperação pós-crash) nasce despausado
    /// (estado em memória), mas RECONSTRÓI `paused` do log via `restore_orchestration_state` → o freio
    /// que o humano acionou NÃO é perdido pelo crash (a fila represada segue represada).
    #[test]
    fn orchestration_pause_state_survives_restart_via_log() {
        let (mut router, sup, dir) = router_with("freio-restart");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("freio-restart");
        let (_rec, mut deliver) = recorder();

        router.pause(&mut ts.store).expect("pause");
        assert!(router.is_paused());

        // "Crash": descarta o Router; um novo sobre o MESMO log nasce DESPAUSADO (estado em memória).
        drop(router);
        let mut router2 = Router::new(Arc::clone(&sup), Mailbox::new(dir.join(".lina")));
        assert!(
            !router2.is_paused(),
            "Router novo nasce despausado (paused é estado em memória)"
        );

        // Reconstrói do log (fonte da verdade, inv #4) → volta a pausado.
        router2
            .restore_orchestration_state(&ts.store)
            .expect("restore");
        assert!(router2.is_paused(), "freio reconstruído do log");

        // Prova observável: uma delegação segue ENFILEIRADA (o crash NÃO drenou a fila represada).
        router2
            .mailbox()
            .enqueue(&MailMessage::new("@A", "@B", "ask", "pós-crash"))
            .expect("enqueue");
        let r = router2.pump(&mut ts.store, 1000, &mut deliver);
        assert!(
            matches!(r[0].1, RouteOutcome::Queued),
            "freio restaurado do log → delegação ainda enfileira (não drena no crash)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **r4 achado #12 (dogfooding 2026-06-11) — o freio cobre TAMBÉM o `lina spawn`.** Antes,
    /// `is_delegation` não incluía `spawn`: sob pausa o terminal NASCIA (SpawnApproved) mas o seu
    /// 1º prompt (um `ask`) ficava represado — nó vivo, tarefa nunca chega (a meia-orquestração
    /// vivida na r3: 6 nós Idle só com bootstrap). O freio é GATE, não kill: o spawn enfileira
    /// DURÁVEL no `.inflight` e acontece INTEIRO (nó + 1º prompt) no resume.
    #[test]
    fn orchestration_pause_queues_spawn_until_resume() {
        let (mut router, sup, dir) = router_with("freio-spawn");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("freio-spawn");
        let (rec, mut deliver) = recorder();

        router.pause(&mut ts.store).expect("pause");

        // Sob o freio: o pedido de spawn ENFILEIRA (não executa) — paridade com a delegação.
        let m = MailMessage::new("@A", "@QA", "spawn", "valide o checkout").with_ref("role:qa");
        router
            .mailbox()
            .enqueue_as("@A", &m)
            .expect("enqueue por-no");
        let r1 = router.pump(&mut ts.store, 1000, &mut deliver);
        assert_eq!(r1.len(), 1);
        assert!(
            matches!(r1[0].1, RouteOutcome::Queued),
            "pausado: o spawn é enfileirado, não decidido; veio {:?}",
            r1[0].1
        );
        assert!(
            records_of_kind(&ts.store, "SpawnRequested").is_empty(),
            "sob o freio o gate de spawn NEM roda (nada decidido/logado)"
        );
        let infl = router
            .mailbox()
            .inflight_dir()
            .join(format!("{}.json", m.id));
        assert!(infl.exists(), "o pedido permanece DURÁVEL no .inflight");

        // Resume → o spawn drena e é decidido AGORA (gate intacto: origem → aprovado).
        let r2 = router
            .resume(&mut ts.store, 2000, &mut deliver)
            .expect("resume");
        assert_eq!(r2.len(), 1, "o resume drenou o spawn enfileirado");
        assert!(
            matches!(r2[0].1, RouteOutcome::SpawnApproved { .. }),
            "ao retomar, o spawn passa pelo gate normalmente; veio {:?}",
            r2[0].1
        );
        assert_eq!(
            records_of_kind(&ts.store, "SpawnRequested").len(),
            1,
            "SpawnRequested logado exatamente 1× (no resume)"
        );
        assert!(
            rec.borrow().is_empty(),
            "spawn não injeta em PTY algum (decisão de gate, não entrega)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────── Round 5 furos #6/#7: wait-for-graph multi-aresta (ADR 0002 reforçado) ─────────────

    /// **Round 5 #7 (desync pending × wait-for-graph).** Um waiter pode ter DOIS awaits abertos a
    /// alvos distintos. O 2º NÃO pode apagar a aresta do 1º (o grafo é multi-aresta por waiter), e
    /// fechar 1 ticket libera SÓ aquela aresta — o outro segue vivo. Antes (1 aresta/waiter) o 2º
    /// sobrescrevia o 1º → o guard de deadlock ficava cego para a aresta perdida.
    #[test]
    fn second_await_does_not_erase_first_multi_edge() {
        let (mut router, sup, dir) = router_with("multi-await");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("multi-await");
        let (_rec, mut deliver) = recorder();

        // B--await-->A (ticket1) e B--await-->C (ticket2): AMBAS as arestas devem coexistir.
        let q1 = MailMessage::new("@B", "@A", "ask", "p1").awaiting();
        assert!(matches!(
            router.route_message(&q1, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let q2 = MailMessage::new("@B", "@C", "ask", "p2").awaiting();
        assert!(matches!(
            router.route_message(&q2, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        // Com ambas vivas, fechar o ciclo por qualquer alvo é deadlock (o guard vê AS DUAS arestas).
        assert!(
            sup.would_await_deadlock(a, b),
            "B--await-->A vivo → A--await-->B é deadlock"
        );
        assert!(
            sup.would_await_deadlock(c, b),
            "B--await-->C vivo → C--await-->B é deadlock"
        );

        // Fecha SÓ o ticket1 (reply A→B casa q1): a aresta B→A sai; B→C SOBREVIVE.
        let reply1 = MailMessage::new("@A", "@B", "ask", "r1").replying_to(q1.id.clone());
        assert!(matches!(
            router.route_message(&reply1, &mut ts.store, 1002, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        assert!(
            !sup.would_await_deadlock(a, b),
            "ticket1 fechado → A--await-->B liberado"
        );
        assert!(
            sup.would_await_deadlock(c, b),
            "ticket2 ainda vivo → fechar 1 NÃO libera o outro (C--await-->B segue deadlock)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Round 5 #6 (broadcast-await apaga aresta viva).** Com um single-target await ABERTO
    /// (B--await-->A, ticket em pending), um broadcast-await do MESMO sender faz a checagem de
    /// deadlock SEM mutar a aresta viva — a aresta B→A SOBREVIVE e o guard NÃO fica cego. Antes, o
    /// ramo chamava `end_await(sender)` incondicional e apagava B→A (deixando A--await-->B passar).
    #[test]
    fn broadcast_await_preserves_live_single_target_edge() {
        let (mut router, sup, dir) = router_with("bcast-await");
        let a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("bcast-await");
        let (_rec, mut deliver) = recorder();

        // B--await-->A (single-target): aresta b→a VIVA (ticket em pending).
        let q = MailMessage::new("@B", "@A", "ask", "single").awaiting();
        assert!(matches!(
            router.route_message(&q, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert!(
            sup.would_await_deadlock(a, b),
            "pré: b→a vivo → a--await-->b é deadlock"
        );

        // B faz um BROADCAST --await: a checagem de deadlock é PURA (não muta) → b→a SOBREVIVE.
        let bc = MailMessage::new("@B", "*", "broadcast", "todos").awaiting();
        let out = router.route_message(&bc, &mut ts.store, 1001, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "broadcast-await entregue (sem deadlock espúrio): {out:?}"
        );

        // O guard NÃO ficou cego: b→a continua vivo → a--await-->b SEGUE deadlock.
        assert!(
            sup.would_await_deadlock(a, b),
            "pós-broadcast: b→a sobreviveu → a--await-->b ainda é deadlock"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────────────── Round 5 #8: binding delivered_root migra p/ a cadeia ativa + poda ───────────────

    /// **Round 5 #8 (binding nunca migra/poda → grafo anti-loop fragmenta).** Passada a janela (nó
    /// OCIOSO, sem await pendente), uma cadeia NOVA entregue ao nó MIGRA o binding para o root ativo
    /// (em vez de congelar no 1º root para sempre). Sem isso, `handoff_would_tight_loop` fragmenta por
    /// sessão. A defesa anti-forja segue: P1 (foreign root FRESCO não migra) é o teste separado.
    #[test]
    fn delivered_root_migrates_to_active_chain_after_window() {
        let (mut router, sup, dir) = router_with("migrate");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("migrate");
        let (_rec, mut deliver) = recorder();

        // A→B sob a cadeia R1: fixa o binding de B em R1.
        let m1 = MailMessage::new("@A", "@B", "ask", "cadeia R1");
        router.route_message(&m1, &mut ts.store, 1000, &mut deliver);
        let r1 = m1.id.clone();

        // Passada a janela (B ocioso, sem await): A→B sob a cadeia NOVA R2 → o binding de B MIGRA.
        turn_done(&sup, b); // F1-0-4: "B ocioso" agora é literal — terminou o turno de R1
        let later = 1000 + DEDUPE_WINDOW_MS + 1;
        let m2 = MailMessage::new("@A", "@B", "ask", "cadeia R2");
        router.route_message(&m2, &mut ts.store, later, &mut deliver);
        let r2 = m2.id.clone();
        assert_ne!(r1, r2, "cada origem A gera um root fresco por msg");

        // B delega (root=None) → deriva do binding ATIVO (R2), não congela em R1.
        let m3 = MailMessage::new("@B", "@C", "ask", "delega");
        assert!(matches!(
            router.route_message(&m3, &mut ts.store, later, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        let routed = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == "MessageRouted")
            .find(|r| r.payload["to"].as_str() == Some("@C"))
            .expect("MessageRouted de B→C");
        assert_eq!(
            routed.payload["root_cause_id"].as_str(),
            Some(r2.as_str()),
            "binding migrou p/ a cadeia ATIVA R2 (não congelou em R1 → anti-loop não fragmenta)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────────────── Round 5 #9: janela do teto de custo é DIÁRIA (zera à meia-noite) ───────────────

    /// **Round 5 #9 (janela do teto não era diária).** O `CostLedger` soma SÓ o uso do dia-UTC
    /// corrente (`utc_day(rec.ts)==utc_day(now_ms)`): uso de ONTEM não conta hoje e um `CostCeilingHit`
    /// de ontem NÃO mantém o workspace pausado hoje → a janela zera sozinha à meia-noite, reduzindo a
    /// dependência do `lina resume` humano. Testa o agregador PURO `from_records` com `ts` controlado
    /// (o `append` carimba wall-clock; aqui construímos os `EventRecord` direto — campos públicos).
    #[test]
    fn cost_window_is_daily_yesterday_excluded() {
        const DAY_MS: u64 = 86_400_000;
        let today = 1_700_000_000_000_u64; // ms fixos → dia-UTC determinístico
        let yesterday = today - DAY_MS;
        let rec = |seq: u64, ts: u64, kind: &str, payload: serde_json::Value| EventRecord {
            seq,
            ts,
            kind: kind.to_string(),
            version: 1,
            payload,
        };
        let recs = vec![
            // ONTEM: uso alto + teto batido (deveria estar "esquecido" hoje).
            rec(
                1,
                yesterday,
                "TokenUsageReported",
                serde_json::json!({ "node": "@B", "tokens": 999 }),
            ),
            rec(
                2,
                yesterday,
                "CostCeilingHit",
                serde_json::json!({ "day": utc_day(yesterday), "tokens": 999 }),
            ),
            // HOJE: só este uso conta.
            rec(
                3,
                today,
                "TokenUsageReported",
                serde_json::json!({ "node": "@B", "tokens": 10 }),
            ),
        ];

        let ledger = CostLedger::from_records(&recs, today);
        assert_eq!(
            ledger.day,
            utc_day(today),
            "o rótulo do dia vem do relógio (now_ms)"
        );
        assert_eq!(
            ledger.tokens, 10,
            "só o uso de HOJE conta na janela (o de ontem não)"
        );
        assert!(
            !ledger.paused,
            "o CostCeilingHit de ONTEM NÃO pausa hoje — a janela zera à meia-noite sem resume"
        );

        // Sanidade: o mesmo log avaliado em ONTEM enxergava o teto batido (uso 999, pausado).
        let ledger_yest = CostLedger::from_records(&recs, yesterday);
        assert_eq!(ledger_yest.tokens, 999, "ontem somava o uso de ontem");
        assert!(
            ledger_yest.paused,
            "ontem estava pausado (CostCeilingHit do dia)"
        );
    }

    // ─────────────── F1-3-6: gate inforjável do `lina spawn` (core, headless) ───────────────

    /// Payloads dos eventos de um `kind` (helper das asserções do gate de spawn).
    fn spawn_payloads(store: &EventStore, kind: &str) -> Vec<serde_json::Value> {
        store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.payload)
            .collect()
    }

    // ─────────────── F3-0-5-core: verbo `lina params set/reset` no router ───────────────

    /// `params.set` de ORIGEM emite `SystemParamsChanged` com key/scope/value do envelope e `by`
    /// carimbado SERVER-SIDE (`human`, pois `hops==0`).
    #[test]
    fn params_set_emits_change_with_server_side_by() {
        let (mut router, sup, _dir) = router_with("params-set");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("params-set");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"fanout_gate","scope":"workspace","value":"8"}"#,
        );
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::ParamsApplied { .. }),
            "veio {out:?}"
        );
        let ev = spawn_payloads(&ts.store, "SystemParamsChanged")
            .pop()
            .expect("evento");
        assert_eq!(ev["key"].as_str(), Some("fanout_gate"));
        assert_eq!(ev["scope"].as_str(), Some("workspace"));
        assert_eq!(ev["new"].as_str(), Some("8"));
        assert_eq!(
            ev["by"].as_str(),
            Some("human"),
            "origem humana → by carimbado server-side"
        );
    }

    /// O `set` RECUSA valor fora da faixa (≠ load, que clampa) — nada é logado.
    #[test]
    fn params_set_out_of_range_is_rejected_without_event() {
        let (mut router, sup, _dir) = router_with("params-oor");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("params-oor");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"delegation_budget","scope":"workspace","value":"9999"}"#,
        );
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::ParamsRejected(_)),
            "veio {out:?}"
        );
        assert!(
            spawn_payloads(&ts.store, "SystemParamsChanged").is_empty(),
            "valor inválido não loga nada"
        );
    }

    /// REGRA-MÃE (ADR 0007): um `by` forjado no payload é IGNORADO — o carimbo é server-side.
    /// NÃO-VACUOSO: quebraria se o handler lesse `payload["by"]` em vez de derivar do binding.
    #[test]
    fn params_set_ignores_forged_by_in_payload() {
        let (mut router, sup, _dir) = router_with("params-forge");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("params-forge");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"fanout_gate","scope":"workspace","value":"8","by":"superuser"}"#,
        );
        router.route_message(&m, &mut ts.store, 1, &mut deliver);
        let ev = spawn_payloads(&ts.store, "SystemParamsChanged")
            .pop()
            .expect("evento");
        assert_eq!(
            ev["by"].as_str(),
            Some("human"),
            "by efetivo = carimbo server-side"
        );
        assert_ne!(
            ev["by"].as_str(),
            Some("superuser"),
            "by do payload IGNORADO"
        );
    }

    /// F3-0-7 §C: REDUZIR um laço balanceador abaixo do default sobe ao gate humano (GatedHard/Ask,
    /// não aplica); AUMENTAR e parâmetro NÃO-defensivo aplicam livres. NÃO-VACUOSO pelos 3 casos.
    #[test]
    fn weakening_a_balancing_loop_is_gated_hard() {
        let (mut router, sup, _dir) = router_with("weaken");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("weaken");
        let (_rec, mut deliver) = recorder();

        // (i) REDUZIR delegation_budget (8 → 4, DENTRO da faixa) → GatedHard, NÃO aplica.
        let m_reduce = MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"delegation_budget","scope":"workspace","value":"4"}"#,
        );
        let out = router.route_message(&m_reduce, &mut ts.store, 1, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::ParamsGated { .. }),
            "reduzir laço → gate; veio {out:?}"
        );
        assert!(
            spawn_payloads(&ts.store, "SystemParamsChanged").is_empty(),
            "gated NÃO aplica (nenhum SystemParamsChanged)"
        );
        let gated = spawn_payloads(&ts.store, "ActionGated")
            .pop()
            .expect("ActionGated logado");
        assert_eq!(gated["class"].as_str(), Some("gated-hard"));
        assert_eq!(
            gated["decision"].as_str(),
            Some("ask"),
            "gated-hard = Ask em todo nível"
        );

        // (ii) AUMENTAR delegation_budget (8 → 16) → aplica livre.
        let m_raise = MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"delegation_budget","scope":"workspace","value":"16"}"#,
        );
        let out2 = router.route_message(&m_raise, &mut ts.store, 2, &mut deliver);
        assert!(
            matches!(out2, RouteOutcome::ParamsApplied { .. }),
            "aumentar → aplica; veio {out2:?}"
        );

        // (iii) parâmetro NÃO-defensivo (scrollback_retention_days) → aplica livre.
        let m_nondef = MailMessage::new(
            "@A",
            "@workspace",
            "params.set",
            r#"{"key":"scrollback_retention_days","scope":"workspace","value":"7"}"#,
        );
        let out3 = router.route_message(&m_nondef, &mut ts.store, 3, &mut deliver);
        assert!(
            matches!(out3, RouteOutcome::ParamsApplied { .. }),
            "não-defensivo → aplica; veio {out3:?}"
        );
    }

    /// `effort.assign` resolve o alvo, valida o effort e emite `EffortAssigned{origin:assigned}`
    /// para o NODE resolvido (não o nome cru), com `origin` carimbado server-side.
    #[test]
    fn effort_assign_emits_effort_assigned_for_resolved_node() {
        let (mut router, sup, _dir) = router_with("effort-assign");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("effort-assign");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "effort.assign",
            r#"{"target":"@B","effort":"high"}"#,
        );
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::EffortApplied { .. }),
            "veio {out:?}"
        );
        let ev = spawn_payloads(&ts.store, "EffortAssigned")
            .pop()
            .expect("evento");
        assert_eq!(
            ev["node"].as_str(),
            Some(b.to_string().as_str()),
            "node = alvo RESOLVIDO"
        );
        assert_eq!(ev["effort"].as_str(), Some("high"));
        assert_eq!(
            ev["origin"].as_str(),
            Some("assigned"),
            "verbo = atribuição explícita"
        );
    }

    /// Effort fora do vocabulário → recusa, nada logado.
    #[test]
    fn effort_assign_invalid_effort_is_rejected() {
        let (mut router, sup, _dir) = router_with("effort-bad");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("effort-bad");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "effort.assign",
            r#"{"target":"@B","effort":"altissimo"}"#,
        );
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::EffortRejected(_)),
            "veio {out:?}"
        );
        assert!(spawn_payloads(&ts.store, "EffortAssigned").is_empty());
    }

    /// Alvo inexistente → recusa (não inventa nó), nada logado.
    #[test]
    fn effort_assign_unknown_target_is_rejected() {
        let (mut router, sup, _dir) = router_with("effort-not");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("effort-not");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "effort.assign",
            r#"{"target":"@Fantasma","effort":"low"}"#,
        );
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::EffortRejected(_)),
            "veio {out:?}"
        );
        assert!(spawn_payloads(&ts.store, "EffortAssigned").is_empty());
    }

    /// REGRA-MÃE (ADR 0007): `origin`/`by` forjados no payload são IGNORADOS — `origin` é sempre
    /// `assigned` (server-side) e `by` vem do binding (origem humana → ausente). NÃO-VACUOSO:
    /// quebraria se o handler lesse `payload["origin"]`/`payload["by"]`.
    #[test]
    fn effort_assign_ignores_forged_origin_and_by() {
        let (mut router, sup, _dir) = router_with("effort-forge");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("effort-forge");
        let (_rec, mut deliver) = recorder();
        let m = MailMessage::new(
            "@A",
            "@workspace",
            "effort.assign",
            r#"{"target":"@B","effort":"low","origin":"observed","by":"superuser"}"#,
        );
        router.route_message(&m, &mut ts.store, 1, &mut deliver);
        let ev = spawn_payloads(&ts.store, "EffortAssigned")
            .pop()
            .expect("evento");
        assert_eq!(
            ev["origin"].as_str(),
            Some("assigned"),
            "origin do payload IGNORADO"
        );
        assert_ne!(
            ev["by"].as_str(),
            Some("superuser"),
            "by do payload IGNORADO (server-side)"
        );
    }

    /// GUARDA F3-0-7 §B2 (ADR 0007): um AGENTE em cascata NÃO pode se AUTO-ATRIBUIR effort (gastar
    /// mais) — recusado, nada logado. NÃO-VACUOSO: o MESMO agente atribuindo a OUTRO terminal é
    /// PERMITIDO (cascata legítima, `by=Some(sender)`); a guarda é cirúrgica.
    #[test]
    fn effort_assign_agent_self_assignment_is_rejected() {
        let (mut router, sup, _dir) = router_with("effort-self");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("effort-self");
        let (_rec, mut deliver) = recorder();
        // A→B fixa o binding de B (B vira cascata, hops>=1).
        router.route_message(
            &MailMessage::new("@A", "@B", "ask", "faça X"),
            &mut ts.store,
            1,
            &mut deliver,
        );
        // B tenta se AUTO-atribuir effort high → RECUSADO (auto-promoção).
        let m_self = MailMessage::new(
            "@B",
            "@workspace",
            "effort.assign",
            r#"{"target":"@B","effort":"high"}"#,
        );
        let out = router.route_message(&m_self, &mut ts.store, 2, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::EffortRejected(_)),
            "auto-atribuição recusada; veio {out:?}"
        );
        assert!(
            spawn_payloads(&ts.store, "EffortAssigned").is_empty(),
            "auto-atribuição não loga nada"
        );
        // ...mas B atribuindo a OUTRO (@C) SEGUE permitido (cascata legítima, by = o agente B).
        let m_other = MailMessage::new(
            "@B",
            "@workspace",
            "effort.assign",
            r#"{"target":"@C","effort":"high"}"#,
        );
        let out2 = router.route_message(&m_other, &mut ts.store, 3, &mut deliver);
        assert!(
            matches!(out2, RouteOutcome::EffortApplied { .. }),
            "agente→OUTRO permitido; veio {out2:?}"
        );
        let ev = spawn_payloads(&ts.store, "EffortAssigned")
            .pop()
            .expect("evento p/ @C");
        assert_eq!(
            ev["node"].as_str(),
            Some(c.to_string().as_str()),
            "node = @C resolvido"
        );
        assert_eq!(
            ev["by"].as_str(),
            Some(b.to_string().as_str()),
            "by = o agente que atribuiu (server-side)"
        );
    }

    /// **Critério 1 (caminho feliz, ORIGEM).** Um nó SEM binding (`hops==0`, pedido direto do
    /// humano) → `SpawnApproved` com os args do envelope; `SpawnRequested` logado (intent-vs-action);
    /// `spawn_count` incrementado em `(root, sender)`; e NADA criado no core (sem `NodeAdded` — a
    /// criação física é seam da tela, Maestro decisão 4: o gate prova a DECISÃO, não o efeito).
    #[test]
    fn spawn_origin_is_approved_and_logs_request() {
        let (mut router, sup, dir) = router_with("spawn-origin");
        let a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("spawn-origin");
        let (_rec, mut deliver) = recorder();

        let m = MailMessage::new("@A", "@QA", "spawn", "valide o checkout").with_ref("role:qa");
        let out = router.route_message(&m, &mut ts.store, 1000, &mut deliver);

        match out {
            RouteOutcome::SpawnApproved {
                name,
                role,
                prompt,
                requested_by,
                root_cause_id,
                model,
                effort,
            } => {
                assert_eq!(name, "@QA");
                assert_eq!(role, "qa");
                assert_eq!(prompt, "valide o checkout");
                assert_eq!(
                    requested_by, a,
                    "requested_by = sender AUTENTICADO (não o `from`)"
                );
                assert_eq!(root_cause_id, m.id, "origem: root EFETIVO = msg.id FRESCO");
                // F3-0-3: o spawn-via-agente não transporta model/effort nesta rodada — o payload do
                // agente NÃO os fabrica (server-side `None`); o caminho humano os escolhe no modal.
                assert_eq!(model, None, "model não vem do payload do agente");
                assert_eq!(effort, None, "effort não vem do payload do agente");
            }
            other => panic!("origem deve aprovar; veio {other:?}"),
        }
        // intent-vs-action: o PEDIDO foi logado SEMPRE, com a cadeia EFETIVA.
        let reqs = spawn_payloads(&ts.store, "SpawnRequested");
        assert_eq!(reqs.len(), 1, "exatamente 1 SpawnRequested");
        assert_eq!(reqs[0]["name"].as_str(), Some("@QA"));
        assert_eq!(reqs[0]["role"].as_str(), Some("qa"));
        assert_eq!(reqs[0]["hops"].as_u64(), Some(0));
        assert_eq!(
            reqs[0]["requested_by"].as_str(),
            Some(a.to_string().as_str())
        );
        // O core NÃO cria o nó (seam da tela): nenhum NodeAdded; origem não é gated.
        assert!(
            spawn_payloads(&ts.store, "NodeAdded").is_empty(),
            "Rodada 3 é CORE-só: o gate NÃO cria o nó (admit_node é seam da tela)"
        );
        assert!(
            spawn_payloads(&ts.store, "SpawnGated").is_empty(),
            "origem aprovada não gera SpawnGated"
        );
        // Cap: o spawn aprovado contabiliza `spawn_count` em (root, sender).
        let n = router
            .budget
            .get(&(m.id.clone(), a))
            .map(|e| e.spawn_count)
            .unwrap_or(0);
        assert_eq!(n, 1, "spawn aprovado incrementa o spawn_count");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 2 (cascata gated).** Um worker que RECEBEU um handoff (tem binding
    /// `delivered_root`) tenta criar terminal → `SpawnGated{cascade}` SEMPRE; evento logado; nada
    /// aprovado. É a defesa central anti-fork-bomb (ADR 0007): spawn-de-spawn pede gesto humano.
    #[test]
    fn spawn_cascade_requires_human_gate() {
        let (mut router, sup, dir) = router_with("spawn-casc");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("spawn-casc");
        let (_rec, mut deliver) = recorder();

        // A→B fixa o binding de B (B agora está numa cadeia → `hops>=1`).
        let m_ab = MailMessage::new("@A", "@B", "ask", "faça X");
        router.route_message(&m_ab, &mut ts.store, 1, &mut deliver);

        // B tenta spawnar @Helper → cascata.
        let m_spawn = MailMessage::new("@B", "@Helper", "spawn", "ajude").with_ref("role:helper");
        let out = router.route_message(&m_spawn, &mut ts.store, 2, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::SpawnGated {
                reason: "cascade".into()
            },
            "cascata exige gate humano SEMPRE"
        );
        let gated = spawn_payloads(&ts.store, "SpawnGated");
        assert_eq!(gated.len(), 1);
        assert_eq!(gated[0]["reason"].as_str(), Some("cascade"));
        assert_eq!(
            gated[0]["requested_by"].as_str(),
            Some(b.to_string().as_str()),
            "requested_by autenticado = B (sender), não o campo `from`"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 3 (NÃO-FORJABILIDADE — irmão do `f005324`).** Com binding presente, um worker
    /// FORJA `hops=0` + `root_cause_id` fresco no envelope (o outbox é canal NÃO-autenticado) →
    /// AINDA `SpawnGated{cascade}`. **NÃO-VACUOSO:** quebraria se o gate relesse `msg.hops`/
    /// `msg.root_cause_id` em vez do binding inforjável (`derive_root_hops`).
    #[test]
    fn spawn_gate_ignores_forged_hops_and_root() {
        let (mut router, sup, dir) = router_with("spawn-forge");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("spawn-forge");
        let (_rec, mut deliver) = recorder();

        let m_ab = MailMessage::new("@A", "@B", "ask", "cadeia");
        router.route_message(&m_ab, &mut ts.store, 1, &mut deliver);

        // B forja "sou origem fresca" nos campos PÚBLICOS/forjáveis do outbox.
        let mut m_forge =
            MailMessage::new("@B", "@Helper", "spawn", "fork").with_ref("role:helper");
        m_forge.hops = 0; // forjado: "sou origem"
        m_forge.root_cause_id = Some("msg_forjado_root_fresco".into()); // forjado: root novo
        let out = router.route_message(&m_forge, &mut ts.store, 2, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::SpawnGated {
                reason: "cascade".into()
            },
            "o gate lê o binding (B recebeu handoff), JAMAIS os campos forjados → segue cascata"
        );
        // Prova não-vacuosa: o root/hops EFETIVOS logados vêm do BINDING, não do payload forjado.
        let last = spawn_payloads(&ts.store, "SpawnRequested")
            .pop()
            .expect("SpawnRequested");
        assert_eq!(
            last["root_cause_id"].as_str(),
            Some(m_ab.id.as_str()),
            "root EFETIVO = id de A (binding), não o forjado"
        );
        assert_eq!(
            last["hops"].as_u64(),
            Some(1),
            "hops EFETIVO = 1 (cascata), não o forjado 0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 4 (cap).** Com `max_spawns_per_turn` spawns já contabilizados sob `(root, sender)`,
    /// o próximo (mesmo root) → `SpawnGated{over_cap}`. Semeado direto no `budget` (mesmo módulo) p/
    /// provar o cap NO LIMIAR — NÃO-VACUOSO: passa só se o gate consulta `spawn_count` contra
    /// `max_spawns_per_turn`.
    #[test]
    fn spawn_over_cap_is_gated() {
        let (mut router, sup, dir) = router_with("spawn-cap");
        let a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("spawn-cap");
        let (_rec, mut deliver) = recorder();

        let m = MailMessage::new("@A", "@C", "spawn", "terceiro").with_ref("role:helper");
        // Origem → root = m.id. Pré-semeia o cap (2) sob (m.id, a); last_ms=now p/ não ser podado.
        router.budget.insert(
            (m.id.clone(), a),
            BudgetEntry {
                count: 0,
                spawn_count: MAX_SPAWNS_PER_TURN,
                last_ms: 1000,
            },
        );
        let out = router.route_message(&m, &mut ts.store, 1000, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::SpawnGated {
                reason: "over_cap".into()
            },
            "spawn no mesmo root acima do cap é gateado"
        );
        let gated = spawn_payloads(&ts.store, "SpawnGated");
        assert_eq!(gated.last().unwrap()["reason"].as_str(), Some("over_cap"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Critério 4b + Maestro decisão 2 (fiação da autonomia, provada HEADLESS).** `RouterConfig`
    /// com `autonomy=Manual` → `handle_spawn` recusa no CORE com `SpawnBlocked` (sem PTY) e loga
    /// `SpawnGated{manual}`. Prova que a matriz de autonomia no `RouterConfig` GOVERNA o gate (sem
    /// ela, manual seria fake — ver GAP em `bridge.rs` na entrega).
    #[test]
    fn spawn_manual_autonomy_blocks_in_core() {
        let dir = std::env::temp_dir().join(format!("lina-spawn-manual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sup = Arc::new(Supervisor::new());
        let _a = sup.register("@A", None, sink());
        let cfg = RouterConfig {
            autonomy: AutonomyLevel::Manual,
            ..RouterConfig::default()
        };
        let mut router =
            Router::with_config(Arc::clone(&sup), Mailbox::new(dir.join(".lina")), cfg);
        let mut ts = TmpStore::new("spawn-manual");
        let (_rec, mut deliver) = recorder();

        let m = MailMessage::new("@A", "@QA", "spawn", "x").with_ref("role:qa");
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::SpawnBlocked,
            "manual recusa o spawn no core"
        );
        let gated = spawn_payloads(&ts.store, "SpawnGated");
        assert_eq!(
            gated.last().unwrap()["reason"].as_str(),
            Some("manual"),
            "o fato é registrado no log (SpawnGated{{manual}})"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Custo (ADR 0005).** Workspace cost-`Paused` (semeado via `CostCeilingHit` no log) → spawn
    /// de ORIGEM `SpawnGated{cost}` (o terminal novo gastaria sob teto estourado). Reusa `CostLedger`.
    #[test]
    fn spawn_gated_when_cost_paused() {
        let now: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("lina-spawn-cost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sup = Arc::new(Supervisor::new());
        let _a = sup.register("@A", None, sink());
        let cfg = RouterConfig {
            token_budget_day: 100,
            ..RouterConfig::default()
        };
        let mut router =
            Router::with_config(Arc::clone(&sup), Mailbox::new(dir.join(".lina")), cfg);
        let mut ts = TmpStore::new("spawn-cost");
        let (_rec, mut deliver) = recorder();

        // Semeia Paused: uso > teto + CostCeilingHit do dia → o ledger reconstrói paused=true.
        ts.store
            .append(&DomainEvent::TokenUsageReported {
                node: "@A".into(),
                tokens: 150,
            })
            .unwrap();
        ts.store
            .append(&DomainEvent::CostCeilingHit {
                day: utc_day(now),
                tokens: 150,
            })
            .unwrap();

        let m = MailMessage::new("@A", "@QA", "spawn", "x").with_ref("role:qa");
        let out = router.route_message(&m, &mut ts.store, now, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::SpawnGated {
                reason: "cost".into()
            },
            "spawn sob workspace cost-paused é gateado"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Defesa de forma.** Envelope de spawn sem `role` → `SpawnBlocked` (o bin recusa antes de
    /// enfileirar; este é o backstop do router). NÃO loga SpawnRequested (pedido malformado, não é
    /// um spawn real).
    #[test]
    fn spawn_missing_role_is_blocked() {
        let (mut router, sup, dir) = router_with("spawn-noargs");
        let _a = sup.register("@A", None, sink());
        let mut ts = TmpStore::new("spawn-noargs");
        let (_rec, mut deliver) = recorder();

        let m = MailMessage::new("@A", "@QA", "spawn", "x"); // sem `--role` (sem ref)
        let out = router.route_message(&m, &mut ts.store, 1, &mut deliver);
        assert_eq!(
            out,
            RouteOutcome::SpawnBlocked,
            "spawn sem papel é recusado"
        );
        assert!(
            spawn_payloads(&ts.store, "SpawnRequested").is_empty(),
            "pedido malformado não vira SpawnRequested"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────── ACHADO-2: prompt-não-pronto = OCUPADO (retenção), nunca falha (strike/DLQ) ───────

    /// **Critério ACHADO-2 (não-vacuoso).** Alvo Idle no lifecycle MAS grid não-pronto (a TUI está
    /// num turno longo) → o `deliver` devolve `Ok(Injected{ready:false})` (NADA injetado). O router
    /// RETÉM (`MessageRetained{prompt_not_ready}` + `Retained`): ZERO `MessageDeliveryFailed`/
    /// `MessageDeadLettered`/`CircuitOpened`, ZERO strike. Mensagem VIVA não morre por o colega
    /// estar trabalhando (o bug: seqs 32-40 do log do gate eram retry→5-strikes→DLQ).
    #[test]
    fn delivery_not_ready_is_retained_not_failed() {
        let (mut router, sup, dir) = router_with("a2-nr-retain");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b); // lifecycle Idle (exatamente o caso do bug)
        let mut ts = TmpStore::new("a2-nr-retain");
        let calls = Rc::new(RefCell::new(0u32));
        let c2 = Rc::clone(&calls);
        let mut deliver = move |_t: NodeId, _f: NodeId, _x: &str| {
            *c2.borrow_mut() += 1;
            Ok(DeliveryOutcome::Injected {
                ready: false,
                bracketed: false,
            })
        };

        let m = MailMessage::new("@A", "@B", "ask", "viva");
        let out = router.route_message(&m, &mut ts.store, 1000, &mut deliver);

        assert_eq!(
            out,
            RouteOutcome::Retained { to: b },
            "não-pronto = RETIDA, não falha"
        );
        assert_eq!(
            *calls.borrow(),
            1,
            "tentou entregar 1× (descobriu o ocupado REAL)"
        );
        let ret = records_of_kind(&ts.store, "MessageRetained");
        assert_eq!(
            ret.len(),
            1,
            "exatamente 1 MessageRetained (anti-amplificação A4)"
        );
        assert_eq!(ret[0].payload["reason"].as_str(), Some("prompt_not_ready"));
        assert!(
            records_of_kind(&ts.store, "MessageDeliveryFailed").is_empty(),
            "prompt-não-pronto NÃO é falha de entrega"
        );
        assert!(
            records_of_kind(&ts.store, "MessageDeadLettered").is_empty(),
            "sem DLQ"
        );
        assert!(
            records_of_kind(&ts.store, "CircuitOpened").is_empty(),
            "sem breaker"
        );
        assert!(
            !router.node_failures.contains_key(&b),
            "prompt-não-pronto NÃO conta strike (era o bug)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Grid pronto → entrega 1×, sem retenção. (O caminho feliz não regride.)
    #[test]
    fn delivery_ready_delivers_once() {
        let (mut router, sup, dir) = router_with("a2-nr-ready");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b);
        let mut ts = TmpStore::new("a2-nr-ready");
        let calls = Rc::new(RefCell::new(0u32));
        let c2 = Rc::clone(&calls);
        let mut deliver = move |_t: NodeId, _f: NodeId, _x: &str| {
            *c2.borrow_mut() += 1;
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        };

        let m = MailMessage::new("@A", "@B", "ask", "ok");
        let out = router.route_message(&m, &mut ts.store, 1000, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::Delivered { .. }),
            "pronto → entregue"
        );
        assert_eq!(*calls.borrow(), 1);
        assert_eq!(records_of_kind(&ts.store, "MessageDelivered").len(), 1);
        assert!(records_of_kind(&ts.store, "MessageRetained").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Regressão NÃO-VACUOSA:** uma falha REAL de entrega (escritor morto/erro de escrita) AINDA
    /// vira strike (`MessageDeliveryFailed` + `node_failures`), NÃO retenção. Quebraria se o fix
    /// reclassificasse TODO problema de entrega como retenção (o oposto do bug).
    #[test]
    fn real_delivery_failure_still_strikes() {
        let (mut router, sup, dir) = router_with("a2-nr-fail");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b);
        let mut ts = TmpStore::new("a2-nr-fail");
        let mut deliver = |_t: NodeId, _f: NodeId, _x: &str| -> Result<DeliveryOutcome, String> {
            Err("escritor morto".into())
        };

        let m = MailMessage::new("@A", "@B", "ask", "vai falhar");
        let out = router.route_message(&m, &mut ts.store, 1000, &mut deliver);
        assert!(
            matches!(out, RouteOutcome::RetryBackoff { .. }),
            "falha REAL entra no motor de retry/strike, obteve {out:?}"
        );
        assert_eq!(
            records_of_kind(&ts.store, "MessageDeliveryFailed").len(),
            1,
            "falha real é STRIKE (livro-razão)"
        );
        assert_eq!(
            router.node_failures.get(&b).copied(),
            Some(1),
            "strike contabilizado"
        );
        assert!(
            records_of_kind(&ts.store, "MessageRetained").is_empty(),
            "falha REAL não é retenção (não confundir com ocupado)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// O ciclo reter→drain: um not-ready retido vira entrada no motor de re-entrega e, quando o grid
    /// fica pronto (após o backoff), `process_due_retries` ENTREGA 1× e limpa a entrada — SEM strike.
    #[test]
    fn retained_not_ready_delivers_when_ready_via_retry_engine() {
        let (mut router, sup, dir) = router_with("a2-nr-cycle");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b);
        let mut ts = TmpStore::new("a2-nr-cycle");

        // 1) not-ready → retém (entrada no motor de re-entrega, backoff agendado).
        let mut not_ready = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: false,
                bracketed: false,
            })
        };
        let m = MailMessage::new("@A", "@B", "ask", "espera o turno");
        let out = router.route_message(&m, &mut ts.store, 1000, &mut not_ready);
        assert_eq!(out, RouteOutcome::Retained { to: b });
        assert!(
            router.retries.contains_key(&m.id),
            "retenção por not-ready vira entrada no motor de re-entrega (bypassa o ledger)"
        );
        assert!(
            !router.node_failures.contains_key(&b),
            "retenção não strike enquanto espera"
        );

        // 2) grid pronto após o backoff → process_due_retries ENTREGA e limpa.
        let mut ready = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        };
        router.process_due_retries(&mut ts.store, 1000 + NOT_READY_BACKOFF_MS + 1, &mut ready);
        assert!(
            !router.retries.contains_key(&m.id),
            "entregue quando pronto → entrada removida"
        );
        let delivered = records_of_kind(&ts.store, "MessageDelivered")
            .into_iter()
            .filter(|r| r.payload["id"].as_str() == Some(m.id.as_str()))
            .count();
        assert_eq!(delivered, 1, "entregue exatamente 1× quando o turno acabou");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─────── SEAM-2 (R1): retenção por not-ready é DURÁVEL no restart (inv#4) ───────

    /// **SEAM-2 (R1) — critério (a), NÃO-VACUOSO.** Roteia → retém (not-ready) → "restart"
    /// (Router NOVO sobre o MESMO store+mailbox, `self.retries` em memória zerado) → a mensagem VIVA
    /// é RE-RETIDA (não cai no braço "roteada-sem-entrega ⇒ assume entregue ⇒ ack ⇒ DROP" do ledger)
    /// e, quando o alvo fica pronto, entregue EXATAMENTE 1× (dedupe). RED (pré-fix): o restart
    /// retornava `Duplicate` (assumia entregue) e ackeava o `.inflight` — a msg evaporava.
    #[test]
    fn retained_not_ready_survives_restart_and_delivers_once() {
        let (mut router, sup, dir) = router_with("seam2-a");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b); // lifecycle Idle; o grid (deliver) é quem diz não-pronto
        let mut ts = TmpStore::new("seam2-a");
        let mut not_ready = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: false,
                bracketed: false,
            })
        };

        let m = MailMessage::new("@A", "@B", "ask", "viva no crash");
        router.mailbox().enqueue_as("@A", &m).expect("enqueue");
        let out1 = router.pump(&mut ts.store, 1000, &mut not_ready);
        assert_eq!(
            out1,
            vec![(m.id.clone(), RouteOutcome::Retained { to: b })],
            "pré-restart: retida (not-ready)"
        );
        let infl = router
            .mailbox()
            .inflight_dir()
            .join(format!("{}.json", m.id));
        assert!(
            infl.exists(),
            "retida fica DURÁVEL no .inflight (não ackeada)"
        );

        // "Restart": descarta o Router (perde self.retries em memória); um NOVO sobre o MESMO log+mailbox.
        drop(router);
        let mut router2 = Router::new(Arc::clone(&sup), Mailbox::new(dir.join(".lina")));
        let r1 = router2.pump(&mut ts.store, 2000, &mut not_ready);
        assert_eq!(
            r1,
            vec![(m.id.clone(), RouteOutcome::Retained { to: b })],
            "pós-restart: RE-RETIDA (NÃO 'Duplicate'=assume-entregue=dropada) — o fix do R1"
        );
        assert!(
            router2.retries.contains_key(&m.id),
            "re-semeada no motor de re-entrega a partir do ledger"
        );
        assert!(
            infl.exists(),
            "segue DURÁVEL no .inflight pós-restart (NÃO ackeada/dropada)"
        );
        assert!(
            records_of_kind(&ts.store, "MessageDelivered").is_empty(),
            "ainda não entregue (alvo segue não-pronto)"
        );

        // Alvo fica pronto após o backoff → entrega 1× e limpa (o ciclo retoma exatamente como sem crash).
        let mut ready = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        };
        router2.process_due_retries(&mut ts.store, 2000 + NOT_READY_BACKOFF_MS + 1, &mut ready);
        assert!(
            !router2.retries.contains_key(&m.id),
            "entregue → entrada removida do motor"
        );
        let delivered = records_of_kind(&ts.store, "MessageDelivered")
            .into_iter()
            .filter(|r| r.payload["id"].as_str() == Some(m.id.as_str()))
            .count();
        assert_eq!(
            delivered, 1,
            "entregue EXATAMENTE 1× após o restart (sem duplicar)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **SEAM-2 (R1) — critério (b), CONTROLE de não-regressão + ordem dos braços.** Uma msg cuja
    /// entrega JÁ foi LOGADA (`MessageDelivered`) — mesmo que o `.inflight` ainda exista (crash entre
    /// logar e ackear) — NÃO é re-retida nem re-entregue no restart: o braço "entregue" do ledger
    /// vence o de "retida". Garante que o fix do R1 não ressuscita mensagem já entregue (anti
    /// re-injeção irreversível, D1).
    #[test]
    fn delivered_before_restart_is_not_re_retained() {
        let (mut router, sup, dir) = router_with("seam2-b");
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b);
        let mut ts = TmpStore::new("seam2-b");
        let mut not_ready = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: false,
                bracketed: false,
            })
        };

        // Retém (MessageRouted + MessageRetained no log; msg DURÁVEL no .inflight).
        let m = MailMessage::new("@A", "@B", "ask", "ja entregue");
        router.mailbox().enqueue_as("@A", &m).expect("enqueue");
        let _ = router.pump(&mut ts.store, 1000, &mut not_ready);
        let infl = router
            .mailbox()
            .inflight_dir()
            .join(format!("{}.json", m.id));
        assert!(infl.exists(), "retida no .inflight");
        // Simula: o alvo ficou pronto, a entrega foi LOGADA, mas o crash veio ANTES do ack
        // (a msg segue no .inflight com `MessageDelivered` no log — o estado-limite do ledger).
        ts.store
            .append(&DomainEvent::MessageDelivered {
                id: m.id.clone(),
                to: b,
            })
            .expect("append delivered");

        drop(router);
        let mut router2 = Router::new(Arc::clone(&sup), Mailbox::new(dir.join(".lina")));
        let calls = Rc::new(RefCell::new(0u32));
        let c2 = Rc::clone(&calls);
        let mut counting = move |_t: NodeId, _f: NodeId, _x: &str| {
            *c2.borrow_mut() += 1;
            Ok(DeliveryOutcome::Injected {
                ready: true,
                bracketed: true,
            })
        };
        let r = router2.pump(&mut ts.store, 2000, &mut counting);
        assert_eq!(
            r,
            vec![(m.id.clone(), RouteOutcome::Duplicate)],
            "entregue-no-log ⇒ no-op (ack), NÃO re-retida (braço 'entregue' vence 'retida')"
        );
        assert!(
            router2.retries.is_empty(),
            "NÃO re-semeia uma msg já entregue (ordem dos braços do ledger)"
        );
        assert_eq!(
            *calls.borrow(),
            0,
            "ZERO re-entrega (anti re-injeção irreversível)"
        );
        assert!(!infl.exists(), "resolvida no restart → .inflight ackeado");
        let delivered = records_of_kind(&ts.store, "MessageDelivered")
            .into_iter()
            .filter(|r| r.payload["id"].as_str() == Some(m.id.as_str()))
            .count();
        assert_eq!(delivered, 1, "segue 1 entrega (sem duplicar pós-restart)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **SEAM-2 (R1) — critério (c), NÃO-VACUOSO.** O backstop de 600 s (DLQ) conta o tempo TOTAL
    /// desde a retenção ORIGINAL (restaurado do `ts` do `MessageRetained` no log), NÃO reinicia no
    /// restart. Sem isso, uma sequência de restarts adiaria a DLQ para sempre. Distingue da escolha
    /// "reseta no restart": se o relógio reiniciasse, no instante testado NÃO haveria DLQ.
    #[test]
    fn retention_backstop_counts_total_time_across_restart() {
        let dir = std::env::temp_dir().join(format!(
            "lina-router-mb-seam2c-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let sup = Arc::new(Supervisor::new());
        let _a = sup.register("@A", None, sink());
        let b = sup.register("@B", None, sink());
        turn_done(&sup, b);
        let mk_cfg = || RouterConfig {
            retention_timeout_ms: 5_000, // > NOT_READY_BACKOFF_MS (2s) para a re-checagem ficar devida
            ..RouterConfig::default()
        };
        let mut router =
            Router::with_config(Arc::clone(&sup), Mailbox::new(dir.join(".lina")), mk_cfg());
        let mut ts = TmpStore::new("seam2-c");
        let mut not_ready = |_t: NodeId, _f: NodeId, _x: &str| {
            Ok(DeliveryOutcome::Injected {
                ready: false,
                bracketed: false,
            })
        };

        let m = MailMessage::new("@A", "@B", "ask", "espera longa");
        router.mailbox().enqueue_as("@A", &m).expect("enqueue");
        let _ = router.pump(&mut ts.store, 1000, &mut not_ready);
        // `ts` do MessageRetained = instante REAL (wall-clock) da retenção original — a âncora durável.
        let rt = records_of_kind(&ts.store, "MessageRetained")
            .into_iter()
            .find(|r| r.payload["id"].as_str() == Some(m.id.as_str()))
            .expect("MessageRetained logado")
            .ts;

        // "Restart" ~4 s após a retenção original (tempo PASSOU antes do crash).
        drop(router);
        let mut router2 =
            Router::with_config(Arc::clone(&sup), Mailbox::new(dir.join(".lina")), mk_cfg());
        let r = router2.pump(&mut ts.store, rt + 4_000, &mut not_ready);
        assert_eq!(
            r,
            vec![(m.id.clone(), RouteOutcome::Retained { to: b })],
            "re-retida no restart (relógio de retenção restaurado do log)"
        );

        // Re-checagem fica devida (next_ms = rt+4000+backoff); o backstop mede do `rt` ORIGINAL:
        // total = (rt+6001) - rt = 6001 ms >= 5000 → DLQ. Se o relógio tivesse RESETADO no restart,
        // o total seria 6001-4000 = 2001 ms < 5000 → SEM DLQ (o que provaria a regressão).
        router2.process_due_retries(
            &mut ts.store,
            rt + 4_000 + NOT_READY_BACKOFF_MS + 1,
            &mut not_ready,
        );
        let dlq = records_of_kind(&ts.store, "MessageDeadLettered")
            .into_iter()
            .filter(|r| r.payload["id"].as_str() == Some(m.id.as_str()))
            .count();
        assert_eq!(
            dlq, 1,
            "backstop conta o TEMPO TOTAL desde a retenção original (não reinicia no restart)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **SEAM-1 / M4 (NÃO-VACUOSO):** um nó cujo binding foi SEMEADO (`seed_delivered_root` — o filho
    /// de um spawn) tem seus PRÓPRIOS spawns tratados como CASCATA (gate humano), não origem. Sem o
    /// seed, seria origem (aprovado direto) — derrotando o anti-fork-bomb end-to-end. Controle
    /// positivo (sem seed → aprovado) + prova (com seed → gated) na MESMA execução.
    #[test]
    fn seeded_binding_makes_child_spawn_cascade() {
        let (mut router, sup, dir) = router_with("seed-cascade");
        let c = sup.register("@C", None, sink()); // o "filho" spawnado
        let mut ts = TmpStore::new("seed-cascade");
        let (_rec, mut deliver) = recorder();

        // CONTROLE (não-vacuosidade): SEM binding semeado, @C é ORIGEM → spawn de @neto é APROVADO.
        let m0 = MailMessage::new("@C", "@neto", "spawn", "trabalhe").with_ref("role:helper");
        assert!(
            matches!(
                router.route_message(&m0, &mut ts.store, 1000, &mut deliver),
                RouteOutcome::SpawnApproved { .. }
            ),
            "controle: sem binding, @C é origem → aprovado (hoje seria assim p/ TODO filho)"
        );

        // COM o seed (o filho nasceu de um spawn de root R, hops 0 — o app semeia na admissão):
        router.seed_delivered_root(c, "msg_root_do_pai".into(), 0, 2000);
        let m1 = MailMessage::new("@C", "@neto2", "spawn", "trabalhe").with_ref("role:helper");
        assert_eq!(
            router.route_message(&m1, &mut ts.store, 3000, &mut deliver),
            RouteOutcome::SpawnGated {
                reason: "cascade".into()
            },
            "com binding semeado, @C é CASCATA → gate humano (anti-fork-bomb END-TO-END)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1 (anti-envenenamento) preservado na porta: `seed_delivered_root` NÃO sobrescreve um binding
    /// VIVO de root DIFERENTE (mesma defesa de `route_message`); mesmo root re-semeia (idempotente).
    #[test]
    fn seed_delivered_root_respects_p1() {
        let (mut router, sup, dir) = router_with("seed-p1");
        let c = sup.register("@C", None, sink());
        router.seed_delivered_root(c, "R1".into(), 0, 1000);
        router.seed_delivered_root(c, "R2".into(), 0, 2000); // root diferente, vivo → recusa (P1)
        let (root, _h) = router.derive_root_hops(&MailMessage::new("@C", "@x", "ask", ""), c);
        assert_eq!(
            root, "R1",
            "P1: binding vivo de root diferente NÃO é sobrescrito"
        );
        router.seed_delivered_root(c, "R1".into(), 0, 3000); // mesmo root → idempotente
        let (root2, _h2) = router.derive_root_hops(&MailMessage::new("@C", "@x", "ask", ""), c);
        assert_eq!(root2, "R1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────── F3-1: juiz estrutural + OUTER loop ─────────────────────────

    fn crit_cmd(desc: &str, cmd: &str) -> AcceptanceCriterion {
        AcceptanceCriterion {
            desc: desc.into(),
            check_kind: CheckKind::Command,
            check_arg: Some(cmd.into()),
        }
    }

    /// F3-1-2: `Command` exit 0 → Pass; exit != 0 → Fail com evidência observada + defect_class.
    #[test]
    fn acceptance_command_exit0_passa_exit1_falha() {
        let (v, _, _) =
            run_acceptance(&[crit_cmd("ok", "true")], AutonomyLevel::Assisted).expect("decide");
        assert_eq!(v, GoalVerdict::Pass, "exit 0 passa");

        let (v, evidence, defect) =
            run_acceptance(&[crit_cmd("x", "false")], AutonomyLevel::Assisted).expect("decide");
        assert_eq!(v, GoalVerdict::Fail, "exit != 0 falha");
        assert!(!evidence.is_empty(), "evidência observada do exit != 0");
        assert_eq!(defect, "criterio_nao_cumprido");
    }

    /// F3-1-2 (spec 52 §Seg 3): o juiz NÃO roda ação irreversível para verificar — `git push origin
    /// main` (GatedHard) degrada a Fail mesmo em autônomo (o gate nunca afrouxa hard).
    #[test]
    fn acceptance_gated_hard_nao_e_executado_pelo_juiz() {
        let (v, evidence, _) = run_acceptance(
            &[crit_cmd("deploy", "git push origin main")],
            AutonomyLevel::Autonomous,
        )
        .expect("decide");
        assert_eq!(v, GoalVerdict::Fail, "GatedHard não é executado pelo juiz");
        assert!(
            evidence.contains("humana") || evidence.contains("gate"),
            "evidência diz que exige confirmação humana: {evidence}"
        );
    }

    /// F3-1-2: `HumanReview` DEFERE (o juiz estrutural não dá veredito automático — exige gate).
    #[test]
    fn acceptance_human_review_defere() {
        let crit = AcceptanceCriterion {
            desc: "olhar a tela".into(),
            check_kind: CheckKind::HumanReview,
            check_arg: None,
        };
        assert!(
            run_acceptance(&[crit], AutonomyLevel::Assisted).is_none(),
            "HumanReview defere ao humano (None)"
        );
    }

    /// F3-1-2: todos os critérios precisam passar (Pass sse TODOS exit 0; o 1º Fail interrompe).
    #[test]
    fn acceptance_exige_todos_os_criterios() {
        let crits = [crit_cmd("a", "true"), crit_cmd("b", "false")];
        let (v, _, _) = run_acceptance(&crits, AutonomyLevel::Assisted).expect("decide");
        assert_eq!(v, GoalVerdict::Fail, "um critério falho derruba o conjunto");
    }

    /// Semeia uma Goal + itens atribuídos (com `acceptance`) no store, e dá `claim` do `@Dev` em cada.
    /// Devolve o router/sup/store/deliver prontos para os `plan.check` do OUTER loop.
    fn seed_goal(
        tag: &str,
        items: &[(&str, &str)], // (id, comando de acceptance)
    ) -> (Router, Arc<Supervisor>, TmpStore, std::path::PathBuf) {
        let (mut router, sup, dir) = router_with(tag);
        let _dev = sup.register("@Dev", None, sink());
        let mut ts = TmpStore::new(tag);
        let (_rec, mut deliver) = recorder();
        ts.store
            .append(&DomainEvent::GoalDefined {
                goal_id: "g".into(),
                statement: "meta de teste".into(),
                root_cause_id: "r".into(),
                origin: "@Maestro".into(),
                budget_tokens: 0,
            })
            .unwrap();
        for (i, (id, cmd)) in items.iter().enumerate() {
            let t = 1000u64 + i as u64;
            router.seed_plan_item(&mut ts.store, *id, "tarefa").unwrap();
            ts.store
                .append(&DomainEvent::PlanItemAttributed {
                    item: (*id).into(),
                    goal_id: Some("g".into()),
                    parents: vec![],
                    acceptance: vec![crit_cmd("criterio", cmd)],
                    budget_tokens: 0,
                    paths: vec![],
                })
                .unwrap();
            router.route_message(
                &MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref(format!("plan:{id}")),
                &mut ts.store,
                t,
                &mut deliver,
            );
        }
        (router, sup, ts, dir)
    }

    fn check(router: &mut Router, ts: &mut TmpStore, item: &str, t: u64) {
        let (_rec, mut deliver) = recorder();
        router.route_message(
            &MailMessage::new("@Dev", "plan", "plan.check", "").with_ref(format!("plan:{item}")),
            &mut ts.store,
            t,
            &mut deliver,
        );
    }

    fn recs_of<'a>(events: &'a [EventRecord], kind: &str) -> Vec<&'a EventRecord> {
        events.iter().filter(|e| e.kind == kind).collect()
    }
    fn field<'a>(rec: &'a EventRecord, k: &str) -> Option<&'a str> {
        rec.payload.get(k).and_then(serde_json::Value::as_str)
    }

    /// Gate (b): Goal com 2 itens exit-0 fecha com EXATAMENTE um `GoalAchieved` ao 2º Pass, e NENHUM
    /// re-spawn/despacho depois. (Inclui gate d parte 1: todo ReviewVerdict tem reviewer != target.)
    #[test]
    fn gate_b_goal_fecha_so_com_todos_pass_sem_despacho_depois() {
        let (mut router, _sup, mut ts, dir) =
            seed_goal("gate-b", &[("T1", "true"), ("T2", "true")]);
        check(&mut router, &mut ts, "T1", 2000); // Pass, mas T2 pendente → não fecha.
        check(&mut router, &mut ts, "T2", 2001); // Pass, todos → GoalAchieved.

        let events = ts.store.events().unwrap();
        let achieved = recs_of(&events, "GoalAchieved");
        assert_eq!(achieved.len(), 1, "exatamente um GoalAchieved");
        assert_eq!(
            achieved[0]
                .payload
                .get("iterations")
                .and_then(serde_json::Value::as_u64),
            Some(1),
            "fechou na 1ª volta de cada item"
        );
        let achieved_seq = achieved[0].seq;
        assert!(
            !events
                .iter()
                .any(|e| e.seq > achieved_seq && e.kind == "SpawnRequested"),
            "nenhum re-spawn após GoalAchieved"
        );
        let verdicts = recs_of(&events, "ReviewVerdict");
        assert_eq!(verdicts.len(), 2, "um veredito por item");
        for v in &verdicts {
            assert_ne!(
                field(v, "reviewer"),
                field(v, "target"),
                "gate d: juiz != executor em todo veredito"
            );
            assert_eq!(field(v, "verdict"), Some("Pass"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Gate (c): critério que SEMPRE falha → após `goal_max_iterations` (3) Fail, exatamente um
    /// `GoalEscalated{turn_budget_exhausted}` e ZERO iteração a mais.
    #[test]
    fn gate_c_turn_budget_freia_apos_max_iterations() {
        let (mut router, _sup, mut ts, dir) = seed_goal("gate-c", &[("T1", "false")]);
        for t in [2000u64, 2001, 2002] {
            check(&mut router, &mut ts, "T1", t);
        }
        let events = ts.store.events().unwrap();
        let fails = events
            .iter()
            .filter(|e| e.kind == "ReviewVerdict" && field(e, "verdict") == Some("Fail"))
            .count();
        assert_eq!(fails, 3, "exatamente 3 Fail (turn-budget = 3)");
        let escalated = recs_of(&events, "GoalEscalated");
        assert_eq!(escalated.len(), 1, "um GoalEscalated");
        assert_eq!(field(escalated[0], "reason"), Some("turn_budget_exhausted"));
        let esc_seq = escalated[0].seq;
        assert!(
            !events.iter().any(|e| e.seq > esc_seq
                && matches!(
                    e.kind.as_str(),
                    "ReviewVerdict" | "GoalAchieved" | "SpawnRequested"
                )),
            "zero iteração após GoalEscalated"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Gate (d), parte 2 (adversarial): um `ReviewVerdict` forjado com `reviewer == target` é RECUSADO
    /// pelo supervisor (não loga) — o executor nunca se auto-avalia.
    #[test]
    fn gate_d_review_verdict_recusa_auto_avaliacao() {
        let mut ts = TmpStore::new("gate-d");
        let dev = NodeId::from_u128(1);
        let qa = NodeId::from_u128(2);

        let forged = emit_review_verdict(
            &mut ts.store,
            "g",
            "T1",
            dev,
            dev,
            GoalVerdict::Pass,
            "",
            "",
            1,
        )
        .unwrap();
        assert!(forged.is_none(), "reviewer == target é recusado");
        assert_eq!(
            recs_of(&ts.store.events().unwrap(), "ReviewVerdict").len(),
            0,
            "nada logado quando recusado"
        );

        let ok = emit_review_verdict(
            &mut ts.store,
            "g",
            "T1",
            dev,
            qa,
            GoalVerdict::Pass,
            "",
            "",
            1,
        )
        .unwrap();
        assert!(ok.is_some(), "reviewer != target é aceito");
    }

    /// Gate (e): a cada Fail abaixo do turn-budget, o re-spawn SOBE um degrau da `effort_ladder` — a
    /// escala estritamente maior do doc-fonte (linha 12). (A asserção `Ord`-nativa entra quando
    /// `Effort` derivar `Ord` — escalado ao Maestro; aqui a escala é provada pelos degraus emitidos.)
    #[test]
    fn gate_e_re_spawn_escala_effort_um_degrau_por_volta() {
        let (mut router, _sup, mut ts, dir) = seed_goal("gate-e", &[("T1", "false")]);
        // escada de 3 degraus + teto alto p/ ver 2 re-spawns crescentes antes do turn-budget.
        router.config.effort_ladder = &[Effort::Low, Effort::Medium, Effort::High];
        router.config.goal_max_iterations = 5;
        check(&mut router, &mut ts, "T1", 2000); // Fail iter1 → re-spawn degrau 0 (Low).
        check(&mut router, &mut ts, "T1", 2001); // Fail iter2 → re-spawn degrau 1 (Medium).

        let events = ts.store.events().unwrap();
        let efforts: Vec<&str> = recs_of(&events, "SpawnRequested")
            .iter()
            .filter_map(|e| field(e, "effort"))
            .collect();
        assert_eq!(efforts.len(), 2, "dois re-spawns informados");
        assert_eq!(efforts[0], "low", "1ª volta no degrau base da escada");
        assert_eq!(
            efforts[1], "medium",
            "2ª volta SOBE um degrau (low → medium)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-1 (superfície): `goal.define` cunha o `goal_id` SERVER-SIDE e emite `GoalDefined`.
    #[test]
    fn handle_goal_define_cunha_goal_id_server_side() {
        let (mut router, sup, dir) = router_with("goal-def");
        let _u = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("goal-def");
        let (_rec, mut deliver) = recorder();
        let msg = MailMessage::new(
            "@User",
            "goal",
            "goal.define",
            r#"{"statement":"criar landing"}"#,
        );
        let goal_id = match router.route_message(&msg, &mut ts.store, 1000, &mut deliver) {
            RouteOutcome::GoalApplied { intent, goal_id } => {
                assert_eq!(intent, "goal.define");
                goal_id
            }
            other => panic!("esperava GoalApplied, veio {other:?}"),
        };
        assert!(!goal_id.is_empty(), "goal_id cunhado server-side (UUIDv7)");
        let events = ts.store.events().unwrap();
        let defs = recs_of(&events, "GoalDefined");
        assert_eq!(defs.len(), 1);
        assert_eq!(field(defs[0], "statement"), Some("criar landing"));
        assert_eq!(field(defs[0], "goal_id"), Some(goal_id.as_str()));
        // F3-2-1: `origin` é o CANAL de entrada carimbado server-side (do roster), não o sender.
        // Sem Tradutor no roster → @Maestro (a porta padrão).
        assert_eq!(
            field(defs[0], "origin"),
            Some("@Maestro"),
            "sem Tradutor no roster, o canal de entrada é o Maestro"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-1 (fiação `entry_origin`): com um Tradutor no roster, a Goal nasce com `origin:"@Tradutor"`
    /// — o canal de entrada do leigo, carimbado SERVER-SIDE pela projeção do roster (não pelo payload).
    #[test]
    fn handle_goal_define_carimba_origin_tradutor_quando_ha_tradutor_no_roster() {
        let (mut router, sup, dir) = router_with("goal-trad");
        let _u = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("goal-trad");
        // Setup: um nó com papel TRADUTOR no roster projetado (estado pré-existente do Espaço).
        let trad = NodeId::from_u128(0x5452_4144);
        ts.store
            .append(&DomainEvent::NodeAdded {
                node: trad,
                kind: "terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .unwrap();
        ts.store
            .append(&DomainEvent::NodeRoleAssigned {
                node: trad,
                role: "TRADUTOR".into(),
            })
            .unwrap();
        let (_rec, mut deliver) = recorder();
        let msg = MailMessage::new(
            "@User",
            "goal",
            "goal.define",
            r#"{"statement":"criar landing"}"#,
        );
        router.route_message(&msg, &mut ts.store, 1000, &mut deliver);
        let events = ts.store.events().unwrap();
        let defs = recs_of(&events, "GoalDefined");
        assert_eq!(defs.len(), 1);
        assert_eq!(
            field(defs[0], "origin"),
            Some("@Tradutor"),
            "com Tradutor no roster, o canal de entrada da Goal é o Tradutor"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-1 (ENFORCEMENT crítico, spec 52 §Segurança 2): `goal.confirm.by` é o `sender` AUTENTICADO,
    /// JAMAIS o `by` que o payload tenta forjar — o gate humano da meta não é forjável por um agente.
    #[test]
    fn handle_goal_confirm_carimba_by_server_side_ignora_payload() {
        let (mut router, sup, dir) = router_with("goal-conf");
        let user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("goal-conf");
        let (_rec, mut deliver) = recorder();
        // O payload TENTA forjar `by` com outro id — deve ser ignorado pelo carimbo server-side.
        let forged_by = NodeId::from_u128(999).to_string();
        let payload = format!(r#"{{"goal_id":"g1","by":"{forged_by}"}}"#);
        let msg = MailMessage::new("@User", "goal", "goal.confirm", &payload);
        router.route_message(&msg, &mut ts.store, 1000, &mut deliver);

        let events = ts.store.events().unwrap();
        let confs = recs_of(&events, "GoalConfirmed");
        assert_eq!(confs.len(), 1);
        let user_str = user.to_string();
        assert_eq!(
            field(confs[0], "by"),
            Some(user_str.as_str()),
            "by é o sender autenticado (@User), não o payload"
        );
        assert_ne!(
            field(confs[0], "by"),
            Some(forged_by.as_str()),
            "by forjado no payload é IGNORADO"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **F3-3 (M-DETECTOR):** uma mensagem cujo payload ABRE com `[LINA::CORRECTION]` é captada pelo
    /// caminho REAL (`route_message`) → `CorrectionObserved` com o `role` do SENDER autenticado
    /// (server-side), o summary como DADO e `refs` = a mensagem-fonte. NÃO entrega a PTY.
    /// **Segurança (regra-mãe ADR 0007):** o `role` vem do binding do nó, JAMAIS do payload — o texto
    /// tenta forjar `role=admin` e é IGNORADO. Caminho real (não `store.append` à mão): prova a costura.
    #[test]
    fn correction_sentinel_emits_observed_with_server_side_role() {
        let (mut router, sup, dir) = router_with("correction");
        let dev = sup.register("@Dev", None, sink());
        let mut ts = TmpStore::new("correction");
        // O SENDER tem papel BACKEND no roster projetado (estado pré-existente do Espaço).
        ts.store
            .append(&DomainEvent::NodeAdded {
                node: dev,
                kind: "terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .unwrap();
        ts.store
            .append(&DomainEvent::NodeRoleAssigned {
                node: dev,
                role: "backend".into(),
            })
            .unwrap();
        let (_rec, mut deliver) = recorder();

        // O texto TENTA forjar um papel — tem que ser IGNORADO (o role vem do sender autenticado).
        let msg = MailMessage::new(
            "@Dev",
            "@Maestro",
            "status",
            "[LINA::CORRECTION] use pnpm, não npm — role=admin",
        );
        match router.route_message(&msg, &mut ts.store, 1000, &mut deliver) {
            RouteOutcome::CorrectionObserved { role } => {
                assert_eq!(
                    role, "backend",
                    "role = o do SENDER (server-side), nunca do payload"
                );
            }
            other => panic!("esperava CorrectionObserved, veio {other:?}"),
        }
        let events = ts.store.events().unwrap();
        let obs = recs_of(&events, "CorrectionObserved");
        assert_eq!(obs.len(), 1, "exatamente 1 CorrectionObserved logado");
        assert_eq!(field(obs[0], "role"), Some("backend"));
        assert_ne!(
            field(obs[0], "role"),
            Some("admin"),
            "role forjado no texto do payload é IGNORADO (regra-mãe ADR 0007)"
        );
        assert_eq!(
            field(obs[0], "summary"),
            Some("use pnpm, não npm — role=admin")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Gate (a): o ciclo completo via verbos (define→interpret→confirm) + decomposição + check fecha a
    /// Goal; reabrir o store do MESMO log e re-projetar reconstrói a Goal IDÊNTICA (replay = verdade).
    #[test]
    fn gate_a_replay_reconstroi_a_goal_apos_reabrir() {
        let (mut router, sup, dir) = router_with("gate-a");
        let _user = sup.register("@User", None, sink());
        let _dev = sup.register("@Dev", None, sink());
        let mut ts = TmpStore::new("gate-a");
        let (_rec, mut deliver) = recorder();

        let goal_id = match router.route_message(
            &MailMessage::new("@User", "goal", "goal.define", r#"{"statement":"criar x"}"#),
            &mut ts.store,
            1000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            o => panic!("{o:?}"),
        };
        let interp =
            format!(r#"{{"goal_id":"{goal_id}","interpretation":"simples","strategy":"1 dev"}}"#);
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp),
            &mut ts.store,
            1001,
            &mut deliver,
        );
        router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.confirm",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1002,
            &mut deliver,
        );
        router
            .seed_plan_item(&mut ts.store, "T1", "tarefa")
            .unwrap();
        ts.store
            .append(&DomainEvent::PlanItemAttributed {
                item: "T1".into(),
                goal_id: Some(goal_id.clone()),
                parents: vec![],
                acceptance: vec![crit_cmd("ok", "true")],
                budget_tokens: 0,
                paths: vec![],
            })
            .unwrap();
        ts.store
            .append(&DomainEvent::GoalDecomposed {
                goal_id: goal_id.clone(),
                plan_items: vec!["T1".into()],
            })
            .unwrap();
        router.route_message(
            &MailMessage::new("@Dev", "plan", "plan.claim", "").with_ref("plan:T1"),
            &mut ts.store,
            1003,
            &mut deliver,
        );
        check(&mut router, &mut ts, "T1", 1004); // Pass → GoalAchieved.

        let live = crate::project_goals(&ts.store.events().unwrap());
        // Reabre o store do MESMO log em disco (simula matar/reabrir) e re-projeta.
        let reopened = EventStore::open(&ts.dir).expect("reabrir store");
        let replayed = crate::project_goals(&reopened.events().unwrap());

        assert_eq!(
            live, replayed,
            "replay reconstrói a Goal idêntica após reabrir"
        );
        assert_eq!(live.len(), 1, "uma Goal");
        assert_eq!(live[0].goal_id, goal_id);
        assert_eq!(live[0].phase, crate::GoalPhase::Achieved);
        assert_eq!(live[0].items, vec!["T1".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Roda define→interpret→confirm pelo CAMINHO REAL (route_message) e devolve o goal_id CONFIRMADO.
    // `descs` viram critérios de aceite no formato EXATO que a CLI serializa (`acceptance:[objeto]`).
    fn confirma_goal(
        router: &mut Router,
        ts: &mut TmpStore,
        statement: &str,
        descs: &[&str],
    ) -> String {
        let (_rec, mut deliver) = recorder();
        let goal_id = match router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.define",
                format!(r#"{{"statement":"{statement}"}}"#),
            ),
            &mut ts.store,
            1000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            o => panic!("define: {o:?}"),
        };
        let crits: Vec<AcceptanceCriterion> = descs
            .iter()
            .map(|d| AcceptanceCriterion {
                desc: (*d).into(),
                check_kind: CheckKind::HumanReview,
                check_arg: None,
            })
            .collect();
        let acc = serde_json::to_value(&crits).unwrap();
        let interp = serde_json::json!({
            "goal_id": goal_id, "interpretation": "entendi", "strategy": "1 dev",
            "proposed_team": [], "acceptance": acc,
        })
        .to_string();
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp),
            &mut ts.store,
            1001,
            &mut deliver,
        );
        router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.confirm",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1002,
            &mut deliver,
        );
        goal_id
    }

    /// GL-2 (descasamento CLI↔core): `goal.interpret` PERSISTE os critérios no formato que a CLI
    /// realmente serializa (`acceptance:[AcceptanceCriterion]`), não a chave fantasma `accept`.
    #[test]
    fn goal_interpret_persiste_acceptance_no_formato_da_cli() {
        let (mut router, sup, dir) = router_with("gl2-acc");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("gl2-acc");
        let (_rec, mut deliver) = recorder();
        let goal_id = match router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.define",
                r#"{"statement":"criar a landing"}"#,
            ),
            &mut ts.store,
            1000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            o => panic!("define: {o:?}"),
        };
        // Payload IDÊNTICO ao da CLI: `acceptance` é array de objetos AcceptanceCriterion serializados.
        let acc = serde_json::to_value(vec![AcceptanceCriterion {
            desc: "a pagina abre sem erro".into(),
            check_kind: CheckKind::HumanReview,
            check_arg: None,
        }])
        .unwrap();
        let interp = serde_json::json!({
            "goal_id": goal_id, "interpretation": "LP simples", "strategy": "1 dev",
            "proposed_team": [], "acceptance": acc,
        })
        .to_string();
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp),
            &mut ts.store,
            1001,
            &mut deliver,
        );
        let goals = crate::project_goals(&ts.store.events().unwrap());
        let g = goals
            .iter()
            .find(|g| g.goal_id == goal_id)
            .expect("goal existe");
        assert_eq!(
            g.acceptance.len(),
            1,
            "o criterio de aceite da CLI foi persistido"
        );
        assert_eq!(g.acceptance[0].desc, "a pagina abre sem erro");
        assert_eq!(
            g.acceptance[0].check_kind,
            CheckKind::HumanReview,
            "Goal = HumanReview (invariante spec 52 §1)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// GL-1 (intent `plan.seed` sem handler): a meta CONFIRMADA decompõe em ≥1 `PlanItemAttributed`
    /// com goal_id (gate de saída F3-1 (a)) pelo CAMINHO REAL (route_message), não montado à mão.
    #[test]
    fn plan_seed_decompoe_goal_confirmada_em_item_atribuido() {
        let (mut router, sup, dir) = router_with("gl1-seed");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("gl1-seed");
        let goal_id = confirma_goal(&mut router, &mut ts, "criar a landing", &["a pagina abre"]);
        let (_rec, mut deliver) = recorder();
        let out = router.route_message(
            &MailMessage::new(
                "@User",
                "plan",
                "plan.seed",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1003,
            &mut deliver,
        );
        assert!(
            matches!(out, RouteOutcome::GoalApplied { .. }),
            "seed aplicou: {out:?}"
        );
        let events = ts.store.events().unwrap();
        assert_eq!(
            events.iter().filter(|r| r.kind == "GoalDecomposed").count(),
            1,
            "exatamente um GoalDecomposed"
        );
        assert!(
            events.iter().any(|r| r.kind == "PlanItemAttributed"),
            "ao menos um PlanItemAttributed"
        );
        let goals = crate::project_goals(&events);
        let g = goals.iter().find(|g| g.goal_id == goal_id).unwrap();
        assert_eq!(
            g.phase,
            crate::GoalPhase::Decomposed,
            "fase avancou para decomposta"
        );
        assert_eq!(g.items.len(), 1, "um item semeado");
        let plan = ts.store.project().unwrap().plan;
        let item = plan
            .itens
            .iter()
            .find(|i| i.id == g.items[0])
            .expect("item no plano");
        assert_eq!(
            item.goal_id.as_deref(),
            Some(goal_id.as_str()),
            "item atribuido a goal"
        );
        assert_eq!(item.acceptance.len(), 1, "item herdou o criterio da goal");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// GL-1 (gate de ciclo): `plan.seed` de meta AINDA NÃO confirmada é recusado — zero decomposição.
    #[test]
    fn plan_seed_recusa_meta_nao_confirmada() {
        let (mut router, sup, dir) = router_with("gl1-naoconf");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("gl1-naoconf");
        let (_rec, mut deliver) = recorder();
        let goal_id = match router.route_message(
            &MailMessage::new("@User", "goal", "goal.define", r#"{"statement":"x"}"#),
            &mut ts.store,
            1000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            o => panic!("{o:?}"),
        };
        // interpreta, mas NÃO confirma (fase Interpreted).
        let interp = format!(r#"{{"goal_id":"{goal_id}","interpretation":"i","strategy":"s"}}"#);
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp),
            &mut ts.store,
            1001,
            &mut deliver,
        );
        let out = router.route_message(
            &MailMessage::new(
                "@User",
                "plan",
                "plan.seed",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1002,
            &mut deliver,
        );
        assert!(
            matches!(out, RouteOutcome::GoalRejected(_)),
            "recusou meta nao confirmada: {out:?}"
        );
        let events = ts.store.events().unwrap();
        assert_eq!(
            events.iter().filter(|r| r.kind == "GoalDecomposed").count(),
            0,
            "nenhuma decomposicao"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// GL-1 + invariante #4: após `plan.seed` REAL, reabrir o store e re-projetar reconstrói a Goal E o
    /// Plan idênticos (replay = verdade), incluindo o item semeado e sua atribuição.
    #[test]
    fn plan_seed_real_reconstroi_no_replay() {
        let (mut router, sup, dir) = router_with("gl1-replay");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("gl1-replay");
        let goal_id = confirma_goal(&mut router, &mut ts, "criar x", &["abre"]);
        let (_rec, mut deliver) = recorder();
        router.route_message(
            &MailMessage::new(
                "@User",
                "plan",
                "plan.seed",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1003,
            &mut deliver,
        );
        let live_goals = crate::project_goals(&ts.store.events().unwrap());
        let live_plan = ts.store.project().unwrap().plan;
        let reopened = EventStore::open(&ts.dir).expect("reabrir store");
        let replayed_goals = crate::project_goals(&reopened.events().unwrap());
        let replayed_plan = reopened.project().unwrap().plan;
        assert_eq!(live_goals, replayed_goals, "Goal identica no replay");
        assert_eq!(
            live_plan, replayed_plan,
            "Plan identico no replay (round-trip)"
        );
        assert_eq!(live_goals[0].phase, crate::GoalPhase::Decomposed);
        assert!(!live_plan.itens.is_empty(), "plano tem o item semeado");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// GL-1: `plan.add` cria o item E o atribui à Goal (`PlanItemAdded` + `PlanItemAttributed`). Ao
    /// contrário do nível-Goal (HumanReview forçado), o ITEM RESPEITA o `check_kind` — é o DoD real.
    #[test]
    fn plan_add_cria_item_atribuido_a_goal() {
        let (mut router, sup, dir) = router_with("gl1-add");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("gl1-add");
        let goal_id = confirma_goal(&mut router, &mut ts, "criar x", &[]);
        let (_rec, mut deliver) = recorder();
        let acc = serde_json::to_value(vec![AcceptanceCriterion {
            desc: "compila".into(),
            check_kind: CheckKind::Command,
            check_arg: Some("true".into()),
        }])
        .unwrap();
        let payload = serde_json::json!({
            "item": "T9", "desc": "montar a API", "goal_id": goal_id,
            "parents": [], "acceptance": acc, "budget_tokens": 0,
        })
        .to_string();
        let out = router.route_message(
            &MailMessage::new("@User", "plan", "plan.add", &payload),
            &mut ts.store,
            1003,
            &mut deliver,
        );
        assert!(
            matches!(out, RouteOutcome::PlanApplied { .. }),
            "add aplicou: {out:?}"
        );
        let plan = ts.store.project().unwrap().plan;
        let item = plan.itens.iter().find(|i| i.id == "T9").expect("T9 existe");
        assert_eq!(item.goal_id.as_deref(), Some(goal_id.as_str()));
        assert_eq!(item.acceptance.len(), 1);
        assert_eq!(
            item.acceptance[0].check_kind,
            CheckKind::Command,
            "item RESPEITA o check_kind (DoD verificavel por maquina)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Leva uma Goal a Interpreted via @User (route_message) e devolve o goal_id (aguardando o gate humano).
    fn interpreta_goal(router: &mut Router, ts: &mut TmpStore, statement: &str) -> String {
        let (_rec, mut deliver) = recorder();
        let goal_id = match router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.define",
                format!(r#"{{"statement":"{statement}"}}"#),
            ),
            &mut ts.store,
            1000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            o => panic!("define: {o:?}"),
        };
        let interp = format!(r#"{{"goal_id":"{goal_id}","interpretation":"i","strategy":"s"}}"#);
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp),
            &mut ts.store,
            1001,
            &mut deliver,
        );
        goal_id
    }

    fn last_confirmed_by(ts: &TmpStore) -> NodeId {
        let ev = ts
            .store
            .events()
            .unwrap()
            .into_iter()
            .rev()
            .find(|r| r.kind == "GoalConfirmed")
            .map(|r| serde_json::from_value::<DomainEvent>(r.payload).unwrap())
            .expect("GoalConfirmed emitido");
        match ev {
            DomainEvent::GoalConfirmed { by, .. } => by,
            other => panic!("esperava GoalConfirmed, veio {other:?}"),
        }
    }

    /// ADR 0036: o GESTO HUMANO DIRETO (botão do card) confirma a Goal carimbando `by` = sentinel
    /// HUMANO server-side, pelo canal in-process `human_intent` — NÃO por `route_message` (A2A).
    #[test]
    fn human_intent_confirma_goal_com_by_sentinel_humano() {
        let (mut router, sup, dir) = router_with("human-confirm");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("human-confirm");
        let goal_id = interpreta_goal(&mut router, &mut ts, "criar x");
        let out = router.human_intent(
            "goal.confirm",
            &format!(r#"{{"goal_id":"{goal_id}"}}"#),
            &mut ts.store,
        );
        assert!(
            matches!(out, RouteOutcome::GoalApplied { .. }),
            "gesto humano confirmou: {out:?}"
        );
        assert_eq!(
            last_confirmed_by(&ts),
            HUMAN_GESTURE,
            "by carimbado com o sentinel humano (ADR 0036), nao escolhido pelo cliente"
        );
        let goals = crate::project_goals(&ts.store.events().unwrap());
        assert_eq!(goals[0].phase, crate::GoalPhase::Confirmed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Red-team (spec 52 §Seg2 / ADR 0007): um AGENTE confirmando via `route_message` carimba o PRÓPRIO
    /// NodeId autenticado — NUNCA o sentinel humano. O `by="human"` só nasce do canal in-process.
    #[test]
    fn agente_nao_forja_by_humano_no_confirm() {
        let (mut router, sup, dir) = router_with("human-redteam");
        let user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("human-redteam");
        let goal_id = interpreta_goal(&mut router, &mut ts, "x");
        let (_rec, mut deliver) = recorder();
        router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.confirm",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1002,
            &mut deliver,
        );
        assert_eq!(
            last_confirmed_by(&ts),
            user,
            "o agente carimba o PROPRIO NodeId autenticado"
        );
        assert_ne!(
            last_confirmed_by(&ts),
            HUMAN_GESTURE,
            "um agente NUNCA produz o sentinel humano (by inforjavel)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────── F3-2: o loop do Maestro nativo (CORE) ─────────────────────

    /// define + interpreta uma Goal com `team` proposto, devolvendo o `goal_id`. PARA antes do
    /// confirm — o teste decide QUEM confirma (gesto humano vs agente) e em qual autonomia.
    fn interpreta_com_time(
        router: &mut Router,
        ts: &mut TmpStore,
        statement: &str,
        team: &[&str],
    ) -> String {
        let (_rec, mut deliver) = recorder();
        let goal_id = match router.route_message(
            &MailMessage::new(
                "@User",
                "goal",
                "goal.define",
                format!(r#"{{"statement":"{statement}"}}"#),
            ),
            &mut ts.store,
            1000,
            &mut deliver,
        ) {
            RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
            o => panic!("define: {o:?}"),
        };
        let interp = serde_json::json!({
            "goal_id": goal_id, "interpretation": "entendi", "strategy": "time",
            "proposed_team": team, "acceptance": [],
        })
        .to_string();
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp),
            &mut ts.store,
            1001,
            &mut deliver,
        );
        goal_id
    }

    /// Os `SpawnRequested` da MONTAGEM do time (id `goal-team:`), em ordem de log.
    fn team_spawns(events: &[EventRecord]) -> Vec<&EventRecord> {
        recs_of(events, "SpawnRequested")
            .into_iter()
            .filter(|r| field(r, "id").is_some_and(|id| id.starts_with("goal-team:")))
            .collect()
    }

    /// F3-2-3 (gate c): ao confirmar (gesto humano), a montagem emite UM `SpawnRequested` por papel
    /// do `proposed_team`, com `effort` no envelope (degrau base da escada) e `goal_id` que justifica
    /// o recurso. Gate (b): os spawns vêm DEPOIS do `GoalConfirmed`.
    #[test]
    fn f3_2_3_confirm_humano_monta_um_spawn_por_papel_com_effort() {
        let (mut router, sup, dir) = router_with("f323-team");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("f323-team");
        let goal_id = interpreta_com_time(
            &mut router,
            &mut ts,
            "criar landing",
            &["Frontend", "Backend"],
        );
        // Gesto humano (UI) confirma → executa a montagem mesmo em assistido (autonomia default).
        let out = router.human_intent(
            "goal.confirm",
            &format!(r#"{{"goal_id":"{goal_id}"}}"#),
            &mut ts.store,
        );
        assert!(
            matches!(out, RouteOutcome::GoalApplied { .. }),
            "confirm aplicou: {out:?}"
        );

        let events = ts.store.events().unwrap();
        let spawns = team_spawns(&events);
        assert_eq!(
            spawns.len(),
            2,
            "um SpawnRequested por papel do proposed_team"
        );
        let roles: Vec<&str> = spawns.iter().filter_map(|s| field(s, "role")).collect();
        assert!(
            roles.contains(&"Frontend") && roles.contains(&"Backend"),
            "um spawn por papel: {roles:?}"
        );
        for s in &spawns {
            assert_eq!(
                field(s, "effort"),
                Some("medium"),
                "effort no envelope (degrau base da escada)"
            );
            assert_eq!(
                field(s, "goal_id"),
                Some(goal_id.as_str()),
                "goal_id que justifica o recurso"
            );
            assert_eq!(
                field(s, "root_cause_id"),
                Some(goal_id.as_str()),
                "raiz = a Goal"
            );
        }
        let conf_seq = recs_of(&events, "GoalConfirmed")[0].seq;
        assert!(
            spawns.iter().all(|s| s.seq > conf_seq),
            "gate (b): montagem só APÓS o GoalConfirmed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-2 (gate b): ANTES do `GoalConfirmed` não há montagem de time nem decomposição; `plan.seed`
    /// pré-confirm é recusado. DEPOIS do confirm humano, a montagem aparece.
    #[test]
    fn f3_2_2_zero_spawn_de_time_antes_do_confirmed() {
        let (mut router, sup, dir) = router_with("f322-ordem");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("f322-ordem");
        let goal_id = interpreta_com_time(&mut router, &mut ts, "criar x", &["Frontend"]);

        let pre = ts.store.events().unwrap();
        assert!(
            team_spawns(&pre).is_empty(),
            "zero SpawnRequested de time antes do GoalConfirmed"
        );
        assert!(
            recs_of(&pre, "GoalDecomposed").is_empty(),
            "zero GoalDecomposed antes do GoalConfirmed"
        );
        let (_r, mut deliver) = recorder();
        let seed = router.route_message(
            &MailMessage::new(
                "@User",
                "plan",
                "plan.seed",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1500,
            &mut deliver,
        );
        assert!(
            matches!(seed, RouteOutcome::GoalRejected(_)),
            "seed pré-confirm recusado: {seed:?}"
        );

        router.human_intent(
            "goal.confirm",
            &format!(r#"{{"goal_id":"{goal_id}"}}"#),
            &mut ts.store,
        );
        let post = ts.store.events().unwrap();
        assert_eq!(
            team_spawns(&post).len(),
            1,
            "1 spawn de time após o confirm humano"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-2 (§327): em `assistido`, a EXECUÇÃO (montar o time) espera o GESTO HUMANO. Um agente
    /// confirmando via `route_message` REGISTRA o `GoalConfirmed` (fato, `by` server-side) mas NÃO
    /// dispara a montagem — propõe, o humano executa.
    #[test]
    fn f3_2_2_confirm_de_agente_em_assistido_nao_monta_o_time() {
        let (mut router, sup, dir) = router_with("f322-assist");
        // autonomia default = Assisted.
        let _maestro = sup.register("@Maestro", Some("maestro".into()), sink());
        let mut ts = TmpStore::new("f322-assist");
        let goal_id = {
            let (_rec, mut deliver) = recorder();
            let gid = match router.route_message(
                &MailMessage::new(
                    "@Maestro",
                    "goal",
                    "goal.define",
                    r#"{"statement":"criar x"}"#,
                ),
                &mut ts.store,
                1000,
                &mut deliver,
            ) {
                RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
                o => panic!("define: {o:?}"),
            };
            let interp = serde_json::json!({
                "goal_id": gid, "interpretation": "entendi", "strategy": "time",
                "proposed_team": ["Frontend"], "acceptance": [],
            })
            .to_string();
            router.route_message(
                &MailMessage::new("@Maestro", "goal", "goal.interpret", &interp),
                &mut ts.store,
                1001,
                &mut deliver,
            );
            gid
        };
        let (_r, mut deliver) = recorder();
        router.route_message(
            &MailMessage::new(
                "@Maestro",
                "goal",
                "goal.confirm",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1002,
            &mut deliver,
        );
        let events = ts.store.events().unwrap();
        assert_eq!(
            recs_of(&events, "GoalConfirmed").len(),
            1,
            "o GoalConfirmed é REGISTRADO (by server-side), mesmo de um agente"
        );
        assert!(
            team_spawns(&events).is_empty(),
            "§327: em assistido a montagem do time espera o gesto humano — agente não executa"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-2 (§327): em `autonomo`, o agente confirma E monta o time (auto, dentro do escopo).
    #[test]
    fn f3_2_2_autonomo_confirm_de_agente_monta_o_time() {
        let (mut router, sup, dir) = router_with("f322-auto");
        let _maestro = sup.register("@Maestro", Some("maestro".into()), sink());
        let mut ts = TmpStore::new("f322-auto");
        router.config.autonomy = AutonomyLevel::Autonomous;
        let goal_id = {
            let (_rec, mut deliver) = recorder();
            let gid = match router.route_message(
                &MailMessage::new(
                    "@Maestro",
                    "goal",
                    "goal.define",
                    r#"{"statement":"criar x"}"#,
                ),
                &mut ts.store,
                1000,
                &mut deliver,
            ) {
                RouteOutcome::GoalApplied { goal_id, .. } => goal_id,
                o => panic!("define: {o:?}"),
            };
            let interp = serde_json::json!({
                "goal_id": gid, "interpretation": "entendi", "strategy": "time",
                "proposed_team": ["Backend"], "acceptance": [],
            })
            .to_string();
            router.route_message(
                &MailMessage::new("@Maestro", "goal", "goal.interpret", &interp),
                &mut ts.store,
                1001,
                &mut deliver,
            );
            gid
        };
        let (_r, mut deliver) = recorder();
        router.route_message(
            &MailMessage::new(
                "@Maestro",
                "goal",
                "goal.confirm",
                format!(r#"{{"goal_id":"{goal_id}"}}"#),
            ),
            &mut ts.store,
            1002,
            &mut deliver,
        );
        let events = ts.store.events().unwrap();
        assert_eq!(
            team_spawns(&events).len(),
            1,
            "§327: em autonomo o agente monta o time (auto)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-2 ("Quero ajustar"): re-enviar `goal.interpret` faz a ÚLTIMA interpretação vencer, e a
    /// montagem usa o time CORRIGIDO (o antigo é descartado).
    #[test]
    fn f3_2_2_re_interpret_ultima_interpretacao_vence() {
        let (mut router, sup, dir) = router_with("f322-reinterp");
        let _user = sup.register("@User", None, sink());
        let mut ts = TmpStore::new("f322-reinterp");
        let goal_id = interpreta_com_time(&mut router, &mut ts, "criar x", &["Frontend"]);
        let (_r, mut deliver) = recorder();
        let interp2 = serde_json::json!({
            "goal_id": goal_id, "interpretation": "ajustado", "strategy": "outro",
            "proposed_team": ["Backend", "QA"], "acceptance": [],
        })
        .to_string();
        router.route_message(
            &MailMessage::new("@User", "goal", "goal.interpret", &interp2),
            &mut ts.store,
            1010,
            &mut deliver,
        );
        let goals = crate::project_goals(&ts.store.events().unwrap());
        let g = goals.iter().find(|g| g.goal_id == goal_id).unwrap();
        assert_eq!(
            g.interpretation.as_deref(),
            Some("ajustado"),
            "última interpretação vence na projeção"
        );

        router.human_intent(
            "goal.confirm",
            &format!(r#"{{"goal_id":"{goal_id}"}}"#),
            &mut ts.store,
        );
        let events = ts.store.events().unwrap();
        let roles: Vec<&str> = team_spawns(&events)
            .iter()
            .filter_map(|s| field(s, "role"))
            .collect();
        assert_eq!(roles.len(), 2, "time corrigido tem 2 papéis");
        assert!(
            roles.contains(&"Backend") && roles.contains(&"QA"),
            "monta pelo time da ÚLTIMA interpretação: {roles:?}"
        );
        assert!(
            !roles.contains(&"Frontend"),
            "o time antigo (Frontend) foi descartado pela correção"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-5 (gate d): 1ª falha re-despacha ao MESMO dev (effort do degrau base); ao SUBIR o degrau,
    /// é um NOVO desenvolvedor com effort estritamente maior (`Ord`) e prompt carregando "tentativas
    /// anteriores".
    #[test]
    fn f3_2_5_re_mesmo_no_degrau_base_depois_novo_dev_com_effort_maior() {
        let (mut router, _sup, mut ts, dir) = seed_goal("f325-newdev", &[("T1", "false")]);
        router.config.effort_ladder = &[Effort::Medium, Effort::High];
        router.config.goal_max_iterations = 5;
        check(&mut router, &mut ts, "T1", 2000); // Fail iter1 → re-despacho ao MESMO (Medium).
        check(&mut router, &mut ts, "T1", 2001); // Fail iter2 → NOVO dev (High, estritamente maior).

        let events = ts.store.events().unwrap();
        let spawns = recs_of(&events, "SpawnRequested");
        assert_eq!(spawns.len(), 2, "dois re-escalonamentos");
        // 1ª volta: re-despacho ao MESMO dev (@Dev), effort do degrau base.
        assert_eq!(field(spawns[0], "effort"), Some("medium"));
        assert_eq!(
            field(spawns[0], "name"),
            Some("@Dev"),
            "re-despacho ao MESMO target"
        );
        // 2ª volta: NOVO dev, effort estritamente maior, prompt com tentativas anteriores.
        assert_eq!(
            field(spawns[1], "effort"),
            Some("high"),
            "effort Ord crescente (medium→high)"
        );
        assert_ne!(
            field(spawns[1], "name"),
            Some("@Dev"),
            "NOVO desenvolvedor (não re-despacho ao mesmo)"
        );
        assert!(
            field(spawns[1], "prompt")
                .unwrap()
                .contains("tentativas anteriores"),
            "prompt do novo dev carrega as tentativas anteriores"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F3-2-5 (breaker sticky): com a escada no TOPO (effort não tem para onde subir), o re-spawn
    /// IDÊNTICO (mesma tarefa+effort) é SUPRIMIDO — deixa o turn-budget/gate humano resolver.
    #[test]
    fn f3_2_5_breaker_sticky_suprime_re_spawn_identico() {
        let (mut router, _sup, mut ts, dir) = seed_goal("f325-sticky", &[("T1", "false")]);
        // escada de 1 degrau: o effort não tem para onde subir → o 2º re-spawn seria IDÊNTICO.
        router.config.effort_ladder = &[Effort::High];
        router.config.goal_max_iterations = 5; // alto p/ o breaker morder ANTES do turn-budget.
        check(&mut router, &mut ts, "T1", 2000); // Fail iter1 → re-spawn High.
        check(&mut router, &mut ts, "T1", 2001); // Fail iter2 → seria High IDÊNTICO → suprimido.
        check(&mut router, &mut ts, "T1", 2002); // Fail iter3 → idem, suprimido.

        let events = ts.store.events().unwrap();
        let spawns = recs_of(&events, "SpawnRequested");
        assert_eq!(
            spawns.len(),
            1,
            "breaker sticky: após o 1º re-spawn no topo da escada, idênticos são suprimidos"
        );
        assert_eq!(field(spawns[0], "effort"), Some("high"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
