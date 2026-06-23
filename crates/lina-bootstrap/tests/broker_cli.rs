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
        // ADR 0026: estes testes provam o fallback `agente-desconhecido` (sem ficha) — isola
        // do env de spawn de um terminal do Lina (que agora também identifica o nó).
        .env_remove("LINA_NODE_NAME")
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

/// Fio 1+2 (lado-bin): `lina do` deposita o pedido na FILA DE BROKER DEDICADA por-nó
/// (`<LINA_HOME>/broker/outbox/<no>/`), NÃO no outbox A2A (`<LINA_HOME>/outbox/`). Sem
/// `.lina/bootstrap.json` no cwd, o requester cai no fallback `agente-desconhecido` (subdir válido).
#[test]
fn lina_do_enqueues_request_in_dedicated_broker_queue_per_node() {
    let home = TempHome::new("brokerq");
    let out = run_do(&home, &["deploy", "--env", "prod"]);
    assert!(
        out.status.success(),
        "lina do deveria registrar com sucesso"
    );

    let broker_outbox = home.path().join("broker").join("outbox");
    let node_dir = broker_outbox.join("agente-desconhecido");
    let jsons: Vec<PathBuf> = std::fs::read_dir(&node_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        jsons.len(),
        1,
        "o pedido custodiado deve estar no subdir por-nó da fila de broker ({})",
        node_dir.display()
    );

    // E NÃO pode ter vazado para o outbox A2A (senão o Router o trataria como entrega).
    let a2a_outbox = home.path().join("outbox");
    assert!(
        !a2a_outbox.exists()
            || std::fs::read_dir(&a2a_outbox)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(true),
        "o pedido de broker NAO pode aparecer no outbox A2A"
    );
}

fn run_resume(home: &TempHome, extra: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut argv = vec!["resume"];
    argv.extend_from_slice(extra);
    Command::new(exe)
        .args(&argv)
        .env("LINA_HOME", home.path())
        // ADR 0026: idem `run_do` — o fallback sem-ficha é o que está sob teste.
        .env_remove("LINA_NODE_NAME")
        .output()
        .expect("executar o binário lina")
}

/// **ROUND 5 hole 1 (lado-agente):** `lina resume` (mesmo com `--confirm`) NÃO des-pausa por
/// construção — o caminho do agente NEM ABRE o event store; só REGISTRA um `resume.request` na fila
/// de broker por-nó. Logo é IMPOSSÍVEL ele apendar `CostCeilingResumed`. (A transição paused→retomada
/// pelo gate humano é provada in-process no teste do app `broker_pump_resume_requires_human_gate_to_unpause`.)
#[test]
fn lina_resume_does_not_unpause_only_requests() {
    let home = TempHome::new("resume");

    let out = run_resume(&home, &["--confirm"]);
    assert!(
        out.status.success(),
        "lina resume deveria registrar o pedido com sucesso; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    // O contrato do agente: ele PEDE confirmação humana, NÃO des-pausa.
    let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(
        stdout.contains("confirmacao") || stdout.contains("nao des-pausa"),
        "o agente deixa claro que só PEDE (gate humano); stdout={stdout}"
    );

    // PROVA FORTE: o caminho de resume do agente NÃO toca o event store (não cria events_dir) → não há
    // como ele apendar CostCeilingResumed. (Robusto a mudanças internas de persistência do core.)
    assert!(
        !home.events_dir().exists(),
        "lina resume NAO pode abrir/escrever o event store (so enfileira o pedido)"
    );

    // E o pedido foi registrado na fila de broker por-nó (origem autenticada no drain do app).
    let node_dir = home
        .path()
        .join("broker")
        .join("outbox")
        .join("agente-desconhecido");
    let count = std::fs::read_dir(&node_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(
        count,
        1,
        "resume.request na fila de broker por-nó ({})",
        node_dir.display()
    );
}

/// Um único token não-builtin via `lina do` é tratado como canal SEM ação → recusado (exit != 0) e
/// não apenda evento (o store nem é aberto antes da validação dos argumentos).
#[test]
fn lina_do_unknown_action_is_rejected() {
    let home = TempHome::new("unknown");
    let out = run_do(&home, &["destruir-tudo"]);
    assert!(
        !out.status.success(),
        "ação não custodiada / canal sem ação deve falhar (exit != 0)"
    );
    assert!(
        event_kinds(&home).is_empty(),
        "argumento inválido não deve apendar evento"
    );
}

/// **Gate (b) F4-0-3 lado-agente:** `lina do <canal-stub> send` SEM confirmação → registra e bloqueia.
/// ActionGated{gated-hard-external, ask} + BrokerDenied{unconfirmed}; o agente NUNCA produz
/// BrokerExecuted (não tem o token do escopo `channel:<nome>` nem confirmação humana). O `cmd` do
/// ActionGated identifica o canal+ação (auditoria do gate humano vê QUAL efeito de QUAL canal).
#[test]
fn lina_do_channel_action_registers_and_blocks_without_executing() {
    let home = TempHome::new("channel");
    let out = run_do(&home, &["slack-stub", "send", "ola-mundo"]);
    assert!(
        out.status.success(),
        "lina do <canal> <ação> deveria registrar com sucesso; stderr={}",
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
        gated.1["cmd"]
            .as_str()
            .unwrap_or_default()
            .contains("slack-stub"),
        "o cmd do gate deve identificar o canal: {:?}",
        gated.1["cmd"]
    );

    assert!(
        events
            .iter()
            .any(|(k, p)| k == "BrokerDenied" && p["reason"] == "unconfirmed"),
        "efeito de canal sem confirmação deve apendar BrokerDenied{{unconfirmed}}"
    );
    assert!(
        !events.iter().any(|(k, _)| k == "BrokerExecuted"),
        "o agente NUNCA pode produzir BrokerExecuted (não tem o segredo do canal)"
    );
}

/// O pedido de canal entra na FILA DE BROKER por-nó com ref `do:channel:<canal>:<ação>` — o
/// supervisor o reconhece para montar `BrokerRequest::for_channel` (escopo `channel:<canal>` derivado
/// SERVER-SIDE; o agente não dita o escopo do segredo). E NÃO vaza para o outbox A2A.
#[test]
fn lina_do_channel_enqueues_with_channel_ref() {
    let home = TempHome::new("channelq");
    let out = run_do(&home, &["slack-stub", "send", "oi"]);
    assert!(out.status.success(), "registro do pedido de canal");

    let node_dir = home
        .path()
        .join("broker")
        .join("outbox")
        .join("agente-desconhecido");
    let jsons: Vec<PathBuf> = std::fs::read_dir(&node_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(jsons.len(), 1, "1 pedido de canal na fila de broker por-nó");

    let raw = std::fs::read_to_string(&jsons[0]).expect("ler o pedido enfileirado");
    assert!(
        raw.contains("do:channel:slack-stub:send"),
        "o ref deve transportar canal+ação para o supervisor derivar o escopo: {raw}"
    );

    // Não pode ter vazado para o outbox A2A (senão o Router o trataria como entrega a um colega).
    let a2a_outbox = home.path().join("outbox");
    assert!(
        !a2a_outbox.exists()
            || std::fs::read_dir(&a2a_outbox)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(true),
        "o pedido de broker NAO pode aparecer no outbox A2A"
    );
}

/// `lina do <canal>` SEM a ação é recusado (exit != 0) e não apenda evento — a forma de canal exige
/// `<canal> <ação>`, e a validação dos argumentos precede a abertura do store.
#[test]
fn lina_do_channel_without_action_is_rejected() {
    let home = TempHome::new("channel-noaction");
    let out = run_do(&home, &["slack-stub"]);
    assert!(
        !out.status.success(),
        "canal sem ação deve falhar (exit != 0)"
    );
    assert!(
        event_kinds(&home).is_empty(),
        "canal sem ação não deve apendar evento"
    );
}
