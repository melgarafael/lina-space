//! W3-6c AC-6.5 (lado-agente, headless) — o verbo `lina do <action>` REGISTRA uma ação custodiada
//! e prova que o agente, sozinho, não tem caminho de execução: apenda
//! `ActionGated{class:"gated-hard-external", decision:"ask"}` + `BrokerDenied{reason:"unconfirmed"}`,
//! e NUNCA produz `BrokerExecuted`. (A execução COM o segredo do cofre, após gate humano, é o
//! `broker::run_custody`, provado headless com MockStore nos testes de unidade de `lina-core`.)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use lina_core::EventStore;

struct TempHome(PathBuf);

impl TempHome {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "lina-w36c-cli-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&p).expect("criar LINA_HOME temporário");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn events_dir(&self) -> PathBuf {
        self.0.join("events")
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_do(home: &TempHome, action_args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut argv = vec!["do"];
    argv.extend_from_slice(action_args);
    Command::new(exe)
        .args(&argv)
        .env("LINA_HOME", home.path())
        .output()
        .expect("executar o binário lina")
}

fn event_kinds(home: &TempHome) -> Vec<(String, serde_json::Value)> {
    if !home.events_dir().join("lina.db").exists() {
        return Vec::new();
    }
    let store = EventStore::open(home.events_dir()).expect("abrir event store");
    store
        .events()
        .expect("ler eventos")
        .into_iter()
        .map(|r| (r.kind, r.payload))
        .collect()
}

/// AC-6.5 (1) lado-agente: `lina do deploy --env prod` SEM confirmação → registra e bloqueia.
/// ActionGated{gated-hard-external, ask} + BrokerDenied{unconfirmed}; a ação real NÃO roda.
#[test]
fn lina_do_deploy_registers_and_blocks_without_executing() {
    let home = TempHome::new("deploy");
    let out = run_do(&home, &["deploy", "--env", "prod"]);
    assert!(
        out.status.success(),
        "lina do deveria registrar com sucesso; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let events = event_kinds(&home);
    let gated = events
        .iter()
        .find(|(k, _)| k == "ActionGated")
        .expect("ActionGated apendado");
    assert_eq!(gated.1["class"], "gated-hard-external");
    assert_eq!(gated.1["decision"], "ask");

    assert!(
        events
            .iter()
            .any(|(k, p)| k == "BrokerDenied" && p["reason"] == "unconfirmed"),
        "deve apendar BrokerDenied{{unconfirmed}} — o agente não tem confirmação"
    );
    assert!(
        !events.iter().any(|(k, _)| k == "BrokerExecuted"),
        "o agente NUNCA pode produzir BrokerExecuted (não tem o segredo)"
    );
}

/// Ação não custodiada via `lina do` é recusada (exit != 0) e não apenda evento.
#[test]
fn lina_do_unknown_action_is_rejected() {
    let home = TempHome::new("unknown");
    let out = run_do(&home, &["destruir-tudo"]);
    assert!(
        !out.status.success(),
        "ação não custodiada deve falhar (exit != 0)"
    );
    assert!(
        event_kinds(&home).is_empty(),
        "ação desconhecida não deve apendar evento"
    );
}
