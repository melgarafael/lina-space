//! **F3-3 · Mentality — gate (g): adoção da sentinela `[LINA::CORRECTION]` medida desde o dia 1.**
//!
//! Lição do gate F1-3 (onda-f3-3 gate g): *plan.md teve adoção 0% — MEDIR, não assumir.* O bin
//! `intelligence_adoption` (F3-CONF-3 R2) reserva o slot `corrections` para a sentinela da Mentality.
//! Agora que `CorrectionObserved` EXISTE (contrato commitado), o slot tem de CONTAR o evento no log.
//!
//! Teste BLACK-BOX (o bin é `src/bin/`, não importável — roda-se o executável via `CARGO_BIN_EXE_*` e
//! lê-se o `--json`). Eval-first (spec 35 §7): controle POSITIVO (o bin roda e lê o log que escrevi) E
//! NEGATIVO (eventos não-correção não inflam a contagem).
//!
//! **ESTADO (reportado ao Maestro):** o bin HOJE fixa `corrections: 0` (slot "futuro F3-3" da R2). A
//! fiação — contar `CorrectionObserved` — é ~3 linhas em `intelligence_adoption.rs` (bin do J/CV3R2),
//! fora da minha fronteira (só tests/). Por isso o teste-alvo abaixo é `#[ignore]` (não envenena o gate
//! do pacote enquanto a fiação não chega); o controle de harness fica VERDE provando o caminho.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use lina_core::{DomainEvent, EventStore};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lina-f33adoc-{tag}-{}-{:?}-{n}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// Escreve um log com `n_corr` correções (papéis variados) + ruído (1 meta). Devolve o diretório de
/// eventos (`<tmp>/.lina/events`) que o bin lê (`log.jsonl`). O store é fechado (flush) ao retornar.
fn write_log(tag: &str, n_corr: usize) -> (PathBuf, PathBuf) {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");
    {
        let mut store = EventStore::open(&dir).expect("abrir store");
        for i in 0..n_corr {
            store
                .append(&DomainEvent::CorrectionObserved {
                    role: if i % 2 == 0 { "BACKEND" } else { "FRONTEND" }.to_string(),
                    summary: format!("correção {i}"),
                    refs: vec![],
                })
                .expect("append correção");
        }
        // ruído (controle −): uma meta NÃO conta como correção.
        store
            .append(&DomainEvent::GoalDefined {
                goal_id: "g-noise".to_string(),
                statement: "criar landing".to_string(),
                root_cause_id: "rc".to_string(),
                origin: "@Maestro".to_string(),
                budget_tokens: 0,
            })
            .expect("append goal");
    }
    (tmp, dir)
}

/// Roda o bin `intelligence_adoption <dir> --json` e devolve o JSON parseado (stdout).
fn run_bin(dir: &PathBuf) -> serde_json::Value {
    let exe = env!("CARGO_BIN_EXE_intelligence_adoption");
    let out = Command::new(exe)
        .arg(dir)
        .arg("--json")
        .output()
        .expect("rodar intelligence_adoption");
    assert!(
        out.status.success(),
        "o bin saiu com erro: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("stdout utf-8");
    serde_json::from_str(&stdout).expect("stdout é JSON válido")
}

/// **(g-controle) O harness black-box funciona: o bin roda e LÊ o log que escrevi.** Prova que o
/// caminho (escrever log → rodar o executável → parsear `--json`) é sólido — assim, o RED do teste-alvo
/// abaixo é especificamente o slot `corrections`, não um teste quebrado. Asserta o funil de Goal (que o
/// bin já mede) sobre o ruído que injetei.
#[test]
fn g_bin_roda_black_box_e_le_o_log() {
    let (tmp, dir) = write_log("harness", 3);
    let report = run_bin(&dir);
    assert_eq!(
        report
            .get("goals_defined")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "o bin LEU meu log (a meta-ruído foi contada) — harness sólido"
    );
    // o slot existe na saída (o contrato do JSON tem o campo).
    assert!(
        report.get("corrections").is_some(),
        "o slot 'corrections' existe no relatório (pronto para a fiação)"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// **(g) O slot `corrections` CONTA `CorrectionObserved` no log.** Escrevo 4 correções e 1 meta-ruído;
/// o relatório reporta `corrections == 4` (controle +), e a meta NÃO infla a contagem (controle −).
/// Mede a adoção da sentinela `[LINA::CORRECTION]` desde o dia 1 (lição plan.md 0%: medir, não assumir).
///
/// VERDE desde a fiação do bin `intelligence_adoption` (contar o kind `CorrectionObserved`, commit
/// "gate (g) bin conta CorrectionObserved"). Antes era `#[ignore]` (slot hard-coded 0) — eval-first:
/// o alvo nasceu RED-provado (`Some(0)` ≠ `Some(4)`) e flipou ao ligar a contagem. QUEBRA SE… a fiação
/// contasse o kind errado ou inflasse com ruído (a meta entra na contagem).
#[test]
fn g_corrections_conta_a_sentinela() {
    let (tmp, dir) = write_log("conta-sentinela", 4);
    let report = run_bin(&dir);
    assert_eq!(
        report
            .get("corrections")
            .and_then(serde_json::Value::as_u64),
        Some(4),
        "o slot conta as 4 sentinelas [LINA::CORRECTION] do log (controle + e −: a meta não infla)"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
