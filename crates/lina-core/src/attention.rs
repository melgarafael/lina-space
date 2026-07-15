//! F1-1-7 · Fila de atenção UNIFICADA (custódia + permissão) — projeção do event log.
//!
//! ## Arquitetura (story F1-1-7; ADR 0021 §3/§6; fonte 13.13 achados 4/6)
//! - **Uma fila só**, com precedência **custódia > permissão** (13.13 achado 4; a
//!   custódia é o gate duro do ADR 0004 — nunca fica atrás de um y/n) e **drain justo
//!   round-robin por nó** dentro de cada classe (um nó que flooda não enterra o pedido
//!   único de outro — zero starvation, provado por teste headless).
//! - **Event-sourced de verdade:** o estado SÓ muda via [`AttentionQueue::observe`]
//!   (fold de [`DomainEvent`]). [`AttentionQueue::resolve`]/[`AttentionQueue::dismiss`]
//!   são **comandos puros**: validam contra o estado e devolvem o evento a apendar —
//!   o chamador apenda no [`EventStore`](crate::events::EventStore) e re-alimenta o
//!   `observe`. Replay ≡ live **por construção** (crash com pendência → reabrir
//!   reconstrói via [`AttentionQueue::replay`]).
//! - **Dedup entre camadas (decisão do Maestro):** o fallback de grid fica ativo
//!   também para CLIs com hook — o grid detecta em ~1,3s e a `Notification` chega
//!   ~5,8s depois (latência intrínseca medida na F1-1-6). O item NASCE da primeira
//!   camada e a segunda **enriquece o MESMO item** (janela
//!   [`LAYER_MERGE_WINDOW_MS`] por nó; ver [`AttentionQueue::observe`]).
//! - **Mitigação de FP é produto (decisão do Maestro):** botão "não era um pedido"
//!   ([`AttentionQueue::dismiss`] → `PermissionDismissed`, alimenta a telemetria do
//!   detector) e allowlist por nó ([`AttentionQueue::set_node_muted`] →
//!   `NodeDetectionMuted`, persistido e reversível — último evento do nó vence).
//!
//! ## Fronteira (ADR 0021 §6 — inegociável)
//! Esta projeção **mostra, enfileira, audita e decide** — e NADA mais. O estado
//! `Escalated` aos 5 min é **VISUAL** (badge/alerta). **Nenhum caminho daqui escreve
//! no PTY**: o write do y/n (inclusive o auto-deny do SLA aos 10 min) é exclusivo do
//! executor da F1-1-8, atrás do ADR 0021. A custódia continua inteira no caminho
//! existente (`broker::run_custody` + pump do app — zero regressão): a fila apenas
//! ESPELHA as pendências de custódia para ordenação/exibição unificada
//! ([`AttentionQueue::custody_enqueued`]/[`AttentionQueue::custody_resolved`]); a
//! durabilidade delas é a fila de broker em disco (re-drain repõe pós-crash) e a
//! auditoria é a já existente (`ActionGated`/`BrokerExecuted`/`BrokerDenied`).

use std::collections::HashMap;

/// Re-export para o consumidor de UI (`lina_core::attention::PromptKind`): o shell
/// decide a apresentação por formato — `Yn` rende toast aprovável; `Choice`/`Trust`
/// rendem alerta + foco no terminal, SEM aprovar/recusar (direção do fundador R2b).
/// (Re-export daqui, e não do lib.rs, porque o lib.rs está em obra de outro dono
/// nesta rodada — costura coordenada.)
pub use crate::events::PromptKind;
use crate::events::{
    ApprovalDecision, DomainEvent, EventRecord, PermissionEvidence, ReclaimCandidate, ResolutionVia,
};
use crate::lifecycle::reason;
use crate::NodeStatus;

/// SLA de escalação VISUAL (ADR 0021 §3): pendência sem resposta há ≥ 5 min vira
/// [`AttentionState::Escalated`] (badge pulsante/entrada persistente na UI). O auto-deny
/// aos 10 min é IDENTIFICADO aqui ([`AttentionQueue::auto_deny_due`]), mas o WRITE da
/// recusa no PTY continua sendo do executor F1-1-8 — esta projeção não tem porta de
/// escrita (fronteira intacta).
pub const ESCALATE_AFTER_MS: u64 = 300_000;

/// SLA de **auto-deny** (ADR 0021 §3): pendência de PERMISSÃO sem resposta há ≥ 10 min é
/// NEGADA automaticamente (`decision=deny, via=timeout` — **NUNCA approve, sem knob**;
/// anti-regressão pelo gate AC-0021.5). DISTINTO do `RETENTION_TIMEOUT_MS` do router (que
/// rege a retenção de mensagem A2A, não a permissão). 10 min = mesma ordem de grandeza da
/// atenção humana num workspace vivo (ADR 0020: turnos reais 200–600 s; `retention_timeout`
/// = 10 min). [`AttentionQueue::auto_deny_due`] só IDENTIFICA as pendências vencidas; o
/// write da recusa (com check de tela) é do executor — esta projeção nunca escreve no PTY.
pub const AUTO_DENY_AFTER_MS: u64 = 600_000;

// Invariante de COMPILAÇÃO (ADR 0021 §3) — escalate-before-auto-deny: a escalação VISUAL
// precede o auto-deny, i.e. o pedido alerta o humano (5 min) ANTES de ser negado por
// timeout (10 min). Recalibrar uma constante para violar a ordem QUEBRA O BUILD, nunca
// silenciosamente o produto.
const _: () = assert!(ESCALATE_AFTER_MS < AUTO_DENY_AFTER_MS);

/// Janela de merge entre camadas POR NÓ: um `PermissionAsked` de camada DIFERENTE da
/// de um item ainda pendente do mesmo nó, chegando dentro desta janela, é o MESMO
/// pedido físico (enriquece, não duplica). Calibre: a `Notification` do Claude Code
/// chega ~5,8s (estável, medido na F1-1-6) depois do diálogo visível no grid —
/// 10s cobre com margem. Curta o bastante para não fundir pedidos distintos: um
/// prompt y/n BLOQUEIA o CLI no PTY, então um segundo pedido real do mesmo nó dentro
/// da janela exigiria o primeiro ainda não-respondido — não acontece num TUI serial.
pub const LAYER_MERGE_WINDOW_MS: u64 = 10_000;

/// FIX-2 (dogfood) — **TTL do ask do guard** (hook `PreToolUse`). Diferente do y/n de permissão, o
/// hook-ask NÃO emite um evento de resolução quando respondido no terminal, e no core não há sinal
/// determinístico de "respondido" (o nó fica `Busy` desenhando o próprio dialog — sinal ambíguo;
/// `recolhe-no-Busy` é follow-up de app). Então o item some por TEMPO desde o ÚLTIMO ask do nó
/// (renovável). 3 min: generoso para a resposta humana típica (ADR 0020: turnos 200–600 s),
/// priorizando VISIBILIDADE (direção do fundador: nenhum bloqueio invisível) e abaixo de
/// [`ESCALATE_AFTER_MS`] (5 min). **Limitação conhecida (v1):** se o humano demora > TTL num único
/// ask, o item some antes de responder; e pode persistir até o TTL após já respondido.
pub const GUARD_ASK_TTL_MS: u64 = 180_000;

/// W3 (ponto-cego do ADR 0019 §4) — **graça do "despacho engolido"**: quanto tempo um nó pode
/// ficar parado em `Idle`, DEPOIS de receber um handoff (`MessageDelivered{to}`) e sem produzir
/// UM ÚNICO progresso atribuível, antes de virar o alarme "recebeu, não começou". O relógio corre
/// a partir do RETORNO a `Idle` (não da entrega): assim a janela absorve o gap natural
/// fim-de-turno → [`DomainEvent::TokenUsageReported`] de um turno REAL (o app emite o uso de tokens
/// alguns segundos após o `Idle`), sem gritar falso-positivo. 90s: bem abaixo de
/// [`ESCALATE_AFTER_MS`] (o Maestro enxerga o engolido cedo) e folgado sobre o atraso típico do
/// sinal de progresso. **Conservador e tunável** (futuro `RouterConfig`); um turno real consome
/// tokens → desarma, então o engolido (zero turno, zero token) é o que sobra.
pub const DELIVERED_NO_PROGRESS_WINDOW_MS: u64 = 90_000;

/// Classe do item na fila — define a precedência (custódia > permissão > spawn > guard-ask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionKind {
    /// Gate duro de custódia/resume (`lina do`/`lina resume`, ADR 0004) — espelhado
    /// da pump do app. `Resume` entra como `Custody` (mesmo canal humano W3-6).
    Custody,
    /// Pedido de permissão y/n detectado (F1-1-6).
    Permission,
    /// SEAM-1 (ADR 0019 §6): um agente em CASCATA pediu criar um terminal (`SpawnGated{cascade}`) —
    /// gate humano (anti-fork-bomb). Aprovar = o terminal nasce (1×, dedupe durável M3); recusar =
    /// nada nasce. Precedência ABAIXO de permissão (criar processo pode esperar; um bloqueante de
    /// permissão trava um turno em curso).
    Spawn,
    /// FIX-2 (dogfood): um ASK do guard do Lina (hook `PreToolUse` → `ActionGated{decision:"ask"}`)
    /// que BLOQUEIA o agente, mas o detector de permissão (F1-1-6) não pega — o dialog de hook-ask
    /// tem outro formato que o nativo (e em `bypassPermissions` o nativo nem existe). Era o maior
    /// buraco de "bloqueio invisível ao usuário". Resolução v1: ALERTA + FOCA o terminal (sem
    /// injeção remota — espelha a decisão do fundador p/ não-`Yn`: o gesto resolve NO terminal).
    /// Precedência mais baixa (responder y/n no terminal pode esperar atrás dos gates que travam um
    /// turno), mas NUNCA invisível (direção do fundador 2026-06-07: todo bloqueante alerta).
    GuardAsk,
    /// F3-4-4 (spec 36 §2): CONFLITO DE CÓDIGO por pertencimento — um commit do time (`CodeChanged`)
    /// tocou um arquivo que ESTE nó reservou (`paths`×claim ≠ ∅, de OUTRO autor). O nó dono do claim
    /// PARA e PEDE direção humana (rebase? renegociar? desconflitar?) — não-trivial por natureza, daí a
    /// precedência da família guard-ask (entre [`AttentionKind::GuardAsk`] e
    /// [`AttentionKind::DeliveredNoProgress`]). Alerta + foco (`Choice`), NUNCA `Yn` aprovável daqui
    /// (`resolve`/`auto_deny_due` só olham `Yn`): a escolha resolve no fluxo de integração, não com um
    /// y/n na fila. NUNCA auto-some por TTL (perder um conflito em silêncio violaria inv #6).
    CodeConflict,
    /// W3 (ponto-cego do ADR 0019 §4): um handoff foi ENTREGUE (`MessageDelivered{to}`), o nó voltou
    /// a `Idle` e ficou parado ≥ [`DELIVERED_NO_PROGRESS_WINDOW_MS`] sem UM ÚNICO progresso atribuível
    /// — o "despacho engolido" (#22/#23). É OBSERVABILIDADE, não bloqueante humano: o stall detector
    /// só vigia `Busy` (`lifecycle.rs`), então delivered→Idle→silêncio era cego e o Maestro PERGUNTAVA
    /// ao humano "o terminal recebeu?" (#15). Precedência MAIS BAIXA (alerta de monitoramento, nunca
    /// trava um turno); NUNCA aprovável (alerta + foco — o Maestro age re-despachando, não com y/n).
    DeliveredNoProgress,
    /// F4-WA-5 (ADR 0035 §4 · ADR 0020): uma entrega A2A esgotou retry/retenção e foi para a
    /// **dead-letter queue** durável (`MessageDeadLettered`) — caso típico: um webhook chegou e o
    /// terminal-alvo está MORTO/fechado (nunca ficou pronto). É a materialização de "nada some em
    /// silêncio" (ADR 0020) na superfície do usuário: o evento + o arquivo da DLQ já são duráveis, mas
    /// sem item na fila o dono do Espaço nunca saberia que um aviso de fora ficou sem casa. OBSERVABILIDADE
    /// acionável (o retry é MANUAL — ADR 0020 — ou pelo `--resume`, porta aberta não-implementada), NUNCA
    /// aprovável daqui (não há y/n: re-enfileirar é um gesto fora da fila). Precedência acima do engolido
    /// (perder a entrega é mais grave que recebê-la-e-não-começar). NUNCA some por TTL (some só quando a
    /// MESMA `id` for re-entregue — `MessageDelivered` pós-reenfileiramento).
    DeadLetter,
}

/// Camada de origem da evidência — confiabilidade NÃO-uniforme por design: `Hook` é
/// estrutural; `Grid` é heurística (FP medido, #28174); `Custody` é o canal brokerado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionEvidence {
    Hook,
    Grid,
    Custody,
}

impl From<PermissionEvidence> for AttentionEvidence {
    fn from(e: PermissionEvidence) -> Self {
        match e {
            PermissionEvidence::Hook => Self::Hook,
            PermissionEvidence::Grid => Self::Grid,
        }
    }
}

/// Estado VISUAL do item (computado de `now_ms` em [`AttentionQueue::items`] — puro,
/// relógio injetado; nada de timer interno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionState {
    Pending,
    /// ≥ [`ESCALATE_AFTER_MS`] sem resposta (ADR 0021 §3 — escalação aos 5 min).
    Escalated,
}

/// Um item da fila unificada — o CONTRATO com a UI (toast/fila/badge). `detail` é
/// dado de EXIBIÇÃO (do `PreToolUse` correlacionado ou da linha do prompt) — jamais
/// autoridade (doutrina ADR 0021 §5: o gesto referencia `stable_id`, nunca posição
/// nem texto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionItem {
    pub stable_id: String,
    pub node_id: String,
    pub kind: AttentionKind,
    /// Ex.: `"git push origin/master"` (hook) ou a linha do prompt (grid) ou o
    /// `display` da custódia.
    pub detail: Option<String>,
    pub evidence: AttentionEvidence,
    pub created_ts: u64,
    pub state: AttentionState,
    /// R2b: formato do bloqueante — a UI apresenta por ele: `Yn` = toast aprovável;
    /// `Choice`/`Trust` = alerta + foco, sem aprovar/recusar (e [`AttentionQueue::resolve`]
    /// devolve `None` para não-`Yn`, defesa em profundidade). Custódia carrega `Yn`
    /// nominal (o campo não se aplica — o gate dela é o ⌘⏎ da pump).
    pub prompt_kind: PromptKind,
    /// R2b (ADR 0021 §1): a Captura 1 do detector — é o que torna a fila reconstruída
    /// por replay APROVÁVEL (o gesto passa este hash ao `deliver_approval` como
    /// `expected_hash`). `None` = origem sem captura (hook) ou item de custódia.
    pub vt_snapshot_hash: Option<String>,
}

/// Uma pendência VENCIDA pelo SLA de auto-deny (ADR 0021 §3) — o que
/// [`AttentionQueue::auto_deny_due`] entrega ao chamador para drenar pelo MESMO pipeline
/// validado do executor com `decision=Deny, via=Timeout`. **NÃO é um write** nem um
/// evento: é a IDENTIFICAÇÃO da pendência; o write (com re-snapshot + check de tela) é
/// exclusivo do executor (fronteira F1-1-8). Por NÃO carregar decisão, nenhum caminho
/// daqui pode produzir approve (a decisão é fixada `Deny` no ponto de drenagem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoDenyDue {
    /// `stable_id` CANÔNICO da pendência (o executor cruza com o binding do log — R4).
    pub stable_id: String,
    /// Nó dono (do `PermissionAsked` — binding de fonte interna; o executor re-verifica).
    pub node_id: String,
    /// Captura 1 (vira o `expected_hash` do check de tela do executor). `None` = origem
    /// sem captura (hook-only): o executor não tem baseline ⇒ aborta fail-safe
    /// (deny-não-entregue), nunca escreve às cegas.
    pub vt_snapshot_hash: Option<String>,
}

/// Pendência de permissão no fold (com os aliases do merge entre camadas).
#[derive(Debug, Clone)]
struct PendingPermission {
    stable_id: String,
    node_id: String,
    detail: Option<String>,
    evidence: PermissionEvidence,
    created_ts: u64,
    /// `stable_id`s ABSORVIDOS pelo merge entre camadas — resolver por qualquer um
    /// resolve o item; o evento sai sempre com o canônico (`stable_id`).
    aliases: Vec<String>,
    /// Formato do bloqueante (R2b). No merge, só a camada de GRID atualiza — é ela
    /// que VÊ a forma visual do prompt; o hook não sabe o formato.
    prompt_kind: PromptKind,
    /// Captura 1 (R2b) — preenche-se se ausente no merge (o grid é quem captura).
    vt_snapshot_hash: Option<String>,
}

impl PendingPermission {
    fn matches(&self, id: &str) -> bool {
        self.stable_id == id || self.aliases.iter().any(|a| a == id)
    }
}

/// Pendência de custódia ESPELHADA da pump do app (ver §Fronteira no topo).
#[derive(Debug, Clone)]
struct PendingCustody {
    id: String,
    node_id: String,
    display: String,
    created_ts: u64,
}

/// SEAM-1: pendência de SPAWN gateado (cascata) aguardando o gate humano. Reconstruída do log
/// (`SpawnRequested` traz name/role/requested_by; `SpawnGated{cascade}` promove a pendente) — então
/// um banner pendente SOBREVIVE a crash/reabrir (replay). `requested_by` é `String` (NodeId
/// serializado), como os demais ids da fila; o app re-lê o `SpawnRequested` TIPADO ao executar.
#[derive(Debug, Clone)]
struct PendingSpawn {
    /// `= SpawnRequested.id` (== `root_cause_id` na origem) — a chave do gesto/dedupe.
    id: String,
    /// Quem pediu (sender AUTENTICADO do gate), para exibição/identidade.
    requested_by: String,
    /// `@Nome` do terminal pedido.
    name: String,
    /// Papel pedido.
    role: String,
    created_ts: u64,
}

/// FIX-2: pendência de um ASK do guard (hook `PreToolUse`), POR NÓ. Dedup por nó: o guard dispara
/// 1×/tool gated — um item só; o último ask renova `last_ts`/`cmd`. Some por TTL
/// ([`GUARD_ASK_TTL_MS`]) desde `last_ts` (resolução v1 por timestamp).
#[derive(Debug, Clone)]
struct PendingGuardAsk {
    /// NOME do terminal (`LINA_NODE_NAME`) — o foco da fila é POR NOME (`attention_goto_node`).
    node: String,
    /// Resumo do comando barrado (copy leiga "vá ao terminal aprovar X"); o último ask vence.
    cmd: String,
    /// 1ª vez visto (idade exibida; estável no replay).
    created_ts: u64,
    /// Último ask do nó — renova o TTL (dedup N→1).
    last_ts: u64,
}

/// W3 (ponto-cego do ADR 0019 §4): vigia de um handoff ENTREGUE que pode ter sido **engolido**.
/// Projeção PURA derivada do log (decisão de arquiteto: SEM evento novo): o stall detector só corre
/// em `Busy` (`lifecycle.rs`), então `delivered → Idle → silêncio` é cego. Armado por
/// `MessageDelivered{to}` (a entrega mais nova vence; 1 vigia por nó), desarmado por progresso
/// atribuível (token de turno, output novo, mensagem enviada — ADR 0019 §2b).
#[derive(Debug, Clone)]
struct PendingDispatch {
    /// NodeId (serializado) do alvo da entrega — a CHAVE (1 vigia por nó; re-entrega re-arma).
    node_id: String,
    /// `ts` da entrega (`MessageDelivered`) — idade do "engolido" exibida na fila.
    delivered_ts: u64,
    /// `ts` do 1º retorno a `Idle` após a entrega — abre o ponto-cego E é o relógio do alarme
    /// (`now − idle_ts ≥ janela`). `None` = ainda em `Busy`/trabalhando (turf do stall detector,
    /// não deste alarme — zero sobreposição). Medir daqui (não da entrega) absorve o gap natural
    /// fim-de-turno → `TokenUsageReported` de um turno REAL.
    idle_ts: Option<u64>,
}

/// F3-4-4 (spec 36 §2): conflito de pertencimento ABERTO — um commit de OUTRO nó tocou paths que
/// `owner` reservou. Dedup por `(owner, branch)`: commits sucessivos do mesmo colega na mesma branch
/// renovam `last_ts` e UNEM os paths, não empilham. PURA projeção do log (sem evento novo): armada por
/// `CodeChanged` × claims; NÃO some por TTL (conflito não-resolvido nunca evapora — inv #6). Reconstruída
/// por replay como o resto da fila.
#[derive(Debug, Clone)]
struct PendingCodeConflict {
    /// NOME do nó dono do claim que PARA (foco da fila por nome, como [`PendingGuardAsk`]).
    owner: String,
    /// Branch do colega cujo commit tocou o arquivo reservado (exibição leiga).
    branch: String,
    /// Paths em CONFLITO (a interseção) — copy leiga; unidos a cada commit novo do mesmo par.
    paths: Vec<String>,
    /// 1ª vez visto (idade exibida; estável no replay).
    created_ts: u64,
    /// Último commit conflitante deste par (renova a recência).
    last_ts: u64,
}

/// F3-5-8 (ADR 0043): uma PROPOSTA de poda de disco aguardando o gesto humano custodiado. Vira item
/// [`AttentionKind::Custody`] na fila — o gate IRREVERSÍVEL (apagar bytes), gate humano em TODO nível.
/// Projeção PURA do log (event-sourced, SEM evento novo): armada por `DiskReclaimProposed`, resolvida
/// por `DiskReclaimExecuted` (a poda ocorreu). Reconstruída por replay (sobrevive a crash/reabrir). O
/// `approved_by` de `DiskReclaimApproved` é EXIBIÇÃO — NÃO resolve o item (a autoridade é o gesto, não
/// o campo; ADR 0007). Dedup por `stable_id`: re-proposta dos mesmos caminhos (probe periódico) não
/// empilha. NÃO some por TTL: disco cheio é problema vivo (inv #6, nunca perder em silêncio).
#[derive(Debug, Clone)]
struct PendingDiskReclaim {
    /// Id estável derivado dos caminhos (`disk_budget::reclaim_stable_id`) — o gesto o referencia
    /// (ADR 0021 §5: nunca texto/posição).
    stable_id: String,
    /// Copy leiga do que será liberado (de `reclaimable_bytes` + nº de candidatos).
    detail: String,
    created_ts: u64,
}

/// F4-WA-5: uma entrega que aterrissou na dead-letter queue (`MessageDeadLettered`) e aguarda um
/// gesto humano (re-enfileiramento manual / `--resume`). Indexada por `id` (= `delivery_id` do
/// webhook ou `msg_` do A2A) — a chave de resolução: `MessageDelivered{id}` pós-reenfileiramento.
/// NÃO guarda o `reason`: o motivo técnico já é durável no `MessageDeadLettered` do log (a fila é
/// projeção de exibição LEIGA — zero jargão na superfície, inv #6; quem quer o motivo lê do log).
#[derive(Debug, Clone)]
struct PendingDeadLetter {
    /// `id` da entrega dead-letterada — chave de dedup (replay) E de resolução (re-entrega).
    id: String,
    /// Alvo (`to` do evento) — NOME ou `NodeId` serializado; a UI humaniza (UUID→@Nome).
    to: String,
    created_ts: u64,
}

/// A fila de atenção unificada — uma por workspace. Ver doc do módulo.
#[derive(Debug, Default)]
pub struct AttentionQueue {
    /// Pendências de permissão, em ordem de chegada (= ordem de `created_ts`).
    permissions: Vec<PendingPermission>,
    /// Pendências de custódia espelhadas, em ordem de chegada.
    custody: Vec<PendingCustody>,
    /// SEAM-1: spawns gateados (cascata) aguardando o gate humano (banner). Pendente = `SpawnGated
    /// {cascade}` sem `SpawnAdmitted`/`SpawnDeclined` posterior.
    spawns: Vec<PendingSpawn>,
    /// SEAM-1: detalhes de TODO `SpawnRequested` visto (id → (requested_by, name, role, ts)) — o
    /// `SpawnGated{cascade}` posterior promove a pendente usando-os (os dois são apendados em
    /// sequência por `handle_spawn`). Removido quando o spawn resolve (admitido/recusado).
    spawn_requests: HashMap<String, (String, String, String, u64)>,
    /// FIX-2: asks do guard (hook `PreToolUse`) pendentes, um por nó (dedup N→1). Em ordem de
    /// chegada (= `created_ts`); somem por TTL no [`AttentionQueue::items`].
    guard_asks: Vec<PendingGuardAsk>,
    /// W3: vigias de handoffs entregues (1 por nó, ordem de chegada) — viram alarme
    /// [`AttentionKind::DeliveredNoProgress`] no [`AttentionQueue::items`] quando o nó voltou a
    /// `Idle` e ficou parado ≥ [`DELIVERED_NO_PROGRESS_WINDOW_MS`] sem progresso atribuível.
    dispatches: Vec<PendingDispatch>,
    /// Allowlist por nó (`NodeDetectionMuted`, último vence): `true` = fallback de
    /// grid DESLIGADO para o nó.
    muted: HashMap<String, bool>,
    /// F3-4-4: paths que cada item RESERVA (de `PlanItemAttributed`, último vence) — o lado "claims"
    /// da comparação ×`CodeChanged`. Sem evento novo: a fila já vê o log inteiro.
    item_paths: HashMap<String, Vec<String>>,
    /// F3-4-4: claims ATIVOS (item → NOME do dono). `PlanClaimed` ativa; `PlanChecked` libera (item
    /// concluído não reserva mais). Só claim VIVO entra na comparação de conflito.
    claims: HashMap<String, String>,
    /// F3-4-4: conflitos de código abertos (1 por `(owner, branch)`) — o "PARA" do pertencimento.
    code_conflicts: Vec<PendingCodeConflict>,
    /// F3-5-8: propostas de poda de disco pendentes de gesto (entram como `Custody`). Armadas por
    /// `DiskReclaimProposed`, resolvidas por `DiskReclaimExecuted`; dedup por `stable_id`.
    disk_reclaims: Vec<PendingDiskReclaim>,
    /// F4-WA-5: entregas na DLQ aguardando gesto humano (1 por `id`, dedup no fold). Armadas por
    /// `MessageDeadLettered`, resolvidas por `MessageDelivered{id}` (re-enfileiramento). NUNCA somem
    /// por TTL — perder um aviso de fora em silêncio violaria "nada some" (ADR 0020) / inv #6.
    dead_letters: Vec<PendingDeadLetter>,
    /// Tela do fundador (2026-06-25): `NodeId serializado → NOME` (de `NodeRenamed`). A morte de um
    /// terminal chega por `NodeId`, mas guard-asks/dead-letters/conflitos são chaveados por NOME —
    /// sem este mapa, encerrar um nó não limpava esses tipos e a fila acumulava pedidos de sessões
    /// já finalizadas (153 terminais que nem existem mais na tela do fundador).
    node_names: HashMap<String, String>,
}

impl AttentionQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// **O fold** — único caminho de mutação por evento (replay ≡ live). `ts` é o
    /// timestamp do registro (`EventRecord::ts` no replay; o `now_ms` do append ao
    /// vivo).
    pub fn observe(&mut self, event: &DomainEvent, ts: u64) {
        match event {
            DomainEvent::PermissionAsked {
                node_id,
                detail,
                evidence,
                stable_id,
                vt_snapshot_hash,
                prompt_kind,
                ..
            } => self.fold_asked(
                node_id,
                detail.clone(),
                *evidence,
                stable_id,
                vt_snapshot_hash.clone(),
                *prompt_kind,
                ts,
            ),
            // Remoção da pendência: decisão (Resolved), rótulo de FP (Dismissed) ou
            // prompt respondido DIRETO no terminal (PromptCleared, R2b item 5 — sem
            // decisão fabricada; `matches` cobre canônico E aliases do merge).
            DomainEvent::PermissionResolved { stable_id, .. }
            | DomainEvent::PermissionPromptCleared { stable_id, .. } => {
                self.permissions.retain(|p| !p.matches(stable_id));
            }
            // Tela do fundador (2026-06-25): «dispensar» é GENÉRICO. O botão "Limpar fila" e o "Não
            // era um pedido" emitem `PermissionDismissed{stable_id}` para QUALQUER item da fila (não
            // só permissões) — remove o item de qualquer fila pelo `stable_id` EXIBIDO (a identidade
            // do item na UI, ADR 0021 §6). É o que dá ao usuário o controle de zerar pendências que
            // nenhum encerramento automático alcança (dead-letters a alvos que nunca existiram). Para
            // um stable_id de permissão real, os retains dos demais tipos são no-op (os formatos
            // `dead-letter:`/`guard:`/… não casam) — não regride o "Não era um pedido".
            DomainEvent::PermissionDismissed { stable_id } => {
                self.permissions.retain(|p| !p.matches(stable_id));
                self.custody.retain(|c| c.id != *stable_id);
                self.disk_reclaims.retain(|d| d.stable_id != *stable_id);
                self.spawns.retain(|s| s.id != *stable_id);
                self.guard_asks
                    .retain(|g| format!("guard:{}", g.node) != *stable_id);
                self.code_conflicts
                    .retain(|c| format!("code-conflict:{}:{}", c.owner, c.branch) != *stable_id);
                self.dead_letters
                    .retain(|d| format!("dead-letter:{}", d.id) != *stable_id);
                self.dispatches
                    .retain(|d| format!("delivered-no-progress:{}", d.node_id) != *stable_id);
            }
            DomainEvent::NodeDetectionMuted { node_id, muted } => {
                self.muted.insert(node_id.clone(), *muted);
                if *muted {
                    // Mutar = o humano marcou a família de FP daquele nó: as pendências
                    // de GRID dele saem da fila (determinístico no replay — o próprio
                    // evento de mute está no log). Itens com evidência de HOOK ficam
                    // (camada estrutural, não-heurística — não é alvo do mute).
                    self.permissions.retain(|p| {
                        p.node_id != *node_id || p.evidence != PermissionEvidence::Grid
                    });
                }
            }
            // SEAM-1: captura os args de TODO pedido de spawn (precede o gate). Origem (que vira
            // SpawnApproved, sem SpawnGated) nunca é promovida a banner; só a CASCATA abaixo.
            DomainEvent::SpawnRequested {
                id,
                requested_by,
                name,
                role,
                ..
            } => {
                self.spawn_requests.insert(
                    id.clone(),
                    (requested_by.to_string(), name.clone(), role.clone(), ts),
                );
            }
            // SEAM-1: só a CASCATA vira banner (gate humano anti-fork-bomb). Outros motivos
            // (manual/over_cap/cost) são recusas/limites, não pedidos human-gated nesta rodada.
            DomainEvent::SpawnGated { id, reason, .. } if reason == "cascade" => {
                if self.spawns.iter().any(|s| s.id == *id) {
                    return; // replay defensivo: não re-entra
                }
                if let Some((requested_by, name, role, req_ts)) = self.spawn_requests.get(id) {
                    self.spawns.push(PendingSpawn {
                        id: id.clone(),
                        requested_by: requested_by.clone(),
                        name: name.clone(),
                        role: role.clone(),
                        created_ts: *req_ts,
                    });
                }
            }
            // SEAM-1: spawn resolvido (admitido pelo app OU recusado pelo humano) → sai do banner.
            DomainEvent::SpawnAdmitted { id, .. } | DomainEvent::SpawnDeclined { id } => {
                self.spawns.retain(|s| s.id != *id);
                self.spawn_requests.remove(id);
            }
            // FIX-2: ASK do guard (hook PreToolUse) COM nó identificado → item GuardAsk (alerta+foco).
            // SÓ `decision=="ask"` E `node:Some` entram: `deny`/`allow` são terminais/silenciosos (não
            // bloqueiam o humano) e `node:None` é custódia/log-antigo (sem terminal para focar — a
            // custódia já tem item próprio). Dedup por NÓ no fold (N→1).
            DomainEvent::ActionGated {
                decision,
                cmd,
                node: Some(node),
                ..
            } if decision == "ask" => self.fold_guard_ask(node, cmd, ts),
            // ───────── W3 (ponto-cego do ADR 0019 §4): o alarme do "despacho engolido" ─────────
            // Projeção PURA derivada (SEM evento novo). 3 facetas: ARMAR (entrega), DESARMAR
            // (progresso atribuível — virou trabalho) e ABRIR o ponto-cego (retorno a `Idle`).
            DomainEvent::MessageDelivered { id, to } => {
                // F4-WA-5: a MESMA id voltou a ser entregue (re-enfileiramento manual da DLQ, ADR
                // 0020) → a notificação de dead-letter resolve (some). No-op se id não estava na DLQ.
                self.dead_letters.retain(|d| d.id != *id);
                self.arm_dispatch(&to.to_string(), ts);
            }
            // ───────── F4-WA-5 (ADR 0035 §4 · ADR 0020): a DLQ NOTIFICA o usuário ─────────
            // Webhook a um alvo morto (ou A2A que esgotou retry/retenção) → a entrega já é durável na
            // DLQ (arquivo + evento); a fila de atenção é onde "nada some em silêncio" aparece ao dono
            // do Espaço. Dedup por `id` (replay defensivo: o mesmo evento reaplicado não duplica).
            DomainEvent::MessageDeadLettered { id, to, .. } => {
                if self.dead_letters.iter().any(|d| d.id == *id) {
                    return; // já registrada (replay/re-apresentação) — idempotente
                }
                self.dead_letters.push(PendingDeadLetter {
                    id: id.clone(),
                    to: to.clone(),
                    created_ts: ts,
                });
            }
            // `TokenUsageReported` é o sinal POR-TURNO (todo turno real consome tokens; o engolido
            // não roda turno → zero tokens → nunca chega aqui). `node` já é o NodeId serializado.
            DomainEvent::TokenUsageReported { node, .. } => self.remove_dispatch(node),
            // `MessageRouted{from}` = o nó DELEGOU adiante (agiu) — progresso atribuível ao remetente.
            DomainEvent::MessageRouted { from, .. } => self.remove_dispatch(&from.to_string()),
            DomainEvent::NodeStatusChanged {
                node,
                status,
                reason: why,
                ..
            } => {
                let node = node.to_string();
                if status == NodeStatus::Busy.as_str() && why == reason::PTY_OUTPUT {
                    // Output novo no PTY (proxy do `tail_hash`, que é efêmero/fora do log) = progresso.
                    // Filtra o carimbo `a2a_delivery` da própria entrega (router.rs), que NÃO é trabalho.
                    self.remove_dispatch(&node);
                } else if status == NodeStatus::Idle.as_str() {
                    // Voltou a `Idle`: abre o ponto-cego (o relógio de stall não corre fora de `Busy`).
                    self.mark_dispatch_idle(&node, ts);
                } else if status == NodeStatus::Dead.as_str() {
                    // Tela do fundador (2026-06-25): nó morto — INCLUI a morte póstuma do boot
                    // (`close_previous_generation` emite `Dead` para a geração anterior). Limpa TODAS
                    // as pendências dele, não só o alarme: é o que faz a fila NÃO acumular pedidos de
                    // sessões já finalizadas (via replay, a morte das gerações antigas limpa os órfãos).
                    self.purge_node_pendings(&node);
                }
            }
            // FIX-2/tela do fundador: mapeia `NodeId → NOME` para casar a morte (que vem por NodeId)
            // com as pendências chaveadas por nome (guard-ask/dead-letter/conflito) ao limpar.
            DomainEvent::NodeRenamed { node, name } => {
                self.node_names.insert(node.to_string(), name.clone());
            }
            // Nó removido/terminal encerrado → limpa TODAS as suas pendências (sem terminal vivo, nada
            // a aprovar/re-despachar/re-entregar). Mesma rotina da morte por `Dead`.
            DomainEvent::NodeRemoved { node } | DomainEvent::TerminalExited { node } => {
                self.purge_node_pendings(&node.to_string());
            }
            // ───────── F3-4-4 (spec 36 §2): conflito de código por pertencimento ─────────
            // Projeção PURA derivada (SEM evento novo): a fila junta os paths reservados (do item) +
            // o claim (item→dono) + o sinal de mudança, e cruza DETERMINISTICAMENTE (ZERO LLM, inv #1).
            // `paths`/`author_node` são DADO, jamais autoridade: a interseção só ABRE alerta (PARA e
            // pergunta), nunca concede posse (família ADR 0007).
            DomainEvent::PlanItemAttributed { item, paths, .. } => {
                // Último vence (re-atribuição substitui). Vazio é legítimo (item sem reserva).
                self.item_paths.insert(item.clone(), paths.clone());
            }
            DomainEvent::PlanClaimed { item, by, .. } => {
                // Claim ATIVA a reserva do item para o dono (NOME — mesmo espaço de `author_node`).
                self.claims.insert(item.clone(), by.clone());
            }
            DomainEvent::PlanChecked { item, .. } => {
                // Item concluído → libera a reserva (não há mais o que PARAR nele).
                self.claims.remove(item);
            }
            DomainEvent::CodeChanged {
                branch,
                paths,
                author_node,
                ..
            } => self.fold_code_changed(branch, paths, author_node, ts),
            // ───────── F3-5-8 (ADR 0043): proposta de poda → gate `Custody`; poda → resolve ─────────
            DomainEvent::DiskReclaimProposed {
                candidates,
                reclaimable_bytes,
                ..
            } => self.fold_disk_reclaim_proposed(candidates, *reclaimable_bytes, ts),
            // A poda OCORREU → a proposta saiu (resolvida). `DiskReclaimApproved` NÃO resolve aqui:
            // `approved_by` é exibição, não autoridade — só a execução real (Executed) limpa o gate.
            DomainEvent::DiskReclaimExecuted { paths, .. } => {
                let stable_id = crate::disk_budget::reclaim_stable_id(paths);
                self.disk_reclaims.retain(|d| d.stable_id != stable_id);
            }
            // Pós-decisão do executor (F1-1-8): sem efeito na fila — o item já saiu
            // no `PermissionResolved`. Um abort (`screen_changed`) reapresenta via
            // RE-DETECÇÃO (novo `PermissionAsked`, novo stable_id — ADR 0021 §1).
            _ => {}
        }
    }

    /// Reconstrói a fila varrendo o log (crash com pendência → reabrir reconstrói —
    /// critério da story). Registros não-decodificáveis (kind de versão futura) são
    /// pulados: a fila é projeção derivada, não o validador do log (isso é o
    /// `project()` do store).
    #[must_use]
    pub fn replay(records: &[EventRecord]) -> Self {
        let mut q = Self::new();
        q.observe_records(records);
        q
    }

    /// Aplica eventos do log (já em ordem de `seq`) à fila VIVA — base da projeção INCREMENTAL.
    /// `replay` é `new()` + `observe_records(todos)`; aplicar SÓ os registros novos
    /// (`seq > último aplicado`) à fila persistida converge ao MESMO estado, pois `observe` é um
    /// fold sequencial determinístico. É o que tira o full-replay `O(N)` da thread de UI no
    /// `AttentionHub::sync` (mesmo padrão do cache incremental de `EventStore::project`).
    pub fn observe_records(&mut self, records: &[EventRecord]) {
        for rec in records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                self.observe(&ev, rec.ts);
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // espelho 1:1 dos campos do evento (fold)
    fn fold_asked(
        &mut self,
        node_id: &str,
        detail: Option<String>,
        evidence: PermissionEvidence,
        stable_id: &str,
        vt_snapshot_hash: Option<String>,
        prompt_kind: PromptKind,
        ts: u64,
    ) {
        // Defesa em profundidade: nó mutado não acumula pendência de GRID (o chamador
        // idealmente nem roda `observe_grid` — ver `is_node_muted` —, mas o replay de
        // um log com asks pós-mute também precisa convergir).
        if evidence == PermissionEvidence::Grid && self.is_node_muted(node_id) {
            return;
        }
        // Replay defensivo: o mesmo stable_id (canônico ou alias) não re-entra.
        if self.permissions.iter().any(|p| p.matches(stable_id)) {
            return;
        }
        // Dedup ENTRE CAMADAS (decisão do Maestro): pendência do MESMO nó, de camada
        // OPOSTA, dentro da janela → mesmo pedido físico; enriquece o item existente.
        if let Some(existing) = self.permissions.iter_mut().find(|p| {
            p.node_id == node_id
                && p.evidence != evidence
                && ts.saturating_sub(p.created_ts) <= LAYER_MERGE_WINDOW_MS
        }) {
            if evidence == PermissionEvidence::Hook {
                // Hook chegando depois do grid (caminho dominante: ~1,3s vs ~5,8s):
                // promove a evidência (estrutural > heurística) e prefere o detail do
                // hook (comando real correlacionado, ex.: "git push origin/master",
                // contra a linha crua do prompt).
                existing.evidence = PermissionEvidence::Hook;
                if detail.is_some() {
                    existing.detail = detail;
                }
            } else {
                // Grid chegando (antes OU depois do hook): é a camada que VÊ o prompt —
                // atualiza o formato do bloqueante e preenche exibição ausente.
                existing.prompt_kind = prompt_kind;
                if existing.detail.is_none() {
                    existing.detail = detail;
                }
            }
            // Captura 1: preenche se ausente (o grid é quem captura; hook traz None).
            if existing.vt_snapshot_hash.is_none() {
                existing.vt_snapshot_hash = vt_snapshot_hash;
            }
            existing.aliases.push(stable_id.to_string());
            return;
        }
        self.permissions.push(PendingPermission {
            stable_id: stable_id.to_string(),
            node_id: node_id.to_string(),
            detail,
            evidence,
            created_ts: ts,
            aliases: Vec::new(),
            prompt_kind,
            vt_snapshot_hash,
        });
    }

    /// FIX-2: funde um ASK do guard no item do nó (dedup N→1: renova `last_ts`/`cmd`; o 1º fixa
    /// `created_ts`). O guard pode disparar uma vez por tool gated — o mesmo nó nunca empilha dois
    /// itens. Determinístico no replay (eventos em ordem de `ts`).
    fn fold_guard_ask(&mut self, node: &str, cmd: &str, ts: u64) {
        if let Some(existing) = self.guard_asks.iter_mut().find(|g| g.node == node) {
            existing.last_ts = ts;
            existing.cmd = cmd.to_string();
            return;
        }
        self.guard_asks.push(PendingGuardAsk {
            node: node.to_string(),
            cmd: cmd.to_string(),
            created_ts: ts,
            last_ts: ts,
        });
    }

    // ───────────────────── F3-4-4 (spec 36 §2): conflito de código por pertencimento ─────────────────────

    /// Cruza um `CodeChanged` com os claims ATIVOS: para cada item reservado por OUTRO nó cujos paths
    /// intersectam os do commit, abre/renova um `CodeConflict` para o dono. DETERMINÍSTICO (ZERO LLM):
    /// interseção de conjuntos via [`crate::code::intersecting_paths`]. Coleta antes de mutar (não dá
    /// para iterar `self.claims` e gravar em `self.code_conflicts` ao mesmo tempo).
    fn fold_code_changed(&mut self, branch: &str, paths: &[String], author_node: &str, ts: u64) {
        let mut found: Vec<(String, Vec<String>)> = Vec::new();
        for (item, owner) in &self.claims {
            if owner == author_node {
                continue; // o próprio autor do commit nunca é avisado do seu trabalho
            }
            if let Some(reserved) = self.item_paths.get(item) {
                let hits = crate::code::intersecting_paths(paths, reserved);
                if !hits.is_empty() {
                    found.push((owner.clone(), hits));
                }
            }
        }
        for (owner, hits) in found {
            self.fold_code_conflict(&owner, branch, hits, ts);
        }
    }

    /// Funde um conflito no item do par `(owner, branch)` (dedup: renova `last_ts` e UNE os paths; o 1º
    /// fixa `created_ts`). Commits sucessivos do mesmo colega na mesma branch nunca empilham dois itens.
    /// Determinístico no replay (eventos em ordem de `ts`). NUNCA remove (conflito não-resolvido persiste
    /// — inv #6; a resolução por gesto humano é seam de v2).
    fn fold_code_conflict(&mut self, owner: &str, branch: &str, hits: Vec<String>, ts: u64) {
        if let Some(existing) = self
            .code_conflicts
            .iter_mut()
            .find(|c| c.owner == owner && c.branch == branch)
        {
            existing.last_ts = ts;
            for p in hits {
                if !existing.paths.contains(&p) {
                    existing.paths.push(p);
                }
            }
            existing.paths.sort();
            return;
        }
        self.code_conflicts.push(PendingCodeConflict {
            owner: owner.to_string(),
            branch: branch.to_string(),
            paths: hits, // já vem ordenado de `intersecting_paths`
            created_ts: ts,
            last_ts: ts,
        });
    }

    // ───────────────────── W3: vigia do "despacho engolido" (ponto-cego ADR 0019) ─────────────────────

    /// ARMA o vigia de um handoff entregue ao nó: a entrega MAIS NOVA vence (re-arma, zerando o
    /// estado anterior — `idle_ts` volta a `None`). 1 vigia por nó (a chave é o nó).
    fn arm_dispatch(&mut self, node: &str, ts: u64) {
        self.dispatches.retain(|d| d.node_id != node);
        self.dispatches.push(PendingDispatch {
            node_id: node.to_string(),
            delivered_ts: ts,
            idle_ts: None,
        });
    }

    /// DESARMA o vigia do nó — sem alarme. Dois chamadores, mesma ação, intenções distintas: o
    /// nó PROGREDIU (token de turno, output novo, mensagem enviada — virou trabalho) OU MORREU
    /// (`Dead`/removido — nada a re-despachar). Remover (vs. marcar) mantém o fold mínimo e
    /// idempotente: sem vigia = sem alarme.
    fn remove_dispatch(&mut self, node: &str) {
        self.dispatches.retain(|d| d.node_id != node);
    }

    /// Tela do fundador (2026-06-25): limpa TODAS as pendências de um nó encerrado/morto — não só o
    /// alarme de despacho. As filas usam chaves heterogêneas (permissões/custódia/despacho por NodeId;
    /// guard-ask/dead-letter/conflito por NOME), então casa o `NodeId` morto, o NOME mapeado
    /// (`node_names`, de `NodeRenamed`) e a forma `@Nome`. Sem isto, encerrar um terminal deixava seus
    /// pedidos órfãos e a fila acumulava o lixo de sessões já finalizadas. Idempotente no replay.
    /// `spawns` (chave `requested_by`) fica de fora: o solicitante de um spawn pode estar vivo.
    fn purge_node_pendings(&mut self, node_id: &str) {
        self.remove_dispatch(node_id);
        let name = self.node_names.get(node_id).cloned();
        let matches = |key: &str| -> bool {
            key == node_id
                || name
                    .as_deref()
                    .is_some_and(|n| key == n || key.trim_start_matches('@') == n)
        };
        self.permissions.retain(|p| !matches(&p.node_id));
        self.guard_asks.retain(|g| !matches(&g.node));
        self.dead_letters.retain(|d| !matches(&d.to));
        self.custody.retain(|c| !matches(&c.node_id));
        self.code_conflicts.retain(|c| !matches(&c.owner));
    }

    /// O nó voltou a `Idle` após a entrega → registra o instante (abre o ponto-cego: o relógio de
    /// stall não corre fora de `Busy`). O 1º `Idle` vence (`get_or_insert` não reabre a janela se
    /// o nó oscilar Idle→Busy→Idle sem progresso). No-op se não há vigia para o nó.
    fn mark_dispatch_idle(&mut self, node: &str, ts: u64) {
        if let Some(d) = self.dispatches.iter_mut().find(|d| d.node_id == node) {
            d.idle_ts.get_or_insert(ts);
        }
    }

    /// F3-5-8: arma uma proposta de poda como pendência `Custody` (dedup por `stable_id` derivado dos
    /// caminhos — re-proposta do probe periódico não empilha). A copy leiga vem de `reclaimable_bytes`
    /// (humanizado em GB/MB) + nº de candidatos; o gesto referencia o `stable_id`, nunca o texto.
    fn fold_disk_reclaim_proposed(
        &mut self,
        candidates: &[ReclaimCandidate],
        reclaimable_bytes: u64,
        ts: u64,
    ) {
        let paths: Vec<String> = candidates.iter().map(|c| c.path.clone()).collect();
        let stable_id = crate::disk_budget::reclaim_stable_id(&paths);
        if self.disk_reclaims.iter().any(|d| d.stable_id == stable_id) {
            return;
        }
        self.disk_reclaims.push(PendingDiskReclaim {
            stable_id,
            detail: format!(
                "liberar {} de disco ({} alvo(s)) — precisa da sua confirmação",
                crate::disk_budget::human_bytes(reclaimable_bytes),
                candidates.len()
            ),
            created_ts: ts,
        });
    }

    // ───────────────────────── espelho da custódia (pump do app) ─────────────────────────

    /// A pump enfileirou um gate de custódia/resume (`PendingGate`): espelha aqui para
    /// a ordenação unificada. `id` = id da `MailMessage` (a chave que a pump já usa);
    /// `node_id` = requester AUTENTICADO (dir-dono do drain — nunca campo de payload).
    /// Idempotente por `id` (re-drain pós-crash não duplica o espelho).
    pub fn custody_enqueued(
        &mut self,
        id: impl Into<String>,
        node_id: impl Into<String>,
        display: impl Into<String>,
        ts: u64,
    ) {
        let id = id.into();
        if self.custody.iter().any(|c| c.id == id) {
            return;
        }
        self.custody.push(PendingCustody {
            id,
            node_id: node_id.into(),
            display: display.into(),
            created_ts: ts,
        });
    }

    /// A pump resolveu (confirmou/recusou) o gate de custódia `id` — sai do espelho.
    /// A auditoria do desfecho é a já existente do broker (`ActionGated`/
    /// `BrokerExecuted`/`BrokerDenied`) — zero mudança no caminho da custódia.
    pub fn custody_resolved(&mut self, id: &str) {
        self.custody.retain(|c| c.id != id);
    }

    // ───────────────────────── comandos (puros: estado → evento) ─────────────────────────

    /// Comando de decisão humana sobre uma PERMISSÃO pendente. Devolve o
    /// `PermissionResolved` a entregar ao executor (com o `stable_id` CANÔNICO, mesmo
    /// se chamado por um alias do merge) — ou `None` se: id desconhecido/já resolvido
    /// (idempotência da fila: clique duplo não gera segundo evento); item de CUSTÓDIA
    /// (a custódia decide pelo caminho existente da pump — zero regressão); ou
    /// **`prompt_kind` não-`Yn`** (R2b: `Choice`/`Trust` alertam + focam, sem
    /// aprovar/recusar — injetar `y` numa caixa de escolha seria input errado; defesa
    /// em profundidade além do gate de apresentação da UI).
    #[must_use]
    pub fn resolve(
        &self,
        stable_id: &str,
        decision: ApprovalDecision,
        via: ResolutionVia,
    ) -> Option<DomainEvent> {
        let item = self.permissions.iter().find(|p| p.matches(stable_id))?;
        if item.prompt_kind != PromptKind::Yn {
            return None;
        }
        Some(DomainEvent::PermissionResolved {
            stable_id: item.stable_id.clone(),
            decision,
            via,
        })
    }

    /// Comando "não era um pedido" (mitigação de FP como produto). Devolve o
    /// `PermissionDismissed` a apendar (stable_id canônico) — `None` se
    /// desconhecido/já saiu. NÃO é deny: nada será escrito nem negado ao agente; o
    /// chamador também alimenta `PermissionDetector::record_false_positive` (rótulo
    /// externo — nunca auto-inferido, #28174).
    #[must_use]
    pub fn dismiss(&self, stable_id: &str) -> Option<DomainEvent> {
        // Permission: emite o canônico (o gesto pode ter vindo por alias do merge entre camadas).
        if let Some(p) = self.permissions.iter().find(|p| p.matches(stable_id)) {
            return Some(DomainEvent::PermissionDismissed {
                stable_id: p.stable_id.clone(),
            });
        }
        // Tela do fundador (2026-06-25): «dispensar» é GENÉRICO (botão "Limpar fila"). Reconhece
        // QUALQUER item da fila pelo `stable_id` EXIBIDO — sem isto o evento nunca era emitido para
        // dead-letter/guard/etc., o fold genérico nunca rodava e o botão "não fazia nada". O
        // `stable_id` desses tipos já É o canônico (vem direto de `items()`).
        self.has_item_with_stable_id(stable_id)
            .then(|| DomainEvent::PermissionDismissed {
                stable_id: stable_id.to_string(),
            })
    }

    /// `true` se ALGUM item da fila (qualquer tipo, não só permissão) tem o `stable_id` EXIBIDO —
    /// espelha os formatos de [`Self::items`]. Usado por [`Self::dismiss`] para reconhecer
    /// não-permissões no "Limpar fila"; evita emitir `PermissionDismissed` de lixo (id inexistente).
    fn has_item_with_stable_id(&self, stable_id: &str) -> bool {
        self.dead_letters
            .iter()
            .any(|d| format!("dead-letter:{}", d.id) == stable_id)
            || self
                .guard_asks
                .iter()
                .any(|g| format!("guard:{}", g.node) == stable_id)
            || self.custody.iter().any(|c| c.id == stable_id)
            || self.disk_reclaims.iter().any(|d| d.stable_id == stable_id)
            || self.spawns.iter().any(|s| s.id == stable_id)
            || self
                .code_conflicts
                .iter()
                .any(|c| format!("code-conflict:{}:{}", c.owner, c.branch) == stable_id)
            || self
                .dispatches
                .iter()
                .any(|d| format!("delivered-no-progress:{}", d.node_id) == stable_id)
    }

    /// Comando da allowlist por nó: liga/desliga o fallback de GRID para `node_id`.
    /// Devolve o evento a apendar (persistido e REVERSÍVEL — último evento vence).
    #[must_use]
    pub fn set_node_muted(&self, node_id: impl Into<String>, muted: bool) -> DomainEvent {
        DomainEvent::NodeDetectionMuted {
            node_id: node_id.into(),
            muted,
        }
    }

    /// `true` se o fallback de grid está DESLIGADO para o nó — o loop de detecção
    /// consulta antes de chamar `PermissionDetector::observe_grid` (e o fold também
    /// barra, defensivamente).
    #[must_use]
    pub fn is_node_muted(&self, node_id: &str) -> bool {
        self.muted.get(node_id).copied().unwrap_or(false)
    }

    // ───────────────────────────── leitura (contrato com a UI) ─────────────────────────────

    /// A fila ORDENADA para exibição/drain: precedência **custódia > permissão**;
    /// dentro de cada classe, **round-robin por nó** (rodada 0 = item mais antigo de
    /// cada nó, na ordem do nó mais antigo; depois rodada 1; …) — um nó com N
    /// pendências não enterra o pedido único de outro (zero starvation). `state` é
    /// computado de `now_ms` (puro): ≥ 5 min sem resposta → `Escalated` (visual).
    #[must_use]
    pub fn items(&self, now_ms: u64) -> Vec<AttentionItem> {
        let state_of = |created: u64| {
            if now_ms.saturating_sub(created) >= ESCALATE_AFTER_MS {
                AttentionState::Escalated
            } else {
                AttentionState::Pending
            }
        };
        let custody = self.custody.iter().map(|c| AttentionItem {
            stable_id: c.id.clone(),
            node_id: c.node_id.clone(),
            kind: AttentionKind::Custody,
            detail: Some(c.display.clone()),
            evidence: AttentionEvidence::Custody,
            created_ts: c.created_ts,
            state: state_of(c.created_ts),
            prompt_kind: PromptKind::Yn, // nominal: não se aplica à custódia (gate ⌘⏎)
            vt_snapshot_hash: None,
        });
        let perms = self.permissions.iter().map(|p| AttentionItem {
            stable_id: p.stable_id.clone(),
            node_id: p.node_id.clone(),
            kind: AttentionKind::Permission,
            detail: p.detail.clone(),
            evidence: p.evidence.into(),
            created_ts: p.created_ts,
            state: state_of(p.created_ts),
            prompt_kind: p.prompt_kind,
            vt_snapshot_hash: p.vt_snapshot_hash.clone(),
        });
        // SEAM-1: banners de spawn (cascata) — precedência ABAIXO de permissão. `detail` = copy leiga
        // ("trazer um especialista de <role>"); a UI enriquece com o nome do solicitante (model).
        let spawns = self.spawns.iter().map(|s| AttentionItem {
            stable_id: s.id.clone(),
            node_id: s.requested_by.clone(),
            kind: AttentionKind::Spawn,
            detail: Some(format!("trazer um especialista de {} ({})", s.role, s.name)),
            evidence: AttentionEvidence::Custody, // request estruturado (não-heurístico), como custódia
            created_ts: s.created_ts,
            state: state_of(s.created_ts),
            prompt_kind: PromptKind::Yn, // y/n: deixar nascer? (gate humano)
            vt_snapshot_hash: None,
        });
        // FIX-2: asks do guard (hook PreToolUse) — precedência ABAIXO de spawn. SOMEM por TTL desde o
        // ÚLTIMO ask (`last_ts`): resolução v1 por timestamp. `detail` = comando barrado (copy leiga;
        // a UI enriquece com o nome). `prompt_kind = Choice`: alerta + foco, NUNCA aprovável daqui
        // (defesa em profundidade — `resolve`/`auto_deny_due` só olham as permissões `Yn`).
        let guard = self
            .guard_asks
            .iter()
            .filter(|g| now_ms.saturating_sub(g.last_ts) < GUARD_ASK_TTL_MS)
            .map(|g| AttentionItem {
                stable_id: format!("guard:{}", g.node),
                node_id: g.node.clone(),
                kind: AttentionKind::GuardAsk,
                detail: Some(g.cmd.clone()),
                evidence: AttentionEvidence::Hook, // ask ESTRUTURAL do gate (não-heurístico)
                created_ts: g.created_ts,
                state: state_of(g.created_ts),
                prompt_kind: PromptKind::Choice,
                vt_snapshot_hash: None,
            });
        // F3-4-4 (spec 36 §2): conflitos de pertencimento — precedência da família guard-ask (ENTRE
        // guard e o engolido). 1 por `(owner, branch)` (dedup no fold) ⇒ anti-amplificação. `detail` =
        // copy leiga ("o colega mexeu em X que você reservou"); `Choice`: alerta + foco, a escolha
        // (rebase/renegociar/desconflitar) resolve no fluxo de integração, NUNCA y/n na fila. Estrutural
        // (projeção do log, não-heurística). PURO de `now_ms` ⇒ replay reconstrói a fila idêntica.
        let conflicts = self.code_conflicts.iter().map(|c| AttentionItem {
            stable_id: format!("code-conflict:{}:{}", c.owner, c.branch),
            node_id: c.owner.clone(),
            kind: AttentionKind::CodeConflict,
            detail: Some(format!(
                "o colega mexeu em {} que você reservou ({}) — rebase, renegociar ou desconflitar?",
                c.paths.join(", "),
                c.branch
            )),
            evidence: AttentionEvidence::Hook,
            created_ts: c.created_ts,
            state: state_of(c.created_ts),
            prompt_kind: PromptKind::Choice,
            vt_snapshot_hash: None,
        });
        // W3 (ponto-cego do ADR 0019): despachos ENTREGUES que voltaram a `Idle` e ficaram parados
        // ≥ DELIVERED_NO_PROGRESS_WINDOW_MS (desde o `Idle`) sem UM ÚNICO progresso atribuível → o
        // "engolido". Precedência MAIS BAIXA (alerta de observabilidade, nunca bloqueante). 1 por nó
        // (a entrega mais nova venceu no fold) ⇒ anti-amplificação por construção (padrão `NodeStalled`).
        // `idle_ts == None` (nó ainda `Busy`) NÃO entra: aí quem vigia é o stall detector — zero
        // sobreposição. PURO de `now_ms` ⇒ replay reconstrói a fila idêntica.
        let swallowed = self.dispatches.iter().filter_map(|d| {
            let idle_ts = d.idle_ts?;
            (now_ms.saturating_sub(idle_ts) >= DELIVERED_NO_PROGRESS_WINDOW_MS).then(|| {
                AttentionItem {
                    stable_id: format!("delivered-no-progress:{}", d.node_id),
                    node_id: d.node_id.clone(),
                    kind: AttentionKind::DeliveredNoProgress,
                    detail: Some("recebeu a tarefa e ainda não começou".to_string()),
                    // Projeção ESTRUTURAL do log (não-heurística), como o ask do guard — não é grid.
                    evidence: AttentionEvidence::Hook,
                    created_ts: d.delivered_ts, // idade = desde a ENTREGA (há quanto está engolido)
                    state: state_of(d.delivered_ts),
                    // Choice: alerta + foco, NUNCA aprovável daqui (`resolve`/`auto_deny_due` só olham
                    // permissões `Yn`) — o Maestro age re-despachando, não com y/n.
                    prompt_kind: PromptKind::Choice,
                    vt_snapshot_hash: None,
                }
            })
        });
        // F4-WA-5 (ADR 0035 §4 · ADR 0020): entregas na DLQ — tipicamente um aviso de fora (webhook)
        // que não pôde ser entregue porque o terminal-alvo está morto/fechado. OBSERVABILIDADE
        // acionável (precede o engolido: perder a entrega é mais grave que recebê-la-e-não-começar),
        // NUNCA aprovável daqui (`Choice` — o retry é manual/`--resume`, não y/n; defesa em
        // profundidade: `resolve`/`auto_deny_due` só olham `Yn`). `detail` é copy LEIGA pura — o motivo
        // técnico fica no log, nunca na superfície (zero jargão, inv #6). PURO de `now_ms` (estado só
        // muda por `state_of`) ⇒ replay reconstrói a fila idêntica.
        let dead = self.dead_letters.iter().map(|d| AttentionItem {
            stable_id: format!("dead-letter:{}", d.id),
            node_id: d.to.clone(),
            kind: AttentionKind::DeadLetter,
            // Copy correta para AMBOS os casos do evento (webhook OU A2A) — `MessageDeadLettered` é
            // genérico; "mensagem" não mente quando a DLQ não veio de webhook.
            detail: Some(
                "não consegui entregar uma mensagem — guardei e dá pra reenviar".to_string(),
            ),
            evidence: AttentionEvidence::Hook, // projeção estrutural do log (não-heurística)
            created_ts: d.created_ts,
            state: state_of(d.created_ts),
            prompt_kind: PromptKind::Choice,
            vt_snapshot_hash: None,
        });
        // F3-5-8: as propostas de poda compartilham a precedência de `Custody` (gate humano duro). O
        // `node_id` é o pseudo-nó de sistema "disco" — a pressão é do workspace, não de um terminal.
        let disk = self.disk_reclaims.iter().map(|d| AttentionItem {
            stable_id: d.stable_id.clone(),
            node_id: "disco".to_string(),
            kind: AttentionKind::Custody,
            detail: Some(d.detail.clone()),
            evidence: AttentionEvidence::Custody,
            created_ts: d.created_ts,
            state: state_of(d.created_ts),
            prompt_kind: PromptKind::Yn, // gate aprovável: liberar o disco? (gesto custodiado)
            vt_snapshot_hash: None,
        });
        let mut custody_items: Vec<AttentionItem> = custody.collect();
        custody_items.extend(disk);
        let mut out = round_robin_by_node(custody_items);
        out.extend(round_robin_by_node(perms.collect()));
        out.extend(round_robin_by_node(spawns.collect()));
        out.extend(round_robin_by_node(guard.collect()));
        out.extend(round_robin_by_node(conflicts.collect()));
        out.extend(round_robin_by_node(dead.collect()));
        out.extend(round_robin_by_node(swallowed.collect()));
        out
    }

    /// SEAM-1: comando de RECUSA de um banner de spawn pendente. Devolve o `SpawnDeclined` a apendar
    /// (`None` se id desconhecido/já resolvido — idempotência). A APROVAÇÃO NÃO mora aqui: criar o
    /// terminal é do app (`admit_node` + `SpawnAdmitted`, fora do core); o app ramifica por
    /// `kind == Spawn` e re-lê o `SpawnRequested` TIPADO do log para executar.
    #[must_use]
    pub fn decline_spawn(&self, id: &str) -> Option<DomainEvent> {
        self.spawns.iter().find(|s| s.id == id)?;
        Some(DomainEvent::SpawnDeclined { id: id.to_string() })
    }

    /// SEAM-1: `true` se `id` é um banner de spawn PENDENTE (o app ramifica o gesto: aprovar → admite;
    /// recusar → [`AttentionQueue::decline_spawn`]).
    #[must_use]
    pub fn is_pending_spawn(&self, id: &str) -> bool {
        self.spawns.iter().any(|s| s.id == id)
    }

    /// **Driver de auto-deny (ADR 0021 §3)** — as pendências de PERMISSÃO **Yn** sem
    /// resposta há ≥ [`AUTO_DENY_AFTER_MS`] em `now_ms`. PURO (relógio injetado; sem
    /// timer/thread). NÃO escreve no PTY nem apenda eventos (fronteira F1-1-8: o write
    /// da recusa, com check de tela, é do executor) — devolve apenas a IDENTIFICAÇÃO.
    ///
    /// O chamador drena cada candidato pelo MESMO pipeline validado do executor com
    /// `decision=Deny, via=Timeout` (simetria total do §3: tela válida ⇒ escreve a
    /// recusa; tela divergiu ⇒ aborta sem escrever, deny-não-entregue). Como o candidato
    /// **não carrega decisão**, ZERO caminho daqui produz approve (sem knob de
    /// auto-approve — anti-regressão AC-0021.5).
    ///
    /// Recortes: **custódia** fica de fora (gate próprio da pump — ⌘⏎); **não-`Yn`**
    /// (`Choice`/`Trust`) fica de fora (alerta+foco; digitar `n` numa caixa de escolha
    /// seria input errado — mesma regra de [`AttentionQueue::resolve`]).
    #[must_use]
    pub fn auto_deny_due(&self, now_ms: u64) -> Vec<AutoDenyDue> {
        self.permissions
            .iter()
            .filter(|p| p.prompt_kind == PromptKind::Yn)
            .filter(|p| now_ms.saturating_sub(p.created_ts) >= AUTO_DENY_AFTER_MS)
            .map(|p| AutoDenyDue {
                stable_id: p.stable_id.clone(),
                node_id: p.node_id.clone(),
                vt_snapshot_hash: p.vt_snapshot_hash.clone(),
            })
            .collect()
    }

    /// Contagem de pendências (badge da UI: custódia + permissão).
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.custody.len() + self.permissions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending_count() == 0
    }
}

/// Intercala itens (já em ordem de chegada) em rodadas round-robin por nó:
/// rodada k = o (k+1)-ésimo item de cada nó, nós na ordem do seu item mais antigo.
/// Determinístico e sem cursor mutável — a mesma fila produz sempre a mesma ordem
/// (replay-friendly; a justiça é provada por teste, não por agendador).
fn round_robin_by_node(items: Vec<AttentionItem>) -> Vec<AttentionItem> {
    // Agrupa preservando ordem de chegada (itens já chegam por created_ts).
    let mut nodes: Vec<(String, Vec<AttentionItem>)> = Vec::new();
    for item in items {
        match nodes.iter_mut().find(|(n, _)| *n == item.node_id) {
            Some((_, bucket)) => bucket.push(item),
            None => nodes.push((item.node_id.clone(), vec![item])),
        }
    }
    let mut out = Vec::new();
    let mut round = 0_usize;
    loop {
        let mut emitted = false;
        for (_, bucket) in &nodes {
            if let Some(item) = bucket.get(round) {
                out.push(item.clone());
                emitted = true;
            }
        }
        if !emitted {
            return out;
        }
        round += 1;
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventStore;
    use serial_test::serial;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    const T0: u64 = 1_750_000_000_000;

    fn ask(node: &str, evidence: PermissionEvidence, stable_id: &str) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node.into(),
            tool: None,
            detail: Some(format!("detail-{stable_id}")),
            evidence,
            stable_id: stable_id.into(),
            vt_snapshot_hash: None,
            prompt_kind: PromptKind::Yn,
        }
    }

    /// `ask` com formato + Captura 1 (R2b) — para os testes de prompt_kind/hash.
    fn ask_kind(
        node: &str,
        evidence: PermissionEvidence,
        stable_id: &str,
        prompt_kind: PromptKind,
        vt_snapshot_hash: Option<&str>,
    ) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node.into(),
            tool: None,
            detail: Some(format!("detail-{stable_id}")),
            evidence,
            stable_id: stable_id.into(),
            vt_snapshot_hash: vt_snapshot_hash.map(str::to_string),
            prompt_kind,
        }
    }

    fn dead_lettered(id: &str, to: &str, reason: &str) -> DomainEvent {
        DomainEvent::MessageDeadLettered {
            id: id.into(),
            to: to.into(),
            reason: reason.into(),
        }
    }

    /// F4-WA-5 (gate e): uma mensagem na DLQ (`MessageDeadLettered`) — webhook a um alvo morto —
    /// vira EXATAMENTE 1 item na fila de atenção (nada some em silêncio), carregando o alvo e
    /// NUNCA aprovável daqui (retry é manual/`--resume`, não y/n).
    #[test]
    fn dead_letter_event_enqueues_one_attention_item() {
        let mut q = AttentionQueue::new();
        q.observe(
            &dead_lettered("wh_1", "@Dev", "alvo morto — tentativas esgotadas"),
            T0,
        );
        let dl: Vec<_> = q
            .items(T0 + 1_000)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::DeadLetter)
            .collect();
        assert_eq!(dl.len(), 1, "DLQ → 1 item de atenção (0 perda)");
        assert_eq!(dl[0].node_id, "@Dev", "o item carrega o alvo (UI humaniza)");
        assert_ne!(
            dl[0].prompt_kind,
            PromptKind::Yn,
            "DLQ não é aprovável na fila (retry manual/--resume, nunca y/n)"
        );
    }

    /// Re-enfileiramento manual (ADR 0020): movido de volta ao outbox e ENTREGUE → a notificação
    /// resolve (some quando a MESMA `id` recebe `MessageDelivered`). Sem isso o item ficaria preso.
    #[test]
    fn dead_letter_resolved_when_same_id_redelivered() {
        let mut q = AttentionQueue::new();
        q.observe(&dead_lettered("wh_7", "@Dev", "morto"), T0);
        assert_eq!(
            q.items(T0 + 1)
                .iter()
                .filter(|i| i.kind == AttentionKind::DeadLetter)
                .count(),
            1
        );
        // Humano re-enfileira; a entrega ocorre para a MESMA id → notificação resolve.
        q.observe(
            &DomainEvent::MessageDelivered {
                id: "wh_7".into(),
                to: NodeId::now_v7(),
            },
            T0 + 5_000,
        );
        assert_eq!(
            q.items(T0 + 6_000)
                .iter()
                .filter(|i| i.kind == AttentionKind::DeadLetter)
                .count(),
            0,
            "re-entregue → a notificação de DLQ some (0 lixo preso)"
        );
    }

    /// A resolução é POR `id`: uma entrega de OUTRA mensagem não apaga uma DLQ pendente.
    #[test]
    fn dead_letter_not_resolved_by_unrelated_delivery() {
        let mut q = AttentionQueue::new();
        q.observe(&dead_lettered("wh_a", "@Dev", "morto"), T0);
        q.observe(
            &DomainEvent::MessageDelivered {
                id: "wh_b".into(), // id DIFERENTE
                to: NodeId::now_v7(),
            },
            T0 + 1_000,
        );
        assert_eq!(
            q.items(T0 + 2_000)
                .iter()
                .filter(|i| i.kind == AttentionKind::DeadLetter)
                .count(),
            1,
            "entrega de outra msg não resolve esta DLQ"
        );
    }

    /// Replay defensivo / idempotência: o MESMO `MessageDeadLettered` reaplicado (e a fila
    /// reconstruída do log) não duplica o item — `AttentionQueue::replay ≡ observe ao vivo`.
    #[test]
    fn dead_letter_dedup_on_replay() {
        let mut q = AttentionQueue::new();
        q.observe(&dead_lettered("wh_x", "@Dev", "morto"), T0);
        q.observe(&dead_lettered("wh_x", "@Dev", "morto"), T0 + 10); // re-aplicação (replay)
        assert_eq!(
            q.items(T0 + 1_000)
                .iter()
                .filter(|i| i.kind == AttentionKind::DeadLetter)
                .count(),
            1,
            "DLQ deduplicada por id (replay idempotente)"
        );
    }

    /// Critério da story (teste headless do round-robin): nó A flooda 3 pedidos; o
    /// pedido ÚNICO de B (posterior a todos) sai em 2º — alternância por nó, zero
    /// starvation. A ordem completa prova a justiça.
    #[test]
    fn round_robin_no_starvation_under_flood() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "a1"), T0);
        // Fora da janela de merge (mesma camada nunca funde; janelas distintas provam
        // que são pedidos independentes do mesmo nó).
        q.observe(&ask("A", PermissionEvidence::Grid, "a2"), T0 + 20_000);
        q.observe(&ask("A", PermissionEvidence::Grid, "a3"), T0 + 40_000);
        q.observe(&ask("B", PermissionEvidence::Grid, "b1"), T0 + 60_000);
        let order: Vec<String> = q
            .items(T0 + 61_000)
            .into_iter()
            .map(|i| i.stable_id)
            .collect();
        assert_eq!(
            order,
            vec!["a1", "b1", "a2", "a3"],
            "rodada 0 = mais antigo de CADA nó (B não espera o flood de A)"
        );
    }

    /// Precedência: custódia chega DEPOIS, mas sai ANTES (custódia > permissão —
    /// ADR 0004 é o gate duro; 13.13 achado 4).
    #[test]
    fn custody_precedes_permission_regardless_of_arrival() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0);
        q.custody_enqueued("msg_c1", "B", "lina do deploy [deploy/prod]", T0 + 5_000);
        let items = q.items(T0 + 6_000);
        assert_eq!(items[0].stable_id, "msg_c1");
        assert_eq!(items[0].kind, AttentionKind::Custody);
        assert_eq!(items[0].evidence, AttentionEvidence::Custody);
        assert_eq!(items[1].stable_id, "p1");
        // Espelho idempotente (re-drain pós-crash) e resolução remove.
        q.custody_enqueued("msg_c1", "B", "lina do deploy [deploy/prod]", T0 + 7_000);
        assert_eq!(q.pending_count(), 2);
        q.custody_resolved("msg_c1");
        assert_eq!(q.items(T0 + 8_000).len(), 1);
    }

    /// Tela do fundador (2026-06-24): pedidos de permissão de sessões JÁ encerradas acumulavam
    /// para sempre — com o terminal morto, o grid não é mais escaneado, `PermissionPromptCleared`
    /// nunca chega e ninguém aprova/recusa um nó que não existe mais. Encerrar (`NodeRemoved`/
    /// `TerminalExited`) limpa as permissões órfãs DAQUELE nó (e só dele); replay idempotente.
    #[test]
    fn terminal_encerrado_limpa_permissoes_orfas() {
        let morto = NodeId::from_u128(1);
        let vivo = NodeId::from_u128(2);
        let mut q = AttentionQueue::new();
        q.observe(&ask(&morto.to_string(), PermissionEvidence::Hook, "p1"), T0);
        q.observe(&ask(&vivo.to_string(), PermissionEvidence::Hook, "p2"), T0);
        assert_eq!(q.items(T0 + 1_000).len(), 2, "duas permissões pendentes");

        q.observe(&DomainEvent::NodeRemoved { node: morto }, T0 + 2_000);
        let items = q.items(T0 + 3_000);
        assert_eq!(items.len(), 1, "a permissão órfã do nó morto saiu da fila");
        assert_eq!(items[0].node_id, vivo.to_string(), "a do nó vivo permanece");

        // Idempotente: `TerminalExited` do mesmo nó (replay/dupla via) não quebra nada.
        q.observe(&DomainEvent::TerminalExited { node: morto }, T0 + 4_000);
        assert_eq!(q.items(T0 + 5_000).len(), 1);
    }

    /// Tela do fundador (2026-06-25): a fila acumulava 153 pedidos de sessões JÁ finalizadas —
    /// guard-asks e dead-letters de terminais que nem existem mais (não só permissões). Encerrar/matar
    /// um nó — INCLUSIVE a morte póstuma do boot (`NodeStatusChanged{Dead}` de
    /// `close_previous_generation`) — limpa TODAS as pendências dele, casando as chaves heterogêneas
    /// (NodeId × NOME × @Nome). Via replay, a morte das gerações antigas zera os órfãos.
    #[test]
    fn terminal_morto_limpa_pendencias_de_todos_os_tipos() {
        let a = NodeId::from_u128(1);
        let vivo = NodeId::from_u128(2);
        let mut q = AttentionQueue::new();
        // A morte chega por NodeId; guard-ask/dead-letter são por NOME → o mapa vem de `NodeRenamed`.
        q.observe(
            &DomainEvent::NodeRenamed {
                node: a,
                name: "Terminal A".into(),
            },
            T0,
        );
        // 3 pendências do MESMO nó, com CHAVES diferentes:
        q.observe(&ask(&a.to_string(), PermissionEvidence::Hook, "p1"), T0); // permission: NodeId
        q.observe(
            &DomainEvent::ActionGated {
                cmd: "cd /repo && lina plan read".into(),
                class: "gated-hard".into(),
                decision: "ask".into(),
                node: Some("Terminal A".into()), // guard-ask: NOME
            },
            T0,
        );
        q.observe(
            &DomainEvent::MessageDeadLettered {
                id: "m1".into(),
                to: "@Terminal A".into(), // dead-letter: @Nome
                reason: "node_dead".into(),
            },
            T0,
        );
        // Controle: pendência de um nó VIVO não pode sumir.
        q.observe(&ask(&vivo.to_string(), PermissionEvidence::Hook, "p2"), T0);
        assert_eq!(
            q.items(T0 + 1_000).len(),
            4,
            "3 do nó A (permission+guard-ask+dead-letter) + 1 do vivo"
        );

        // Morte póstuma do boot — exatamente o que `close_previous_generation` emite ao reabrir.
        q.observe(
            &DomainEvent::NodeStatusChanged {
                node: a,
                status: NodeStatus::Dead.as_str().to_string(),
                from: NodeStatus::Idle.as_str().to_string(),
                reason: "app_reopened".into(),
            },
            T0 + 2_000,
        );
        let items = q.items(T0 + 3_000);
        assert_eq!(
            items.len(),
            1,
            "permission + guard-ask + dead-letter do nó morto saíram juntos"
        );
        assert_eq!(
            items[0].node_id,
            vivo.to_string(),
            "só o do nó vivo permanece"
        );
    }

    /// Tela do fundador (2026-06-25, REGRESSÃO do botão "Limpar"): o botão chamava `dismiss(id)`, mas
    /// o PRODUTOR `dismiss` só reconhecia PERMISSÕES — para um dead-letter retornava `None`, então o
    /// evento `PermissionDismissed` NUNCA era emitido e o fold (já genérico) nunca rodava. "Clico e
    /// nada ocorre". Testa o CAMINHO REAL (produtor → evento → fold), não o fold direto.
    #[test]
    fn dismiss_reconhece_qualquer_tipo_caminho_real() {
        let mut q = AttentionQueue::new();
        q.observe(
            &DomainEvent::MessageDeadLettered {
                id: "m1".into(),
                to: "@Especialista em IA".into(),
                reason: "alvo inexistente".into(),
            },
            T0,
        );
        q.observe(
            &DomainEvent::ActionGated {
                cmd: "x".into(),
                class: "gated-hard".into(),
                decision: "ask".into(),
                node: Some("Maestro 01".into()),
            },
            T0,
        );
        assert_eq!(q.items(T0 + 1_000).len(), 2);

        // O PRODUTOR reconhece dead-letter e guard-ask (não só permissão) e emite o evento.
        let ev_dl = q
            .dismiss("dead-letter:m1")
            .expect("dismiss reconhece dead-letter");
        assert!(
            matches!(&ev_dl, DomainEvent::PermissionDismissed { stable_id } if stable_id == "dead-letter:m1")
        );
        let ev_g = q
            .dismiss("guard:Maestro 01")
            .expect("dismiss reconhece guard-ask");

        // Caminho REAL: observar os eventos produzidos LIMPA de verdade (não o fold à mão).
        q.observe(&ev_dl, T0 + 2_000);
        q.observe(&ev_g, T0 + 2_000);
        assert!(
            q.items(T0 + 3_000).is_empty(),
            "dismiss + observe zera a fila — o botão Limpar funciona de ponta a ponta"
        );

        // stable_id que não casa item algum → None (não emite evento de lixo).
        assert!(q.dismiss("dead-letter:inexistente").is_none());
    }

    /// Tela do fundador (2026-06-25): o botão "Limpar fila" precisa dispensar QUALQUER tipo, não só
    /// permissão. `PermissionDismissed{stable_id}` agora remove o item pelo `stable_id` exibido de
    /// qualquer fila — é como o usuário zera dead-letters de alvos que nunca existiram (sem morte
    /// para o purge pegar). Um stable_id de permissão real NÃO afeta as outras filas (no-op).
    #[test]
    fn dismiss_generico_remove_qualquer_item_por_stable_id() {
        let mut q = AttentionQueue::new();
        q.observe(
            &DomainEvent::MessageDeadLettered {
                id: "m1".into(),
                to: "@Especialista em IA".into(),
                reason: "alvo inexistente".into(),
            },
            T0,
        );
        q.observe(
            &DomainEvent::ActionGated {
                cmd: "cd /repo && lina plan read".into(),
                class: "gated-hard".into(),
                decision: "ask".into(),
                node: Some("Maestro 01".into()),
            },
            T0,
        );
        q.observe(&ask("Z", PermissionEvidence::Hook, "perm1"), T0);
        assert_eq!(
            q.items(T0 + 1_000).len(),
            3,
            "dead-letter + guard-ask + permissão"
        );

        // "Limpar" dispensa o dead-letter pelo stable_id exibido (`dead-letter:<id>`).
        q.observe(
            &DomainEvent::PermissionDismissed {
                stable_id: "dead-letter:m1".into(),
            },
            T0 + 2_000,
        );
        assert_eq!(q.items(T0 + 3_000).len(), 2, "dead-letter dispensado");
        // E o guard-ask (`guard:<nome>`).
        q.observe(
            &DomainEvent::PermissionDismissed {
                stable_id: "guard:Maestro 01".into(),
            },
            T0 + 3_000,
        );
        let rest = q.items(T0 + 4_000);
        assert_eq!(rest.len(), 1, "guard-ask dispensado; só a permissão fica");
        assert_eq!(rest[0].stable_id, "perm1");
    }

    /// Dedup entre camadas (decisão do Maestro): item nasce do GRID (~1,3s), hook
    /// chega ~5,8s depois → MESMO item, evidência promovida a hook, detail do hook
    /// (comando real); resolver por QUALQUER dos dois ids emite o canônico (grid).
    #[test]
    fn hook_enriches_grid_born_item_within_window() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "g1"), T0);
        q.observe(
            &DomainEvent::PermissionAsked {
                node_id: "A".into(),
                tool: Some("Bash".into()),
                detail: Some("git push origin/master".into()),
                evidence: PermissionEvidence::Hook,
                stable_id: "h1".into(),
                vt_snapshot_hash: None,
                prompt_kind: PromptKind::Yn,
            },
            T0 + 5_800,
        );
        let items = q.items(T0 + 6_000);
        assert_eq!(items.len(), 1, "1 pedido físico = 1 item (não 2)");
        assert_eq!(items[0].stable_id, "g1", "canônico = quem nasceu primeiro");
        assert_eq!(items[0].evidence, AttentionEvidence::Hook, "promovida");
        assert_eq!(items[0].detail.as_deref(), Some("git push origin/master"));

        // Resolver pelo ALIAS (h1) emite o evento com o canônico (g1).
        let ev = q
            .resolve("h1", ApprovalDecision::Approve, ResolutionVia::Human)
            .expect("alias resolve");
        match &ev {
            DomainEvent::PermissionResolved { stable_id, .. } => assert_eq!(stable_id, "g1"),
            other => panic!("evento errado: {other:?}"),
        }
        q.observe(&ev, T0 + 7_000);
        assert!(q.is_empty(), "resolver via alias remove o item inteiro");
    }

    /// R2b: item `Choice` expõe formato + Captura 1 ao consumidor de UI; `resolve`
    /// devolve `None` (alerta + foco, sem aprovar/recusar — injetar `y` numa caixa de
    /// escolha seria input errado); `dismiss` (botão de FP) continua funcionando.
    #[test]
    fn choice_item_exposes_kind_and_is_not_resolvable() {
        let mut q = AttentionQueue::new();
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "c1",
                PromptKind::Choice,
                Some("cap1-xyz"),
            ),
            T0,
        );
        let items = q.items(T0 + 1_000);
        assert_eq!(items[0].prompt_kind, PromptKind::Choice);
        assert_eq!(items[0].vt_snapshot_hash.as_deref(), Some("cap1-xyz"));
        assert!(
            q.resolve("c1", ApprovalDecision::Approve, ResolutionVia::Human)
                .is_none(),
            "Choice não é aprovável pela fila (defesa além do gate da UI)"
        );
        assert!(
            q.resolve("c1", ApprovalDecision::Deny, ResolutionVia::Timeout)
                .is_none(),
            "nem recusável — o caminho é alerta + foco no terminal"
        );
        assert!(
            q.dismiss("c1").is_some(),
            "FP dismiss vale p/ qualquer formato"
        );
    }

    /// R2b: no merge entre camadas, o GRID é a fonte do formato (é quem VÊ o prompt) —
    /// hook posterior não rebaixa `Choice` p/ `Yn`; grid posterior atualiza o formato
    /// de item nascido do hook; a Captura 1 preenche-se quando ausente.
    #[test]
    fn merge_grid_owns_prompt_kind_and_fills_capture() {
        // Grid-born Choice + cap1; hook (Yn, sem captura) enriquece SEM rebaixar.
        let mut q = AttentionQueue::new();
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "g1",
                PromptKind::Choice,
                Some("cap1-a"),
            ),
            T0,
        );
        q.observe(&ask("A", PermissionEvidence::Hook, "h1"), T0 + 5_000);
        let items = q.items(T0 + 6_000);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].prompt_kind, PromptKind::Choice, "hook não rebaixa");
        assert_eq!(items[0].vt_snapshot_hash.as_deref(), Some("cap1-a"));

        // Hook-born (Yn, sem captura); grid posterior traz formato + Captura 1.
        let mut q2 = AttentionQueue::new();
        q2.observe(&ask("B", PermissionEvidence::Hook, "h2"), T0);
        q2.observe(
            &ask_kind(
                "B",
                PermissionEvidence::Grid,
                "g2",
                PromptKind::Trust,
                Some("cap1-b"),
            ),
            T0 + 2_000,
        );
        let items2 = q2.items(T0 + 3_000);
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].stable_id, "h2", "canônico = quem nasceu primeiro");
        assert_eq!(
            items2[0].prompt_kind,
            PromptKind::Trust,
            "grid define o formato"
        );
        assert_eq!(items2[0].vt_snapshot_hash.as_deref(), Some("cap1-b"));
    }

    /// R2b item 5: prompt respondido DIRETO no terminal (`PermissionPromptCleared`)
    /// remove a pendência — inclusive via ALIAS do merge (o detector emite o id do
    /// lado grid; o item pode ter canônico hook-born). Id desconhecido = no-op.
    /// Sem decisão fabricada: nenhum Resolved/Dismissed no fluxo.
    #[test]
    fn prompt_cleared_removes_item_including_via_alias() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "h1"), T0);
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "g1",
                PromptKind::Yn,
                Some("cap"),
            ),
            T0 + 2_000,
        );
        assert_eq!(q.pending_count(), 1, "merge: 1 pedido físico");
        q.observe(
            &DomainEvent::PermissionPromptCleared {
                node_id: "A".into(),
                stable_id: "g1".into(),
            },
            T0 + 9_000,
        );
        assert!(q.is_empty(), "cleared via alias remove o item inteiro");

        // Id desconhecido: no-op (replay defensivo).
        q.observe(
            &DomainEvent::PermissionPromptCleared {
                node_id: "A".into(),
                stable_id: "nope".into(),
            },
            T0 + 10_000,
        );
        assert!(q.is_empty());
    }

    /// Fora da janela de merge → pedidos DISTINTOS (não funde).
    #[test]
    fn cross_layer_outside_window_stays_two_items() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "g1"), T0);
        q.observe(
            &ask("A", PermissionEvidence::Hook, "h1"),
            T0 + LAYER_MERGE_WINDOW_MS + 1,
        );
        assert_eq!(q.items(T0 + 20_000).len(), 2);
    }

    /// Critério da story: decisão remove da fila + evento correlacionado ao
    /// stable_id; segunda via do MESMO id → `None` (idempotência da fila — clique
    /// duplo não gera segundo evento).
    #[test]
    fn resolve_removes_and_is_idempotent() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0);
        let ev = q
            .resolve("p1", ApprovalDecision::Deny, ResolutionVia::Human)
            .expect("resolve 1");
        q.observe(&ev, T0 + 100);
        assert!(q.is_empty());
        assert!(
            q.resolve("p1", ApprovalDecision::Deny, ResolutionVia::Human)
                .is_none(),
            "já resolvido → None"
        );
        assert!(q.dismiss("p1").is_none(), "já saiu → None");
    }

    /// Botão "não era um pedido": `dismiss` → `PermissionDismissed` → observe remove.
    #[test]
    fn dismiss_emits_and_removes() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "p1"), T0);
        let ev = q.dismiss("p1").expect("dismiss");
        match &ev {
            DomainEvent::PermissionDismissed { stable_id } => assert_eq!(stable_id, "p1"),
            other => panic!("evento errado: {other:?}"),
        }
        q.observe(&ev, T0 + 100);
        assert!(q.is_empty());
    }

    /// Allowlist por nó: mute derruba as pendências de GRID do nó (família de FP),
    /// preserva as de HOOK (estrutural), barra asks de grid novos, e o unmute
    /// (muted:false — REVERSÍVEL, último vence) re-habilita.
    #[test]
    fn mute_drops_grid_keeps_hook_and_is_reversible() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "g1"), T0);
        q.observe(&ask("A", PermissionEvidence::Hook, "h1"), T0 + 60_000);
        q.observe(&ask("B", PermissionEvidence::Grid, "gb"), T0 + 60_000);

        let mute = q.set_node_muted("A", true);
        q.observe(&mute, T0 + 61_000);
        assert!(q.is_node_muted("A"));
        let ids: Vec<String> = q
            .items(T0 + 62_000)
            .into_iter()
            .map(|i| i.stable_id)
            .collect();
        assert!(!ids.contains(&"g1".to_string()), "grid do nó mutado sai");
        assert!(ids.contains(&"h1".to_string()), "hook do nó mutado FICA");
        assert!(ids.contains(&"gb".to_string()), "outro nó intocado");

        // Ask de grid novo do nó mutado não entra (defensivo no fold).
        q.observe(&ask("A", PermissionEvidence::Grid, "g2"), T0 + 63_000);
        assert_eq!(q.pending_count(), 2);

        // Reversível: unmute re-habilita.
        let unmute = q.set_node_muted("A", false);
        q.observe(&unmute, T0 + 64_000);
        assert!(!q.is_node_muted("A"));
        q.observe(&ask("A", PermissionEvidence::Grid, "g3"), T0 + 80_000);
        assert_eq!(q.pending_count(), 3);
    }

    /// Escalação VISUAL aos 5 min (ADR 0021 §3): puro de `now_ms` — antes Pending,
    /// depois Escalated. NENHUM write/auto-deny nasce daqui (fronteira F1-1-8).
    #[test]
    fn escalates_visually_at_five_minutes() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0);
        assert_eq!(
            q.items(T0 + ESCALATE_AFTER_MS - 1)[0].state,
            AttentionState::Pending
        );
        assert_eq!(
            q.items(T0 + ESCALATE_AFTER_MS)[0].state,
            AttentionState::Escalated
        );
    }

    // ───────────── §3 · driver de auto-deny aos 10 min (NUNCA approve) ─────────────

    /// SLA de auto-deny (ADR 0021 §3): pendência de PERMISSÃO **Yn** sem resposta há ≥
    /// [`AUTO_DENY_AFTER_MS`] é candidata a NEGAÇÃO automática — PURO de `now_ms` (sem
    /// timer/thread). NÃO-Yn (`Choice`/`Trust`) fica de fora (alerta+foco; injetar tecla
    /// seria input errado); custódia fica de fora (gate próprio da pump). O threshold é
    /// DISTINTO e MAIOR que o escalate (5 min). O candidato NÃO carrega decisão — quem
    /// drena fixa Deny/Timeout pelo executor; por construção a fila nunca produz approve.
    #[test]
    fn auto_deny_due_after_ten_minutes_yn_only() {
        // A ordem escalate (5 min) < auto-deny (10 min) é invariante de COMPILAÇÃO
        // (`ESCALATE_BEFORE_AUTO_DENY`, no nível do módulo); aqui pino só o valor.
        assert_eq!(AUTO_DENY_AFTER_MS, 600_000, "10 min (calibração ADR 0020)");

        let mut q = AttentionQueue::new();
        // Yn com Captura 1 (o que o executor usará como `expected_hash` no check de tela).
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "yn1",
                PromptKind::Yn,
                Some("cap-yn"),
            ),
            T0,
        );
        // Choice: NUNCA auto-denied pela fila (defesa em profundidade além do gate de UI).
        q.observe(
            &ask_kind(
                "B",
                PermissionEvidence::Grid,
                "ch1",
                PromptKind::Choice,
                Some("cap-ch"),
            ),
            T0,
        );
        // Custódia: gate próprio da pump (⌘⏎) — fora do auto-deny.
        q.custody_enqueued("cust1", "C", "lina do deploy", T0);

        // Antes dos 10 min: nada vence (CONTROLE: os 3 itens existem e seguem pendentes).
        assert!(
            q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS - 1).is_empty(),
            "antes de 10 min: nada vence"
        );
        assert_eq!(
            q.items(T0 + AUTO_DENY_AFTER_MS - 1).len(),
            3,
            "controle positivo: os 3 itens seguem pendentes"
        );

        // Aos 10 min: SÓ o Yn vence; carrega o binding (nó) + a Captura 1.
        let due = q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS);
        assert_eq!(
            due.len(),
            1,
            "só o Yn vence (Choice e custódia ficam de fora)"
        );
        assert_eq!(due[0].stable_id, "yn1");
        assert_eq!(
            due[0].node_id, "A",
            "binding de fonte interna (PermissionAsked), nunca posição da fila"
        );
        assert_eq!(
            due[0].vt_snapshot_hash.as_deref(),
            Some("cap-yn"),
            "Captura 1 vira o expected_hash do check de tela do executor"
        );
    }

    /// Escada do SLA no MESMO item Yn: Pending → Escalated (visual, 5 min) → candidato a
    /// auto-deny (10 min). O escalate é estado VISUAL; o auto-deny é IDENTIFICAÇÃO p/ o
    /// executor — nenhum write/decisão nasce na fila (fronteira F1-1-8). Item hook-only
    /// (sem Captura 1) também vence: o executor aborta fail-safe (deny-não-entregue).
    #[test]
    fn sla_ladder_escalate_then_auto_deny() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0); // hook-only: sem Captura 1

        // 5 min: visual Escalated, mas AINDA não vence o auto-deny.
        assert_eq!(
            q.items(T0 + ESCALATE_AFTER_MS)[0].state,
            AttentionState::Escalated
        );
        assert!(
            q.auto_deny_due(T0 + ESCALATE_AFTER_MS).is_empty(),
            "5 min escala (visual), não nega"
        );

        // 10 min: continua Escalated E agora vence o auto-deny (Captura 1 ausente → None).
        assert_eq!(
            q.items(T0 + AUTO_DENY_AFTER_MS)[0].state,
            AttentionState::Escalated
        );
        let due = q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].vt_snapshot_hash, None, "hook-only não tem Captura 1");
    }

    /// Replay defensivo: o MESMO stable_id duas vezes no log (não deveria, mas o log
    /// é histórico) não duplica o item.
    #[test]
    fn duplicate_stable_id_in_log_does_not_duplicate_item() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "p1"), T0);
        q.observe(&ask("A", PermissionEvidence::Grid, "p1"), T0 + 100);
        assert_eq!(q.pending_count(), 1);
    }

    // ───────────── replay contra um EventStore real (critério da story) ─────────────

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir()
                .join(format!("lina-f117-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            self.0.as_ref()
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Critério da story: crash com pendência → reabrir reconstrói a fila do log.
    /// 3 asks + 1 resolved + 1 dismissed no store; `replay` devolve SÓ a pendência
    /// viva, idêntica à da fila que viveu o fluxo (replay ≡ live).
    #[test]
    #[serial]
    fn replay_from_store_rebuilds_pending_queue() {
        let tmp = TempDir::new("replay");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let mut live = AttentionQueue::new();

        let evs = [
            ask("A", PermissionEvidence::Hook, "p1"),
            ask("B", PermissionEvidence::Grid, "p2"),
            ask("C", PermissionEvidence::Hook, "p3"),
        ];
        for ev in &evs {
            store.append(ev).expect("append ask");
        }
        let resolve = DomainEvent::PermissionResolved {
            stable_id: "p1".into(),
            decision: ApprovalDecision::Approve,
            via: ResolutionVia::Human,
        };
        store.append(&resolve).expect("append resolved");
        let dismiss = DomainEvent::PermissionDismissed {
            stable_id: "p3".into(),
        };
        store.append(&dismiss).expect("append dismissed");

        // A fila viva consome os MESMOS fatos com os ts persistidos.
        let records = store.events().expect("events");
        for rec in &records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                live.observe(&ev, rec.ts);
            }
        }

        // "Crash + reabrir": reconstrói do log e compara com a fila que viveu.
        let rebuilt = AttentionQueue::replay(&records);
        let now = records.last().map(|r| r.ts).unwrap_or(T0);
        assert_eq!(rebuilt.items(now), live.items(now), "replay ≡ live");
        let ids: Vec<String> = rebuilt
            .items(now)
            .into_iter()
            .map(|i| i.stable_id)
            .collect();
        assert_eq!(ids, vec!["p2"], "só a pendência não-resolvida sobrevive");
    }

    /// INVARIANTE do fix P0 (anti-travamento): aplicar SÓ os eventos novos (`seq > cursor`) a uma
    /// fila viva, em dois lotes, produz EXATAMENTE o mesmo estado que reconstruir do log inteiro
    /// (`replay`). É a equivalência que autoriza tirar o full-replay `O(N)` da thread de UI.
    #[test]
    #[serial]
    fn incremental_observe_records_equals_full_replay() {
        let tmp = TempDir::new("incremental");
        let mut store = EventStore::open(tmp.path()).expect("open");
        for ev in [
            ask("A", PermissionEvidence::Hook, "p1"),
            ask("B", PermissionEvidence::Grid, "p2"),
            ask("C", PermissionEvidence::Hook, "p3"),
            DomainEvent::PermissionResolved {
                stable_id: "p1".into(),
                decision: ApprovalDecision::Approve,
                via: ResolutionVia::Human,
            },
            ask("D", PermissionEvidence::Hook, "p4"),
            DomainEvent::PermissionDismissed {
                stable_id: "p3".into(),
            },
        ] {
            store.append(&ev).expect("append");
        }
        let all = store.events().expect("events");
        let now = all.last().map(|r| r.ts).unwrap_or(T0);

        // INCREMENTAL: fila viva recebe o lote [seq<=3] e depois SÓ os novos (seq>3) — cursor.
        let split = 3usize;
        let mut live = AttentionQueue::new();
        live.observe_records(&all[..split]);
        let cursor = all[..split].last().map(|r| r.seq).unwrap_or(0);
        let new_only: Vec<EventRecord> = all.iter().filter(|r| r.seq > cursor).cloned().collect();
        assert_eq!(
            new_only.len(),
            all.len() - split,
            "filtro seq>cursor pega só os novos"
        );
        live.observe_records(&new_only);

        // FULL-REPLAY: reconstrói do log inteiro.
        let full = AttentionQueue::replay(&all);

        assert_eq!(
            live.items(now),
            full.items(now),
            "incremental ≡ full-replay"
        );
    }

    /// **SEAM-1: banner de spawn cascata.** `SpawnRequested`+`SpawnGated{cascade}` → item `Spawn`
    /// pendente; ORIGEM (sem `SpawnGated`) NÃO vira banner; `decline_spawn` devolve `SpawnDeclined`
    /// e, observado, tira o banner.
    #[test]
    fn spawn_cascade_becomes_pending_banner_and_declines() {
        let by = uuid::Uuid::from_u128(1);
        let mut q = AttentionQueue::new();

        // ORIGEM: SpawnRequested SEM SpawnGated → NÃO é banner (auto-admite, sem gate humano).
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "msg_orig".into(),
                requested_by: by,
                name: "@A".into(),
                role: "qa".into(),
                root_cause_id: "msg_orig".into(),
                hops: 0,
                prompt: "x".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0,
        );
        assert!(
            q.items(T0).iter().all(|i| i.kind != AttentionKind::Spawn),
            "origem (sem SpawnGated) não vira banner"
        );

        // CASCATA: SpawnRequested + SpawnGated{cascade} → 1 banner pendente.
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "msg_casc".into(),
                requested_by: by,
                name: "@Helper".into(),
                role: "helper".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "ajude".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0 + 1,
        );
        q.observe(
            &DomainEvent::SpawnGated {
                id: "msg_casc".into(),
                requested_by: by,
                reason: "cascade".into(),
            },
            T0 + 2,
        );
        let spawn: Vec<_> = q
            .items(T0 + 3)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::Spawn)
            .collect();
        assert_eq!(spawn.len(), 1, "cascata → 1 banner pendente");
        assert_eq!(spawn[0].stable_id, "msg_casc");
        assert!(
            spawn[0]
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("helper"),
            "copy leiga traz o papel pedido"
        );
        assert!(q.is_pending_spawn("msg_casc"));

        // RECUSA: decline_spawn → SpawnDeclined; observado, sai do banner.
        let decl = q.decline_spawn("msg_casc").expect("declinável");
        assert!(matches!(&decl, DomainEvent::SpawnDeclined { id } if id == "msg_casc"));
        q.observe(&decl, T0 + 4);
        assert!(!q.is_pending_spawn("msg_casc"), "recusado → sai do banner");
    }

    /// Aprovação: `SpawnAdmitted` (apendado pelo app no admit) tira o banner (resolução + dedupe M3).
    #[test]
    fn spawn_admitted_clears_banner() {
        let by = uuid::Uuid::from_u128(1);
        let node = uuid::Uuid::from_u128(2);
        let mut q = AttentionQueue::new();
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "m".into(),
                requested_by: by,
                name: "@H".into(),
                role: "h".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "p".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0,
        );
        q.observe(
            &DomainEvent::SpawnGated {
                id: "m".into(),
                requested_by: by,
                reason: "cascade".into(),
            },
            T0 + 1,
        );
        assert!(q.is_pending_spawn("m"));
        q.observe(
            &DomainEvent::SpawnAdmitted {
                id: "m".into(),
                node,
            },
            T0 + 2,
        );
        assert!(
            !q.is_pending_spawn("m"),
            "admitido → sai do banner (resolvido)"
        );
    }

    // ───────────────────────── FIX-2: ask do guard (hook PreToolUse) ─────────────────────────

    /// `ActionGated{decision:"ask", node:Some(nó)}` — o ask do guard que bloqueia o agente.
    fn guard_ask(node: &str, cmd: &str) -> DomainEvent {
        DomainEvent::ActionGated {
            cmd: cmd.into(),
            class: "gated-hard".into(),
            decision: "ask".into(),
            node: Some(node.into()),
        }
    }

    /// **FIX-2: o ASK do guard vira item `GuardAsk` (alerta + foco).** Carrega o NÓ (foco POR NOME)
    /// e o COMANDO (copy leiga "vá ao terminal aprovar X"). É o buraco do dogfooding: o guard
    /// bloqueia mas o detector F1-1-6 não pega o formato do dialog de hook-ask.
    #[test]
    fn guard_ask_becomes_guard_ask_item() {
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("Bug Finder", "git push --force origin main"), T0);
        let items = q.items(T0 + 1_000);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, AttentionKind::GuardAsk);
        assert_eq!(items[0].node_id, "Bug Finder", "nó p/ foco por nome");
        assert_eq!(
            items[0].detail.as_deref(),
            Some("git push --force origin main")
        );
    }

    /// **FIX-2: dedup por nó (N→1).** O guard dispara 1×/tool gated; o MESMO nó nunca empilha — 1
    /// item, e o último ask renova o comando exibido.
    #[test]
    fn guard_ask_dedups_per_node_keeping_latest_cmd() {
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("A", "rm -rf build"), T0);
        q.observe(&guard_ask("A", "git push --force"), T0 + 5_000);
        q.observe(&guard_ask("A", "deploy prod"), T0 + 9_000);
        let items: Vec<_> = q
            .items(T0 + 10_000)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::GuardAsk)
            .collect();
        assert_eq!(items.len(), 1, "3 asks do mesmo nó → 1 item");
        assert_eq!(
            items[0].detail.as_deref(),
            Some("deploy prod"),
            "último comando renova a copy"
        );
    }

    /// **FIX-2: só `ask` COM nó alerta.** `deny`/`allow` (não bloqueiam o humano) e `ask` SEM nó
    /// (custódia/log antigo — sem terminal p/ focar) NUNCA viram item.
    #[test]
    fn deny_allow_and_node_less_ask_never_alert() {
        let mut q = AttentionQueue::new();
        let mk = |decision: &str, node: Option<&str>| DomainEvent::ActionGated {
            cmd: "x".into(),
            class: "gated-hard".into(),
            decision: decision.into(),
            node: node.map(str::to_string),
        };
        q.observe(&mk("deny", Some("A")), T0);
        q.observe(&mk("allow", Some("A")), T0 + 1);
        q.observe(&mk("ask", None), T0 + 2); // custódia / log antigo
        assert!(
            q.items(T0 + 1_000)
                .iter()
                .all(|i| i.kind != AttentionKind::GuardAsk),
            "nenhum desses vira GuardAsk"
        );
    }

    /// **FIX-2: resolução v1 por TTL.** O item some `GUARD_ASK_TTL_MS` após o ÚLTIMO ask (no core
    /// não há sinal de resolução determinístico — `recolhe-no-Busy` é follow-up de app). Um novo ask
    /// antes de expirar RENOVA a janela.
    #[test]
    fn guard_ask_expires_by_ttl_and_renews() {
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("A", "deploy"), T0);
        assert_eq!(
            q.items(T0 + GUARD_ASK_TTL_MS - 1).len(),
            1,
            "vivo dentro do TTL"
        );
        assert_eq!(q.items(T0 + GUARD_ASK_TTL_MS).len(), 0, "expira no TTL");
        // Novo ask 1ms antes de expirar → renova a janela (dedup + renovação do last_ts).
        q.observe(&guard_ask("A", "deploy"), T0 + GUARD_ASK_TTL_MS - 1);
        assert_eq!(
            q.items(T0 + GUARD_ASK_TTL_MS + 100).len(),
            1,
            "novo ask renova o TTL"
        );
    }

    /// **FIX-2: precedência custódia > permissão > spawn > guard-ask.** O ask do guard fica ABAIXO de
    /// tudo (um turno travado por permissão/custódia/spawn decide primeiro), mas NUNCA invisível.
    #[test]
    fn guard_ask_precedence_is_last() {
        let by = uuid::Uuid::from_u128(1);
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("G", "deploy"), T0);
        q.observe(&ask("P", PermissionEvidence::Hook, "p1"), T0 + 1);
        q.custody_enqueued("c1", "C", "lina do deploy", T0 + 2);
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "s1".into(),
                requested_by: by,
                name: "@H".into(),
                role: "h".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "p".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0 + 3,
        );
        q.observe(
            &DomainEvent::SpawnGated {
                id: "s1".into(),
                requested_by: by,
                reason: "cascade".into(),
            },
            T0 + 4,
        );
        let kinds: Vec<AttentionKind> = q.items(T0 + 5).into_iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AttentionKind::Custody,
                AttentionKind::Permission,
                AttentionKind::Spawn,
                AttentionKind::GuardAsk,
            ]
        );
    }

    /// **FIX-2: crash + reabrir reconstrói o GuardAsk (replay ≡ live).** O `ActionGated{ask}` está no
    /// log → reabrir a fila re-mostra o alerta (durabilidade — invariantes #4/#6).
    #[test]
    #[serial]
    fn guard_ask_survives_replay() {
        let tmp = TempDir::new("guardask");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .append(&guard_ask("Bug Finder", "deploy prod"))
            .expect("append");
        let records = store.events().expect("events");
        let now = records.last().map(|r| r.ts).unwrap_or(T0) + 1_000;
        let rebuilt = AttentionQueue::replay(&records);
        let items: Vec<_> = rebuilt
            .items(now)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::GuardAsk)
            .collect();
        assert_eq!(items.len(), 1, "GuardAsk reconstruído do log");
        assert_eq!(items[0].node_id, "Bug Finder");
    }

    // ───────────── W3 · alarme do "despacho engolido" (ponto-cego do ADR 0019 §4) ─────────────

    use crate::{lifecycle::reason as lc_reason, NodeId, NodeStatus as NS};

    fn nid(n: u128) -> NodeId {
        NodeId::from_u128(n)
    }
    /// `MessageDelivered{to}` — ARMA o vigia (o `id` não importa para o alarme).
    fn delivered(to: NodeId) -> DomainEvent {
        DomainEvent::MessageDelivered { id: "m".into(), to }
    }
    fn status_ev(node: NodeId, status: NS, why: &str) -> DomainEvent {
        DomainEvent::NodeStatusChanged {
            node,
            status: status.as_str().to_string(),
            from: String::new(),
            reason: why.to_string(),
        }
    }
    /// Retorno a `Idle` por fim-de-resposta (o que abre o ponto-cego no #22/#23).
    fn idle(node: NodeId) -> DomainEvent {
        status_ev(node, NS::Idle, lc_reason::END_OF_RESPONSE)
    }
    /// Uso de tokens de um turno REAL (o app emite ao fim do turno) — o discriminador.
    fn token(node: NodeId, tokens: u64) -> DomainEvent {
        DomainEvent::TokenUsageReported {
            node: node.to_string(),
            tokens,
        }
    }
    /// Os `node_id`s dos alarmes de engolido visíveis em `now`.
    fn swallowed(q: &AttentionQueue, now: u64) -> Vec<String> {
        q.items(now)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::DeliveredNoProgress)
            .map(|i| i.node_id)
            .collect()
    }

    /// Critério-1 da story: entregue → voltou a Idle → janela sem progresso → **1** alarme. Antes
    /// da janela (medida do retorno a Idle), nada. O item é Choice (alerta+foco), NUNCA aprovável
    /// pela fila e NUNCA candidato a auto-deny — defesa em profundidade além do tipo.
    #[test]
    fn swallowed_dispatch_raises_one_alarm_after_window() {
        let a = nid(1);
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        // No #22/#23 o nó volta a Idle logo (o turno ANTERIOR fechando), sem rodar o despacho.
        q.observe(&idle(a), T0 + 1_000);

        let idle_ts = T0 + 1_000;
        assert!(
            swallowed(&q, idle_ts + DELIVERED_NO_PROGRESS_WINDOW_MS - 1).is_empty(),
            "dentro da graça: ainda não alarma"
        );

        let now = idle_ts + DELIVERED_NO_PROGRESS_WINDOW_MS;
        let items: Vec<_> = q
            .items(now)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::DeliveredNoProgress)
            .collect();
        assert_eq!(
            items.len(),
            1,
            "engolido dispara 1 item (anti-amplificação)"
        );
        assert_eq!(
            items[0].node_id,
            a.to_string(),
            "stable_id do nó, nunca posição"
        );
        assert_eq!(items[0].prompt_kind, PromptKind::Choice);
        assert_eq!(
            items[0].detail.as_deref(),
            Some("recebeu a tarefa e ainda não começou")
        );
        assert!(
            q.resolve(
                &items[0].stable_id,
                ApprovalDecision::Deny,
                ResolutionVia::Timeout
            )
            .is_none(),
            "alarme de observabilidade NÃO é aprovável/recusável pela fila"
        );
        assert!(
            q.auto_deny_due(now).is_empty(),
            "engolido nunca entra no driver de auto-deny (só permissões Yn)"
        );
    }

    /// Sem retorno a `Idle` (nó segue `Busy`/trabalhando) → NUNCA alarma, por mais que demore: esse
    /// é o turf do stall detector (ADR 0019 §3). ZERO sobreposição — o W3 só cobre o ponto-cego.
    #[test]
    fn busy_without_idle_never_alarms() {
        let a = nid(1);
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        assert!(
            swallowed(&q, T0 + 20 * DELIVERED_NO_PROGRESS_WINDOW_MS).is_empty(),
            "Busy sem Idle é do stall detector, não deste alarme"
        );
    }

    /// Critério-2 da story (turno legítimo NÃO dispara): o turno real consome tokens → o app emite
    /// `TokenUsageReported` → desarma. O engolido (zero turno, zero token) é o que sobra. Vale nas
    /// DUAS ordens (token antes OU depois do Idle) — o fold é robusto à ordem de chegada no log.
    #[test]
    fn token_usage_disarms_legit_turn_either_order() {
        let a = nid(1);
        let b = nid(2);

        // (i) Idle e DEPOIS o uso de tokens (ordem comum: fim-de-turno → contabilização).
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        q.observe(&idle(a), T0 + 5_000);
        q.observe(&token(a, 1_234), T0 + 6_000);
        assert!(
            swallowed(&q, T0 + 5 * DELIVERED_NO_PROGRESS_WINDOW_MS).is_empty(),
            "turno real (tokens após Idle) não dispara"
        );

        // (ii) Uso de tokens ANTES do Idle — também desarma (sem vigia = sem alarme).
        let mut q2 = AttentionQueue::new();
        q2.observe(&delivered(b), T0);
        q2.observe(&token(b, 99), T0 + 4_000);
        q2.observe(&idle(b), T0 + 5_000);
        assert!(
            swallowed(&q2, T0 + 5 * DELIVERED_NO_PROGRESS_WINDOW_MS).is_empty(),
            "turno real (tokens antes do Idle) não dispara"
        );
    }

    /// Progresso por OUTPUT novo (`NodeStatusChanged{Busy, pty_output}` — proxy do tail_hash, que é
    /// efêmero/fora do log) desarma. E o carimbo `a2a_delivery` da PRÓPRIA entrega (router.rs) NÃO
    /// é trabalho: não pode desarmar — senão todo despacho se auto-limparia.
    #[test]
    fn pty_output_disarms_but_a2a_delivery_stamp_does_not() {
        let a = nid(1);
        let b = nid(2);

        // pty_output = trabalho real → desarma.
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        q.observe(&status_ev(a, NS::Busy, lc_reason::PTY_OUTPUT), T0 + 2_000);
        q.observe(&idle(a), T0 + 50_000);
        assert!(
            swallowed(&q, T0 + 5 * DELIVERED_NO_PROGRESS_WINDOW_MS).is_empty(),
            "output novo desarma"
        );

        // a2a_delivery (carimbo do router, antes E depois da entrega) NÃO conta como progresso.
        let mut q2 = AttentionQueue::new();
        q2.observe(&status_ev(b, NS::Busy, "a2a_delivery"), T0);
        q2.observe(&delivered(b), T0 + 100);
        q2.observe(&status_ev(b, NS::Busy, "a2a_delivery"), T0 + 200);
        q2.observe(&idle(b), T0 + 1_000);
        assert_eq!(
            swallowed(&q2, T0 + 1_000 + DELIVERED_NO_PROGRESS_WINDOW_MS).len(),
            1,
            "o carimbo da entrega não é trabalho — o engolido segue alarmando"
        );
    }

    /// O nó que DELEGOU adiante (`MessageRouted{from}`) agiu → progresso atribuível ao remetente,
    /// desarma. Receber (ser `to`) NÃO é progresso (é o gatilho).
    #[test]
    fn node_sending_message_disarms() {
        let a = nid(1);
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        q.observe(&idle(a), T0 + 1_000);
        q.observe(
            &DomainEvent::MessageRouted {
                id: "x".into(),
                from: a,
                to: "B".into(),
                intent: "handoff".into(),
                root_cause_id: String::new(),
                hops: 0,
                to_node: None,
            },
            T0 + 2_000,
        );
        assert!(
            swallowed(&q, T0 + 5 * DELIVERED_NO_PROGRESS_WINDOW_MS).is_empty(),
            "delegar adiante é progresso"
        );
    }

    /// A 2ª tentativa idêntica do #22/#23 (que "funciona") RE-ARMA: a entrega mais nova zera o
    /// relógio (idle_ts volta a None) e o progresso seguinte limpa — sem alarme órfão da 1ª.
    #[test]
    fn redelivery_rearms_and_clears() {
        let a = nid(1);
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        q.observe(&idle(a), T0 + 1_000);
        assert_eq!(
            swallowed(&q, T0 + 100_000).len(),
            1,
            "a 1ª entrega foi engolida — alarma"
        );

        // 2ª entrega (a que funciona): re-arma, zerando o relógio no MESMO instante.
        q.observe(&delivered(a), T0 + 100_000);
        assert!(
            swallowed(&q, T0 + 100_000).is_empty(),
            "re-entrega zera o relógio (idle_ts = None)"
        );
        // E agora progride de verdade.
        q.observe(&token(a, 50), T0 + 101_000);
        assert!(
            swallowed(&q, T0 + 500_000).is_empty(),
            "a 2ª entrega virou trabalho"
        );
    }

    /// Nó MORTO/removido após a entrega → sem alarme (não há terminal para o Maestro re-despachar).
    #[test]
    fn dead_or_removed_node_disarms() {
        let a = nid(1);
        let b = nid(2);

        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        q.observe(&idle(a), T0 + 1_000);
        q.observe(&status_ev(a, NS::Dead, lc_reason::PTY_EXIT), T0 + 2_000);
        assert!(swallowed(&q, T0 + 500_000).is_empty(), "nó Dead não alarma");

        let mut q2 = AttentionQueue::new();
        q2.observe(&delivered(b), T0);
        q2.observe(&idle(b), T0 + 1_000);
        q2.observe(&DomainEvent::NodeRemoved { node: b }, T0 + 2_000);
        assert!(
            swallowed(&q2, T0 + 500_000).is_empty(),
            "nó removido não alarma"
        );
    }

    /// Atribuição correta + 1-por-nó: A engolido, B trabalhou (tokens) → só A alarma. Dois nós, um
    /// alarme — prova que o progresso de B não silencia A nem vice-versa.
    #[test]
    fn two_nodes_only_the_swallowed_one_alarms() {
        let a = nid(1);
        let b = nid(2);
        let mut q = AttentionQueue::new();
        q.observe(&delivered(a), T0);
        q.observe(&delivered(b), T0);
        q.observe(&idle(a), T0 + 1_000);
        q.observe(&idle(b), T0 + 1_000);
        q.observe(&token(b, 100), T0 + 2_000); // B trabalhou; A não

        assert_eq!(
            swallowed(&q, T0 + 200_000),
            vec![a.to_string()],
            "só o engolido (A); B desarmou"
        );
    }

    /// Critério-3 da story (replay reconstrói a fila idêntica): num `EventStore` real, entregar +
    /// voltar a Idle; `replay` ≡ live e o alarme do engolido sobrevive à reconstrução.
    #[test]
    #[serial]
    fn replay_rebuilds_swallowed_alarm() {
        let tmp = TempDir::new("w3-swallowed");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let mut live = AttentionQueue::new();
        let a = nid(7);

        for ev in [delivered(a), idle(a)] {
            store.append(&ev).expect("append");
        }
        let records = store.events().expect("events");
        for rec in &records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                live.observe(&ev, rec.ts);
            }
        }

        let idle_ts = records.last().map(|r| r.ts).unwrap_or(T0);
        let now = idle_ts + DELIVERED_NO_PROGRESS_WINDOW_MS + 1;
        let rebuilt = AttentionQueue::replay(&records);
        assert_eq!(rebuilt.items(now), live.items(now), "replay ≡ live");
        assert_eq!(
            swallowed(&rebuilt, now),
            vec![a.to_string()],
            "o engolido sobrevive ao replay"
        );
    }

    // ───────── F3-4-4 (spec 36 §2): conflito de código por pertencimento ─────────

    /// `EventRecord` sintético do PRÓPRIO `DomainEvent` (carrega a tag interna), `ts` controlado —
    /// exercita o decode REAL (`from_record`) no replay, como em `mentality.rs`.
    fn cc_rec(event: DomainEvent, ts: u64) -> EventRecord {
        EventRecord {
            seq: 0,
            ts,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa evento"),
        }
    }

    fn attributed(item: &str, paths: &[&str]) -> DomainEvent {
        DomainEvent::PlanItemAttributed {
            item: item.into(),
            goal_id: None,
            parents: vec![],
            acceptance: vec![],
            budget_tokens: 0,
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
        }
    }

    fn claimed(item: &str, by: &str) -> DomainEvent {
        DomainEvent::PlanClaimed {
            id: format!("claim-{item}-{by}"),
            item: item.into(),
            by: by.into(),
        }
    }

    fn code_changed(branch: &str, paths: &[&str], author: &str) -> DomainEvent {
        DomainEvent::CodeChanged {
            branch: branch.into(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
            author_node: author.into(),
            commit: format!("commit-{branch}"),
        }
    }

    /// Controle +: item reservado por @B; um commit de @A toca o arquivo → @B vê um `CodeConflict`
    /// com `stable_id`, `Choice` (alerta+foco) e a copy leiga com o path em conflito.
    #[test]
    fn code_conflict_fires_on_intersecting_claim_of_another_node() {
        let mut q = AttentionQueue::new();
        q.observe(&attributed("T1", &["src/leads.rs"]), T0);
        q.observe(&claimed("T1", "@B"), T0 + 1);
        q.observe(
            &code_changed("lina/a", &["src/leads.rs", "src/mod.rs"], "@A"),
            T0 + 2,
        );
        let items = q.items(T0 + 3);
        let conflict = items
            .iter()
            .find(|i| i.kind == AttentionKind::CodeConflict)
            .expect("um conflito de código");
        assert_eq!(conflict.node_id, "@B", "o DONO do claim PARA");
        assert_eq!(conflict.stable_id, "code-conflict:@B:lina/a");
        assert_eq!(
            conflict.prompt_kind,
            PromptKind::Choice,
            "alerta+foco, NUNCA Yn aprovável"
        );
        assert!(conflict
            .detail
            .as_deref()
            .expect("detail")
            .contains("src/leads.rs"));
    }

    /// Controle −: o commit não toca nada reservado → nenhum conflito (estado inalterado).
    #[test]
    fn no_conflict_when_paths_are_disjoint() {
        let mut q = AttentionQueue::new();
        q.observe(&attributed("T1", &["src/leads.rs"]), T0);
        q.observe(&claimed("T1", "@B"), T0 + 1);
        q.observe(&code_changed("lina/a", &["src/outro.rs"], "@A"), T0 + 2);
        assert!(
            q.items(T0 + 3)
                .iter()
                .all(|i| i.kind != AttentionKind::CodeConflict),
            "sem interseção → sem conflito"
        );
    }

    /// Controle −: o PRÓPRIO dono commitando no que reservou NÃO gera conflito (é o trabalho dele).
    #[test]
    fn own_commit_does_not_conflict() {
        let mut q = AttentionQueue::new();
        q.observe(&attributed("T1", &["src/leads.rs"]), T0);
        q.observe(&claimed("T1", "@B"), T0 + 1);
        q.observe(&code_changed("lina/b", &["src/leads.rs"], "@B"), T0 + 2);
        assert!(
            q.items(T0 + 3)
                .iter()
                .all(|i| i.kind != AttentionKind::CodeConflict),
            "autor == dono → sem conflito"
        );
    }

    /// Item concluído (`PlanChecked`) LIBERA a reserva → commit posterior não conflita.
    #[test]
    fn checked_item_releases_reservation() {
        let mut q = AttentionQueue::new();
        q.observe(&attributed("T1", &["src/leads.rs"]), T0);
        q.observe(&claimed("T1", "@B"), T0 + 1);
        q.observe(
            &DomainEvent::PlanChecked {
                id: "chk".into(),
                item: "T1".into(),
                by: "@B".into(),
            },
            T0 + 2,
        );
        q.observe(&code_changed("lina/a", &["src/leads.rs"], "@A"), T0 + 3);
        assert!(
            q.items(T0 + 4)
                .iter()
                .all(|i| i.kind != AttentionKind::CodeConflict),
            "claim liberado → sem conflito"
        );
    }

    /// Commits sucessivos do mesmo colega na mesma branch DEDUPLICAM (1 item, paths unidos).
    #[test]
    fn successive_commits_same_pair_dedup_and_union_paths() {
        let mut q = AttentionQueue::new();
        q.observe(&attributed("T1", &["src/a.rs", "src/b.rs"]), T0);
        q.observe(&claimed("T1", "@B"), T0 + 1);
        q.observe(&code_changed("lina/a", &["src/a.rs"], "@A"), T0 + 2);
        q.observe(&code_changed("lina/a", &["src/b.rs"], "@A"), T0 + 3);
        let conflicts: Vec<_> = q
            .items(T0 + 4)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::CodeConflict)
            .collect();
        assert_eq!(conflicts.len(), 1, "mesmo (owner,branch) → 1 item");
        let detail = conflicts[0].detail.clone().expect("detail");
        assert!(
            detail.contains("src/a.rs") && detail.contains("src/b.rs"),
            "paths unidos"
        );
    }

    /// Precedência da família guard-ask: o conflito sai DEPOIS do guard e ANTES do engolido.
    #[test]
    fn code_conflict_precedence_between_guard_and_swallowed() {
        // (1) guard + conflito, consultados a fresco: guard antes do conflito.
        let mut q1 = AttentionQueue::new();
        q1.observe(&guard_ask("@G", "git push"), T0);
        q1.observe(&attributed("T1", &["src/x.rs"]), T0 + 1);
        q1.observe(&claimed("T1", "@B"), T0 + 2);
        q1.observe(&code_changed("lina/a", &["src/x.rs"], "@A"), T0 + 3);
        let kinds1: Vec<AttentionKind> = q1.items(T0 + 10).iter().map(|i| i.kind).collect();
        let gi = kinds1
            .iter()
            .position(|k| *k == AttentionKind::GuardAsk)
            .expect("guard");
        let ci = kinds1
            .iter()
            .position(|k| *k == AttentionKind::CodeConflict)
            .expect("conflito");
        assert!(gi < ci, "guard antes do conflito");

        // (2) conflito + engolido, consultados além da janela: conflito antes do engolido.
        let mut q2 = AttentionQueue::new();
        q2.observe(&attributed("T1", &["src/x.rs"]), T0);
        q2.observe(&claimed("T1", "@B"), T0 + 1);
        q2.observe(&code_changed("lina/a", &["src/x.rs"], "@A"), T0 + 2);
        q2.observe(&delivered(nid(7)), T0 + 3);
        q2.observe(&idle(nid(7)), T0 + 4);
        let now = T0 + 4 + DELIVERED_NO_PROGRESS_WINDOW_MS + 1;
        let kinds2: Vec<AttentionKind> = q2.items(now).iter().map(|i| i.kind).collect();
        let ci2 = kinds2
            .iter()
            .position(|k| *k == AttentionKind::CodeConflict)
            .expect("conflito");
        let si = kinds2
            .iter()
            .position(|k| *k == AttentionKind::DeliveredNoProgress)
            .expect("engolido");
        assert!(ci2 < si, "conflito antes do engolido");
    }

    /// `author_node` FORJADO no payload é IGNORADO: o que importa é o que o supervisor carimba
    /// (server-side). Aqui provamos que a projeção compara contra o `author_node` do EVENTO (o
    /// carimbado), não contra um campo do item — o handler do router é quem garante o carimbo.
    #[test]
    fn conflict_uses_event_author_not_item_owner_field() {
        // @B reservou; o commit é de @A (carimbado no evento) → conflito para @B.
        let mut q = AttentionQueue::new();
        q.observe(&attributed("T1", &["src/x.rs"]), T0);
        q.observe(&claimed("T1", "@B"), T0 + 1);
        q.observe(&code_changed("lina/a", &["src/x.rs"], "@A"), T0 + 2);
        let c = q
            .items(T0 + 3)
            .into_iter()
            .find(|i| i.kind == AttentionKind::CodeConflict)
            .expect("conflito");
        assert_eq!(c.node_id, "@B");
    }

    /// **replay idempotente (gate f):** reconstruir do log duas vezes dá os MESMOS itens; e o
    /// conflito SOBREVIVE ao replay (a fila reconstrói o `CodeConflict` do log).
    #[test]
    fn code_conflict_replay_is_idempotent() {
        let log = vec![
            cc_rec(attributed("T1", &["src/leads.rs"]), T0),
            cc_rec(claimed("T1", "@B"), T0 + 1),
            cc_rec(code_changed("lina/a", &["src/leads.rs"], "@A"), T0 + 2),
        ];
        let a = AttentionQueue::replay(&log);
        let b = AttentionQueue::replay(&log);
        assert_eq!(a.items(T0 + 3), b.items(T0 + 3), "replay determinístico");
        assert_eq!(
            a.items(T0 + 3)
                .iter()
                .filter(|i| i.kind == AttentionKind::CodeConflict)
                .count(),
            1,
            "o conflito sobrevive ao replay"
        );
    }

    // ───────────────── F3-5-8 · proposta de poda como gate Custody (ADR 0043) ─────────────────

    fn disk_proposed(path: &str, reclaimable: u64) -> DomainEvent {
        DomainEvent::DiskReclaimProposed {
            candidates: vec![ReclaimCandidate {
                path: path.to_string(),
                bytes: reclaimable,
                kind: "cargo_target".to_string(),
            }],
            reclaimable_bytes: reclaimable,
            proposed_at_ms: 0,
        }
    }

    #[test]
    fn disk_reclaim_proposed_enters_as_custody() {
        let mut q = AttentionQueue::new();
        q.observe(&disk_proposed("/ws/target", 30 * (1 << 30)), T0);
        let items = q.items(T0 + 1);
        let disk = items
            .iter()
            .find(|i| i.stable_id == "disk-reclaim:/ws/target")
            .expect("a proposta de poda entra na fila");
        assert_eq!(disk.kind, AttentionKind::Custody, "gate humano duro");
        assert_eq!(disk.evidence, AttentionEvidence::Custody);
        let detail = disk.detail.as_deref().unwrap_or_default();
        assert!(
            detail.contains("liberar") && detail.contains("GB"),
            "copy leiga: {detail}"
        );
    }

    #[test]
    fn disk_reclaim_executed_resolves_the_gate() {
        let mut q = AttentionQueue::new();
        q.observe(&disk_proposed("/ws/target", 30 * (1 << 30)), T0);
        q.observe(
            &DomainEvent::DiskReclaimExecuted {
                reclaimed_bytes: 30 * (1 << 30),
                paths: vec!["/ws/target".to_string()],
                executed_at_ms: 0,
            },
            T0 + 2,
        );
        assert!(
            !q.items(T0 + 3)
                .iter()
                .any(|i| i.stable_id == "disk-reclaim:/ws/target"),
            "a poda executada resolve o gate"
        );
    }

    #[test]
    fn disk_reclaim_approved_alone_does_not_resolve_gate() {
        // `approved_by` é EXIBIÇÃO: um `DiskReclaimApproved` (mesmo forjado) NÃO limpa o gate — só a
        // execução real (Executed) resolve. A autoridade é o gesto, não o campo (ADR 0007).
        let mut q = AttentionQueue::new();
        q.observe(&disk_proposed("/ws/target", 30 * (1 << 30)), T0);
        q.observe(
            &DomainEvent::DiskReclaimApproved {
                candidate_paths: vec!["/ws/target".to_string()],
                approved_by: "forjado".to_string(),
                approved_at_ms: 0,
            },
            T0 + 2,
        );
        assert!(
            q.items(T0 + 3)
                .iter()
                .any(|i| i.stable_id == "disk-reclaim:/ws/target"),
            "approved_by não resolve o gate (só a execução custodiada o faz)"
        );
    }

    #[test]
    fn disk_reclaim_proposed_dedups_same_paths() {
        let mut q = AttentionQueue::new();
        q.observe(&disk_proposed("/ws/target", 30 * (1 << 30)), T0);
        q.observe(&disk_proposed("/ws/target", 30 * (1 << 30)), T0 + 300_000); // re-probe
        assert_eq!(
            q.items(T0 + 300_001)
                .iter()
                .filter(|i| i.stable_id == "disk-reclaim:/ws/target")
                .count(),
            1,
            "re-proposta dos mesmos caminhos não empilha"
        );
    }

    #[test]
    fn disk_reclaim_pending_survives_replay() {
        // Crash com proposta pendente → reabrir reconstrói o gate do log (event-sourced).
        let log = vec![cc_rec(disk_proposed("/ws/target", 30 * (1 << 30)), T0)];
        let q = AttentionQueue::replay(&log);
        assert!(
            q.items(T0 + 1)
                .iter()
                .any(|i| i.kind == AttentionKind::Custody
                    && i.stable_id == "disk-reclaim:/ws/target"),
            "a proposta de poda sobrevive ao replay"
        );
    }
}
