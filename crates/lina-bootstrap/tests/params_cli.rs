//! F3-0-5 — `lina params show` (parte SÓ-LEITURA): projeta os parâmetros de orquestração do event
//! log (espelho `log.jsonl`, NUNCA o SQLite — doutrina do verbo read-only) e lista os com override +
//! a camada de ORIGEM, com a precedência do core (`terminal` vence `workspace`). Roda o BINÁRIO real
//! (padrão de `space_state_cli.rs`/`retro_cli.rs`). `set`/`reset` entram quando o supervisor
//! (`handle_params`) integrar — aqui prova-se a projeção, que só depende de `from_records`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempWs {
    /// `home` = LINA_HOME (espelho do log em `events/log.jsonl`).
    home: PathBuf,
    /// `cwd` = diretório do terminal com `.lina/bootstrap.json`.
    cwd: PathBuf,
}

impl TempWs {
    fn new(tag: &str) -> Self {
        Self::with_autonomy(tag, "assisted")
    }

    fn with_autonomy(tag: &str, autonomy: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("lina-f305-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("shared");
        let cwd = root.join("terminal-i");
        std::fs::create_dir_all(home.join("events")).expect("criar LINA_HOME/events");
        std::fs::create_dir_all(cwd.join(".lina")).expect("criar cwd/.lina");
        let bootstrap = format!(
            r#"{{"terminal_name":"Terminal I","roster":["Terminal I"],"vault_path":"/tmp/none","autonomy":"{autonomy}"}}"#
        );
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

fn run(ws: &TempWs, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    Command::new(exe)
        .args(args)
        .current_dir(&ws.cwd)
        .env("LINA_HOME", &ws.home)
        .env_remove("LINA_NODE_NAME")
        .output()
        .expect("executar o binário lina")
}

/// Uma linha de `SystemParamsChanged` no formato verbatim do espelho. O `from_records` do core lê
/// `payload.{scope,key,new}`; `by`='human' é carimbo server-side do supervisor (aqui só compõe o
/// registro de teste, não autoridade).
fn spc(seq: u64, scope: &str, key: &str, new: &str) -> String {
    format!(
        r#"{{"seq":{seq},"ts":{seq},"kind":"SystemParamsChanged","version":1,"payload":{{"event":"SystemParamsChanged","key":"{key}","scope":"{scope}","new":"{new}","target":null,"old":"","by":"human"}}}}"#
    )
}

/// `show` lista um parâmetro com override e a camada que o aplicou.
#[test]
fn show_lista_override_com_origem() {
    let ws = TempWs::new("show-override");
    ws.seed_log(&[&spc(1, "workspace", "fanout_gate", "8")]);

    let out = run(&ws, &["params", "show"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "show sai com sucesso; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("fanout_gate"),
        "lista o parâmetro com override: {stdout}"
    );
    assert!(stdout.contains('8'), "mostra o valor efetivo: {stdout}");
    assert!(
        stdout.contains("workspace"),
        "mostra a camada de origem: {stdout}"
    );
}

/// Precedência do core: `terminal` vence `workspace` (o último evento da camada mais alta é o efetivo).
#[test]
fn show_precedencia_terminal_vence_workspace() {
    let ws = TempWs::new("show-prec");
    ws.seed_log(&[
        &spc(1, "workspace", "fanout_gate", "8"),
        &spc(2, "terminal", "fanout_gate", "12"),
    ]);

    let out = run(&ws, &["params", "show"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        stdout.contains("fanout_gate = 12"),
        "efetivo é o de terminal (precedência mais alta): {stdout}"
    );
    assert!(stdout.contains("terminal"), "origem é terminal: {stdout}");
}

/// Sem eventos no log: projeção honesta de tudo no default (nada de tela em branco).
#[test]
fn show_sem_eventos_e_tudo_default() {
    let ws = TempWs::new("show-empty");
    ws.seed_log(&[]);

    let out = run(&ws, &["params", "show"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        stdout.to_lowercase().contains("default"),
        "sem evento = tudo default: {stdout}"
    );
}

/// Envelopes que o bin depositou no outbox por-nó do remetente (`Terminal I`), desserializados.
fn enqueued(ws: &TempWs) -> Vec<serde_json::Value> {
    let dir = ws.home.join("outbox").join("Terminal I");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    rd.filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect()
}

/// `set` enfileira o envelope do contrato (b): intent `params.set` + payload {key,scope,value}. O bin
/// NÃO carimba `by` (server-side no supervisor) nem aplica — só deposita.
#[test]
fn set_enfileira_envelope_do_contrato_b() {
    let ws = TempWs::new("set-enq");
    let out = run(
        &ws,
        &["params", "set", "fanout_gate", "8", "--scope", "workspace"],
    );
    assert!(
        out.status.success(),
        "set enfileira; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envs = enqueued(&ws);
    assert_eq!(envs.len(), 1, "exatamente 1 envelope enfileirado");
    assert_eq!(
        envs[0]["intent"], "params.set",
        "intent do contrato: {}",
        envs[0]
    );
    let payload: serde_json::Value =
        serde_json::from_str(envs[0]["payload"].as_str().expect("payload é string"))
            .expect("payload é JSON");
    assert_eq!(payload["key"], "fanout_gate");
    assert_eq!(payload["scope"], "workspace");
    assert_eq!(payload["value"], "8");
}

/// `reset` enfileira `params.reset` com `value` vazio (no replay vira None → default).
#[test]
fn reset_enfileira_value_vazio() {
    let ws = TempWs::new("reset-enq");
    let out = run(
        &ws,
        &["params", "reset", "fanout_gate", "--scope", "workspace"],
    );
    assert!(out.status.success());
    let envs = enqueued(&ws);
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0]["intent"], "params.reset");
    let payload: serde_json::Value =
        serde_json::from_str(envs[0]["payload"].as_str().unwrap()).unwrap();
    assert_eq!(payload["value"], "", "reset = value vazio");
}

/// Em autonomia `manual` o próprio comando recusa a mutação (o agente PROPÕE ao humano) — e NADA
/// enfileira (o gate é antes do enqueue, espelha `lina spawn`).
#[test]
fn set_em_manual_recusa_e_nao_enfileira() {
    let ws = TempWs::with_autonomy("set-manual", "manual");
    let out = run(
        &ws,
        &["params", "set", "fanout_gate", "8", "--scope", "workspace"],
    );
    assert!(!out.status.success(), "manual recusa a mutacao");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .to_uppercase()
            .contains("MANUAL"),
        "explica o gate manual"
    );
    assert!(enqueued(&ws).is_empty(), "nada enfileirado em manual");
}

/// Escopo fora do enum é recusado na FORMA (antes de enfileirar), nomeando o escopo inválido.
#[test]
fn set_scope_invalido_recusa() {
    let ws = TempWs::new("set-badscope");
    let out = run(
        &ws,
        &["params", "set", "fanout_gate", "8", "--scope", "galaxy"],
    );
    assert!(!out.status.success(), "scope fora do enum recusa");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("galaxy"),
        "nomeia o escopo invalido"
    );
    assert!(enqueued(&ws).is_empty(), "nada enfileirado");
}

/// `effort @T <nível>` enfileira o envelope `effort.assign` com payload {target, effort} (contrato
/// aprovado). O bin NÃO carimba by/origin — server-side no `handle_effort`.
#[test]
fn effort_enfileira_envelope_do_contrato() {
    let ws = TempWs::new("effort-enq");
    let out = run(&ws, &["effort", "@QA", "high"]);
    assert!(
        out.status.success(),
        "effort enfileira; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envs = enqueued(&ws);
    assert_eq!(envs.len(), 1, "exatamente 1 envelope enfileirado");
    assert_eq!(envs[0]["intent"], "effort.assign");
    let payload: serde_json::Value =
        serde_json::from_str(envs[0]["payload"].as_str().expect("payload é string"))
            .expect("payload é JSON");
    assert_eq!(payload["target"], "@QA");
    assert_eq!(payload["effort"], "high");
}

/// `effort` em autonomia `manual` recusa antes de enfileirar (o agente PROPÕE ao humano).
#[test]
fn effort_em_manual_recusa_e_nao_enfileira() {
    let ws = TempWs::with_autonomy("effort-manual", "manual");
    let out = run(&ws, &["effort", "@QA", "high"]);
    assert!(!out.status.success(), "manual recusa a atribuicao");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .to_uppercase()
            .contains("MANUAL"),
        "explica o gate manual"
    );
    assert!(enqueued(&ws).is_empty(), "nada enfileirado em manual");
}

/// Nível fora do contrato neutro é recusado na FORMA (antes de enfileirar), nomeando o nível.
#[test]
fn effort_nivel_invalido_recusa() {
    let ws = TempWs::new("effort-badlevel");
    let out = run(&ws, &["effort", "@QA", "turbo"]);
    assert!(!out.status.success(), "nivel fora do contrato recusa");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("turbo"),
        "nomeia o nivel invalido"
    );
    assert!(enqueued(&ws).is_empty(), "nada enfileirado");
}
