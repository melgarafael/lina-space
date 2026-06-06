//! Gate F1-0-3 — state-machine de lifecycle + heartbeat com progresso (ADR 0019), headless
//! com **PTY REAL**:
//!
//! - **Critério 1:** nó emitindo output → `Busy`; após fim-de-resposta → `Idle`; PTY morto →
//!   `Dead` — cada transição é um evento com timestamp no `log.jsonl` (asserção sobre a
//!   SEQUÊNCIA completa, lida do espelho JSONL em disco, não do objeto em memória).
//! - **Critério 2 (positivo):** grid congelado (hash invariante) além do threshold gera
//!   `NodeStalled` observável no log — exatamente 1×. O anti-falso-positivo ("thinking longo
//!   com grid mudando NÃO gera") é provado na unidade (`lifecycle::tests::changing_tail_never_stalls`).
//! - **Critério 4** (replay ≡ roster vivo) é provado na unidade
//!   (`lifecycle::tests::replay_reconstructs_roster_statuses_identically`).
//!
//! O sinal de `Idle` é o fim-de-resposta EXPLÍCITO (decisão de design do módulo `lifecycle`):
//! aqui ele é disparado direto no engine — a fiação real `EndDetector`/W0-10 → engine pertence
//! ao caminho de entrega (F1-0-4+).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lina_core::lifecycle::{reason, LifecycleEngine, SampleOutcome};
use lina_core::{EventStore, NodeStatus, PtyCommand, PtyHost, Supervisor};
use uuid::Uuid;

const T: Duration = Duration::from_secs(10);

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("lina-gate-f103-{tag}-{}", Uuid::now_v7()));
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

/// Espera-POR-CONDIÇÃO com timeout (poll) — nunca sleep fixo de asserção.
fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Linhas do `log.jsonl` em disco, como JSON cru (prova no ARTEFATO durável, não em memória).
fn jsonl_records(dir: &Path) -> Vec<serde_json::Value> {
    let raw = std::fs::read_to_string(dir.join("log.jsonl")).expect("ler log.jsonl");
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("linha do JSONL desserializável"))
        .collect()
}

#[test]
fn gate_f103_lifecycle_with_real_pty() {
    let tmp = TempDir::new("seq");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    let sup = Supervisor::new();
    let mut host = PtyHost::new();
    let mut eng = LifecycleEngine::new();

    let node = sup.register("Terminal A", None, Box::new(std::io::sink()));

    // Spawn concluído → Ready (caller na criação do nó).
    eng.transition(&sup, &mut store, node, NodeStatus::Ready, reason::SPAWN)
        .expect("→ Ready");

    // PTY REAL: uma rajada de output e depois silêncio prolongado (vivo, grid congelado).
    let pty = host
        .spawn(
            PtyCommand::new("sh")
                .arg("-c")
                .arg("printf 'LINA_GATE_OUT\\n'; sleep 600"),
            80,
            24,
        )
        .expect("spawn sh real");

    let cycles = |h: &PtyHost| h.metrics(pty).map_or(0, |m| m.batches);
    let tail_of = |h: &PtyHost| {
        h.with_grid(pty, |vt| vt.last_nonempty_line())
            .unwrap_or_default()
    };

    // Amostra 1 (baseline): ainda sem delta de ciclo conhecido → Quiet.
    let c0 = cycles(&host);
    let s1 = eng
        .sample(
            &sup,
            &mut store,
            node,
            c0,
            LifecycleEngine::tail_hash(&tail_of(&host)),
        )
        .expect("amostra baseline");
    assert_eq!(s1, SampleOutcome::Quiet);

    // O output REAL chega (poll por condição: batches avançaram E o grid mostra a linha).
    assert!(
        poll_until(T, || cycles(&host) > c0
            && tail_of(&host).contains("LINA_GATE_OUT")),
        "output real do sh deve chegar ao grid dentro do timeout"
    );

    // Amostra 2: ciclo avançou → nó quieto vira Busy (transição com evento).
    let s2 = eng
        .sample(
            &sup,
            &mut store,
            node,
            cycles(&host),
            LifecycleEngine::tail_hash(&tail_of(&host)),
        )
        .expect("amostra pós-output");
    assert_eq!(s2, SampleOutcome::BecameBusy);
    assert_eq!(sup.get(node).expect("roster").status, NodeStatus::Busy);

    // Critério 2 (positivo): o sh está em `sleep 600` — grid REALMENTE congelado.
    // 3 amostras consecutivas sem progresso → NodeStalled exatamente 1×.
    let frozen_cycles = cycles(&host);
    let frozen_hash = LifecycleEngine::tail_hash(&tail_of(&host));
    let outcomes: Vec<SampleOutcome> = (0..3)
        .map(|i| {
            eng.sample(&sup, &mut store, node, frozen_cycles, frozen_hash)
                .unwrap_or_else(|e| panic!("amostra congelada {i}: {e}"))
        })
        .collect();
    assert_eq!(
        outcomes,
        vec![
            SampleOutcome::NoProgress(1),
            SampleOutcome::NoProgress(2),
            SampleOutcome::StalledWarned,
        ],
        "3 amostras congeladas em Busy cruzam o threshold (ADR 0019 §3)"
    );

    // Fim-de-resposta explícito (simula o EndDetector W0-10) → Idle.
    eng.on_end_of_response(&sup, &mut store, node)
        .expect("→ Idle");
    assert_eq!(sup.get(node).expect("roster").status, NodeStatus::Idle);

    // PTY morto DE VERDADE → Dead.
    host.kill(pty).expect("kill do sh");
    assert!(
        poll_until(T, || host.state(pty).is_none()),
        "kill deve remover o terminal do host"
    );
    eng.on_pty_exit(&sup, &mut store, node).expect("→ Dead");
    assert_eq!(sup.get(node).expect("roster").status, NodeStatus::Dead);

    // ── Critério 1: a SEQUÊNCIA está no log.jsonl EM DISCO, cada evento com timestamp. ──
    let records = jsonl_records(tmp.path());
    let transitions: Vec<(String, String, String, u64)> = records
        .iter()
        .filter(|r| r["kind"] == "NodeStatusChanged")
        .map(|r| {
            (
                r["payload"]["from"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                r["payload"]["status"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                r["payload"]["reason"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                r["ts"].as_u64().unwrap_or(0),
            )
        })
        .collect();

    let expected = [
        ("Starting", "Ready", reason::SPAWN),
        ("Ready", "Busy", reason::PTY_OUTPUT),
        ("Busy", "Idle", reason::END_OF_RESPONSE),
        ("Idle", "Dead", reason::PTY_EXIT),
    ];
    assert_eq!(
        transitions.len(),
        expected.len(),
        "exatamente as 4 transições do ciclo de vida: {transitions:?}"
    );
    for ((from, to, why, ts), (e_from, e_to, e_why)) in transitions.iter().zip(expected) {
        assert_eq!(
            (from.as_str(), to.as_str(), why.as_str()),
            (e_from, e_to, e_why)
        );
        assert!(*ts > 0, "cada transição carrega timestamp no log.jsonl");
    }

    // NodeStalled: exatamente 1, posicionado DEPOIS do →Busy e ANTES do →Idle.
    let stall_seqs: Vec<u64> = records
        .iter()
        .filter(|r| r["kind"] == "NodeStalled")
        .map(|r| r["seq"].as_u64().unwrap_or(0))
        .collect();
    assert_eq!(
        stall_seqs.len(),
        1,
        "NodeStalled é 1× na transição de stall"
    );
    let seq_of = |to: &str| {
        records
            .iter()
            .find(|r| r["kind"] == "NodeStatusChanged" && r["payload"]["status"] == to)
            .and_then(|r| r["seq"].as_u64())
            .unwrap_or(0)
    };
    assert!(
        seq_of("Busy") < stall_seqs[0] && stall_seqs[0] < seq_of("Idle"),
        "o stall acontece DENTRO da janela Busy (seq Busy {} < stall {} < seq Idle {})",
        seq_of("Busy"),
        stall_seqs[0],
        seq_of("Idle")
    );
}
