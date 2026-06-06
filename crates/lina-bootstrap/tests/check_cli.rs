//! F1-0-6 — `lina check "@X"`: estado VIVO do colega pela projeção do lifecycle
//! (F1-0-3/ADR 0019) + última atividade A2A, lidos de `agents.json` + `log.jsonl` —
//! **sem injetar nada** no PTY do alvo (critério 2: leitura pura, zero efeito).
//! Roda o BINÁRIO real.

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
            std::env::temp_dir().join(format!("lina-check-{tag}-{}-{nanos}", std::process::id()));
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

fn run_check(home: &TempHome, target: &str) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    Command::new(exe)
        .args(["check", target])
        .env("LINA_HOME", home.path())
        .output()
        .expect("executar o binário lina")
}

/// Workspace-fixture: roster + log com lifecycle (F1-0-3) e tráfego A2A do alvo.
/// O `node` do QA é um UUID fixo; o mapeamento nome→nó vem do `NodeRenamed` no log.
fn seed(home: &TempHome) -> &'static str {
    const QA: &str = "0198a0f0-0000-7000-8000-000000000001";
    let agents = r#"[{"name":"QA","role":"qa","status":"Running"},{"name":"Terminal A","role":"terminal","status":"Idle"}]"#;
    std::fs::write(home.path().join("agents.json"), agents).expect("agents.json");

    let events_dir = home.path().join("events");
    std::fs::create_dir_all(&events_dir).expect("events/");
    let log = [
        format!(r#"{{"seq":1,"ts":100,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{QA}","name":"QA"}}}}"#),
        format!(r#"{{"seq":2,"ts":200,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{QA}","status":"Ready","from":"Starting","reason":"spawn"}}}}"#),
        format!(r#"{{"seq":3,"ts":300,"kind":"MessageRouted","version":1,"payload":{{"event":"MessageRouted","id":"msg_1","from":"Terminal A","to":"@QA","to_node":"{QA}","intent":"handoff","root_cause_id":"msg_1","hops":0}}}}"#),
        format!(r#"{{"seq":4,"ts":400,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{QA}","status":"Busy","from":"Ready","reason":"pty_output"}}}}"#),
        format!(r#"{{"seq":5,"ts":500,"kind":"NodeStalled","version":1,"payload":{{"event":"NodeStalled","node":"{QA}","cycle_count":42}}}}"#),
    ]
    .join("\n");
    std::fs::write(events_dir.join("log.jsonl"), log + "\n").expect("log.jsonl");
    QA
}

/// Snapshot do estado em disco (para provar leitura PURA: nada muda).
fn disk_fingerprint(home: &TempHome) -> (String, bool) {
    let log = std::fs::read_to_string(home.path().join("events/log.jsonl")).unwrap_or_default();
    let outbox_exists = home.path().join("outbox").exists();
    (log, outbox_exists)
}

/// O check imprime o estado da state-machine (F1-0-3) com o MOTIVO legível da última
/// transição + o flag de travamento (ADR 0019) + a última atividade A2A — tudo lido da
/// projeção do log, nunca de view cacheada (§5 do ADR).
#[test]
fn check_prints_lifecycle_state_stall_and_last_activity() {
    let home = TempHome::new("estado");
    seed(&home);

    let out = run_check(&home, "@QA");
    assert!(
        out.status.success(),
        "check deveria responder; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let txt = String::from_utf8_lossy(&out.stdout);

    assert!(txt.contains("QA"), "identifica o alvo: {txt}");
    assert!(txt.contains("Busy"), "estado vivo da state-machine: {txt}");
    assert!(
        txt.contains("pty_output"),
        "motivo legível da última transição: {txt}"
    );
    assert!(
        txt.to_lowercase().contains("travado") || txt.to_lowercase().contains("stall"),
        "NodeStalled posterior ao Busy deve aparecer como travamento: {txt}"
    );
    assert!(
        txt.contains("handoff") && txt.contains("Terminal A"),
        "última atividade A2A (quem→quê): {txt}"
    );
    assert!(txt.contains("qa"), "papel do agents.json: {txt}");
}

/// **Critério 2:** `lina check` NÃO injeta nada — leitura pura. Prova: o disco fica
/// byte-idêntico (log intacto, NENHUM outbox criado — sem `MessageRouted`/delivery novo).
#[test]
fn check_is_read_only_no_injection() {
    let home = TempHome::new("puro");
    seed(&home);
    let before = disk_fingerprint(&home);

    let out = run_check(&home, "@QA");
    assert!(out.status.success());

    let after = disk_fingerprint(&home);
    assert_eq!(
        before.0, after.0,
        "log.jsonl byte-idêntico (zero evento novo)"
    );
    assert!(!after.1, "nenhum outbox criado (zero mensagem depositada)");
}

/// Transição posterior LIMPA o travamento (mesma regra da projeção): após o stall, um
/// `NodeStatusChanged → Idle` faz o check reportar Idle SEM travamento.
#[test]
fn check_clears_stall_after_next_transition() {
    let home = TempHome::new("clear");
    let qa = seed(&home);
    let extra = format!(
        r#"{{"seq":6,"ts":600,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{qa}","status":"Idle","from":"Busy","reason":"end_of_response"}}}}"#
    );
    let log_path = home.path().join("events/log.jsonl");
    let mut log = std::fs::read_to_string(&log_path).expect("ler log");
    log.push_str(&extra);
    log.push('\n');
    std::fs::write(&log_path, log).expect("apendar transição");

    let out = run_check(&home, "@QA");
    let txt = String::from_utf8_lossy(&out.stdout);
    assert!(txt.contains("Idle"), "estado mais recente vence: {txt}");
    assert!(txt.contains("end_of_response"), "motivo atualizado: {txt}");
    assert!(
        !txt.to_lowercase().contains("travado"),
        "transição limpa o stall (regra da projeção): {txt}"
    );
}

/// Alvo sem lifecycle no log (nó velho/sem F1-0-3): cai no status do `agents.json`,
/// com honestidade sobre a origem; alvo INEXISTENTE nos dois: erro legível + dica.
#[test]
fn check_falls_back_to_roster_and_fails_loud_on_unknown() {
    let home = TempHome::new("fallback");
    seed(&home);

    // "Terminal A" está no roster mas não tem NodeStatusChanged no log.
    let out = run_check(&home, "Terminal A");
    assert!(out.status.success());
    let txt = String::from_utf8_lossy(&out.stdout);
    assert!(txt.contains("Idle"), "fallback no status do roster: {txt}");

    // Alvo desconhecido: falha visível com dica de `lina list` (nunca silêncio).
    let out = run_check(&home, "@Fantasma");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("lina list"),
        "erro deve apontar o caminho (lina list): {err}"
    );
}
