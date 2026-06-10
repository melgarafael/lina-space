//! F1-3-7 — `lina retro`: auto-aprimoramento v0 (Curator a event-sourcing, ZERO LLM).
//! Roda o BINÁRIO real contra um `events/log.jsonl` semeado, provando os critérios de aceite
//! end-to-end: determinismo byte-idêntico (1), first-run honesto (5) e o GATE INVIOLÁVEL (4 —
//! mutação recusada + log byte-idêntico, prova de não-escrita). O miolo determinístico
//! (projeções/limiares) é provado nos unit tests de `retro.rs`; aqui prova-se o FIO do verbo.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempHome(PathBuf);

impl TempHome {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p =
            std::env::temp_dir().join(format!("lina-retro-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).expect("criar LINA_HOME");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_retro(home: &TempHome, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut full = vec!["retro"];
    full.extend_from_slice(args);
    Command::new(exe)
        .args(&full)
        .env("LINA_HOME", home.path())
        .output()
        .expect("executar o binário lina")
}

/// `now_ms` FIXO (injetado por `--now-ms`) — torna o relatório totalmente determinístico
/// (inclusive o cabeçalho) e ancora os limiares stale/archive sem wall-clock.
const NOW: u64 = 1_000_000_000_000;
const DAY: u64 = 86_400_000;

/// Log rico: exercita skills (ativa/stale/archive + cluster), coordenação, custos e lacunas.
/// `node` em formato UUID (NodeId) — mas o `lina retro` lê os campos como strings opacas.
fn seed_rich(home: &TempHome) {
    const N1: &str = "00000000-0000-0000-0000-000000000001";
    const N2: &str = "00000000-0000-0000-0000-000000000002";
    const N3: &str = "00000000-0000-0000-0000-000000000003";
    let events_dir = home.path().join("events");
    std::fs::create_dir_all(&events_dir).expect("events/");
    let lines = [
        line(
            1,
            NOW - 5 * DAY,
            "SkillInvoked",
            &format!(r#""node":"{N1}","skill":"growthos-sales""#),
        ),
        line(
            2,
            NOW - 2 * DAY,
            "SkillInvoked",
            &format!(r#""node":"{N1}","skill":"growthos-sales""#),
        ),
        line(
            3,
            NOW - DAY,
            "SkillInvoked",
            &format!(r#""node":"{N1}","skill":"growthos-narrative""#),
        ),
        line(
            4,
            NOW - 100 * DAY,
            "SkillInvoked",
            &format!(r#""node":"{N1}","skill":"velha-sem-uso""#),
        ),
        line(
            5,
            60,
            "RouteBlocked",
            r#""id":"m5","reason":"no_target","from":"x","to":"designer""#,
        ),
        line(
            6,
            70,
            "RouteBlocked",
            r#""id":"m6","reason":"no_target","from":"x","to":"designer""#,
        ),
        line(
            7,
            80,
            "SpawnGated",
            &format!(r#""id":"m7","requested_by":"{N1}","reason":"cascade""#),
        ),
        line(
            8,
            90,
            "CircuitOpened",
            &format!(r#""node":"{N3}","failures":3"#),
        ),
        line(
            9,
            100,
            "MessageRouted",
            &format!(
                r#""id":"m9","from":"{N1}","to":"t","intent":"ask","root_cause_id":"r1","hops":0"#
            ),
        ),
        line(
            10,
            110,
            "MessageRouted",
            &format!(
                r#""id":"m10","from":"{N1}","to":"t","intent":"handoff","root_cause_id":"r1","hops":1"#
            ),
        ),
        line(
            11,
            120,
            "TokenUsageReported",
            &format!(r#""node":"{N1}","tokens":100"#),
        ),
        line(
            12,
            125,
            "TokenUsageReported",
            &format!(r#""node":"{N2}","tokens":100"#),
        ),
        line(
            13,
            130,
            "TokenUsageReported",
            &format!(r#""node":"{N3}","tokens":1000"#),
        ),
        line(
            14,
            140,
            "NodeRoleAssigned",
            &format!(r#""node":"{N3}","role":"dev""#),
        ),
    ];
    std::fs::write(events_dir.join("log.jsonl"), lines.join("\n") + "\n").expect("log.jsonl");
}

/// Monta uma linha do `log.jsonl` (= um `EventRecord` serializado) com o payload tagueado.
fn line(seq: u64, ts: u64, kind: &str, fields: &str) -> String {
    format!(
        r#"{{"seq":{seq},"ts":{ts},"kind":"{kind}","version":1,"payload":{{"event":"{kind}",{fields}}}}}"#
    )
}

/// **Critério 1 (determinismo) + leitura real:** o relatório traz os números do log e é
/// BYTE-IDÊNTICO em 2 execuções sobre a mesma fixture (now injetado).
#[test]
fn retro_reports_projections_and_is_byte_identical() {
    let home = TempHome::new("rico");
    seed_rich(&home);

    let a = run_retro(&home, &["--now-ms", &NOW.to_string()]);
    assert!(
        a.status.success(),
        "retro deve rodar; stderr: {}",
        String::from_utf8_lossy(&a.stderr)
    );
    let out = String::from_utf8_lossy(&a.stdout).to_string();

    // skills: 2 invocações de growthos-sales, candidata a archive (>90d), cluster growthos.
    assert!(out.contains("growthos-sales"), "uso por skill: {out}");
    assert!(
        out.contains("ARCHIVE (>90d)"),
        "velha-sem-uso classificada archive: {out}"
    );
    assert!(
        out.contains("arquivar 'velha-sem-uso'"),
        "sugestao de arquivar (gate humano): {out}"
    );
    assert!(
        out.contains("consolidacao"),
        "cluster growthos sugerido: {out}"
    );
    // coordenacao: 2 no_target, 1 cascade, 1 breaker, re-delegacao r1.
    assert!(out.contains("no_target: 2"), "bloqueios por motivo: {out}");
    assert!(out.contains("cascade: 1"), "spawn gated: {out}");
    // custos: outlier N3, papel dev; pedidos de origem ask:1; lacuna designer:2.
    assert!(
        out.contains("OUTLIER"),
        "N3 (1000) vs media → outlier: {out}"
    );
    assert!(out.contains("dev"), "papel correlacionado: {out}");
    assert!(out.contains("designer: 2"), "lacuna de papel: {out}");

    // determinismo: 2ª execução byte-idêntica.
    let b = run_retro(&home, &["--now-ms", &NOW.to_string()]);
    assert_eq!(
        out,
        String::from_utf8_lossy(&b.stdout),
        "relatorio deve ser byte-identico em 2 execucoes"
    );
}

/// **Critério 5 (first-run honesto):** workspace sem eventos de skill → "dados insuficientes",
/// ZERO sugestão de prune (a palavra "arquivar" não aparece).
#[test]
fn retro_first_run_is_honest() {
    let home = TempHome::new("novo");
    // sem log nenhum.
    let out = run_retro(&home, &["--now-ms", &NOW.to_string()]);
    assert!(out.status.success(), "first-run roda sem erro");
    let txt = String::from_utf8_lossy(&out.stdout);
    assert!(
        txt.contains("dados insuficientes"),
        "declara dados insuficientes: {txt}"
    );
    assert!(
        !txt.contains("arquivar"),
        "NAO sugere prune no first-run: {txt}"
    );
}

/// **Critério 4 (gate inviolável):** induzir o verbo a arquivar/aplicar direto → RECUSA (exit≠0)
/// com mensagem de gate humano, E o log fica BYTE-IDÊNTICO (prova observável de não-escrita).
#[test]
fn retro_refuses_mutation_and_never_writes() {
    let home = TempHome::new("gate");
    seed_rich(&home);
    let log_path = home.path().join("events/log.jsonl");
    let before = std::fs::read_to_string(&log_path).expect("ler log antes");

    for attempt in [
        vec!["archive", "velha-sem-uso"],
        vec!["--apply"],
        vec!["pin", "growthos-sales"],
        vec!["prune"],
    ] {
        let out = run_retro(&home, &attempt);
        assert!(
            !out.status.success(),
            "mutacao '{attempt:?}' deve ser recusada (exit != 0)"
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.to_lowercase().contains("gate")
                || err.to_lowercase().contains("nao aplica")
                || err.contains("NAO existe"),
            "recusa deve explicar o gate humano: {err}"
        );
    }

    let after = std::fs::read_to_string(&log_path).expect("ler log depois");
    assert_eq!(
        before, after,
        "log byte-identico: nenhuma mutacao escreveu nada"
    );
    assert!(
        !home.path().join("outbox").exists(),
        "nenhum outbox criado (o verbo nao enfileira nada)"
    );
}
