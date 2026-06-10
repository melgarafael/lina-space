//! F1-0-6 — `lina handoff "@X" "tarefa" [--context arquivo] [--ref plan:ID] [--await]`:
//! handoff ESTRUTURADO usando o contrato `lina/msg@2` (F1-0-5). A doutrina já o promete
//! (CLAUDE.md bloco 4); este teste fecha a divergência doutrina×binário no verbo.
//! Roda o BINÁRIO real (mesmo padrão de `list_cli.rs`/`broker_cli.rs`).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempWs {
    home: PathBuf,
    cwd: PathBuf,
}

impl TempWs {
    /// `home` = LINA_HOME (mailbox compartilhada) · `cwd` = diretório do terminal com
    /// `.lina/bootstrap.json` (de onde o verbo lê o `from` e a AUTONOMIA).
    fn new(tag: &str, autonomy: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("lina-handoff-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("shared");
        let cwd = root.join("terminal-a");
        std::fs::create_dir_all(&home).expect("criar LINA_HOME");
        std::fs::create_dir_all(cwd.join(".lina")).expect("criar cwd/.lina");
        let bootstrap = format!(
            r#"{{"terminal_name":"Terminal A","roster":["Terminal A","QA"],"vault_path":"/tmp/none","autonomy":"{autonomy}"}}"#
        );
        std::fs::write(cwd.join(".lina/bootstrap.json"), bootstrap).expect("bootstrap.json");
        Self { home, cwd }
    }
}

impl Drop for TempWs {
    fn drop(&mut self) {
        if let Some(root) = self.home.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn run_handoff(ws: &TempWs, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut argv = vec!["handoff"];
    argv.extend_from_slice(args);
    Command::new(exe)
        .args(&argv)
        .current_dir(&ws.cwd)
        .env("LINA_HOME", &ws.home)
        // ADR 0026: a identidade testada aqui é a da FICHA — isola do env de spawn que um
        // terminal do Lina injeta (rodar `cargo test` dentro do Espaço não pode mudar o `from`).
        .env_remove("LINA_NODE_NAME")
        .output()
        .expect("executar o binário lina")
}

/// JSONs depositados no outbox POR-NÓ do remetente (o canal autenticado pela origem).
fn outbox_msgs(ws: &TempWs) -> Vec<serde_json::Value> {
    let dir = ws.home.join("outbox").join("Terminal A");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    rd.filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let raw = std::fs::read_to_string(&p).expect("ler msg do outbox");
            serde_json::from_str(&raw).expect("msg do outbox é JSON")
        })
        .collect()
}

fn outbox_is_empty(ws: &TempWs) -> bool {
    let dir = ws.home.join("outbox");
    !dir.exists()
        || std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(Result::ok).all(|e| {
                    std::fs::read_dir(e.path())
                        .map(|sub| sub.count() == 0)
                        .unwrap_or(true)
                })
            })
            .unwrap_or(true)
}

/// O verbo existe e deposita um envelope `lina/msg@2` com `intent=handoff` e o bloco
/// `contract` COMPLETO (campos que o `validate_envelope_v2` do router exige) — nunca o
/// contorno `ask --intent handoff` sem contrato visto em campo (seq 16242).
#[test]
fn handoff_deposits_v2_envelope_with_full_contract() {
    let ws = TempWs::new("v2", "assisted");
    std::fs::write(ws.cwd.join("plano.md"), "## Plano\n- testar tudo\n").expect("contexto");

    let out = run_handoff(
        &ws,
        &["@QA", "rode os testes do módulo X", "--context", "plano.md"],
    );
    assert!(
        out.status.success(),
        "handoff deveria enfileirar; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let msgs = outbox_msgs(&ws);
    assert_eq!(msgs.len(), 1, "exatamente 1 envelope no outbox por-nó");
    let m = &msgs[0];
    assert_eq!(m["schema"], "lina/msg@2", "handoff nasce no schema @2");
    assert_eq!(m["intent"], "handoff");
    assert_eq!(m["from"], "Terminal A");
    assert_eq!(m["to"], "@QA");

    // O payload carrega a tarefa E o conteúdo do --context (anexado, não referenciado).
    let payload = m["payload"].as_str().unwrap_or_default();
    assert!(payload.contains("rode os testes do módulo X"));
    assert!(
        payload.contains("- testar tudo"),
        "conteúdo do arquivo de contexto anexado ao payload"
    );

    // Contrato completo — exatamente o que o router @2 valida campo a campo.
    let c = &m["contract"];
    assert!(
        !c["input_schema"].as_str().unwrap_or_default().is_empty(),
        "input_schema presente"
    );
    assert!(
        !c["output_schema"].as_str().unwrap_or_default().is_empty(),
        "output_schema presente"
    );
    assert!(
        c["timeout_sec"].as_u64().unwrap_or(0) >= 1,
        "timeout_sec >= 1"
    );
    assert!(
        !c["retry_policy"].as_str().unwrap_or_default().is_empty(),
        "retry_policy presente"
    );
}

/// `--ref plan:ID` viaja no campo `ref_id` (cadeia de responsabilidade com o plano) e
/// `--timeout-sec` calibra o contrato.
#[test]
fn handoff_carries_plan_ref_and_custom_timeout() {
    let ws = TempWs::new("ref", "assisted");
    let out = run_handoff(
        &ws,
        &[
            "@QA",
            "valide o item T4",
            "--ref",
            "plan:T4",
            "--timeout-sec",
            "120",
        ],
    );
    assert!(out.status.success());
    let msgs = outbox_msgs(&ws);
    assert_eq!(msgs.len(), 1);
    // No JSON do envelope a chave é `ref` (serde rename — contrato do envelope §3.4).
    assert_eq!(msgs[0]["ref"], "plan:T4");
    assert_eq!(msgs[0]["contract"]["timeout_sec"], 120);
}

/// **Critério 4 da F1-0-6:** em autonomia `manual`, handoff é DELEGAÇÃO e o próprio
/// comando recusa LOCALMENTE (doutrina bloco 5: "garantido pelo próprio comando `lina`,
/// não por hook") — erro legível, NADA enfileirado (sem rotear).
#[test]
fn handoff_blocked_locally_under_manual_autonomy() {
    let ws = TempWs::new("manual", "manual");
    let out = run_handoff(&ws, &["@QA", "tarefa qualquer"]);
    assert!(
        !out.status.success(),
        "manual deve recusar handoff localmente"
    );
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("manual") && (err.contains("bloquea") || err.contains("delega")),
        "erro deve explicar o bloqueio por autonomia manual: {err}"
    );
    assert!(outbox_is_empty(&ws), "nada pode ter sido enfileirado");
}

/// Sem arquivo de contexto legível, o handoff falha VISÍVEL (nunca enfileira um handoff
/// prometendo um contexto que não anexou — fidelidade > contorno).
#[test]
fn handoff_with_missing_context_file_fails_loud() {
    let ws = TempWs::new("noctx", "assisted");
    let out = run_handoff(&ws, &["@QA", "tarefa", "--context", "nao-existe.md"]);
    assert!(!out.status.success());
    assert!(outbox_is_empty(&ws));
}
