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

use crate::events::{AwaitReason, BlockReason, DomainEvent, EventStore, StoreError};
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

/// W3-7b (ADR 0002): ticket em memória de um `await` aberto. Guarda só o necessário para FECHAR
/// (`waiter` → `end_await`; `opened_ms` → timeout); o contexto completo (`target`/`root_cause_id`)
/// já está durável no evento `AwaitOpened` (log = fonte da verdade), sem duplicar aqui.
#[derive(Clone, Copy)]
struct AwaitTicket {
    waiter: NodeId,
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
    /// W3-7 (A2): nó → `(root_cause_id, hops)` da última mensagem ENTREGUE a ele. Quando esse nó
    /// depois submete uma msg com `root=None`, o supervisor DERIVA o root (herdado) e `hops = +1`
    /// daqui — o enforcement de cadeia vem do binding do supervisor, não do campo que a CLI não
    /// preenche. Bounded pelo roster (1 entrada por nó, sobrescrita) → não precisa de poda.
    delivered_root: HashMap<NodeId, (String, u8)>,
    /// W3-7b (ADR 0002): `id` da pergunta → ticket do `await` aberto. Fechado pelo `reply_to` casado
    /// (`Replied`) ou pelo sweep de timeout (`Timeout`) — em ambos, `end_await` libera o wait-for-graph.
    pending: HashMap<String, AwaitTicket>,
}

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
        }
    }

    /// A mailbox observada.
    #[must_use]
    pub fn mailbox(&self) -> &Mailbox {
        &self.mailbox
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
                if !matches!(outcome, RouteOutcome::PersistFailed(_)) {
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
        // ── Guardrail: dedupe por `id` na janela (design §3). Poda `seen` E `budget` (A5).
        self.prune_expired(now_ms);
        if self.seen.contains_key(&msg.id) {
            // ADR 0003: `duplicate` NÃO é logado (anti-amplificação A4 — sob flood viraria um
            // amplificador de escrita). A duplicata é benigna por definição.
            return RouteOutcome::Duplicate;
        }
        // Marca como visto JÁ — uma 2ª cópia do mesmo id (re-drenada/reenviada) é duplicada.
        self.seen.insert(msg.id.clone(), now_ms);

        // ── W3-7b (ADR 0002) + fix reply_to-bypass: se ESTA msg é um REPLY que CASA um await
        //    pendente, FECHA o ticket (end_await + AwaitClosed{Replied}). `is_legit_reply` = casou um
        //    ticket REAL — capturado AQUI (antes do guard de loop, que vem depois e veria o `pending`
        //    já esvaziado). Só um reply legítimo pula o anti-loop (back-edge da pergunta); um
        //    `reply_to` FORJADO sem ticket → `false` → passa pelo anti-loop normal.
        let is_legit_reply = self.close_await_on_reply(msg, store);

        // ── W3-5: intents de plano NÃO são delegação A2A. O supervisor os APLICA ao `plan.md`
        //    (escritor único) e LOGA o evento, sem entregar a nenhuma PTY. Desviam aqui — depois do
        //    dedupe (reenvios são absorvidos), antes do pipeline de delegação/resolução de alvo.
        if is_plan_intent(&msg.intent) {
            return self.handle_plan(msg, store);
        }

        // ── Guardrail 0a: remetente existe no roster vivo. Resolvido ANTES do anti-loop por
        //    profundidade — precisamos do `NodeId` do sender para derivar root/hops do binding (A2).
        let Some(sender) = self.sup.node_by_name(&msg.from) else {
            log_block(store, msg, BlockReason::UnknownSender);
            return RouteOutcome::UnknownSender(msg.from.clone());
        };

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

        // ── Guardrail: gate de fan-out (broadcast > N alvos pede confirmação humana).
        if matches!(recipient, Recipient::Broadcast) && preview.len() > self.config.fanout_gate {
            log_block(store, msg, BlockReason::FanoutGated);
            return RouteOutcome::FanoutGated {
                count: preview.len(),
            };
        }

        let is_deleg = is_delegation(&msg.intent);

        // ── Guardrail 3: orçamento por `(root_cause_id, agente)`. Com a propagação do root (A2),
        //    delegações da mesma cadeia acumulam contra o mesmo root (antes cada msg nova era uma
        //    raiz fresca e a conta nunca batia). Só CHECA aqui; incrementa após o persist.
        if is_deleg {
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
        //    `root_cause_id`. Pega A→B→A formado por mensagens NOVAS sob o mesmo root — que `hops`/
        //    `trace` NÃO pegam (cada `MailMessage::new` zera ambos). Só ask/handoff a 1 alvo
        //    (broadcast é gateado à parte). Topologia de eventos: independe do texto.
        //    Um REPLY LEGÍTIMO (`is_legit_reply`: casou um ticket pendente) é a back-edge esperada
        //    da pergunta → pula o guard. `reply_to` forjado (sem ticket) NÃO pula (passa pelo guard).
        if is_deleg && !is_legit_reply && !matches!(recipient, Recipient::Broadcast) {
            if let Some(&target) = preview.first() {
                match handoff_would_loop(store, &root, sender, target) {
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

        // ── Guardrail 2 / ABERTURA do await (ADR 0002): para `--await`, `begin_await` é a CHECAGEM
        //    de deadlock E a abertura. ask/handoff (1 alvo): se Ok, a aresta FICA (liberada só no
        //    reply/timeout) — `await_target` lembra o alvo p/ abrir o ticket após o persist, ou
        //    rollback se a rota/persist falhar. broadcast: mantém a checagem PURA (rollback), sem
        //    lifecycle (multi-alvo não cabe no wait-for-graph 1-por-waiter; foge ao ADR, single-target).
        let await_target: Option<NodeId> = if msg.await_reply {
            if matches!(recipient, Recipient::Broadcast) {
                for &target in &preview {
                    match self.sup.begin_await(sender, target) {
                        Ok(()) => self.sup.end_await(sender),
                        Err(_) => {
                            log_block(store, msg, BlockReason::Deadlock);
                            return RouteOutcome::Deadlock;
                        }
                    }
                }
                None
            } else {
                match preview.first().copied() {
                    Some(target) => match self.sup.begin_await(sender, target) {
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
            if await_target.is_some() {
                self.sup.end_await(sender); // rollback da aresta aberta acima (rota falhou)
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
            if await_target.is_some() {
                self.sup.end_await(sender); // rollback: não abrimos await sem roteamento durável
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
                self.sup.end_await(sender); // rollback: não abrimos o que não conseguimos logar
                return RouteOutcome::PersistFailed(e.to_string());
            }
            self.pending.insert(
                msg.id.clone(),
                AwaitTicket {
                    waiter: sender,
                    opened_ms: now_ms,
                },
            );
        }
        // Roteamento logado → AGORA contabiliza o orçamento (delegação efetivada). Carimba o tempo
        // para a poda temporal (A5).
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
                    // P1 (anti-envenenamento): NÃO sobrescrever um binding VIVO com um root DIFERENTE
                    //     — senão uma msg de entrada sob root fresco faria a vítima "esquecer" a
                    //     cadeia ativa e escapar do orçamento/loop. Fixa quando ausente; dentro do
                    //     MESMO root, atualiza os hops. (Conservador: na dúvida, mantém o root vivo.)
                    //     Decisão computada ANTES do insert para não segurar o empréstimo do `get`.
                    let keep_existing = matches!(
                        self.delivered_root.get(&target),
                        Some((existing, _)) if *existing != root
                    );
                    if !keep_existing {
                        self.delivered_root.insert(target, (root.clone(), hops));
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
            Some((root, h)) => (root.clone(), h.saturating_add(1)),
            None => (msg.id.clone(), 0),
        }
    }

    /// **W3-7b (ADR 0002): fecha o `await` se `msg` é o REPLY casado.** Devolve `true` SE e só se o
    /// `msg.reply_to` casou um ticket REAL em `pending` (reply legítimo a um await aberto) — então
    /// `end_await(waiter)` (libera o wait-for-graph) + `AwaitClosed{Replied}` + remove de `pending`.
    /// `reply_to` ausente/FORJADO (sem ticket) → `false` (NÃO é reply legítimo → não pula o anti-loop).
    fn close_await_on_reply(&mut self, msg: &MailMessage, store: &mut EventStore) -> bool {
        let Some(reply_to) = msg.reply_to.as_deref() else {
            return false;
        };
        let Some(ticket) = self.pending.remove(reply_to) else {
            return false; // reply_to forjado / sem ticket → não autoriza pular o anti-loop
        };
        self.sup.end_await(ticket.waiter);
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
                self.sup.end_await(ticket.waiter);
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

/// **§2.1: o grafo de handoffs FECHARIA UM CICLO?** Reconstrói as arestas `from→to_node` dos eventos
/// `MessageRouted` do log com o MESMO `root_cause_id` e responde se `target` JÁ ALCANÇA `sender` —
/// nesse caso adicionar `sender→target` fecha um ciclo (A→B→A por mensagens novas sob o mesmo turno).
/// Identidades em `NodeId` (uniformes); independe do texto. Derivável do log (invariante #4).
///
/// # Errors
/// Falha ao ler o event log.
fn handoff_would_loop(
    store: &EventStore,
    root_cause_id: &str,
    sender: NodeId,
    target: NodeId,
) -> Result<bool, StoreError> {
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
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
            adj.entry(f).or_default().push(t);
        }
    }
    Ok(reaches(&adj, target, sender))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DomainEvent, EventStore};
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

    #[test]
    fn broadcast_over_fanout_gate_needs_confirmation() {
        // 4 colegas vivos (> FANOUT_GATE=3) → um broadcast é GATEADO (pede confirmação humana),
        // não entregue. (Guarda de segurança do design §4: broadcast grande não dispara sozinho.)
        let (mut router, sup, dir) = router_with("fanout");
        let _a = sup.register("@A", None, sink());
        for n in ["@B1", "@B2", "@B3", "@B4"] {
            sup.register(n, None, sink());
        }
        let mut ts = TmpStore::new("fanout");
        let (rec, mut deliver) = recorder();

        let msg = MailMessage::new("@A", "*", "broadcast", "atenção a todos");
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::FanoutGated { count: 4 }
        );
        assert!(rec.borrow().is_empty(), "broadcast gateado não entrega");
        assert!(blocked_reasons(&ts.store).contains(&"fanout_gated".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pump_drains_mailbox_and_routes() {
        let (mut router, sup, dir) = router_with("pump");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("pump");
        let (rec, deliver) = recorder();

        router
            .mailbox()
            .enqueue(&MailMessage::new("@A", "@B", "ask", "oi"))
            .expect("enqueue");
        let results = router.pump(&mut ts.store, 1000, deliver);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].1, RouteOutcome::Delivered { .. }));
        assert_eq!(rec.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────────────────────────── W3-5: plano compartilhado ─────────────────────────────

    /// Router + store novos com um plano semeado (workspace + T1/T2 + uma decisão), tudo via log.
    fn seeded_plan_router(tag: &str) -> (Router, std::path::PathBuf, TmpStore) {
        let (mut router, _sup, dir) = router_with(tag);
        let mut ts = TmpStore::new(tag);
        ts.store
            .append(&DomainEvent::WorkspaceCreated {
                name: "App X".into(),
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

    /// W1 + §2.1: A→B→A por mensagens NOVAS, CADA uma com `root_cause_id` FORJADO DISTINTO e hops=0
    /// (o vetor que ANTES burlava o anti-loop) → o forjado é IGNORADO, o root vem do BINDING, o grafo
    /// amarra a cadeia e o 2º salto (B→A) fecha o ciclo → `LoopDetected` + `RouteBlocked{loop_detected}`.
    #[test]
    fn forged_root_does_not_bypass_loop_detection() {
        let (mut router, sup, dir) = router_with("loop");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("loop");
        let (_rec, mut deliver) = recorder();

        // A→B (root forjado "fresh1", IGNORADO): origem → root real = id da msg; loga a aresta A→B.
        let mut m1 = MailMessage::new("@A", "@B", "ask", "ida");
        m1.root_cause_id = Some("fresh1".into());
        assert!(matches!(
            router.route_message(&m1, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        // B→A (root forjado DISTINTO "fresh2", IGNORADO): B herda o root de A (binding) → o grafo sob
        // esse root tem A→B; adicionar B→A fecharia o ciclo → LoopDetected (forja não escapou).
        let mut m2 = MailMessage::new("@B", "@A", "ask", "volta");
        m2.root_cause_id = Some("fresh2".into());
        assert_eq!(
            router.route_message(&m2, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::LoopDetected
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
        router.mailbox().enqueue(&m).expect("enqueue");

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

    // ───────────────────────── Hardening Round 3: regressões do roteador ─────────────────────────

    /// reply_to-bypass: um `reply_to` FORJADO (sem ticket em `pending`) NÃO pula o anti-loop. Um
    /// `reply_to` que CASA um ticket real (reply legítimo) pula (back-edge da pergunta).
    #[test]
    fn forged_reply_to_does_not_bypass_loop() {
        let (mut router, sup, dir) = router_with("replybypass");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("replybypass");
        let (_rec, mut deliver) = recorder();

        // A→B (origem) loga a aresta A→B sob o root do binding.
        let m1 = MailMessage::new("@A", "@B", "ask", "ida");
        assert!(matches!(
            router.route_message(&m1, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));

        // B→A com reply_to FORJADO (não casa ticket) → NÃO pula o anti-loop → fecha o ciclo → bloqueado.
        let forged = MailMessage::new("@B", "@A", "ask", "volta").replying_to("msg_naoexiste");
        assert_eq!(
            router.route_message(&forged, &mut ts.store, 1001, &mut deliver),
            RouteOutcome::LoopDetected,
            "reply_to forjado (sem ticket) não pode escapar do anti-loop"
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
}
