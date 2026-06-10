//! Gate F1-1-8 — injeção y/n validada contra a tela (ADR 0021, ACEITO).
//!
//! Estes testes exercitam o NÚCLEO de segurança — o executor de injeção
//! (`ApprovalExecutor`) e o driver de auto-deny (`AttentionQueue::auto_deny_due`) — pela
//! API pública do `lina-core`, com portas que aplicam a MESMA regra da porta de produção
//! (`check_screen`, fonte única) sobre grids REAIS (`AlacrittyBackend`) e, no fim, sobre
//! um `PtyHost` REAL.
//!
//! Cada teste prova um critério de verificação do ADR (red-team embutido):
//! - **AC-0021.3** (spoofing): nó que imprime fake "(y/n)" + payload malicioso NÃO
//!   atribui a aprovação a outro nó; o binding nasce do canal (log), nunca do texto.
//! - **AC-0021.4** (alvo / fila reordenável): o gesto referencia `stable_id`; a fila
//!   reordena entre exibição e gesto, mas o write vai ao `node_id` do binding do LOG;
//!   `target_mismatch` aborta com ZERO bytes.
//! - **AC-0021.5** (SLA): pendência sintética → `Escalated` aos 5 min + auto-deny aos
//!   10 min pelo MESMO pipeline validado (deny passa por check de tela); NENHUM caminho
//!   produz approve por timeout.
//! - **AC-0021.6** (fronteira F1-1-7): nenhum caminho da fila escreve no PTY sem passar
//!   pelo executor (deny incluso) — a fila não tem porta; só o executor a aciona.
//! - **AC-0021.7** (mecanismo, headless): executor → `PtyHost` destrava um processo real
//!   bloqueado num prompt y/n; o `read` recebe EXATAMENTE o input esperado (prova
//!   positiva do zero-byte: um abort que vazasse bytes faria o `read` consumir outra coisa).

use lina_core::attention::AUTO_DENY_AFTER_MS;
use lina_core::{
    check_screen, prompt_snapshot_hash, AbortReason, AlacrittyBackend, ApprovalDecision,
    ApprovalExecutor, ApprovalGesture, ApprovalKeys, ApprovalOutcomeKind, ApprovalPort,
    AttentionEvidence, AttentionQueue, AttentionState, DomainEvent, PermissionEvidence, PortError,
    PortOutcome, PromptKind, ResolutionVia, ScreenCheck, VtBackend, ESCALATE_AFTER_MS,
    PROMPT_REGION_ROWS,
};
// F1-6-8: exclusivos do AC-0021.7 (PTY real, gateado #[cfg(unix)] abaixo); no Windows viram unused.
#[cfg(unix)]
use lina_core::{PtyCommand, PtyHost};
use std::collections::HashMap;
// F1-6-8: Duration/Instant são usados só pelo AC-0021.7 (gateado) — cfg(unix) evita unused no Windows.
#[cfg(unix)]
use std::time::{Duration, Instant};

const T0: u64 = 1_750_000_000_000;
const K: usize = PROMPT_REGION_ROWS;

// ───────────────────────── helpers de evento ─────────────────────────

fn asked(node: &str, stable_id: &str, hash: Option<&str>) -> DomainEvent {
    asked_detail(node, stable_id, hash, None)
}

fn asked_detail(
    node: &str,
    stable_id: &str,
    hash: Option<&str>,
    detail: Option<&str>,
) -> DomainEvent {
    DomainEvent::PermissionAsked {
        node_id: node.into(),
        tool: None,
        detail: detail.map(str::to_string),
        evidence: PermissionEvidence::Grid,
        stable_id: stable_id.into(),
        vt_snapshot_hash: hash.map(str::to_string),
        prompt_kind: PromptKind::Yn,
    }
}

fn gesture<'a>(
    stable_id: &'a str,
    target: &'a str,
    decision: ApprovalDecision,
    via: ResolutionVia,
    expected_hash: &'a str,
    keys: &'a ApprovalKeys,
) -> ApprovalGesture<'a> {
    ApprovalGesture {
        stable_id,
        target_node: target,
        decision,
        via,
        expected_hash,
        keys,
    }
}

// ───────── porta de teste multi-nó: grid real por nó + log de writes do PTY ─────────

/// Espelha a porta de produção (`PtyHost::deliver_approval`) para os ACs: aplica
/// `check_screen` (fonte única) sobre o grid REAL de CADA nó e registra cada write — o
/// "log de writes do PTY" que prova zero/um byte por nó.
struct GridPort {
    nodes: HashMap<String, NodeGrid>,
}

struct NodeGrid {
    vt: AlacrittyBackend,
    writes: Vec<Vec<u8>>,
}

impl GridPort {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Cria o grid do nó com `bytes` já avançados; devolve a Captura 1 (hash atual) —
    /// é o `expected_hash` que viaja no log até o gesto (tela inalterada ⇒ casa).
    fn add(&mut self, node: &str, bytes: &[u8]) -> String {
        let mut vt = AlacrittyBackend::new(80, 24);
        vt.advance(bytes);
        let h = prompt_snapshot_hash(&vt, K);
        self.nodes.insert(
            node.to_string(),
            NodeGrid {
                vt,
                writes: Vec::new(),
            },
        );
        h
    }

    fn advance(&mut self, node: &str, bytes: &[u8]) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.vt.advance(bytes);
        }
    }

    fn writes(&self, node: &str) -> &[Vec<u8>] {
        self.nodes.get(node).map_or(&[], |n| n.writes.as_slice())
    }

    fn total_writes(&self) -> usize {
        self.nodes.values().map(|n| n.writes.len()).sum()
    }
}

impl ApprovalPort for GridPort {
    fn deliver(
        &mut self,
        node_id: &str,
        expected_hash: &str,
        keys: &[u8],
    ) -> Result<PortOutcome, PortError> {
        let n = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| PortError::NotFound(node_id.to_string()))?;
        match check_screen(&n.vt, expected_hash, K) {
            ScreenCheck::Changed { current_hash } => {
                Ok(PortOutcome::ScreenChanged { current_hash })
            }
            ScreenCheck::Match { vt_snapshot_hash } => {
                n.writes.push(keys.to_vec());
                Ok(PortOutcome::Written { vt_snapshot_hash })
            }
        }
    }
}

/// Tripwire: NÃO escreve em PTY nenhum — só CONTA quando o executor a aciona. Prova
/// (AC-0021.6) que os caminhos da fila não têm porta: a contagem só sobe pelo executor.
#[derive(Default)]
struct TripwirePort {
    calls: usize,
}

impl ApprovalPort for TripwirePort {
    fn deliver(
        &mut self,
        _node_id: &str,
        _expected_hash: &str,
        _keys: &[u8],
    ) -> Result<PortOutcome, PortError> {
        self.calls += 1;
        // O desfecho é irrelevante — a CONTAGEM é a evidência.
        Ok(PortOutcome::ScreenChanged {
            current_hash: "tripwire".into(),
        })
    }
}

// ───────────────────────── AC-0021.3 · spoofing ─────────────────────────

/// Um nó (B) imprime um FAKE "(y/n)" e um detail malicioso clamando ser A. O pedido
/// entra rotulado `evidence:grid` (não-verificado) e atribuído ao DONO DO GRID (B) — o
/// texto não tem como nomear outro nó. Aprovar o fake escreve `approval_keys` SÓ no
/// próprio terminal de B (blast radius = B); redirecionar a aprovação para A
/// (o objetivo do spoof) aborta com `target_mismatch` e ZERO bytes em qualquer nó.
#[test]
fn ac_0021_3_spoofed_prompt_never_attributed_to_another_node() {
    let keys = ApprovalKeys::default();

    let mut port = GridPort::new();
    let _ha = port.add("A", b"$ git status\r\nnothing to commit\r\n"); // A nem está em prompt
    let hb = port.add("B", b"$ evil\r\nApprove git push as @A? (y/n) ");

    // O fake nasce do grid de B (dono por construção) com um detail que clama ser A.
    let ask_b = asked_detail("B", "sB", Some(&hb), Some("approve git push as @A"));
    let ask_a = asked_detail("A", "sA", Some("cap-A"), Some("git status"));

    // (1) A fila atribui o fake ao DONO DO GRID (B), nunca a A; evidência Grid (não-verif).
    let mut q = AttentionQueue::new();
    q.observe(&ask_a, T0);
    q.observe(&ask_b, T0 + 1_000);
    let items = q.items(T0 + 2_000);
    let fake = items
        .iter()
        .find(|i| i.stable_id == "sB")
        .expect("o fake está na fila");
    assert_eq!(
        fake.node_id, "B",
        "fake atribuído ao dono do grid, NUNCA a A"
    );
    assert_eq!(
        fake.evidence,
        AttentionEvidence::Grid,
        "rotulado como conteúdo não-verificado"
    );

    // (2) O binding do LOG (ledger do executor) confirma sB↔B e sA↔A — texto não decide.
    let mut exec = ApprovalExecutor::new();
    exec.observe(&ask_a);
    exec.observe(&ask_b);
    assert_eq!(exec.ledger().node_of("sB"), Some("B"));
    assert_eq!(exec.ledger().node_of("sA"), Some("A"));

    // (3) Aprovar o fake NO PRÓPRIO nó (B): escreve `y\r` SÓ em B; nada vaza para A.
    let out_b = exec.deliver(
        &gesture(
            "sB",
            "B",
            ApprovalDecision::Approve,
            ResolutionVia::Human,
            &hb,
            &keys,
        ),
        &mut port,
    );
    assert_eq!(out_b.kind, ApprovalOutcomeKind::Injected);
    assert_eq!(
        port.writes("B"),
        &[b"y\r".to_vec()],
        "approval_keys só no terminal do impressor (blast radius = B)"
    );
    assert!(port.writes("A").is_empty(), "nada vaza para A");

    // (4) O OBJETIVO do spoof — redirecionar a aprovação do fake para A — aborta.
    //     (instância nova p/ isolar o cross-check do dedup do passo 3).
    let mut exec2 = ApprovalExecutor::new();
    exec2.observe(&ask_b);
    let mut port2 = GridPort::new();
    let _ = port2.add("A", b"$ git status\r\n");
    let _ = port2.add("B", b"... (y/n) ");
    let redirect = exec2.deliver(
        &gesture(
            "sB",
            "A", // ataque: tentar fazer a aprovação do fake cair em A
            ApprovalDecision::Approve,
            ResolutionVia::Human,
            &hb,
            &keys,
        ),
        &mut port2,
    );
    assert_eq!(
        redirect.kind,
        ApprovalOutcomeKind::Aborted(AbortReason::TargetMismatch)
    );
    assert_eq!(
        port2.total_writes(),
        0,
        "redirecionar o fake p/ A: ZERO bytes em QUALQUER nó"
    );
}

// ───────────────────────── AC-0021.4 · alvo / fila reordenável ─────────────────────────

/// 2+ pendências de nós distintos; a fila REORDENA (round-robin sob flood) entre a
/// exibição e o gesto. O gesto referencia `stable_id` (não posição): o write vai ao
/// `node_id` do binding do LOG, imune à reordenação. Usar a POSIÇÃO (mandar o id de B
/// para o nó que lidera a fila, A) aborta com `target_mismatch` e ZERO bytes.
#[test]
fn ac_0021_4_queue_reorder_does_not_move_the_target() {
    let keys = ApprovalKeys::default();

    let mut port = GridPort::new();
    let _ha = port.add("A", b"A asks: (y/n) ");
    let hb = port.add("B", b"B asks: (y/n) ");

    let a1 = asked("A", "a1", Some("hA1"));
    let b1 = asked("B", "b1", Some(&hb));

    // Exibição (T1): round-robin → [a1, b1].
    let mut q = AttentionQueue::new();
    q.observe(&a1, T0);
    q.observe(&b1, T0 + 1_000);
    let order_t1: Vec<String> = q
        .items(T0 + 2_000)
        .into_iter()
        .map(|i| i.stable_id)
        .collect();
    assert_eq!(order_t1, vec!["a1", "b1"]);

    // A fila REORDENA entre exibição e gesto: A floda mais 2 (fora da janela de merge).
    q.observe(&asked("A", "a2", Some("hA2")), T0 + 20_000);
    q.observe(&asked("A", "a3", Some("hA3")), T0 + 40_000);
    let order_t2: Vec<String> = q
        .items(T0 + 41_000)
        .into_iter()
        .map(|i| i.stable_id)
        .collect();
    assert_eq!(
        order_t2,
        vec!["a1", "b1", "a2", "a3"],
        "round-robin reordenou a fila"
    );
    assert_ne!(order_t1, order_t2, "a fila MUDOU entre exibição e gesto");

    // O humano aprova b1 (REFERENCIA stable_id). Binding e Captura 1 vêm do LOG.
    let mut exec = ApprovalExecutor::new();
    for ev in [&a1, &b1] {
        exec.observe(ev);
    }
    exec.observe(&asked("A", "a2", Some("hA2")));
    exec.observe(&asked("A", "a3", Some("hA3")));
    let target = exec
        .ledger()
        .node_of("b1")
        .expect("binding do log")
        .to_string();
    assert_eq!(target, "B", "stable_id→nó vem do LOG, imune à reordenação");
    // A Captura 1 do item (na fila/projeção) == o hash do grid de B (round-trip do log).
    let item_hash = q
        .items(T0 + 41_000)
        .into_iter()
        .find(|i| i.stable_id == "b1")
        .and_then(|i| i.vt_snapshot_hash)
        .expect("Captura 1 do item");
    assert_eq!(
        item_hash, hb,
        "expected_hash vem do LOG (Captura 1), não da UI"
    );

    let out = exec.deliver(
        &gesture(
            "b1",
            &target,
            ApprovalDecision::Approve,
            ResolutionVia::Human,
            &item_hash,
            &keys,
        ),
        &mut port,
    );
    assert_eq!(out.kind, ApprovalOutcomeKind::Injected);
    assert_eq!(
        port.writes("B"),
        &[b"y\r".to_vec()],
        "write foi p/ B (dono do stable_id)"
    );
    assert!(
        port.writes("A").is_empty(),
        "nada p/ A apesar de A LIDERAR a fila"
    );

    // POSITIVO (mismatch): usar a POSIÇÃO (a1 lidera) e mandar b1→A aborta, zero bytes.
    let mut exec2 = ApprovalExecutor::new();
    exec2.observe(&b1);
    let mut port2 = GridPort::new();
    let _ = port2.add("A", b"A asks: (y/n) ");
    let _ = port2.add("B", b"B asks: (y/n) ");
    let mismatch = exec2.deliver(
        &gesture(
            "b1",
            "A",
            ApprovalDecision::Approve,
            ResolutionVia::Human,
            &hb,
            &keys,
        ),
        &mut port2,
    );
    assert_eq!(
        mismatch.kind,
        ApprovalOutcomeKind::Aborted(AbortReason::TargetMismatch)
    );
    assert_eq!(port2.total_writes(), 0);
}

// ───────────────────────── AC-0021.5 · SLA (escalate 5 min, auto-deny 10 min) ─────────────────────────

/// Pendência sintética: `Escalated` (visual) aos 5 min; auto-deny aos 10 min drenado
/// pelo MESMO pipeline validado do executor (deny passa por check de tela) →
/// `PermissionResolved{decision:deny, via:timeout}` + write das DENY keys; nenhum
/// caminho produz approve. Simetria §3: se a tela divergiu, aborta sem escrever.
#[test]
fn ac_0021_5_escalate_at_five_min_then_auto_deny_at_ten_never_approve() {
    let keys = ApprovalKeys::default();

    let mut port = GridPort::new();
    let h = port.add("A", b"$ deploy\r\nDeploy to prod? (y/n) ");

    let mut q = AttentionQueue::new();
    q.observe(&asked("A", "s1", Some(&h)), T0);

    // 5 min: Escalated (visual), mas SEM auto-deny.
    assert_eq!(
        q.items(T0 + ESCALATE_AFTER_MS)[0].state,
        AttentionState::Escalated,
        "escalação visual aos 5 min"
    );
    assert!(
        q.auto_deny_due(T0 + ESCALATE_AFTER_MS).is_empty(),
        "5 min NÃO nega (só escala)"
    );

    // 10 min: o driver identifica a pendência vencida (com binding + Captura 1).
    let due = q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS);
    assert_eq!(due.len(), 1);
    let d = &due[0];
    assert_eq!(d.stable_id, "s1");
    assert_eq!(d.node_id, "A");

    // Drena pelo MESMO pipeline validado do executor: decision=Deny, via=Timeout.
    let mut exec = ApprovalExecutor::new();
    exec.observe(&asked("A", "s1", Some(&h)));
    let target = exec
        .ledger()
        .node_of(&d.stable_id)
        .expect("binding")
        .to_string();
    let out = exec.deliver(
        &gesture(
            &d.stable_id,
            &target,
            ApprovalDecision::Deny, // FIXO Deny — o candidato não carrega decisão
            ResolutionVia::Timeout,
            d.vt_snapshot_hash.as_deref().unwrap_or(""),
            &keys,
        ),
        &mut port,
    );
    assert_eq!(out.kind, ApprovalOutcomeKind::Injected);
    assert_eq!(
        port.writes("A"),
        &[b"n\r".to_vec()],
        "auto-deny digita as DENY keys (não approve)"
    );
    assert_eq!(
        out.events[0],
        DomainEvent::PermissionResolved {
            stable_id: "s1".into(),
            decision: ApprovalDecision::Deny,
            via: ResolutionVia::Timeout,
        },
        "auditado como deny/timeout"
    );
    // NENHUM caminho produz approve por timeout (busca negativa em todo o desfecho).
    assert!(
        !out.events.iter().any(|e| matches!(
            e,
            DomainEvent::PermissionResolved {
                decision: ApprovalDecision::Approve,
                ..
            }
        )),
        "ZERO approve por timeout"
    );

    // Simetria §3 (tela divergiu entre captura e auto-deny): aborta, ZERO bytes.
    let mut port2 = GridPort::new();
    let stale = port2.add("A", b"Deploy to prod? (y/n) ");
    port2.advance("A", b"\r\nWARNING: lock changed\r\nDeploy to prod? (y/n) ");
    let mut exec2 = ApprovalExecutor::new();
    exec2.observe(&asked("A", "s2", Some(&stale)));
    let out2 = exec2.deliver(
        &gesture(
            "s2",
            "A",
            ApprovalDecision::Deny,
            ResolutionVia::Timeout,
            &stale,
            &keys,
        ),
        &mut port2,
    );
    assert_eq!(
        out2.kind,
        ApprovalOutcomeKind::Aborted(AbortReason::ScreenChanged)
    );
    assert_eq!(
        port2.total_writes(),
        0,
        "tela divergiu: deny-não-entregue, zero bytes"
    );
}

// ───────────────────────── AC-0021.6 · fronteira F1-1-7 ─────────────────────────

/// Nenhum caminho da FILA escreve no PTY sem passar pelo executor (deny incluso). A
/// `AttentionQueue` (resolve/dismiss/auto_deny_due) devolve DADO e NÃO recebe porta —
/// não há como escrever. O executor é a única porta: a tripwire só conta quando ele a
/// aciona (controle positivo).
#[test]
fn ac_0021_6_only_the_executor_touches_the_pty_writer() {
    let keys = ApprovalKeys::default();
    let mut tripwire = TripwirePort::default();

    let mut q = AttentionQueue::new();
    q.observe(&asked("A", "s1", Some("h1")), T0);
    q.observe(&asked("B", "s2", Some("h2")), T0);
    q.custody_enqueued("c1", "C", "lina do deploy", T0);

    // Caminhos da FILA: validam/identificam, devolvem DADO — sem acesso a porta.
    let _resolved = q.resolve("s1", ApprovalDecision::Approve, ResolutionVia::Human);
    let _dismissed = q.dismiss("s2");
    let _due = q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS);
    let _custody_noop = q.resolve("c1", ApprovalDecision::Approve, ResolutionVia::Human); // None (custódia)
    assert_eq!(
        tripwire.calls, 0,
        "NENHUM caminho da fila escreve no PTY (a fila não tem porta)"
    );

    // CONTROLE POSITIVO: só o executor aciona a porta (deny por timeout incluso).
    let mut exec = ApprovalExecutor::new();
    exec.observe(&asked("A", "s9", Some("h9")));
    let _ = exec.deliver(
        &gesture(
            "s9",
            "A",
            ApprovalDecision::Deny,
            ResolutionVia::Timeout,
            "h9",
            &keys,
        ),
        &mut tripwire,
    );
    assert_eq!(
        tripwire.calls, 1,
        "o executor é a ÚNICA porta de escrita (deny incluso)"
    );
}

// ───────────────── AC-0021.7 (mecanismo, headless) · executor → PtyHost ─────────────────

#[cfg(unix)] // F1-6-8: helper exclusivo do AC-0021.7 (gateado) — dead_code no Windows.
fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// O caminho de produção do efeito (`ApprovalExecutor::deliver` → `PtyHost` como
/// `ApprovalPort`) destrava um processo REAL bloqueado num prompt y/n: captura o hash,
/// entrega APROVAR pelo executor, e o `read` recebe EXATAMENTE "y" — se um abort tivesse
/// vazado bytes, o `read` teria consumido outra coisa e este passo falharia (prova
/// positiva do zero-byte). É o mecanismo do AC-0021.7 (a validação na tela pelo fundador
/// é o gate manual da onda).
#[cfg(unix)]
// F1-6-8: spawna `sh -c` num PTY real — runtime-Unix; pendente-windows na tabela ci-3so-triagem.md.
#[test]
fn ac_0021_7_executor_through_pty_host_unblocks_real_prompt() {
    const T: Duration = Duration::from_secs(5);

    let mut host = PtyHost::new();
    let node = host
        .spawn(
            PtyCommand::new("sh")
                .arg("-c")
                .arg("printf 'Continue? (y/n) '; read x; printf 'GOT:%s\\n' \"$x\""),
            80,
            24,
        )
        .expect("spawn do prompt sintético");

    assert!(
        poll_until(T, || host
            .with_grid(node, |vt| vt.last_nonempty_line())
            .as_deref()
            == Some("Continue? (y/n)")),
        "o prompt deveria aparecer no grid"
    );

    // Captura 1 (detecção) — o que o detector da F1-1-6 anexaria ao PermissionAsked.
    let hash = host
        .with_grid(node, |vt| prompt_snapshot_hash(vt, PROMPT_REGION_ROWS))
        .expect("hash da detecção");

    let keys = ApprovalKeys::default();
    let target = node.to_string();
    let mut exec = ApprovalExecutor::new();
    exec.observe(&asked(&target, "s1", Some(&hash)));

    // O gesto humano (APROVAR) drenado pelo executor com o PtyHost como porta única.
    let out = exec.deliver(
        &gesture(
            "s1",
            &target,
            ApprovalDecision::Approve,
            ResolutionVia::Human,
            &hash,
            &keys,
        ),
        &mut host,
    );
    assert_eq!(
        out.kind,
        ApprovalOutcomeKind::Injected,
        "executor → PtyHost deve entregar: {:?}",
        out.kind
    );

    assert!(
        poll_until(T, || host
            .with_grid(node, |vt| vt.last_nonempty_line())
            .as_deref()
            == Some("GOT:y")),
        "o processo deveria destravar com EXATAMENTE o input 'y'"
    );

    host.kill(node).ok();
}
