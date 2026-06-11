//! FIX-4 — os verbos contam o ESTADO GLOBAL do Espaço (freio de orquestração / teto de custo) que
//! SÓ o humano vê no rodapé do canvas. A dor real (fundador): com o freio ATIVO, `lina ask` devolvia
//! "ok/enfileirada" e o Maestro pedalou 20+ min contra uma fila congelada. Aqui provamos que, sob
//! pausa, `lina ask`/`handoff`/`broadcast` contam a verdade leiga e PARAM de mandar re-tentar — e a
//! mensagem AINDA enfileira (mecanismo intacto, nada se perde); `lina check`/`whoami` ganham 1 linha
//! de estado global nos dois casos (ativo/pausado). Roda o BINÁRIO real (padrão de `handoff_cli.rs`).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempWs {
    /// `home` = LINA_HOME (mailbox + event log `events/log.jsonl`).
    home: PathBuf,
    /// `cwd` = diretório do terminal com `.lina/bootstrap.json` (de onde o verbo lê o `from`).
    cwd: PathBuf,
}

impl TempWs {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("lina-fix4-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("shared");
        let cwd = root.join("terminal-a");
        std::fs::create_dir_all(home.join("events")).expect("criar LINA_HOME/events");
        std::fs::create_dir_all(cwd.join(".lina")).expect("criar cwd/.lina");
        let bootstrap = r#"{"terminal_name":"Terminal A","roster":["Terminal A","QA"],"vault_path":"/tmp/none","autonomy":"assisted"}"#;
        std::fs::write(cwd.join(".lina/bootstrap.json"), bootstrap).expect("bootstrap.json");
        Self { home, cwd }
    }

    /// Escreve o espelho `log.jsonl` com as linhas dadas (formato verbatim do event store).
    fn seed_log(&self, lines: &[&str]) {
        let body = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };
        std::fs::write(self.home.join("events/log.jsonl"), body).expect("log.jsonl");
    }
}

impl Drop for TempWs {
    fn drop(&mut self) {
        if let Some(root) = self.home.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

// Linhas de log verbatim (o scanner do bin lê só o `kind`; o payload do freio é vazio).
const ORCHESTRATION_PAUSED: &str = r#"{"seq":1,"ts":1,"kind":"OrchestrationPaused","version":1,"payload":{"event":"OrchestrationPaused"}}"#;
const ORCHESTRATION_RESUMED: &str = r#"{"seq":2,"ts":2,"kind":"OrchestrationResumed","version":1,"payload":{"event":"OrchestrationResumed"}}"#;
const COST_CEILING_HIT: &str = r#"{"seq":1,"ts":1,"kind":"CostCeilingHit","version":1,"payload":{"event":"CostCeilingHit","day":"2026-06-11","tokens":999}}"#;

fn run(ws: &TempWs, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    Command::new(exe)
        .args(args)
        .current_dir(&ws.cwd)
        .env("LINA_HOME", &ws.home)
        // Isola do env de spawn que um terminal do Lina injeta (rodar `cargo test` dentro do Espaço
        // não pode mudar o `from`): a identidade aqui é a da FICHA.
        .env_remove("LINA_NODE_NAME")
        .output()
        .expect("executar o binário lina")
}

/// Quantos envelopes existem no outbox por-nó do remetente — prova de que a fila NÃO foi alterada.
fn outbox_count(ws: &TempWs) -> usize {
    let dir = ws.home.join("outbox").join("Terminal A");
    std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

/// Sob freio, `lina ask` conta a verdade leiga (⏸ / ▶ Retomar cooperação), NÃO manda re-tentar, sai
/// com sucesso (o agente narra e PARA) e a mensagem AINDA enfileira (durável — nada se perde).
#[test]
fn ask_under_brake_tells_truth_and_still_enqueues() {
    let ws = TempWs::new("ask-brake");
    ws.seed_log(&[ORCHESTRATION_PAUSED]);

    let out = run(&ws, &["ask", "@QA", "rode os testes"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "ask sob freio sai com sucesso (a msg foi guardada); stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("PAUSADO") && stdout.contains("▶ Retomar cooperação"),
        "deve contar a verdade do freio: {stdout}"
    );
    assert!(
        !stdout.contains("tente de novo"),
        "NÃO pode mandar o agente re-tentar (era a meia-verdade que fazia pedalar): {stdout}"
    );
    assert_eq!(
        outbox_count(&ws),
        1,
        "a mensagem AINDA enfileira — mecanismo intacto, nada se perde"
    );
}

/// Retomado (Paused→Resumed, último vence): caminho NORMAL — sem a verdade do freio; a msg enfileira.
/// (Sem app/router no teste, o poll vence o prazo e cai no "ok: enviada" de sempre — o que importa é
/// que a saída deixou de mentir só quando há freio.)
#[test]
fn ask_when_resumed_behaves_normally() {
    let ws = TempWs::new("ask-resumed");
    ws.seed_log(&[ORCHESTRATION_PAUSED, ORCHESTRATION_RESUMED]);

    let out = run(&ws, &["ask", "@QA", "oi"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        !stdout.contains("PAUSADO"),
        "retomado não conta freio (fluxo normal): {stdout}"
    );
    assert_eq!(outbox_count(&ws), 1, "msg enfileirada normalmente");
}

/// `lina handoff` compartilha o mesmo caminho (`enqueue_and_report`): sob freio, mesma verdade e a
/// delegação AINDA enfileira (autonomia `assisted` da ficha permite delegar).
#[test]
fn handoff_under_brake_tells_truth_and_still_enqueues() {
    let ws = TempWs::new("handoff-brake");
    ws.seed_log(&[ORCHESTRATION_PAUSED]);

    let out = run(&ws, &["handoff", "@QA", "valide o item T4"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "handoff sob freio sai com sucesso; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("PAUSADO") && stdout.contains("▶ Retomar cooperação"),
        "{stdout}"
    );
    assert_eq!(outbox_count(&ws), 1);
}

/// `lina broadcast` (açúcar sobre `run_ask`) também conta a verdade sob freio.
#[test]
fn broadcast_under_brake_tells_truth_and_still_enqueues() {
    let ws = TempWs::new("broadcast-brake");
    ws.seed_log(&[ORCHESTRATION_PAUSED]);

    let out = run(&ws, &["broadcast", "*", "aviso geral"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "broadcast sob freio sai com sucesso; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("PAUSADO"), "{stdout}");
    assert_eq!(outbox_count(&ws), 1);
}

/// Teto de custo (mesma família do freio — achado #11): sob `CostCeilingHit`, `lina ask` conta a
/// verdade do teto e a mensagem enfileira.
#[test]
fn ask_under_cost_ceiling_tells_truth_and_still_enqueues() {
    let ws = TempWs::new("ask-cost");
    ws.seed_log(&[COST_CEILING_HIT]);

    let out = run(&ws, &["ask", "@QA", "oi"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        stdout.contains("teto de custo"),
        "deve contar a verdade do teto: {stdout}"
    );
    assert_eq!(outbox_count(&ws), 1);
}

/// `lina check` ganha a linha de ESTADO GLOBAL nos dois casos — o que o agente NÃO via ao checar
/// um colega "Idle" sem saber que o Espaço estava pausado.
#[test]
fn check_shows_global_state_line_in_both_cases() {
    let ws = TempWs::new("check");
    std::fs::write(
        ws.home.join("agents.json"),
        r#"[{"name":"QA","role":"qa","status":"Idle"}]"#,
    )
    .expect("agents.json");

    ws.seed_log(&[ORCHESTRATION_PAUSED]);
    let paused = run(&ws, &["check", "@QA"]);
    let s = String::from_utf8_lossy(&paused.stdout);
    assert!(
        paused.status.success(),
        "check deve achar QA no roster; stderr: {}",
        String::from_utf8_lossy(&paused.stderr)
    );
    assert!(
        s.contains("PAUSADA"),
        "check sob freio mostra a cooperação PAUSADA: {s}"
    );

    ws.seed_log(&[ORCHESTRATION_PAUSED, ORCHESTRATION_RESUMED]);
    let active = run(&ws, &["check", "@QA"]);
    let s = String::from_utf8_lossy(&active.stdout);
    assert!(
        s.contains("cooperação automática: ativa"),
        "retomado mostra a cooperação ativa: {s}"
    );
}

/// `lina whoami` (ramo humano) ganha a mesma linha de estado global nos dois casos.
#[test]
fn whoami_shows_global_state_line_in_both_cases() {
    let ws = TempWs::new("whoami");

    ws.seed_log(&[ORCHESTRATION_PAUSED]);
    let paused = run(&ws, &["whoami"]);
    let s = String::from_utf8_lossy(&paused.stdout);
    assert!(
        paused.status.success(),
        "whoami deve ler a ficha; stderr: {}",
        String::from_utf8_lossy(&paused.stderr)
    );
    assert!(s.contains("PAUSADA"), "whoami sob freio: {s}");

    ws.seed_log(&[ORCHESTRATION_PAUSED, ORCHESTRATION_RESUMED]);
    let active = run(&ws, &["whoami"]);
    let s = String::from_utf8_lossy(&active.stdout);
    assert!(
        s.contains("cooperação automática: ativa"),
        "whoami retomado: {s}"
    );
}
