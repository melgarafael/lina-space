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

use std::collections::HashMap;
use std::sync::Arc;

use crate::events::{DomainEvent, EventStore};
use crate::mailbox::{parse_target, render_message_block, MailMessage, Mailbox, TargetSpec};
use crate::{A2aEnvelope, DeliveryOutcome, NodeId, Recipient, RolePolicy, Supervisor};

/// Janela de dedupe por `id` (design §3: dedupe em 60s).
pub const DEDUPE_WINDOW_MS: u64 = 60_000;
/// Profundidade máxima de cadeia (= `hops`/`--await`); corta o anti-loop.
pub const MAX_DEPTH: u8 = 4;
/// Teto de delegações por `root_cause_id` por agente (orçamento/turno).
pub const DELEGATION_BUDGET: u32 = 8;
/// Acima de N alvos, um `broadcast` exige confirmação humana (gate de fan-out).
pub const FANOUT_GATE: usize = 3;

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
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            dedupe_window_ms: DEDUPE_WINDOW_MS,
            max_depth: MAX_DEPTH,
            delegation_budget: DELEGATION_BUDGET,
            fanout_gate: FANOUT_GATE,
            autonomy: AutonomyLevel::Assisted,
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
    /// A entrega faseada falhou (PTY do alvo).
    DeliveryFailed(String),
    /// Falha ao persistir o roteamento no event log (não entregamos o que não logamos).
    PersistFailed(String),
}

/// `true` se o `intent` é uma delegação (sujeita ao gate de autonomia). `handshake`/`status` não
/// delegam trabalho.
fn is_delegation(intent: &str) -> bool {
    matches!(intent, "ask" | "handoff" | "broadcast" | "review" | "")
}

/// **Roteador do supervisor.** Detém o `Arc<Supervisor>` (roster + filas serial + pub/sub) e o
/// estado de guardrail (dedupe/orçamento). Single-thread por design (escritor único de `.lina/`).
pub struct Router {
    sup: Arc<Supervisor>,
    mailbox: Mailbox,
    config: RouterConfig,
    /// `id` → carimbo (ms) da primeira vez que vimos — dedupe por janela.
    seen: HashMap<String, u64>,
    /// `(root_cause_id, remetente)` → nº de delegações — orçamento por turno.
    budget: HashMap<(String, NodeId), u32>,
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
        let msgs = self.mailbox.drain().unwrap_or_else(|e| {
            // Falha de I/O ao listar o outbox (disco/permissão): as mensagens NÃO são perdidas
            // (ficam no outbox para o próximo tick), mas a falha é VISÍVEL (stderr), nunca silenciosa.
            eprintln!("lina-core: pump não conseguiu drenar a mailbox (será retentado): {e}");
            Vec::new()
        });
        msgs.into_iter()
            .map(|m| {
                let outcome = self.route_message(&m, store, now_ms, &mut deliver);
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
        // ── Guardrail: dedupe por `id` na janela (design §3). Limpa entradas velhas (O(n) raro).
        self.prune_seen(now_ms);
        if self.seen.contains_key(&msg.id) {
            return RouteOutcome::Duplicate;
        }
        // Marca como visto JÁ — uma 2ª cópia do mesmo id (re-drenada/reenviada) é duplicada.
        self.seen.insert(msg.id.clone(), now_ms);

        // ── Guardrail: anti-loop por profundidade (`hops`).
        if msg.hops > self.config.max_depth {
            return RouteOutcome::HopLimit;
        }

        // ── Guardrail 0a: remetente existe no roster vivo.
        let Some(sender) = self.sup.node_by_name(&msg.from) else {
            return RouteOutcome::UnknownSender(msg.from.clone());
        };

        // ── Guardrail 0b: alvo existe (resolve SEM efeito colateral — não publica ainda).
        let (recipient, policy) = match parse_target(&msg.to) {
            TargetSpec::Broadcast => (Recipient::Broadcast, RolePolicy::All),
            TargetSpec::Role(r) => (Recipient::Role(r), RolePolicy::FirstIdle),
            TargetSpec::Name(n) => match self.sup.node_by_name(&n) {
                Some(id) => (Recipient::Node(id), RolePolicy::FirstIdle),
                None => return RouteOutcome::NoTarget,
            },
        };
        let preview = self.sup.resolve(&recipient, policy, Some(sender));
        if preview.is_empty() {
            return RouteOutcome::NoTarget;
        }

        // ── Guardrail 1: autonomia (manual recusa delegação).
        if self.config.autonomy.blocks_delegation() && is_delegation(&msg.intent) {
            return RouteOutcome::BlockedByAutonomy;
        }

        // ── Guardrail: gate de fan-out (broadcast > N alvos pede confirmação humana).
        if matches!(recipient, Recipient::Broadcast) && preview.len() > self.config.fanout_gate {
            return RouteOutcome::FanoutGated {
                count: preview.len(),
            };
        }

        // ── Guardrail 3: orçamento de delegação por `root_cause_id`/agente. Sem root explícito,
        //    a própria mensagem é a raiz (o turno do usuário que disparou a CLI). Aqui só CHECAMOS;
        //    o incremento vem DEPOIS do persist bem-sucedido — uma falha de log não envenena a
        //    conta (a conta reflete delegação EFETIVADA, não tentada).
        let root = msg.root_cause_id.clone().unwrap_or_else(|| msg.id.clone());
        let is_deleg = is_delegation(&msg.intent);
        if is_deleg {
            let count = self
                .budget
                .get(&(root.clone(), sender))
                .copied()
                .unwrap_or(0);
            if count >= self.config.delegation_budget {
                return RouteOutcome::BudgetExceeded;
            }
        }

        // ── Guardrail 2: anti-deadlock (só `--await`): se registrar `sender→target` fecharia um
        //    ciclo no wait-for-graph, recusa. É uma CHECAGEM PURA — desfazemos a aresta na hora
        //    (o modelo de mailbox é fire-and-forget; a CLI não bloqueia de fato, então uma aresta
        //    persistente seria órfã e geraria falsos deadlocks depois).
        if msg.await_reply {
            for &target in &preview {
                match self.sup.begin_await(sender, target) {
                    Ok(()) => self.sup.end_await(sender), // rollback imediato (só checagem)
                    Err(_) => return RouteOutcome::Deadlock,
                }
            }
        }

        // ── Passou. Carimba o envelope canônico (root_cause_id, hops, await) e ROTEIA (publica
        //    `BusEvent::Message` + devolve os alvos frescos, sem o remetente nem nós já no trace).
        let mut env = A2aEnvelope::new(sender, recipient.clone(), Some(intent_or_ask(&msg.intent)));
        env.root_cause_id = Some(root.clone());
        env.hops = msg.hops;
        env.await_reply = msg.await_reply;
        let targets = self.sup.route(&env, policy);
        if targets.is_empty() {
            return RouteOutcome::NoTarget;
        }

        // Persiste o ROTEAMENTO antes de entregar (não entregamos o que não conseguimos logar).
        if let Err(e) = store.append(&DomainEvent::MessageRouted {
            id: msg.id.clone(),
            from: sender,
            to: msg.to.clone(),
            intent: intent_or_ask(&msg.intent),
        }) {
            return RouteOutcome::PersistFailed(e.to_string());
        }
        // Roteamento logado → AGORA contabiliza o orçamento (delegação efetivada).
        if is_deleg {
            *self.budget.entry((root, sender)).or_insert(0) += 1;
        }

        // Renderiza o bloco [LINA::MSG] (sentinela + terminador LINA) e entrega a cada alvo.
        let to_label = recipient_label(&recipient, &msg.to);
        let block = render_message_block(&msg.id, &msg.from, &to_label, &msg.intent, &msg.payload);
        let mut delivered = Vec::with_capacity(targets.len());
        for target in targets {
            match deliver(target, sender, &block) {
                Ok(_) => {
                    delivered.push(target);
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

    /// Remove entradas de dedupe mais velhas que a janela (mantém o mapa enxuto).
    fn prune_seen(&mut self, now_ms: u64) {
        let window = self.config.dedupe_window_ms;
        self.seen
            .retain(|_, &mut t| now_ms.saturating_sub(t) < window);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventStore;
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

    #[test]
    fn delegation_budget_is_enforced_per_root_cause() {
        let (mut router, sup, dir) = router_with("budget");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("budget");
        let (rec, mut deliver) = recorder();

        // 8 delegações com o MESMO root_cause_id passam; a 9ª estoura.
        for i in 0..DELEGATION_BUDGET {
            let mut msg = MailMessage::new("@A", "@B", "ask", format!("t{i}"));
            msg.root_cause_id = Some("turn_1".into());
            assert!(
                matches!(
                    router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "delegação {i} deveria passar"
            );
        }
        let mut over = MailMessage::new("@A", "@B", "ask", "demais");
        over.root_cause_id = Some("turn_1".into());
        assert_eq!(
            router.route_message(&over, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::BudgetExceeded
        );
        assert_eq!(rec.borrow().len(), DELEGATION_BUDGET as usize);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hops_over_max_depth_is_anti_loop_dropped() {
        let (mut router, sup, dir) = router_with("hops");
        let _a = sup.register("@A", None, sink());
        let _b = sup.register("@B", None, sink());
        let mut ts = TmpStore::new("hops");
        let (rec, mut deliver) = recorder();

        let mut msg = MailMessage::new("@A", "@B", "ask", "oi");
        msg.hops = MAX_DEPTH + 1;
        assert_eq!(
            router.route_message(&msg, &mut ts.store, 1000, &mut deliver),
            RouteOutcome::HopLimit
        );
        assert!(rec.borrow().is_empty());
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
}
