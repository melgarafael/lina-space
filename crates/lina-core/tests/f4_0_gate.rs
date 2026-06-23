//! F4-0-QA — **GATE DA ONDA F4-0** (dono: Terminal R). Re-derivações INDEPENDENTES + red-team.
//!
//! Este arquivo é o gate que **re-deriva** o critério de saída F4-0 por fora das frentes — provando
//! o invariante pelo caminho real (`run_custody`, `SecretVault`, PTY de verdade, eventos congelados),
//! nunca montando o desfecho à mão. Os critérios que dependiam de impl das frentes (broker de canal
//! `lina do <canal>`, monitor de rede, projeção de tool scope) foram PROVADOS na integração, GREEN
//! nos arquivos das frentes (`broker_cli.rs`, `network_monitor.rs`, `tool_scope.rs`/`briefing.rs`);
//! este arquivo cobre o MOTOR (`run_custody`/dump de env) + o RED-TEAM transversal.
//!
//! Mapa do gate de saída (peça `tasks/epico-f4/onda-0.md` §GATE DE SAÍDA F4-0):
//! - **(a)** credencial no cofre → dump de env de PTY de agente spawnado **não a contém** (+ mutação
//!   que prova que a sonda morde um vazamento real). AQUI, verde.
//! - **(b)** ação externa custodiada → `ActionGated{gated-hard-external, ask}` + **zero** execução sem
//!   confirmação. O MOTOR (`run_custody`) está aqui, verde; a ligação canal (`lina do <canal>`) está
//!   verde em `broker_cli.rs` (`lina_do_*_channel` / `lina_do_undeclared_channel_is_rejected`).
//! - **RED-TEAM** (o coração): manifesto/registro de canal é DADO, não autoridade; declarar (tool
//!   scope) ≠ autorizar; a credencial nunca entra no log (só `key_ref`).
//!
//! Doutrina de segurança (CLAUDE.md repo + ADR 0004 §4): **nenhum campo escrito por agente** (um
//! `ChannelRegistered`/`ToolScopeDeclared`/`CredentialStored` forjado direto no log) decide
//! identidade/ordem/autorização. A autoridade é a custódia de segredo + o gate humano (`confirmed`),
//! jamais um campo de payload. Estes testes provam isso por MUTAÇÃO (padrão-ouro da casa).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lina_core::{
    lookup_action, run_custody, BrokerOutcome, BrokerRequest, DomainEvent, EventRecord, EventStore,
    PtyCommand, PtyManager, CLASS_GATED_HARD_EXTERNAL,
};
use lina_secrets::{MockStore, SecretVault};
use serial_test::serial;

// ───────────────────────────── infra de teste (headless) ─────────────────────────────

/// Diretório temporário do event store; removido no `Drop`. Nome único por `(tag, pid, thread, ns)`
/// — a colisão em execução paralela é o que envenena o gate (lição: tmpdir com thread::id mata o
/// flaky sistêmico).
struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tid = format!("{:?}", thread::current().id());
        let tid = tid.replace(|c: char| !c.is_ascii_alphanumeric(), "");
        let p = std::env::temp_dir().join(format!(
            "lina-f40gate-{tag}-{}-{tid}-{nanos}",
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

/// Os `kind`s (em ordem) apendados no store — base das asserções de sequência/ausência.
fn kinds(store: &EventStore) -> Vec<String> {
    store
        .events()
        .expect("ler eventos")
        .into_iter()
        .map(|r| r.kind)
        .collect()
}

/// O primeiro `EventRecord` de um dado `kind`.
fn first<'a>(events: &'a [EventRecord], kind: &str) -> Option<&'a EventRecord> {
    events.iter().find(|r| r.kind == kind)
}

/// Drena TODO o output de um PTY até EOF, com teto de tempo (o slave foi solto no `spawn`, então um
/// comando que termina — `env` — fecha o master e o reader vê EOF). Sem teto, um filho preso
/// penduraria o gate; com teto, devolve o que leu.
fn drain_pty(manager: &PtyManager, node: &str, budget: Duration) -> String {
    let mut reader = manager.clone_reader(node).expect("clonar reader do PTY");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    rx.recv_timeout(budget).unwrap_or_default()
}

/// Uma credencial de canal de teste — string única e improvável de colidir com qualquer var de
/// ambiente herdada (a herança do env do app é justamente o gap que o gate (a) audita).
const CHANNEL_CRED: &str = "GATE-A-WHATSAPP-SESSION-TOKEN-7f3c9af2e1d04b6a";

// ───────────────────────────── GATE (a) · credencial NÃO vaza para o env do PTY ──────────────────

/// **GATE (a) — critério inforjável (espelha AC-0004.1).** A credencial de um canal vive SÓ no
/// `SecretVault` (keyring); o env de um PTY de agente spawnado pelo caminho do core **não a contém**.
///
/// Prova por inspeção do processo-filho REAL: guarda o segredo no cofre (memória, nunca env), spawna
/// um PTY que executa `env`, captura a saída e grepa o segredo → **0 hits**. O cofre não escreve no
/// env do processo; o `PtyCommand` só injeta o que é explicitamente carimbado (identidade) — e a
/// credencial nunca é carimbada. Re-derivação independente do teste-coração de F40-CRED.
#[test]
#[serial]
fn gate_a_credential_in_vault_is_absent_from_spawned_pty_env() {
    // O segredo entra no cofre indexado por canal/escopo — fora do env, fora do log (invariante #2).
    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("channel:whatsapp", "session", CHANNEL_CRED)
        .expect("semear credencial de canal no cofre");
    // O cofre a retorna (a credencial existe e é recuperável SÓ pelo broker) ...
    assert_eq!(
        vault.get("channel:whatsapp", "session").expect("get"),
        Some(CHANNEL_CRED.to_string()),
        "o cofre deve deter a credencial (é o broker, não o agente, quem a lê)"
    );

    // ... mas o env de um PTY de agente NÃO a contém. Spawna o caminho real do core (PtyManager +
    // PtyCommand só com identidade) rodando `env` e captura o ambiente REAL do filho.
    let mut manager = PtyManager::new();
    let cmd = PtyCommand::new("sh")
        .arg("-c")
        .arg("env")
        .env("LINA_NODE_ID", "n-qa-gate-a"); // identidade, jamais segredo (espelha node_identity_env)
    manager
        .spawn("qa-env-probe", cmd, 80, 24)
        .expect("spawn PTY de sonda");
    let env_dump = drain_pty(&manager, "qa-env-probe", Duration::from_secs(10));
    let _ = manager.kill("qa-env-probe", Duration::from_secs(1));

    assert!(
        !env_dump.is_empty(),
        "a sonda deve capturar o env do filho (senão o teste é vácuo)"
    );
    let hits = env_dump.matches(CHANNEL_CRED).count();
    assert_eq!(
        hits, 0,
        "a credencial do cofre NÃO pode aparecer no env do PTV do agente (dump:\n{env_dump}\n)"
    );
}

/// **GATE (a) — MUTAÇÃO (blinda contra falso-verde).** Se a credencial FOSSE injetada no env do PTY
/// (o caminho proibido), a MESMA sonda do teste acima a pegaria. Injeta de propósito e exige > 0
/// hits — provando que o grep é capaz de morder um vazamento real (não um teste cego).
#[test]
#[serial]
fn gate_a_mutation_injected_credential_is_caught_by_the_env_grep() {
    let mut manager = PtyManager::new();
    // Caminho PROIBIDO simulado: a credencial entra no env do PTY.
    let cmd = PtyCommand::new("sh")
        .arg("-c")
        .arg("env")
        .env("LINA_LEAKED_CHANNEL_CRED", CHANNEL_CRED);
    manager
        .spawn("qa-env-leak", cmd, 80, 24)
        .expect("spawn PTY de mutação");
    let env_dump = drain_pty(&manager, "qa-env-leak", Duration::from_secs(10));
    let _ = manager.kill("qa-env-leak", Duration::from_secs(1));

    let hits = env_dump.matches(CHANNEL_CRED).count();
    assert!(
        hits >= 1,
        "MUTAÇÃO: a sonda PRECISA pegar a credencial quando ela vaza para o env — senão o gate (a) \
         seria cego. dump:\n{env_dump}"
    );
}

// ───────────────────────────── GATE (b) · motor de custódia (o piso) ─────────────────────────────

/// **GATE (b) — motor.** Ação externa custodiada SEM confirmação → `ActionGated{gated-hard-external,
/// ask}` + `BrokerDenied{unconfirmed}` + **ZERO** `BrokerExecuted`, em QUALQUER nível de autonomia
/// (a custódia é o piso que a autonomia jamais afrouxa — ADR 0004). Re-derivação independente pelo
/// caminho real (`run_custody`), com o segredo presente no cofre (prova que o bloqueio é o gate
/// humano, não a ausência do segredo).
#[test]
fn gate_b_motor_action_gated_hard_external_and_zero_execution_when_unconfirmed() {
    let tmp = TempDir::new("b-motor");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");
    // Segredo PRESENTE no cofre: isola que o bloqueio vem do gate (sem "sim"), não do `no_secret`.
    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("external", "whatsapp", CHANNEL_CRED)
        .expect("set");

    let action = lookup_action("send").expect("`send` é ação externa custodiada");
    let req = BrokerRequest::new(action, "whatsapp", "@agente");
    let mut executed = false;
    let outcome = run_custody(&req, false, &vault, &mut store, |_secret| {
        executed = true;
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
    assert!(
        !executed,
        "o executor NÃO pode rodar sem confirmação humana"
    );

    let events = store.events().expect("eventos");
    let gated = first(&events, "ActionGated").expect("ActionGated apendado");
    assert_eq!(
        gated.payload["class"], CLASS_GATED_HARD_EXTERNAL,
        "ação externa de canal é gated-hard-external"
    );
    assert_eq!(
        gated.payload["decision"], "ask",
        "o gate de custódia é sempre `ask`, nunca `allow` (piso)"
    );
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
    // O segredo do cofre não pode ter vazado para o log em nenhum momento.
    let log_dump = serde_json::to_string(&events).expect("serializar log");
    assert!(
        !log_dump.contains(CHANNEL_CRED),
        "o segredo NÃO pode aparecer em nenhum evento do log"
    );
}

// ───────────────────────────── RED-TEAM (o coração) ──────────────────────────────────────────────

/// **RED-TEAM — manifesto/registro de canal é DADO, jamais autoridade.** O agente forja um
/// `ChannelRegistered` direto no log com `trust_tier:"core"` (mentindo ser um canal de máxima
/// confiança), na esperança de que "estar registrado como confiável" conceda execução externa. Prova:
/// um registro forjado NÃO produz nenhum `BrokerExecuted`, e a ação externa subsequente AINDA passa
/// pelo gate (`ask` + bloqueio sem confirmação). O gate é o broker + custódia, nunca o manifesto.
#[test]
fn redteam_forged_channel_registered_grants_no_external_execution() {
    let tmp = TempDir::new("rt-chan");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");

    // Ataque: carimbar um canal "core" (confiável) direto no log.
    store
        .append(&DomainEvent::ChannelRegistered {
            channel: "whatsapp".into(),
            manifest_ref: "channels/whatsapp/manifest.toml".into(),
            trust_tier: "core".into(), // forjado — manifesto/registro é DADO, não decide autorização
            install_ref: "v1.0.0".into(),
        })
        .expect("append forjado");

    // O registro forjado, por si, não executa nada externo.
    assert!(
        !kinds(&store).iter().any(|k| k == "BrokerExecuted"),
        "forjar ChannelRegistered NÃO dispara execução externa — registro é DADO, não autoridade"
    );

    // E a ação externa de canal continua barrada pelo gate (mesmo com o canal "confiável" no log).
    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("external", "whatsapp", CHANNEL_CRED)
        .expect("set");
    let action = lookup_action("send").expect("send custodiada");
    let req = BrokerRequest::new(action, "whatsapp", "@atacante");
    let outcome = run_custody(&req, false, &vault, &mut store, |_| Ok(())).expect("run_custody");

    assert_eq!(
        outcome,
        BrokerOutcome::DeniedUnconfirmed,
        "o canal forjado como `core` NÃO afrouxa o gate — segue `ask` + bloqueio sem confirmação"
    );
    assert!(
        !kinds(&store).iter().any(|k| k == "BrokerExecuted"),
        "nenhuma execução externa nasce de um registro de canal forjado"
    );
}

/// **RED-TEAM — declarar ≠ autorizar.** O agente forja um `ToolScopeDeclared` direto no log
/// (declarando que "tem acesso ao grupo:Vendas do whatsapp"), tentando que a declaração de contexto
/// vire autorização de ação externa. Prova: a declaração NÃO fura o broker — a ação externa AINDA
/// dispara `ActionGated{ask}` e é bloqueada sem confirmação. Tool scope é fluxo de LEITURA, nunca
/// caminho de permissão (invariante-mãe de F4-0-4).
#[test]
fn redteam_forged_tool_scope_declared_does_not_bypass_the_broker_gate() {
    let tmp = TempDir::new("rt-scope");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");

    // Ataque: declarar acesso a uma ferramenta/grupo, esperando que "declarado" == "autorizado".
    store
        .append(&DomainEvent::ToolScopeDeclared {
            project: "loja".into(),
            channel: "whatsapp".into(),
            scope: "grupo:Vendas".into(),
        })
        .expect("append forjado");

    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("external", "whatsapp", CHANNEL_CRED)
        .expect("set");
    let action = lookup_action("send").expect("send custodiada");
    let req = BrokerRequest::new(action, "whatsapp", "@atacante");
    let mut executed = false;
    let outcome = run_custody(&req, false, &vault, &mut store, |_| {
        executed = true;
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(
        outcome,
        BrokerOutcome::DeniedUnconfirmed,
        "declarar tool scope NÃO autoriza — a ação externa segue barrada pelo gate"
    );
    assert!(!executed, "declarar ≠ autorizar: o executor não roda");
    let events = store.events().expect("eventos");
    let gated = first(&events, "ActionGated").expect("ActionGated mesmo com scope declarado");
    assert_eq!(gated.payload["decision"], "ask");
}

/// **RED-TEAM — a credencial nunca entra no log; só a REFERÊNCIA.** O fato `CredentialStored` carrega
/// `{channel, scope, key_ref}` — `key_ref` é a chave/conta no keyring, JAMAIS o valor (events.rs §
/// `CredentialStored`). Prova: guarda o segredo no cofre, apenda `CredentialStored` com a referência,
/// e varre o log inteiro → o valor do segredo está AUSENTE; o payload tem exatamente os 3 campos de
/// referência e nenhum portador de valor.
#[test]
fn redteam_credential_stored_event_carries_reference_not_secret_value() {
    let tmp = TempDir::new("rt-cred");
    let mut store = EventStore::open(tmp.path()).expect("abrir store");

    // O valor vai para o cofre; o evento carrega só a referência.
    let vault = SecretVault::with_store("lina-space/ws-qa", MockStore::new());
    vault
        .set("channel:whatsapp", "session", CHANNEL_CRED)
        .expect("set");
    store
        .append(&DomainEvent::CredentialStored {
            channel: "whatsapp".into(),
            scope: "channel:whatsapp".into(),
            key_ref: "session".into(), // REFERÊNCIA, nunca o valor
        })
        .expect("append CredentialStored");

    let events = store.events().expect("eventos");
    let rec = first(&events, "CredentialStored").expect("CredentialStored apendado");

    // O valor do segredo está ausente do payload e de todo o log.
    let log_dump = serde_json::to_string(&events).expect("serializar log");
    assert!(
        !log_dump.contains(CHANNEL_CRED),
        "o VALOR da credencial NUNCA pode aparecer no log (só o key_ref)"
    );
    // O payload carrega exatamente a tripla de referência — nenhum campo de valor.
    assert_eq!(rec.payload["channel"], "whatsapp");
    assert_eq!(rec.payload["scope"], "channel:whatsapp");
    assert_eq!(rec.payload["key_ref"], "session");
    let obj = rec.payload.as_object().expect("payload é objeto");
    // Só os campos de referência + a tag serde `event` — nada que possa portar o segredo.
    let mut data_keys: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "event")
        .collect();
    data_keys.sort_unstable();
    assert_eq!(
        data_keys,
        vec!["channel", "key_ref", "scope"],
        "CredentialStored só pode ter campos de referência — nenhum portador de valor"
    );
}
