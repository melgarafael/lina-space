//! GATE DE SAÍDA — Onda 3 (auto-orquestração + guardrails determinísticos), HEADLESS sobre o
//! event store. É o teste FINAL que fecha a onda: prova, por replay do `log.jsonl`, os 7 cenários
//! (a-g) do design (`tasks/_gate-onda3.md`).
//!
//! **Invariantes materializados** (CLAUDE.md): #1 (zero LLM — todo guardrail é contador/grafo/
//! timer/pattern-match), #4 (o EVENT LOG é a fonte da verdade — **cada assert lê o JSONL, nunca o
//! `RouteOutcome` em memória**), #6 (recusa é evento, pausa é projeção; nada é kill silencioso).
//!
//! **Validação INDEPENDENTE.** Este gate só LÊ o core; não altera `src/`. Onde a CLI é quem apenda
//! um evento (`lina guard` → `ActionGated`; `lina handshake` → `Handshake`; `lina resume` →
//! `CostCeilingResumed`), o gate reproduz a regra documentada do verbo e assere o CONTRATO do evento
//! no log. Se um mecanismo divergisse do design, o gate falharia em vez de "consertar" o core.
//!
//! ## SKIPPED rastreado (não implementado — entra com W3-6c)
//! - **custódia de segredo (AC-6.5)** e **A3 auth (outbox por-nó)** — `lina do` não existe; o outbox
//!   segue não-autenticado (a cadeia é decidida pelo binding do supervisor, não por campos do outbox).
//!   Hoje só a classificação `gated-hard → ask` (cenário b) é provável; a parte "deploy/pagamento
//!   bloqueado por ausência de credencial" entra quando o broker `lina do` + custódia entrarem.
//!   O gate roda com (a)-(g); esses 2 viram asserts obrigatórios quando o mecanismo for commitado.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::{
    check_action, parse_autonomy, render_message_block, render_message_block_full, AutonomyLevel,
    CostLedger, Decision, DeliveryOutcome, DomainEvent, EventRecord, EventStore, MailMessage,
    Mailbox, NodeId, RouteOutcome, Router, RouterConfig, Supervisor, DELEGATION_BUDGET,
    MAIL_SCHEMA_V1,
};

// ───────────────────────────── substrato headless ─────────────────────────────

/// Contador de unicidade p/ diretórios temporários (sem depender de relógio/uuid no teste).
static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-gate3-{tag}-{}-{n}", std::process::id()))
}

/// Writer-sink para registrar nós no supervisor sem PTY real (o gate da Onda 3 assere o LOG, não a
/// entrega ao PTY — esse caminho já é coberto pelo `gate_w34`).
fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// `deliver` trivial: a entrega ao PTY não é o objeto deste gate (é o `gate_w34`). O que importa é o
/// EVENTO no log; aqui só confirmamos que a rota "entregaria".
fn ok_deliver() -> impl FnMut(NodeId, NodeId, &str) -> Result<DeliveryOutcome, String> {
    |_target, _from, _text| {
        Ok(DeliveryOutcome::Injected {
            ready: true,
            bracketed: true,
        })
    }
}

/// Um Espaço headless: store (SQLite WAL + espelho `log.jsonl`) + supervisor + roteador. Remove o
/// diretório no `Drop` (best-effort).
struct Env {
    tmp: PathBuf,
    store: EventStore,
    sup: Arc<Supervisor>,
    router: Router,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

impl Env {
    /// Diretório do event store (onde vive o `log.jsonl` — `<.lina>/events`).
    fn store_dir(&self) -> PathBuf {
        self.tmp.join(".lina").join("events")
    }

    /// F1-0-4: o nó "terminou o turno" (Idle). Desde a entrega ciente de estado, TODA
    /// entrega marca o alvo `Busy` (serialização P1) — sequências que entregam de novo ao
    /// MESMO alvo simulam o fim do turno entre as entregas (como o lifecycle real faria
    /// no fim-de-resposta). Sem isto a entrega seguinte é `Retained{target_busy}`.
    fn turn_done(&self, name: &str) {
        let node = self.sup.node_by_name(name).expect("nó no roster");
        self.sup
            .set_status(node, lina_core::NodeStatus::Idle)
            .expect("set Idle");
    }
}

/// Sobe um Espaço com o `preset` (workspace criado) e a config dada.
fn env(tag: &str, cfg: RouterConfig) -> Env {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: format!("Gate Onda 3 — {tag}"),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated (preset)");
    let sup = Arc::new(Supervisor::new());
    let router = Router::with_config(Arc::clone(&sup), Mailbox::new(&lina), cfg);
    Env {
        tmp,
        store,
        sup,
        router,
    }
}

// ───────────────────────────── leitura do EVENT LOG (JSONL) ─────────────────────────────

/// **Lê o espelho append-only `<store>/events/log.jsonl`** e parseia cada linha num `EventRecord`.
/// É a fonte da verdade do gate (invariante #4): todo assert deriva DESTE log, reaplicável por replay
/// — nunca do `RouteOutcome` em memória (que só um harness in-process enxerga).
fn jsonl(dir: &std::path::Path) -> Vec<EventRecord> {
    let path = dir.join("log.jsonl");
    let data =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("ler {}: {e}", path.display()));
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<EventRecord>(l).expect("linha do log.jsonl = EventRecord"))
        .collect()
}

/// Campo string do payload de um registro (ex.: `root_cause_id`, `reason`, `by`).
fn p_str<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload.get(key).and_then(|v| v.as_str())
}

/// Campo numérico do payload (ex.: `hops`, `tokens`).
fn p_u64(rec: &EventRecord, key: &str) -> Option<u64> {
    rec.payload.get(key).and_then(serde_json::Value::as_u64)
}

/// Os `reason` (snake_case) de todos os `RouteBlocked` no log — o livro-razão das recusas (ADR 0003).
fn block_reasons(recs: &[EventRecord]) -> Vec<String> {
    recs.iter()
        .filter(|r| r.kind == "RouteBlocked")
        .filter_map(|r| p_str(r, "reason").map(String::from))
        .collect()
}

// ════════════════════════════════════ os 7 cenários ════════════════════════════════════

/// **(a) Auto-orquestração sem instrução (só turno-0).** Dois agentes resolvem presença
/// (`Handshake`), reivindicação de plano (`PlanClaimed`) e A2A (`MessageRouted`) — TUDO derivado de
/// um único disparo. Prova: a sequência `Handshake×2 → PlanClaimed → MessageRouted` está no log e
/// **nenhum** evento carrega um `root_cause_id` estranho ao turno-0 (root derivado do binding, hops 0
/// na origem). Um 2º root sem novo turno humano FALHARIA — seria orquestração manual vazando.
#[test]
fn a_auto_orquestracao_so_turno_zero() {
    let mut e = env("a-auto", RouterConfig::default());
    let a = e.sup.register("@A", Some("dev".into()), sink());
    let b = e.sup.register("@B", Some("dev".into()), sink());

    // (1) Presença: cada colega anuncia `lina handshake` (auto — o roster vivo é do Supervisor; aqui
    //     só registramos a entrada no log, events.rs:124). Nenhum input humano além do turno-0.
    for (node, name) in [(a, "@A"), (b, "@B")] {
        e.store
            .append(&DomainEvent::Handshake {
                node,
                name: name.into(),
            })
            .expect("Handshake");
    }

    // (2) Plano compartilhado: o preset semeia T1; @A o reivindica (W3-5) — produz `PlanClaimed`.
    e.router
        .seed_plan_item(&mut e.store, "T1", "desenhar schema")
        .expect("seed T1");
    let mut deliver = ok_deliver();
    let claim = MailMessage::new("@A", "plan", "plan.claim", "").with_ref("plan:T1");
    assert!(
        matches!(
            e.router
                .route_message(&claim, &mut e.store, 1000, &mut deliver),
            RouteOutcome::PlanApplied { .. }
        ),
        "plan.claim de @A deveria aplicar"
    );

    // (3) A2A: @A delega a @B (turno-0). @A é ORIGEM → o root EFETIVO é o id desta msg, hops 0.
    let ask = MailMessage::new("@A", "@B", "ask", "toca o item");
    let ask_id = ask.id.clone();
    assert!(matches!(
        e.router
            .route_message(&ask, &mut e.store, 1001, &mut deliver),
        RouteOutcome::Delivered { .. }
    ));

    // ── Assert DO LOG ──
    let recs = jsonl(&e.store_dir());

    // Handshake ×2, um por agente.
    let hs: Vec<&EventRecord> = recs.iter().filter(|r| r.kind == "Handshake").collect();
    assert_eq!(hs.len(), 2, "um Handshake por agente");
    let hs_nodes: BTreeSet<&str> = hs.iter().filter_map(|r| p_str(r, "node")).collect();
    let (a_s, b_s) = (a.to_string(), b.to_string());
    assert!(
        hs_nodes.contains(a_s.as_str()) && hs_nodes.contains(b_s.as_str()),
        "Handshake de @A e @B no log"
    );

    // PlanClaimed com os campos certos (mesmo registro).
    let pc = recs
        .iter()
        .find(|r| r.kind == "PlanClaimed")
        .expect("PlanClaimed no log");
    assert_eq!(p_str(pc, "by"), Some("@A"));
    assert_eq!(p_str(pc, "item"), Some("T1"));

    // MessageRouted: root derivado do turno-0 (= id da msg de origem), hops 0.
    let routed: Vec<&EventRecord> = recs.iter().filter(|r| r.kind == "MessageRouted").collect();
    assert_eq!(routed.len(), 1, "uma delegação A2A");
    assert_eq!(
        p_str(routed[0], "root_cause_id"),
        Some(ask_id.as_str()),
        "root é o turno de origem (id de @A — derivado do binding, não forjável)"
    );
    assert_eq!(
        p_u64(routed[0], "hops"),
        Some(0),
        "@A é origem → profundidade 0"
    );

    // ZERO root estranho: todos os eventos que CARREGAM root (MessageRouted/AwaitOpened) têm o mesmo.
    let roots: BTreeSet<&str> = recs
        .iter()
        .filter(|r| r.kind == "MessageRouted" || r.kind == "AwaitOpened")
        .filter_map(|r| p_str(r, "root_cause_id"))
        .collect();
    assert_eq!(
        roots.len(),
        1,
        "um único root_cause_id no log — nenhum 2º turno humano vazou"
    );

    // Sequência: Handshake×2 ANTES de PlanClaimed ANTES de MessageRouted (ordem do escritor único).
    let hs_max = hs.iter().map(|r| r.seq).max().unwrap();
    assert!(
        hs_max < pc.seq && pc.seq < routed[0].seq,
        "ordem Handshake → PlanClaimed → MessageRouted no log"
    );
}

/// **(b) Gate duro determinístico (ação irreversível).** `classify`/`decide` (guard.rs) + o contrato
/// `ActionGated` (apendado SÓ quando a decisão ≠ allow, events.rs:175-177). A matriz NUNCA retorna
/// `deny` — `gated-hard` é `ask` (piso confirmável). Assere **"não-allow"**, não "deny".
#[test]
fn b_gate_duro_classifica_e_loga() {
    let mut e = env("b-guard", RouterConfig::default());

    // O gate reproduz o verbo `lina guard`: classifica/decide (core puro) e apenda `ActionGated` só
    // quando a decisão NÃO é allow (events.rs:175-177). Devolve a decisão para o assert direto.
    let gate = |store: &mut EventStore, cmd: &str, autonomy: &str| -> Decision {
        let lvl = parse_autonomy(autonomy).expect("autonomia válida");
        let v = check_action(cmd, lvl);
        if v.decision != Decision::Allow {
            store
                .append(&DomainEvent::ActionGated {
                    cmd: cmd.to_string(),
                    class: v.class.as_str().to_string(),
                    decision: v.decision.as_str().to_string(),
                    node: None, // FIX-2: campo aditivo; este gate de teste não modela um nó.
                })
                .expect("ActionGated");
        }
        v.decision
    };

    // force-push em main: ask (gated-hard) em TODOS os níveis — autonomia jamais afrouxa o piso.
    for lvl in ["manual", "assistido", "autonomo"] {
        let d = gate(&mut e.store, "git push --force origin main", lvl);
        assert_ne!(
            d,
            Decision::Allow,
            "force-push em main NUNCA é allow ({lvl})"
        );
        assert_eq!(
            d,
            Decision::Ask,
            "gated-hard é ask (piso), nunca deny ({lvl})"
        );
    }
    // push em feature: ask em assistido (gated-soft) … e allow SEM evento em autônomo (afrouxa soft).
    assert_eq!(
        gate(&mut e.store, "git push origin feature/x", "assistido"),
        Decision::Ask
    );
    assert_eq!(
        gate(&mut e.store, "git push origin feature/x", "autonomo"),
        Decision::Allow,
        "autônomo afrouxa gated-soft → allow"
    );
    // rotina: allow sem evento.
    assert_eq!(
        gate(&mut e.store, "cargo test", "manual"),
        Decision::Allow,
        "rotina sempre allow"
    );

    // ── Assert DO LOG ── exatamente as 4 ações NÃO-allow viraram evento (3 hard + 1 soft).
    let recs = jsonl(&e.store_dir());
    let gated: Vec<&EventRecord> = recs.iter().filter(|r| r.kind == "ActionGated").collect();
    assert_eq!(gated.len(), 4, "só ações não-allow poluem o log");
    let hard = gated
        .iter()
        .filter(|r| p_str(r, "class") == Some("gated-hard"))
        .count();
    let soft = gated
        .iter()
        .filter(|r| p_str(r, "class") == Some("gated-soft"))
        .count();
    assert_eq!(hard, 3, "3 force-push em main = gated-hard");
    assert_eq!(soft, 1, "1 push feature/assistido = gated-soft");
    assert!(
        gated.iter().all(|r| p_str(r, "decision") == Some("ask")),
        "toda recusa logada é ask — a matriz nunca emite deny nem loga allow"
    );
}

/// **(c) Anti-loop A→B→A sob o mesmo `root_cause_id`.** Grafo de arestas `from→to_node` filtrado pelo
/// root; um ciclo sob o MESMO turno é bloqueado (`loop_detected`). É **por-turno**: a mesma aresta
/// B→A, numa origem fresca (root distinto, sem A→B no grafo daquele root), PASSA.
#[test]
fn c_anti_loop_por_turno() {
    // (c1) LOOP APERTADO: ping-pong repetido sob o mesmo turno → cortado após `MAX_CYCLE_REVISITS`.
    //      A 1ª volta (A→B→A) é DIÁLOGO legítimo e PASSA (ver router::tests::dialogue_reply_delivers_…);
    //      aqui empurramos o ping-pong até cruzar o limite e provamos que ainda barra + loga.
    let mut e = env("c-loop", RouterConfig::default());
    e.sup.register("@A", None, sink());
    e.sup.register("@B", None, sink());
    let mut deliver = ok_deliver();

    // As voltas dentro do limite entregam (diálogo) — herdam o root R1 do binding (hops < 4).
    for (i, (from, to)) in [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")]
        .iter()
        .enumerate()
    {
        // F1-0-4: cada perna do ping-pong só acontece após o turno anterior terminar.
        e.turn_done("@A");
        e.turn_done("@B");
        let m = MailMessage::new(*from, *to, "ask", "ping");
        assert!(
            matches!(
                e.router
                    .route_message(&m, &mut e.store, 1000 + i as u64, &mut deliver),
                RouteOutcome::Delivered { .. }
            ),
            "as primeiras voltas são diálogo (entregam)"
        );
    }
    // A repetição que cruza `MAX_CYCLE_REVISITS` fecha o ciclo apertado → LoopDetected.
    e.turn_done("@A");
    e.turn_done("@B"); // F1-0-4: o corte deve vir do ANTI-LOOP, não da retenção
    let over = MailMessage::new("@A", "@B", "ask", "ping demais");
    assert_eq!(
        e.router
            .route_message(&over, &mut e.store, 1010, &mut deliver),
        RouteOutcome::LoopDetected
    );
    assert!(
        block_reasons(&jsonl(&e.store_dir())).contains(&"loop_detected".to_string()),
        "a recusa por ciclo apertado está NO LOG (RouteBlocked{{loop_detected}})"
    );

    // (c2) TURNO DIFERENTE: a MESMA topologia B→A, numa origem fresca (sem A→B sob esse root), passa.
    let mut e2 = env("c-pass", RouterConfig::default());
    e2.sup.register("@A", None, sink());
    e2.sup.register("@B", None, sink());
    let mut deliver2 = ok_deliver();
    let fresh = MailMessage::new("@B", "@A", "ask", "novo turno"); // B é origem → root próprio, grafo vazio.
    assert!(
        matches!(
            e2.router
                .route_message(&fresh, &mut e2.store, 2000, &mut deliver2),
            RouteOutcome::Delivered { .. }
        ),
        "B→A num turno fresco não fecha ciclo (o anti-loop é por-turno)"
    );
    let recs2 = jsonl(&e2.store_dir());
    assert!(
        recs2.iter().any(|r| r.kind == "MessageRouted"),
        "o turno fresco roteou (MessageRouted no log)"
    );
    assert!(
        !block_reasons(&recs2).contains(&"loop_detected".to_string()),
        "nenhum loop_detected no turno fresco"
    );
}

/// **(d) Teto de custo (`token_budget_day`).** `CostLedger` event-sourced soma `TokenUsageReported`;
/// ao estourar, a delegação vira `CostCeiling` + `CostCeilingHit{day,tokens}` e o workspace fica
/// `Paused` (GATE humano, não kill — invariante #6). `CostCeilingResumed` (o que `lina resume
/// --confirm` apenda) zera a janela e reabre. Anti-amplificação (A4): 1 só `CostCeilingHit`.
#[test]
fn d_teto_de_custo_pausa_e_reabre() {
    // Round 5 #9: a janela do teto agora é DIÁRIA (`utc_day(rec.ts)==utc_day(now_ms)`). O `append`
    // carimba o `ts` com wall-clock real (sem injeção de relógio), então o `now_ms` do replay precisa
    // cair no MESMO dia-UTC dos appends → usamos o relógio real (o intent do gate — teto pausa/reabre,
    // anti-amplificação — é preservado; só a base de relógio muda).
    let now: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let cfg = RouterConfig {
        token_budget_day: 100,
        ..RouterConfig::default()
    };
    let mut e = env("d-cost", cfg);
    e.sup.register("@A", None, sink());
    e.sup.register("@B", None, sink());
    let mut deliver = ok_deliver();

    // (1) ANTES do teto (uso 0 < 100): delegação normal, nenhum CostCeilingHit.
    let m1 = MailMessage::new("@A", "@B", "ask", "antes");
    assert!(matches!(
        e.router.route_message(&m1, &mut e.store, now, &mut deliver),
        RouteOutcome::Delivered { .. }
    ));
    assert!(
        jsonl(&e.store_dir())
            .iter()
            .all(|r| r.kind != "CostCeilingHit"),
        "sem uso reportado, nenhum teto batido"
    );

    // Semeia o USO event-sourced (no app: fim-de-resposta da CLI, W0-10): 70 + 80 = 150 > 100.
    for t in [70u64, 80] {
        e.store
            .append(&DomainEvent::TokenUsageReported {
                node: "@B".into(),
                tokens: t,
            })
            .expect("TokenUsageReported");
    }

    // (2) Uso (150) ≥ teto (100): a próxima delegação é GATEADA (Paused), não entregue.
    e.turn_done("@B"); // F1-0-4: o bloqueio abaixo deve vir do TETO, não da retenção
    let m2 = MailMessage::new("@A", "@B", "ask", "estoura");
    assert_eq!(
        e.router.route_message(&m2, &mut e.store, now, &mut deliver),
        RouteOutcome::CostCeiling
    );
    // (2b) Já pausado: ainda bloqueia, mas NÃO re-apenda (A4).
    let m3 = MailMessage::new("@A", "@B", "ask", "ainda pausado");
    assert_eq!(
        e.router.route_message(&m3, &mut e.store, now, &mut deliver),
        RouteOutcome::CostCeiling
    );

    // ── Assert DO LOG ── exatamente 1 CostCeilingHit{tokens:150, day:YYYY-MM-DD}; Paused observável.
    let recs = jsonl(&e.store_dir());
    let hits: Vec<&EventRecord> = recs.iter().filter(|r| r.kind == "CostCeilingHit").collect();
    assert_eq!(
        hits.len(),
        1,
        "1 CostCeilingHit na transição (anti-amplificação A4)"
    );
    assert_eq!(
        p_u64(hits[0], "tokens"),
        Some(150),
        "o hit registra a soma da janela"
    );
    let day = p_str(hits[0], "day").expect("day no hit").to_string();
    assert!(
        day.len() == 10 && day.as_bytes()[4] == b'-' && day.as_bytes()[7] == b'-',
        "day é um rótulo UTC YYYY-MM-DD; veio {day:?}"
    );
    assert!(
        CostLedger::replay(&e.store, now).expect("ledger").paused,
        "o ledger reconstruído do log indica Paused (projeção observável, invariante #6)"
    );
    assert!(
        !recs
            .iter()
            .any(|r| r.kind == "MessageRouted" && p_str(r, "id") == Some(m2.id.as_str())),
        "a delegação gateada NÃO foi roteada (nada entregue no teto)"
    );

    // (3) RETOMADA: `lina resume --confirm` apenda CostCeilingResumed (reusa o `day` do hit) → reabre.
    e.store
        .append(&DomainEvent::CostCeilingResumed { day })
        .expect("CostCeilingResumed");
    assert!(
        !CostLedger::replay(&e.store, now).expect("ledger").paused,
        "após resume, o workspace não está mais Paused"
    );
    e.turn_done("@B"); // F1-0-4: B terminou o turno de m1 — o que se testa é o pós-resume
    let m4 = MailMessage::new("@A", "@B", "ask", "depois do resume");
    assert!(matches!(
        e.router.route_message(&m4, &mut e.store, now, &mut deliver),
        RouteOutcome::Delivered { .. }
    ));
    assert!(
        jsonl(&e.store_dir())
            .iter()
            .any(|r| r.kind == "MessageRouted" && p_str(r, "id") == Some(m4.id.as_str())),
        "após o resume a delegação volta a rotear (MessageRouted no log)"
    );
}

/// **(e) Await/reply lifecycle (fechamento autenticado — Round 4).** `A --await--> B` apenda
/// `AwaitOpened` e segura a aresta. Um reply de NÃO-target (`@C`) NÃO fecha (confused-deputy);
/// o reply AUTENTICADO de `@B` apenda `AwaitClosed{replied}` e libera o wait-for-graph.
#[test]
fn e_await_reply_autenticado() {
    let mut e = env("e-await", RouterConfig::default());
    let a = e.sup.register("@A", None, sink());
    let b = e.sup.register("@B", None, sink());
    e.sup.register("@C", None, sink());
    let mut deliver = ok_deliver();

    // (1) A --await--> B: abre o ticket (AwaitOpened + aresta a→b viva no wait-for-graph).
    let q = MailMessage::new("@A", "@B", "ask", "pergunta").awaiting();
    assert!(matches!(
        e.router.route_message(&q, &mut e.store, 1000, &mut deliver),
        RouteOutcome::Delivered { .. }
    ));
    assert!(
        e.sup.begin_await(b, a).is_err(),
        "ticket aberto: a→b vivo → b--await-->a seria deadlock (aresta segurada)"
    );

    // (2) @C (não-target) lê o id no log e injeta {from:@C, reply_to:q.id} → NÃO fecha.
    let intruder = MailMessage::new("@C", "@A", "ask", "intruso").replying_to(q.id.clone());
    let _ = e
        .router
        .route_message(&intruder, &mut e.store, 1001, &mut deliver);
    assert!(
        !jsonl(&e.store_dir())
            .iter()
            .any(|r| r.kind == "AwaitClosed"),
        "reply de não-participante NÃO pode logar AwaitClosed"
    );
    assert!(
        e.sup.begin_await(b, a).is_err(),
        "o ticket SOBREVIVE ao reply forjado de @C (aresta a→b intacta)"
    );

    // (3) O target REAL (@B) responde para o waiter (@A) → fecha (AwaitClosed{replied} + libera).
    e.turn_done("@A"); // F1-0-4: A terminou o turno da msg do intruso — testa o AWAIT, não a retenção
    let reply = MailMessage::new("@B", "@A", "ask", "resposta").replying_to(q.id.clone());
    assert!(matches!(
        e.router
            .route_message(&reply, &mut e.store, 1002, &mut deliver),
        RouteOutcome::Delivered { .. }
    ));

    // ── Assert DO LOG ── AwaitOpened{waiter:A,target:B,root} + exatamente 1 AwaitClosed{replied}.
    let recs = jsonl(&e.store_dir());
    let opened = recs
        .iter()
        .find(|r| r.kind == "AwaitOpened")
        .expect("AwaitOpened no log");
    assert_eq!(p_str(opened, "id"), Some(q.id.as_str()));
    assert_eq!(
        p_str(opened, "waiter"),
        Some(a.to_string().as_str()),
        "waiter = @A"
    );
    assert_eq!(
        p_str(opened, "target"),
        Some(b.to_string().as_str()),
        "target = @B"
    );
    assert!(
        p_str(opened, "root_cause_id").is_some(),
        "AwaitOpened carrega o root"
    );

    let closed: Vec<&EventRecord> = recs.iter().filter(|r| r.kind == "AwaitClosed").collect();
    assert_eq!(closed.len(), 1, "só o reply autenticado fecha o ticket");
    assert_eq!(p_str(closed[0], "id"), Some(q.id.as_str()));
    assert_eq!(p_str(closed[0], "reason"), Some("replied"));
    assert!(
        opened.seq < closed[0].seq,
        "o fechamento vem depois da abertura (e depois do reply intruso)"
    );

    // Aresta liberada: a aresta a→b foi removida pelo reply → b--await-->a agora é permitido.
    assert!(
        e.sup.begin_await(b, a).is_ok(),
        "o reply do participante autenticado liberou o wait-for-graph"
    );
    e.sup.end_await(b);
}

/// **(f) Blocks logados — o livro-razão das recusas.** Cada recusa de roteamento apenda seu
/// `RouteBlocked{reason}` (ADR 0003). EXCEÇÃO HONESTA: `Duplicate` é deliberadamente NÃO logado
/// (anti-amplificação A4, events.rs:153) → assere a AUSÊNCIA. (`acao`→`ActionGated` e
/// `custo`→`CostCeilingHit` são asserados em (b) e (d).)
#[test]
fn f_blocks_logados_e_dedupe_ausente() {
    let mut deliver = ok_deliver();

    // loop_detected (ping-pong A↔B repetido além de `MAX_CYCLE_REVISITS`; a 1ª volta é diálogo).
    {
        let mut e = env("f-loop", RouterConfig::default());
        e.sup.register("@A", None, sink());
        e.sup.register("@B", None, sink());
        for (i, (from, to)) in [("@A", "@B"), ("@B", "@A"), ("@A", "@B"), ("@B", "@A")]
            .iter()
            .enumerate()
        {
            // F1-0-4: cada perna só após o turno anterior terminar (ping-pong real).
            e.turn_done("@A");
            e.turn_done("@B");
            let m = MailMessage::new(*from, *to, "ask", "ping");
            e.router
                .route_message(&m, &mut e.store, 1 + i as u64, &mut deliver);
        }
        e.turn_done("@A");
        e.turn_done("@B"); // F1-0-4: o corte abaixo deve vir do anti-loop
        let over = MailMessage::new("@A", "@B", "ask", "ping demais");
        assert_eq!(
            e.router
                .route_message(&over, &mut e.store, 100, &mut deliver),
            RouteOutcome::LoopDetected
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"loop_detected".to_string()));
    }

    // hop_limit (cadeia propagada n0→…→n5 estoura max_depth).
    {
        let mut e = env("f-hop", RouterConfig::default());
        for i in 0..=5 {
            e.sup.register(format!("@n{i}"), None, sink());
        }
        for i in 0..5u64 {
            let m = MailMessage::new(format!("@n{i}"), format!("@n{}", i + 1), "ask", "vai");
            assert!(matches!(
                e.router
                    .route_message(&m, &mut e.store, i + 1, &mut deliver),
                RouteOutcome::Delivered { .. }
            ));
        }
        let over = MailMessage::new("@n5", "@n0", "ask", "estoura");
        assert_eq!(
            e.router
                .route_message(&over, &mut e.store, 100, &mut deliver),
            RouteOutcome::HopLimit
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"hop_limit".to_string()));
    }

    // blocked_by_autonomy (manual recusa delegação).
    {
        let cfg = RouterConfig {
            autonomy: AutonomyLevel::Manual,
            ..RouterConfig::default()
        };
        let mut e = env("f-autonomy", cfg);
        e.sup.register("@A", None, sink());
        e.sup.register("@B", None, sink());
        let m = MailMessage::new("@A", "@B", "ask", "oi");
        assert_eq!(
            e.router.route_message(&m, &mut e.store, 1, &mut deliver),
            RouteOutcome::BlockedByAutonomy
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"blocked_by_autonomy".to_string()));
    }

    // fanout_gated — SÓ na CASCATA (ADR 0007): o invariante MUDOU. A 1ª onda de ORIGEM pedida pelo
    // humano (`hops==0`) entrega a TODOS sem gate; o gate de fan-out dispara quando um nó que RECEBEU
    // uma entrega RE-ESPALHA (`hops>=1`, onde mora a tempestade). Cobrimos OS DOIS lados:
    // (b) CASCATA > FANOUT_GATE → AINDA gateado (os 2 asserts originais, movidos para a cascata).
    {
        let mut e = env("f-fanout-cascata", RouterConfig::default());
        e.sup.register("@A", None, sink());
        e.sup.register("@Relay", None, sink());
        for n in ["@B1", "@B2", "@B3", "@B4"] {
            e.sup.register(n, None, sink());
        }
        // @A→@Relay: @Relay RECEBE uma entrega → ganha binding → seu broadcast vira CASCATA (hops>=1).
        let seed = MailMessage::new("@A", "@Relay", "ask", "espalha isto");
        e.router.route_message(&seed, &mut e.store, 1, &mut deliver);
        // @Relay re-broadcasta a todos os vivos exceto ele (@A,@B1..@B4 = 5 > FANOUT_GATE=3) → GATEADO.
        let m = MailMessage::new("@Relay", "*", "broadcast", "todos");
        assert_eq!(
            e.router.route_message(&m, &mut e.store, 2, &mut deliver),
            RouteOutcome::FanoutGated { count: 5 }
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"fanout_gated".to_string()));
    }
    // (a) ORIGEM > FANOUT_GATE → ENTREGA a TODOS, com ZERO fanout_gated (o NOVO invariante do ADR 0007).
    {
        let mut e = env("f-fanout-origem", RouterConfig::default());
        e.sup.register("@A", None, sink());
        for n in ["@B1", "@B2", "@B3", "@B4"] {
            e.sup.register(n, None, sink());
        }
        // @A nunca recebeu A2A → ORIGEM (hops==0): a 1ª onda do humano entrega a todos, SEM gate humano.
        let m = MailMessage::new("@A", "*", "broadcast", "oi a todos");
        assert!(matches!(
            e.router.route_message(&m, &mut e.store, 1, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert!(
            !block_reasons(&jsonl(&e.store_dir())).contains(&"fanout_gated".to_string()),
            "fan-out de ORIGEM não gera fanout_gated (ADR 0007)"
        );
    }

    // budget_exceeded (root herdado acumula > DELEGATION_BUDGET).
    {
        let mut e = env("f-budget", RouterConfig::default());
        e.sup.register("@A", None, sink());
        e.sup.register("@B", None, sink());
        e.sup.register("@C", None, sink());
        let m0 = MailMessage::new("@A", "@B", "ask", "inicia");
        e.router.route_message(&m0, &mut e.store, 1, &mut deliver);
        for i in 0..DELEGATION_BUDGET {
            e.turn_done("@C"); // F1-0-4: C termina cada turno (testa o ORÇAMENTO)
            let m = MailMessage::new("@B", "@C", "ask", format!("d{i}"));
            assert!(matches!(
                e.router
                    .route_message(&m, &mut e.store, 10 + i as u64, &mut deliver),
                RouteOutcome::Delivered { .. }
            ));
        }
        e.turn_done("@C"); // F1-0-4: o estouro deve vir do orçamento, não da retenção
        let over = MailMessage::new("@B", "@C", "ask", "demais");
        assert_eq!(
            e.router
                .route_message(&over, &mut e.store, 999, &mut deliver),
            RouteOutcome::BudgetExceeded
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"budget_exceeded".to_string()));
    }

    // deadlock (--await fecharia um ciclo no wait-for-graph).
    {
        let mut e = env("f-deadlock", RouterConfig::default());
        let a = e.sup.register("@A", None, sink());
        let b = e.sup.register("@B", None, sink());
        e.sup.begin_await(b, a).expect("B espera A"); // aresta b→a viva
        let m = MailMessage::new("@A", "@B", "ask", "oi").awaiting();
        assert_eq!(
            e.router.route_message(&m, &mut e.store, 1, &mut deliver),
            RouteOutcome::Deadlock
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"deadlock".to_string()));
    }

    // unknown_sender + no_target (ambos apendam RouteBlocked — confirmado por inspeção: router.rs:295/327).
    {
        let mut e = env("f-unknown", RouterConfig::default());
        e.sup.register("@A", None, sink());
        let ghost = MailMessage::new("@Ghost", "@A", "ask", "oi");
        assert_eq!(
            e.router
                .route_message(&ghost, &mut e.store, 1, &mut deliver),
            RouteOutcome::UnknownSender("@Ghost".into())
        );
        let nobody = MailMessage::new("@A", "@Nobody", "ask", "oi");
        assert_eq!(
            e.router
                .route_message(&nobody, &mut e.store, 2, &mut deliver),
            RouteOutcome::NoTarget
        );
        let reasons = block_reasons(&jsonl(&e.store_dir()));
        assert!(reasons.contains(&"unknown_sender".to_string()));
        assert!(reasons.contains(&"no_target".to_string()));
    }

    // Duplicate AUSENTE: dedupe é benigno e NÃO loga (A4) — assere a ausência, não trata como falha.
    {
        let mut e = env("f-dedupe", RouterConfig::default());
        e.sup.register("@A", None, sink());
        e.sup.register("@B", None, sink());
        let m = MailMessage::new("@A", "@B", "ask", "oi");
        assert!(matches!(
            e.router.route_message(&m, &mut e.store, 1000, &mut deliver),
            RouteOutcome::Delivered { .. }
        ));
        assert_eq!(
            e.router.route_message(&m, &mut e.store, 1500, &mut deliver),
            RouteOutcome::Duplicate
        );
        let recs = jsonl(&e.store_dir());
        assert!(
            !block_reasons(&recs).contains(&"duplicate".to_string()),
            "duplicate NÃO pode virar evento (amplificaria escrita sob flood)"
        );
        assert!(
            !recs.iter().any(|r| r.kind == "RouteBlocked"),
            "o caminho de dedupe não apenda nenhum RouteBlocked"
        );
    }
}

/// **(B2 — defesa em profundidade do fan-out, ADR 0007) Forjar `hops`/`root_cause_id` no outbox NÃO
/// compra privilégio de ORIGEM.** O outbox é canal NÃO-autenticado: `msg.hops` e `msg.root_cause_id`
/// são campos que o remetente escreve livremente. A classificação ORIGEM-vs-CASCATA vem SEMPRE do
/// binding interno (`delivered_root`, carimbado numa ENTREGA real — `derive_root_hops`/W1), nunca
/// desses campos. Este gate TRAVA o invariante: um agente em CASCATA que forja `hops=0` (e um
/// `root_cause_id` fresco) continua tratado como cascata — gate de fan-out e orçamento aplicados —
/// enquanto a ORIGEM real (sem binding) com o MESMO `hops=0` no campo passa livre. (Hoje já é
/// garantido ESTRUTURALMENTE — `derive_root_hops` ignora ambos os campos; este teste impede regressão
/// se alguém voltar a confiar no outbox. NÃO muda comportamento do router — só adiciona cobertura.)
///
/// **Por que NÃO é vácuo.** Em (1) e (3) o atacante força `msg.hops=0`; em (2) a origem honesta TAMBÉM
/// tem `msg.hops=0` — o campo é idêntico nos dois lados, só o binding difere. Se o roteador voltasse a
/// LER `msg.hops` (em vez do hops EFETIVO do binding), (1) entregaria a todos e o `assert FanoutGated`
/// QUEBRARIA; se LESSE `msg.root_cause_id` (fresco a cada volta), (3) jamais acumularia `(root, sender)`
/// e o `assert BudgetExceeded` QUEBRARIA. Inverter a fonte de verdade do root/hops faz este teste
/// falhar — ele não passa trivialmente. (Asserts derivam do `log.jsonl`, invariante #4.)
#[test]
fn b2_forja_de_hops_e_root_nao_compra_privilegio_de_origem() {
    let mut deliver = ok_deliver();

    // (1) CASCATA forjando `hops=0` → AINDA gateada no fan-out (não vira origem livre).
    {
        let mut e = env("b2-cascata-fanout", RouterConfig::default());
        e.sup.register("@A", None, sink());
        e.sup.register("@Relay", None, sink());
        for n in ["@B1", "@B2", "@B3", "@B4"] {
            e.sup.register(n, None, sink());
        }
        // Entrega REAL @A→@Relay carimba o binding de @Relay → seu próximo envio tem hops efetivo >= 1.
        let seed = MailMessage::new("@A", "@Relay", "ask", "espalha isto");
        e.router.route_message(&seed, &mut e.store, 1, &mut deliver);
        // ATAQUE: @Relay re-broadcasta a 5 alvos (> FANOUT_GATE=3) FORJANDO credencial de origem.
        let mut forjada = MailMessage::new("@Relay", "*", "broadcast", "todos");
        forjada.hops = 0; // forja: "sou origem, hops 0"
        forjada.root_cause_id = Some("forjado-raiz-fresca".to_string()); // forja: root novo
        assert_eq!(
            e.router
                .route_message(&forjada, &mut e.store, 2, &mut deliver),
            RouteOutcome::FanoutGated { count: 5 },
            "cascata forjando hops=0 NÃO compra fan-out de origem — segue gateada"
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"fanout_gated".to_string()));
    }

    // (2) CONTRASTE — MESMO `hops=0` no campo, mas ORIGEM REAL (sem binding) → fan-out LIVRE.
    //     A ÚNICA diferença para (1) é a AUSÊNCIA de binding; o valor de `msg.hops` é idêntico (0).
    //     Resultados opostos com o mesmo campo ⇒ o campo é irrelevante; o binding é quem decide.
    {
        let mut e = env("b2-origem-livre", RouterConfig::default());
        e.sup.register("@A", None, sink());
        for n in ["@B1", "@B2", "@B3", "@B4"] {
            e.sup.register(n, None, sink());
        }
        let mut origem = MailMessage::new("@A", "*", "broadcast", "oi a todos");
        origem.hops = 0; // mesmo valor de (1) — aqui é HONESTO (sem binding ⇒ origem de fato)
        assert!(
            matches!(
                e.router
                    .route_message(&origem, &mut e.store, 1, &mut deliver),
                RouteOutcome::Delivered { .. }
            ),
            "origem real entrega a todos os 5 vivos mesmo > FANOUT_GATE (ADR 0007)"
        );
        assert!(
            !block_reasons(&jsonl(&e.store_dir())).contains(&"fanout_gated".to_string()),
            "origem real NÃO é gateada — o gate depende do binding, não de msg.hops"
        );
    }

    // (3) CASCATA forjando `root_cause_id` FRESCO a cada volta NÃO zera o orçamento por-cadeia.
    //     O contador é chaveado pelo root EFETIVO (do binding), não pelo campo forjado; e a CHECK só
    //     roda em cascata (hops efetivo >= 1), que o forge `hops=0` também não consegue escapar.
    {
        let mut e = env("b2-cascata-budget", RouterConfig::default());
        e.sup.register("@A", None, sink());
        e.sup.register("@B", None, sink());
        e.sup.register("@C", None, sink());
        // Binding de @B via entrega REAL @A→@B (root R carimbado pelo supervisor).
        let seed = MailMessage::new("@A", "@B", "ask", "inicia");
        e.router.route_message(&seed, &mut e.store, 1, &mut deliver);
        // DELEGATION_BUDGET delegações @B→@C, cada uma FORJANDO root novo + hops=0 → todas entregam
        // (acumulam contra (R, @B) — o root do binding —, não contra o root forjado).
        for i in 0..DELEGATION_BUDGET {
            e.turn_done("@C"); // F1-0-4: C termina cada turno (testa o ORÇAMENTO, não a retenção)
            let mut atq = MailMessage::new("@B", "@C", "ask", format!("forja{i}"));
            atq.hops = 0;
            atq.root_cause_id = Some(format!("forjado-fresco-{i}"));
            assert!(
                matches!(
                    e.router
                        .route_message(&atq, &mut e.store, 10 + i as u64, &mut deliver),
                    RouteOutcome::Delivered { .. }
                ),
                "delegação {i} dentro do orçamento entrega mesmo forjando root fresco"
            );
        }
        // A (n+1)-ésima, AINDA forjando root fresco + hops=0, ESTOURA — o root REAL do binding acumulou.
        e.turn_done("@C"); // F1-0-4: o estouro deve vir do ORÇAMENTO, não da retenção
        let mut over = MailMessage::new("@B", "@C", "ask", "estoura");
        over.hops = 0;
        over.root_cause_id = Some("forjado-fresco-final".to_string());
        assert_eq!(
            e.router
                .route_message(&over, &mut e.store, 999, &mut deliver),
            RouteOutcome::BudgetExceeded,
            "forjar root fresco NÃO reinicia o contador — orçamento por-cadeia segue valendo"
        );
        assert!(block_reasons(&jsonl(&e.store_dir())).contains(&"budget_exceeded".to_string()));
    }
}

/// **(g) Contrato congelado (envelope + terminador).** `MailMessage` (`lina/msg@1`) roundtripa por
/// JSON preservando TODOS os campos; `render_message_block[_full]` emite `[LINA::MSG]…[/LINA::MSG]`
/// e NUNCA contém `ESTUDIO` (pegadinha do doc 21).
#[test]
fn g_contrato_envelope_e_terminador() {
    // Roundtrip com TODOS os campos preenchidos.
    let mut msg = MailMessage::new("@A", "@B", "ask", "corpo da tarefa")
        .awaiting()
        .with_ref("plan:T4")
        .replying_to("msg_pergunta");
    msg.root_cause_id = Some("msg_turno0".into());
    msg.hops = 2;
    msg.ts_ms = 1_700_000_000_000;

    let json = serde_json::to_string(&msg).expect("serialize");
    let back: MailMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, back, "roundtrip preserva todos os campos");
    assert_eq!(back.schema, MAIL_SCHEMA_V1, "schema lina/msg@1");
    assert!(back.await_reply);
    assert_eq!(back.hops, 2);
    assert_eq!(back.root_cause_id.as_deref(), Some("msg_turno0"));
    assert_eq!(back.reply_to.as_deref(), Some("msg_pergunta"));
    assert_eq!(back.plan_ref(), Some("T4"));
    assert_eq!(back.ts_ms, 1_700_000_000_000);

    // Bloco completo (W3-7): sentinela + terminador LINA, com root/hops achatados; nunca ESTUDIO.
    let full = render_message_block_full("msg_1", "@A", "@B", "ask", "oi", "msg_turno0", 2);
    assert!(full.starts_with("[LINA::MSG]"));
    assert!(full.contains("root_cause_id: msg_turno0") && full.contains("hops: 2"));
    assert!(full.contains("from: @A") && full.contains("payload: oi"));
    assert!(full.trim_end().ends_with("[/LINA::MSG]"));
    assert!(
        !full.contains("ESTUDIO"),
        "namespace é LINA, nunca o resíduo ESTUDIO"
    );

    // Bloco básico idem (sem root/hops).
    let basic = render_message_block("msg_1", "@A", "@B", "ask", "oi");
    assert!(basic.starts_with("[LINA::MSG]") && basic.trim_end().ends_with("[/LINA::MSG]"));
    assert!(!basic.contains("ESTUDIO"));
}
