//! W4 · REPRO determinístico do **"delivered-sem-trabalho"** (achados de dogfooding #22/#23).
//!
//! ## O sintoma (o gargalo nº1 da orquestração)
//! `lina handoff` é confirmado como ENTREGUE, o terminal-alvo vai a `Idle`
//! (`on_end_of_response`, lifecycle.rs:309), mas **zero trabalho** e **zero `DomainEvent`
//! novo** — a 2ª tentativa idêntica funciona. (5 ocorrências, severidade ALTA.)
//!
//! ## A causa-raiz reproduzida aqui (hipótese 2 do diagnóstico)
//! `ready_now` (a2a.rs:192) julga PRONTO um TUI que está **fechando o turno**: o
//! `prompt_ready_regex` casa (`❯ `) e nenhum `busy_marker` aparece naquele instante, então
//! `deliver_a2a` injeta `AgentText`+`Submit` e devolve `Injected{ready:true}`. Mas o turno
//! não tinha acabado: o paste é engolido / o Enter chega cedo. O router loga
//! `MessageDelivered` (entrega FALSA) e o nó volta a `Idle` — nada de novo acontece.
//!
//! ## RED→GREEN (histórico do critério, agora GUARDA VIVA)
//! Estes testes afirmam o comportamento CORRETO: um turno-fechando deve ser **RETIDO**,
//! nunca confirmado como entregue. Eram o critério RED→GREEN do fix de W1 (B, `a2a.rs`):
//! nasceram `#[ignore]` e VERMELHOS no HEAD pré-fix (o bug entregava — provado com
//! `--ignored`). Com o fix de W1 em pé eles ficam VERDES, então o `#[ignore]` saiu e
//! agora rodam na suíte como **regressão viva** — se a janela do flicker reabrir, quebram.
//! Dependência de commit: pertencem à MESMA fatia do fix de W1 (sem o fix, são RED).
//!
//! ## Determinismo (sem thread/sleep → sem flake)
//! O grid [`ClosingTurnGrid`] modela o instante por um **contador de leituras**: a 1ª
//! leitura da região de input devolve um prompt OCIOSO (que o `ready_now` atual aprova —
//! o flicker), e QUALQUER releitura devolve o rodapé OCUPADO de volta ("esc to interrupt")
//! — i.e., o turno seguia em curso. Um fix robusto não confia num único snapshot pronto
//! (re-checa antes de submeter, ou exige prontidão estável); ambos os mecanismos pegam o
//! flicker. Um prompt ocioso de verdade (lido pronto SEMPRE) continua entregando — é o que
//! mantém o repro NÃO-VACUOSO (ver `delivery_ready_delivers_once` no router).
//!
//! Exerce o caminho REAL `route_message`→`deliver_a2a` (gate (a): não monta-à-mão).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use lina_core::{
    deliver_a2a, CliProfile, DomainEvent, EventRecord, EventStore, GridSense, InjectPolicy,
    MailMessage, Mailbox, NodeId, NodeStatus, RouteOutcome, Router, RouterConfig, Supervisor,
};
use lina_vt::TermMode;

// ───────────────────────────── substrato headless ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-repro-w4-{tag}-{}-{n}", std::process::id()))
}

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// Região de input REAL do Claude OCIOSO no prompt (espelha `a2a.rs::REAL_IDLE_REGION`):
/// rodapé SEM "esc to interrupt" → `ready_now` aprova. É o flicker que engana o sensor.
const IDLE_REGION: &str = "❯ \n────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle)";
/// Região de input REAL do Claude OCUPADO streamando (espelha `a2a.rs::REAL_BUSY_REGION`):
/// o rodapé carrega "esc to interrupt" → `ready_now` reprova. É o turno AINDA em curso.
const BUSY_REGION: &str =
    "❯ \n────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt";

/// Perfil claude-like (regex/markers REAIS do `profiles/claude-code.toml`), budget de
/// prontidão CURTO para o teste não esperar — espelha `a2a.rs::claude_like_profile`.
fn claude_like_profile() -> CliProfile {
    let src = r#"
        id = "claude-like"
        program = "claude"
        delivery = "pty_inject"
        submit_delay_ms = 5
        prompt_ready_regex = '(?m)^\s*[>❯]\s'
        ready_timeout_ms = 120
        busy_markers = ["esc to interrupt", "Press up to edit queued messages"]
        idle_ms = 100
        ask_timeout_ms = 100000
        [end_signal]
        kind = "stream_json"
        event_type = "result"
    "#;
    CliProfile::from_toml_str(src, "<repro-w4>").expect("perfil claude-like parseia")
}

/// O grid do instante de um turno fechando (ver doc do módulo). `mode()` é irrelevante
/// para o bug (qualquer modo); devolve um `TermMode` literal. A flutuação mora em
/// `input_region_text`: leitura #0 = ocioso (flicker pronto), leituras ≥1 = ocupado.
struct ClosingTurnGrid {
    reads: AtomicUsize,
}

impl ClosingTurnGrid {
    fn new() -> Self {
        Self {
            reads: AtomicUsize::new(0),
        }
    }
    /// Quantas vezes a região foi lida — diagnóstico do mecanismo do fix.
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl GridSense for ClosingTurnGrid {
    fn mode(&self) -> TermMode {
        TermMode {
            bracketed_paste: true,
            app_cursor: false,
        }
    }
    fn last_nonempty_line(&self) -> String {
        // Compartilha o contador com a região (o fix pode sensoriar por qualquer um).
        self.input_region_text()
    }
    fn input_region_text(&self) -> String {
        if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            IDLE_REGION.to_string()
        } else {
            BUSY_REGION.to_string()
        }
    }
}

/// Lê o espelho canônico `log.jsonl` (fonte da verdade — invariante #4: o assert lê o log,
/// não o `RouteOutcome` em memória).
fn jsonl(dir: &std::path::Path) -> Vec<EventRecord> {
    let path = dir.join("log.jsonl");
    let data =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("ler {}: {e}", path.display()));
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<EventRecord>(l).expect("linha do log.jsonl = EventRecord"))
        .collect()
}

fn count_kind(recs: &[EventRecord], kind: &str, msg_id: &str) -> usize {
    recs.iter()
        .filter(|r| r.kind == kind && r.payload.get("id").and_then(|v| v.as_str()) == Some(msg_id))
        .count()
}

// ════════════════════════════ o repro — santo graal (caminho real) ════════════════════════════

/// **REPRO via `route_message`→`deliver_a2a` REAL (gate (a)).** Um `ask` para um alvo `Idle`
/// cujo grid está no instante de um turno fechando.
///
/// - **RED (HEAD):** `deliver_a2a` lê o flicker pronto 1× e injeta → o router loga
///   `MessageDelivered` (entrega FALSA) e devolve `Delivered`. As asserções abaixo (que
///   exigem RETENÇÃO, não entrega) FALHAM.
/// - **GREEN (pós-fix de B):** o fix não confia no snapshot pronto isolado → `deliver_a2a`
///   devolve `Injected{ready:false}` → o router RETÉM (`MessageRetained`), sem
///   `MessageDelivered`. A 2ª tentativa, num instante limpo, é que entrega.
#[test]
fn turno_fechando_nao_pode_virar_entregue_via_route_message() {
    let tmp = unique("e2e");
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let mut store = EventStore::open(lina.join("events")).expect("abrir event store");
    store
        .append(&DomainEvent::WorkspaceCreated {
            name: "Repro W4".into(),
            focus_preset: String::new(),
        })
        .expect("WorkspaceCreated");

    let sup = Arc::new(Supervisor::new());
    let _a = sup.register("@A", None, sink());
    let b = sup.register("@B", None, sink());
    // O cenário do bug: o alvo ACABOU um turno → o lifecycle o julga `Idle` (o grid é
    // quem revela, no instante da entrega, que o turno seguia fechando).
    sup.set_status(b, NodeStatus::Idle).expect("set Idle");

    let mut router = Router::with_config(
        Arc::clone(&sup),
        Mailbox::new(&lina),
        RouterConfig::default(),
    );

    let grid = ClosingTurnGrid::new();
    let prof = claude_like_profile();
    let mut deliver = |target: NodeId, from: NodeId, text: &str| {
        deliver_a2a(
            &sup,
            target,
            from,
            text,
            &prof,
            &grid,
            InjectPolicy::AllowAll,
        )
        .map_err(|e| e.to_string())
    };

    let m = MailMessage::new("@A", "@B", "ask", "começa a tarefa");
    let out = router.route_message(&m, &mut store, 1_000, &mut deliver);

    let recs = jsonl(&lina.join("events"));
    let delivered = count_kind(&recs, "MessageDelivered", &m.id);
    let retained = count_kind(&recs, "MessageRetained", &m.id);

    // ── A propriedade que o fix garante: turno-fechando = RETIDO, jamais "entregue".
    assert_eq!(
        delivered,
        0,
        "turno fechando NÃO pode logar MessageDelivered (entrega FALSA = o bug #22/#23). \
         out={out:?}, leituras do grid={}",
        grid.reads()
    );
    assert!(
        matches!(out, RouteOutcome::Retained { to } if to == b),
        "turno fechando deve RETER (re-tenta num instante limpo), obteve {out:?}"
    );
    assert_eq!(
        retained, 1,
        "exatamente 1 MessageRetained (anti-amplificação A4)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ════════════════════════════ o repro — localização no `deliver_a2a` ════════════════════════════

/// **REPRO no ponto exato (unidade): `deliver_a2a` não pode injetar num turno fechando.**
/// Localiza a causa-raiz para B sem passar pelo router — o caso mais apertado.
///
/// - **RED (HEAD):** `wait_ready` aprova a leitura #0 (flicker) e injeta às cegas →
///   `Injected{ready:true}` + 2 `WriteOp` na fila (AgentText+Submit).
/// - **GREEN (pós-fix):** `Injected{ready:false}` (sinal de OCUPADO → router RETÉM) e
///   **zero `WriteOp`** (a não-injeção anti-texto-colado preservada).
#[test]
fn deliver_a2a_nao_injeta_em_turno_fechando() {
    let sup = Supervisor::new();
    let from = sup.register("@A", None, sink());
    let target = sup.register("@B", None, sink());

    let grid = ClosingTurnGrid::new();
    let prof = claude_like_profile();

    let res = deliver_a2a(
        &sup,
        target,
        from,
        "começa a tarefa",
        &prof,
        &grid,
        InjectPolicy::AllowAll,
    )
    .expect("deliver_a2a não falha — turno-fechando é OCUPADO (Ok), nunca erro");

    // O alvo OCUPADO (turno fechando) deve sinalizar retenção, NÃO entrega.
    assert!(
        !res.injected(),
        "turno fechando = OCUPADO → NÃO injeta (esperado Injected{{ready:false}}), obteve {res:?}; \
         leituras do grid={}",
        grid.reads()
    );
    // Dá tempo da fila serial drenar qualquer write (não deveria haver nenhum).
    std::thread::sleep(std::time::Duration::from_millis(40));
    assert!(
        sup.applied_ops(target).is_empty(),
        "nada pode ser escrito num turno fechando (a não-injeção anti-texto-colado é preservada)"
    );
}
