//! W4-2 — `lina list [--json]` lista os agentes do workspace (nome · papel · status) lendo o
//! `agents.json` que o supervisor (app) escreve. É a VERIFICAÇÃO do M2 "Novo Agente": o agente criado
//! pelo nome aparece aqui com o PAPEL derivado (ex.: `reviewer`). Roda o BINÁRIO real.

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
            std::env::temp_dir().join(format!("lina-list-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).expect("criar LINA_HOME temporário");
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

fn run_list(home: &TempHome, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut argv = vec!["list"];
    argv.extend_from_slice(args);
    Command::new(exe)
        .args(&argv)
        .env("LINA_HOME", home.path())
        .output()
        .expect("executar o binário lina")
}

/// `lina list --json` emite o roster como JSON, com o PAPEL de cada agente (verificação do M2).
#[test]
fn lina_list_json_shows_agents_with_role() {
    let home = TempHome::new("json");
    // O supervisor (app) escreve `agents.json`; aqui simulamos o roster com um agente reviewer.
    let agents = r#"[{"name":"Revisor","role":"reviewer","status":"Running"},{"name":"Terminal A","role":"terminal","status":"Idle"}]"#;
    std::fs::write(home.path().join("agents.json"), agents).expect("escrever agents.json");

    let out = run_list(&home, &["--json"]);
    assert!(
        out.status.success(),
        "lina list --json deve ter sucesso; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("saída é JSON válido");
    let arr = parsed.as_array().expect("o roster é um array JSON");
    assert!(
        arr.iter()
            .any(|a| a["name"] == "Revisor" && a["role"] == "reviewer"),
        "o agente Revisor com papel reviewer aparece no JSON: {stdout}"
    );
}

/// `lina list` (sem --json) imprime forma legível; roster vazio não é erro.
#[test]
fn lina_list_plain_and_empty() {
    let home = TempHome::new("plain");
    // Sem agents.json → roster vazio, sucesso (nunca erro).
    let out_empty = run_list(&home, &[]);
    assert!(out_empty.status.success(), "roster vazio é sucesso");
    assert!(String::from_utf8_lossy(&out_empty.stdout).contains("nenhum agente"));

    // Com um agente, a forma legível mostra nome e papel.
    std::fs::write(
        home.path().join("agents.json"),
        r#"[{"name":"Revisor","role":"reviewer","status":"Running"}]"#,
    )
    .expect("escrever agents.json");
    let out = run_list(&home, &[]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Revisor") && stdout.contains("reviewer"));
}
