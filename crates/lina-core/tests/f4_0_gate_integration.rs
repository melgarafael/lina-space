//! F4-0-QA — **GATE INTEGRADO** (dono: Terminal R). Prova os critérios de saída F4-0 contra a impl
//! REAL das frentes já integradas na `main` (broker de canal, tool scope, monitor de rede), pelo
//! caminho de produção — não mais `#[ignore]`. Complementa `f4_0_gate.rs` (re-derivações que não
//! dependiam das frentes) fechando o gate de saída inteiro por evidência.
//!
//! - **(b) broker de canal** — `BrokerRequest::for_channel` + `run_custody`: ação externa de canal →
//!   `ActionGated{gated-hard-external, ask}` + **zero** execução sem confirmação; e a CUSTÓDIA é
//!   inquebrável (canal/ação forjados + confirmado, mas sem segredo no escopo do canal → não executa).
//! - **(c) tool scope** — `declare_tool_scope`/`revoke_tool_scope` + `ToolScopeSet::replay`: declarar
//!   → a projeção contém; revogar → some no replay seguinte; default-deny (não declarado = invisível).
//! - **(d) monitor de rede** — o bin `network_monitor --json` (caminho real do gate): 0 canais
//!   conectados → `expected_outbound == 0`, mesmo com canais DECLARADOS (registrar ≠ conectar).
//!
//! Doutrina (CLAUDE.md repo + ADR 0004): nenhum campo escrito por agente (canal/manifesto/scope)
//! decide autorização — o gate é o broker + custódia de segredo + gate humano. Provado por mutação.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use lina_core::channel::{register_channel, ChannelManifest, ChannelRegistry, TrustTier};
use lina_core::tool_scope::{declare_tool_scope, revoke_tool_scope, ToolScopeSet};
use lina_core::{run_custody, BrokerOutcome, BrokerRequest, EventStore, CLASS_GATED_HARD_EXTERNAL};
use lina_secrets::{MockStore, SecretVault};

// ───────────────────────────── infra de teste (headless) ─────────────────────────────

/// Diretório temporário do event store; removido no `Drop`. Nome único por `(tag, pid, thread, ns)`.
struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tid = format!("{:?}", std::thread::current().id());
        let tid = tid.replace(|c: char| !c.is_ascii_alphanumeric(), "");
        let p = std::env::temp_dir().join(format!(
            "lina-f40gateint-{tag}-{}-{tid}-{nanos}",
            std::process::id()
        ));
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

const CHANNEL_CRED: &str = "GATE-INT-WHATSAPP-SESSION-TOKEN-b91e44c7a02d";

/// Os manifestos-stub que F40-CHAN entregou (canais "whatsapp"/"email"), parseados pelo caminho real.
const WHATSAPP_MANIFEST: &str = r#"
name = "whatsapp"
transport = "whatsapp_cloud_api"
auth = "api_key"
scopes = ["messages:send"]
[tools]
default_enabled = ["send_message"]
[install]
ref = "v0.1.0"
"#;

const EMAIL_MANIFEST: &str = r#"
name = "email"
transport = "smtp"
auth = "oauth2"
scopes = ["mail:send"]
[install]
ref = "9f1c2ae"
"#;

// ───────────────────────────── GATE (b) · broker de CANAL ────────────────────────────────────────

/// **GATE (b) — broker de canal pelo caminho real.** Uma ação externa de canal (`BrokerRequest::
/// for_channel` → escopo de segredo `channel:<nome>`) dispara `ActionGated{gated-hard-external, ask}`,
/// `BrokerDenied{unconfirmed}` e **ZERO** `BrokerExecuted` sem confirmação humana, em qualquer nível
/// de autonomia (a custódia é o piso — ADR 0004). O segredo do canal está PRESENTE no cofre (isola
/// que o bloqueio é o gate humano, não a ausência do segredo) e NUNCA vaza para o log.
#[test]
fn gate_b_channel_action_is_gated_hard_external_and_blocks_unconfirmed() {
    let tmp = TempDir::new("b-canal");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");
    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("channel:whatsapp", "session", CHANNEL_CRED)
        .expect("semear segredo do canal");

    // Caminho real: pedido brokerado de canal (escopo `channel:whatsapp`), sem confirmação.
    let req = BrokerRequest::for_channel("whatsapp", "send", "session", "@agente");
    let mut executed = false;
    let outcome = run_custody(&req, false, &vault, &mut store, |_secret| {
        executed = true;
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
    assert!(!executed, "ação externa de canal NÃO roda sem confirmação");

    let events = store.events().expect("eventos");
    let gated = events
        .iter()
        .find(|r| r.kind == "ActionGated")
        .expect("ActionGated apendado");
    assert_eq!(gated.payload["class"], CLASS_GATED_HARD_EXTERNAL);
    assert_eq!(gated.payload["decision"], "ask");
    assert!(
        events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "unconfirmed"),
        "sem confirmação → BrokerDenied{{unconfirmed}}"
    );
    assert!(
        !events.iter().any(|r| r.kind == "BrokerExecuted"),
        "NUNCA pode haver BrokerExecuted sem o gate humano"
    );
    let log_dump = serde_json::to_string(&events).expect("serializar log");
    assert!(
        !log_dump.contains(CHANNEL_CRED),
        "o segredo do canal NÃO pode aparecer no log"
    );
}

/// **GATE (b) — RED-TEAM: a custódia é inquebrável (canal/ação FORJADOS não furam).** Um agente forja
/// um pedido de canal inexistente e ação fora de qualquer manifesto, E confirma (`confirmed=true`),
/// mas o cofre NÃO tem segredo no escopo `channel:<forjado>`. Resultado: `DeniedNoSecret` + **zero**
/// execução. Sem o segredo no escopo do canal, não há caminho de execução — independente do que o
/// agente declare (broker.rs §`run_custody`: "mesmo com canal/ação forjados, sem o segredo nada
/// executa"). É o piso do ADR 0004: a custódia, não o nome que o agente diz.
#[test]
fn gate_b_redteam_forged_channel_without_secret_cannot_execute_even_confirmed() {
    let tmp = TempDir::new("b-forge");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");
    // Cofre VAZIO: o segredo do canal forjado não existe em lugar nenhum.
    let vault: SecretVault<MockStore> =
        SecretVault::with_store("lina-space/ws-qa", MockStore::new());

    // Ataque: canal inexistente + ação arbitrária + confirmação humana simulada.
    let req = BrokerRequest::for_channel("canal-pirata", "exfiltrar", "qualquer", "@atacante");
    let mut executed = false;
    let outcome = run_custody(&req, true, &vault, &mut store, |_secret| {
        executed = true;
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(
        outcome,
        BrokerOutcome::DeniedNoSecret,
        "sem segredo no escopo do canal, nem confirmado executa — a custódia é o piso"
    );
    assert!(!executed, "executor NÃO roda: não há segredo a injetar");
    let events = store.events().expect("eventos");
    assert!(
        events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "no_secret"),
        "deve apendar BrokerDenied{{no_secret}}"
    );
    assert!(!events.iter().any(|r| r.kind == "BrokerExecuted"));
}

// ───────────────────────────── GATE (c) · tool scope (declarar ≠ autorizar) ──────────────────────

/// **GATE (c) — declarar → aparece; revogar → some (replay).** Pelo caminho real: `declare_tool_scope`
/// emite `ToolScopeDeclared` → `ToolScopeSet::replay` contém a tripla; `revoke_tool_scope` emite
/// `ToolScopeRevoked` → no replay seguinte a tripla SOME (sem restart, sem variante de remoção extra).
/// Default-deny: uma tripla nunca declarada é invisível à projeção.
#[test]
fn gate_c_tool_scope_declared_appears_then_revoked_disappears_on_replay() {
    let tmp = TempDir::new("c-scope");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");

    // Default-deny: antes de declarar, a tripla NÃO está na projeção.
    let before = ToolScopeSet::replay(&store).expect("replay");
    assert!(
        !before.is_declared("loja", "whatsapp", "grupo:Vendas"),
        "default-deny: não-declarado é invisível"
    );

    // Declarar → a projeção passa a conter.
    declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declarar");
    let after_declare = ToolScopeSet::replay(&store).expect("replay");
    assert!(
        after_declare.is_declared("loja", "whatsapp", "grupo:Vendas"),
        "após declarar, a projeção contém a tripla"
    );

    // Revogar → some no replay seguinte (push de evento, sem restart).
    revoke_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("revogar");
    let after_revoke = ToolScopeSet::replay(&store).expect("replay");
    assert!(
        !after_revoke.is_declared("loja", "whatsapp", "grupo:Vendas"),
        "após revogar, o acesso some da projeção (replay)"
    );
}

/// **GATE (c) — RED-TEAM: declarar (de verdade) ≠ autorizar.** Mesmo com a ferramenta REALMENTE
/// declarada (não forjada) via `declare_tool_scope`, a ação externa de canal AINDA passa pelo gate
/// (`ask` + bloqueio sem confirmação). Tool scope é fluxo de LEITURA da IA, jamais caminho de
/// permissão — complementa o teste do scope forjado em `f4_0_gate.rs`.
#[test]
fn gate_c_redteam_real_declared_scope_does_not_authorize_external_action() {
    let tmp = TempDir::new("c-noauth");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");
    declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declarar");

    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("channel:whatsapp", "session", CHANNEL_CRED)
        .expect("set");
    let req = BrokerRequest::for_channel("whatsapp", "send", "session", "@agente");
    let outcome = run_custody(&req, false, &vault, &mut store, |_| Ok(())).expect("run_custody");

    assert_eq!(
        outcome,
        BrokerOutcome::DeniedUnconfirmed,
        "ter o tool scope declarado NÃO autoriza a ação externa — segue barrada pelo gate"
    );
    assert!(
        !store
            .events()
            .expect("eventos")
            .iter()
            .any(|r| r.kind == "BrokerExecuted"),
        "declarar ≠ autorizar: nenhuma execução"
    );
}

// ───────────────────────────── GATE (d) · monitor de rede (0 conexão por ausência) ───────────────

/// **GATE (d) — 0 canais conectados ⇒ 0 conexão de saída (consome o `--json` do `network_monitor`).**
/// Registra 2 canais REAIS (`register_channel` a partir dos manifestos), roda o bin de produção
/// `network_monitor <dir> --json` e parseia a afirmação: mesmo com canais DECLARADOS, `expected_
/// outbound == 0` (registrar ≠ conectar). É a prova local-first por AUSÊNCIA pelo caminho real — o
/// gate de QA consome a mesma saída `--json` que o auditor lê.
#[test]
fn gate_d_monitor_reports_zero_outbound_with_declared_but_unconnected_channels() {
    let tmp = TempDir::new("d-monitor");
    {
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        let whatsapp = ChannelManifest::parse(WHATSAPP_MANIFEST).expect("manifesto whatsapp");
        let email = ChannelManifest::parse(EMAIL_MANIFEST).expect("manifesto email");
        register_channel(
            &mut store,
            &whatsapp,
            "channels/whatsapp-stub/manifest.toml",
            TrustTier::parse("core"),
        )
        .expect("registrar whatsapp");
        register_channel(
            &mut store,
            &email,
            "channels/email-stub/manifest.toml",
            TrustTier::parse("comunidade"),
        )
        .expect("registrar email");
        // Cross-check pela projeção do core: 2 canais declarados, nenhum conectado (status Declared).
        let registry = ChannelRegistry::replay(&store).expect("replay registry");
        assert_eq!(registry.len(), 2, "2 canais registrados");
        // store dropado aqui → flush garantido antes de o monitor ler o arquivo.
    }

    // Caminho real do gate: o BIN de produção (já compilado por cargo) consumindo o log.
    let bin = env!("CARGO_BIN_EXE_network_monitor");
    let out = Command::new(bin)
        .arg(tmp.path())
        .arg("--json")
        .output()
        .expect("rodar network_monitor");
    assert!(
        out.status.success(),
        "network_monitor deve sair 0 (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("saída --json do monitor é JSON");

    assert_eq!(report["channels_declared"], 2, "2 canais declarados no log");
    assert_eq!(
        report["channels_connected"], 0,
        "registrar ≠ conectar: 0 conectados"
    );
    assert_eq!(
        report["expected_outbound"], 0,
        "0 conectados ⇒ 0 conexão de saída atribuível ao Lina (gate d)"
    );
}

/// **GATE (d) — caso vazio: 0 canais, 0 saída (honestidade do número).** Um log sem canais (só ruído)
/// → o monitor afirma 0 declarados e 0 saída — nunca panica nem inventa número.
#[test]
fn gate_d_monitor_reports_zero_when_no_channels_at_all() {
    let tmp = TempDir::new("d-empty");
    {
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        // Só ruído alheio (um tool scope, NÃO um canal) — garante que o log.jsonl existe sem
        // declarar canal algum.
        declare_tool_scope(&mut store, "loja", "interno", "grupo:X").expect("append ruído");
    }

    let bin = env!("CARGO_BIN_EXE_network_monitor");
    let out = Command::new(bin)
        .arg(tmp.path())
        .arg("--json")
        .output()
        .expect("rodar network_monitor");
    assert!(out.status.success(), "monitor exit 0");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json do monitor");
    assert_eq!(report["channels_declared"], 0);
    assert_eq!(report["expected_outbound"], 0);
}
