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

use crate::events::{AwaitReason, BlockReason, DomainEvent, EventRecord, EventStore, StoreError};
use crate::mailbox::{parse_target, render_message_block_full, MailMessage, Mailbox, TargetSpec};
use crate::plan::PlanError;
use crate::{A2aEnvelope, DeliveryOutcome, NodeId, Recipient, RolePolicy, Supervisor};

/// Janela de dedupe por `id` (design §3: dedupe em 60s).
pub const DEDUPE_WINDOW_MS: u64 = 60_000;
/// Profundidade máxima de cadeia (= `hops`/`--await`); corta o anti-loop.
pub const MAX_DEPTH: u8 = 4;
/// Teto de delegações por `root_cause_id` por agente (orçamento/turno).
pub const DELEGATION_BUDGET: u32 = 8;
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
    pub fanout_gate: usize,
    pub autonomy: AutonomyLevel,
    /// W3-7b: teto de tempo de um `await` sem reply (fecha por `Timeout` no sweep do `pump`).
    pub await_timeout_ms: u64,
    /// W3-7c (§2.2): teto de tokens por workspace/dia. `0` = DESLIGADO (sem teto). Ao a soma da
    /// janela contábil atingir este valor, o workspace entra em `Paused` (gate humano via `lina
    /// resume`, não kill — invariante #6).
    pub token_budget_day: u64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            dedupe_window_ms: DEDUPE_WINDOW_MS,
            max_depth: MAX_DEPTH,
            delegation_budget: DELEGATION_BUDGET,
            fanout_gate: FANOUT_GATE,
            autonomy: AutonomyLevel::Assisted,
            await_timeout_ms: AWAIT_TIMEOUT_MS,
            token_budget_day: 0, // desligado por padrão (opt-in por workspace)
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
#[derive(Clone, Copy)]
struct BudgetEntry {
    count: u32,
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
        // consulta o LOG: um id que JÁ tem `MessageRouted` foi roteado+entregue antes → confirma e
        // PULA. Uma varredura por tick, só quando há o que processar (custo da classe W2, diferido).
        let already_routed: HashSet<String> = if msgs.is_empty() {
            HashSet::new()
        } else {
            routed_ids(store)
        };
        msgs.into_iter()
            .map(|m| {
                if already_routed.contains(&m.id) {
                    // Órfão já roteado/entregue antes do crash → NÃO re-entrega; confirma e segue.
                    if let Err(e) = self.mailbox.ack_inflight(&m.id) {
                        eprintln!("lina-core: pump falhou ao confirmar inflight {}: {e}", m.id);
                    }
                    return (m.id, RouteOutcome::Duplicate);
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
                let retain_transient = matches!(
                    outcome,
                    RouteOutcome::UnknownSender(_) | RouteOutcome::NoTarget
                ) && {
                    let n = self.transient_retries.entry(m.id.clone()).or_insert(0);
                    *n += 1;
                    *n < MAX_TRANSIENT_RETRIES
                };
                if retain_transient {
                    // O `route_message` já marcou esta msg em `seen` (dedupe). Desmarca para que o
                    // PRÓXIMO tick a RE-PROCESSE (senão viraria `Duplicate` e seria ackeada+perdida). O
                    // dedupe DURÁVEL (`routed_ids` no log) segue protegendo contra re-ENTREGA — só a
                    // re-ROTA da msg ainda-não-entregue é reaberta. Seguro: a origem (`from`) continua
                    // autenticada pelo subdir; nada de roteamento muda, só a re-tentativa.
                    self.seen.remove(&m.id);
                }
                let hold = matches!(
                    outcome,
                    RouteOutcome::PersistFailed(_) | RouteOutcome::Queued
                );
                if !hold && !retain_transient {
                    // Desfecho definitivo (entregue, duplicado, hop-limit, ou transiente esgotado):
                    // confirma e limpa o contador (anti-vazamento do mapa).
                    self.transient_retries.remove(&m.id);
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
        if self.paused && msg.reply_to.is_none() && is_delegation(&msg.intent) {
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

        // ── W3-5: intents de plano NÃO são delegação A2A. O supervisor os APLICA ao `plan.md`
        //    (escritor único) e LOGA o evento, sem entregar a nenhuma PTY. Desviam aqui — depois do
        //    dedupe (reenvios são absorvidos) E da resolução de origem (Round 6: `by` = origem
        //    AUTENTICADA, nunca um `from` forjado), antes do pipeline de delegação/resolução de alvo.
        //    (Round 4: intent de plano nunca fecha await — o desvio precede o lifecycle do reply.)
        if is_plan_intent(&msg.intent) {
            return self.handle_plan(msg, store);
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
                        log_block(store, msg, BlockReason::LoopDetected);
                        return RouteOutcome::LoopDetected;
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
                    last_ms: now_ms,
                });
            entry.count += 1;
            entry.last_ms = now_ms;
        }

        // Renderiza o bloco [LINA::MSG] (com root_cause_id/hops achatados — auditoria/UX) e entrega.
        let to_label = recipient_label(&recipient, &msg.to);
        let block = render_message_block_full(
            &msg.id,
            &msg.from,
            &to_label,
            &msg.intent,
            &msg.payload,
            &root,
            hops,
        );
        let mut delivered = Vec::with_capacity(targets.len());
        for target in targets {
            match deliver(target, sender, &block) {
                Ok(_) => {
                    delivered.push(target);
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
                    if let Err(e) = store.append(&DomainEvent::MessageDelivered {
                        id: msg.id.clone(),
                        to: target,
                    }) {
                        // A entrega ao PTY já ocorreu (irreversível) mas não foi logada — o event
                        // log é a fonte da verdade, então a inconsistência é ALTA e VISÍVEL (stderr),
                        // nunca um warn silencioso de tracing.
                        eprintln!(
                            "lina-core: CRÍTICO — entregue mas NÃO logado (MessageDelivered {} → {target}): {e}",
                            msg.id
                        );
                    }
                }
                Err(e) => {
                    eprintln!("lina-core: entrega faseada falhou (alvo {target}): {e}");
                }
            }
        }

        if delivered.is_empty() {
            RouteOutcome::DeliveryFailed("nenhum alvo recebeu".into())
        } else {
            RouteOutcome::Delivered { targets: delivered }
        }
    }

    /// Remove entradas de dedupe (`seen`) E de orçamento (`budget`) mais velhas que a janela. As duas
    /// crescem com o tráfego; sem podar `budget` junto, ele vazaria memória numa sessão longa (A5),
    /// ao contrário de `seen` que já era podado.
    fn prune_expired(&mut self, now_ms: u64) {
        let window = self.config.dedupe_window_ms;
        self.seen
            .retain(|_, &mut t| now_ms.saturating_sub(t) < window);
        self.budget
            .retain(|_, e| now_ms.saturating_sub(e.last_ms) < window);
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
        RouteOutcome::PlanApplied {
            intent: msg.intent.clone(),
            item,
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

/// `true` se o intent é uma operação de plano (aplicada ao `plan.md`, nunca entregue a PTY).
fn is_plan_intent(intent: &str) -> bool {
    matches!(intent, "plan.claim" | "plan.check")
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
fn routed_ids(store: &EventStore) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Ok(recs) = store.events() {
        for rec in recs {
            if rec.kind == "MessageRouted" {
                if let Some(id) = rec.payload.get("id").and_then(|v| v.as_str()) {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    ids
}

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
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("window");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "@B", "ask", "oi");
        router.route_message(&msg, &mut ts.store, 0, &mut deliver);
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
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("budget");
        let (_rec, mut deliver) = recorder();

        // A→B estabelece o binding de B (root = id da msg de A — carimbado na entrega).
        let m0 = MailMessage::new("@A", "@B", "ask", "inicia");
        router.route_message(&m0, &mut ts.store, 1, &mut deliver);

        // B delega DELEGATION_BUDGET vezes, CADA uma com um root_cause_id FORJADO distinto → todas
        // ignoram o forjado e contam contra (root herdado, B); a (teto+1)-ésima estoura.
        for i in 0..DELEGATION_BUDGET {
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
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("budget-origin");
        let (_rec, mut deliver) = recorder();

        // @A nunca recebe A2A → cada ask é ORIGEM (hops==0, root fresco). Manda BUDGET+5 asks 1-a-1.
        for i in 0..(DELEGATION_BUDGET + 5) {
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
        let _b = sup.register("@B", None, sink());
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
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("loop");
        let (_rec, mut deliver) = recorder();

        // As voltas dentro do limite entregam — cada uma com um root FORJADO DISTINTO (todos IGNORADOS;
        // o root real vem do binding, então o grafo NÃO fragmenta e o ciclo é amarrado).
        let pingpong = [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")];
        for (i, (from, to)) in pingpong.iter().enumerate() {
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
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("tightloop");
        let (_rec, mut deliver) = recorder();

        // As voltas dentro do limite entregam (diálogo). Com MAX_CYCLE_REVISITS=2, o salto A→B pode
        // ocorrer 2× e B→A 2× antes do corte.
        let pingpong = [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")];
        for (i, (from, to)) in pingpong.iter().enumerate() {
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
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("budget-inherit");
        let (_rec, mut deliver) = recorder();

        // A→B estabelece o root do turno e o binding de B.
        let m0 = MailMessage::new("@A", "@B", "ask", "inicia");
        router.route_message(&m0, &mut ts.store, 1, &mut deliver);

        // B delega DELEGATION_BUDGET vezes (root herdado, root=None) → todas contam contra (root, B).
        for i in 0..DELEGATION_BUDGET {
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
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("replybypass");
        let (_rec, mut deliver) = recorder();

        // Ping-pong, cada salto com `reply_to` FORJADO (não casa ticket → NÃO pula o guard). As voltas
        // dentro do limite entregam (diálogo); o forjado não compra rodadas extras.
        let pingpong = [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")];
        for (i, (from, to)) in pingpong.iter().enumerate() {
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
        let _b = sup.register("@B", None, sink());
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
        let _b = sup.register("@B", None, sink());
        let _c = sup.register("@C", None, sink());
        let mut ts = TmpStore::new("migrate");
        let (_rec, mut deliver) = recorder();

        // A→B sob a cadeia R1: fixa o binding de B em R1.
        let m1 = MailMessage::new("@A", "@B", "ask", "cadeia R1");
        router.route_message(&m1, &mut ts.store, 1000, &mut deliver);
        let r1 = m1.id.clone();

        // Passada a janela (B ocioso, sem await): A→B sob a cadeia NOVA R2 → o binding de B MIGRA.
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
}
