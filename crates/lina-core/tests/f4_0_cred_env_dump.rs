//! **F4-0-2 · gate (a) — o critério INFORJÁVEL da onda F4-0 (ADR 0004 · espelha AC-0004.1).**
//!
//! A fronteira inegociável: a credencial de um canal vive no keyring (`lina-secrets`), **NUNCA no
//! env do PTY de um agente**. Sem o segredo no env, o agente não consegue disparar a ação externa
//! sozinho — por construção (custódia como gate duro real, ADR 0004).
//!
//! Este teste PROVA isso por inspeção do processo-FILHO real: guarda uma credencial no cofre, SPAWNA
//! um PTY de agente do MESMO jeito do app (`PtyCommand`/`PtyManager`), dumpa o env do filho (via
//! `/usr/bin/env`) e grepa o valor da credencial → **0 hits**. A prova-por-MUTAÇÃO (padrão-ouro)
//! injeta o segredo no env de propósito e exige que o dump o PEGUE (>0 hits) — senão o teste de
//! 0-hits seria vácuo. E o evento `CredentialStored` no log carrega só a REFERÊNCIA, jamais o valor.
//!
//! Headless/CI: `openpty` cria o tty do filho; o teste NÃO exige um tty de controle (não pendura sem
//! terminal). A leitura é feita numa thread com teto de tempo — um filho que travasse não trava o teste.
#![cfg(unix)]

use std::io::Read;
use std::time::Duration;

use lina_core::{DomainEvent, EventStore, PtyCommand, PtyManager};
use lina_secrets::{MockStore, SecretVault};

const COLS: u16 = 80;
const ROWS: u16 = 24;
/// Teto defensivo da leitura do env do filho (`env` sai em ms; isto só blinda contra pendurar em CI).
const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// Lê TODO o output do master do PTY `node` até EOF (o filho `env` sai) — numa thread com teto de
/// tempo: um filho que NÃO saísse devolveria string parcial em vez de pendurar o teste (garantia
/// "non-interactive não trava" da peça). Devolve o env dumpado do processo-filho real.
fn drain_pty(manager: &PtyManager, node: &str) -> String {
    let reader = manager.clone_reader(node).expect("reader do master do PTY");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break, // EOF (o filho saiu, o master fechou) ou erro de leitura.
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(DRAIN_DEADLINE).unwrap_or_default();
    String::from_utf8_lossy(&out).into_owned()
}

/// Spawna um PTY de agente que dumpa o env do FILHO (`/usr/bin/env`) — o MESMO caminho de spawn do
/// app (`PtyCommand`/`PtyManager`) — aplicando `extra_env` no ambiente. Devolve o env real do filho.
fn spawn_agent_env_dump(node: &str, extra_env: &[(&str, &str)]) -> String {
    let mut manager = PtyManager::new();
    let mut cmd = PtyCommand::new("/usr/bin/env");
    for (k, v) in extra_env {
        cmd = cmd.env(*k, *v);
    }
    manager
        .spawn(node, cmd, COLS, ROWS)
        .expect("spawn do PTY de agente");
    let dump = drain_pty(&manager, node);
    let _ = manager.kill(node, Duration::from_secs(2));
    dump
}

/// **GATE (a):** credencial guardada no cofre → NÃO aparece em lugar nenhum do env de um agente
/// spawnado (0 hits, inclusive em nenhuma `LINA_*`). O valor vive só no keyring (out-of-process).
#[test]
fn channel_credential_never_appears_in_spawned_agent_env() {
    // Agulha aleatória (uma credencial real) guardada SÓ no cofre — nunca tocada no env.
    let vault = SecretVault::with_store("lina-space/ws-f40", MockStore::new());
    let needle = lina_secrets::generate_webhook_secret().expect("gera a agulha");
    let cred = vault
        .set_channel_credential("whatsapp", "token", &needle)
        .expect("guarda a credencial no cofre");
    // O cofre tem o VALOR; a referência (o que viaja no log) não é o valor.
    assert_eq!(
        vault
            .get_channel_credential("whatsapp", "token")
            .expect("get")
            .as_deref(),
        Some(needle.as_str())
    );
    assert_ne!(
        cred.key_ref, needle,
        "key_ref é referência (a conta), nunca o valor"
    );

    // Spawna um agente do MESMO jeito do app: com o env de IDENTIDADE, SEM o segredo.
    let env_dump = spawn_agent_env_dump(
        "agente-f40",
        &[("LINA_NODE_ID", "n-teste"), ("LINA_AUTONOMY", "autonomo")],
    );

    // Sanidade ANTES do grep: o dump É mesmo o env do filho (as vars que injetamos aparecem) —
    // senão um dump vazio passaria o "0 hits" vacuamente. (A mutação abaixo blinda o outro lado.)
    assert!(
        env_dump.contains("LINA_NODE_ID=n-teste"),
        "o dump não trouxe o env do filho:\n{env_dump}"
    );

    // GATE (a): a credencial NÃO está em parte alguma do env do filho.
    assert_eq!(
        env_dump.matches(needle.as_str()).count(),
        0,
        "a credencial VAZOU para o env do agente:\n{env_dump}"
    );
    // E nenhuma var de identidade `LINA_*` carrega o segredo (o carimbo de identidade é só identidade).
    for line in env_dump.lines().filter(|l| l.starts_with("LINA_")) {
        assert!(
            !line.contains(needle.as_str()),
            "segredo encontrado numa var LINA_*: {line}"
        );
    }

    // Evidência (sem vazar o valor) — visível com `--nocapture`, para o relatório do gate.
    let lina_vars: Vec<&str> = env_dump
        .lines()
        .filter(|l| l.starts_with("LINA_"))
        .map(|l| l.split('=').next().unwrap_or(l))
        .collect();
    eprintln!(
        "GATE (a) OK · credencial (64-hex) em 0 de {} vars do env do agente · identidade presente: {:?}",
        env_dump.lines().count(),
        lina_vars
    );
}

/// **Prova por MUTAÇÃO (padrão-ouro):** se a credencial FOSSE injetada no env (o caminho proibido),
/// o dump a PEGARIA (>0 hits). Sem isto, um `env` que silenciasse daria falso-verde no teste acima.
#[test]
fn mutation_proves_the_env_dump_would_catch_a_leak() {
    let needle = lina_secrets::generate_webhook_secret().expect("gera a agulha");
    // Injeta o segredo no env DE PROPÓSITO — o caminho que a custódia proíbe.
    let env_dump = spawn_agent_env_dump("agente-f40-mut", &[("LEAKED_SECRET", &needle)]);
    assert!(
        env_dump.matches(needle.as_str()).count() >= 1,
        "o dump de env DEVE pegar um segredo injetado — senão o teste de 0-hits é vácuo:\n{env_dump}"
    );
    assert!(
        env_dump.contains(&format!("LEAKED_SECRET={needle}")),
        "a var injetada aparece literal no dump:\n{env_dump}"
    );
}

/// O evento `CredentialStored` no log carrega a REFERÊNCIA (`key_ref`), **JAMAIS o valor** —
/// invariante #2 ("nada em claro no disco") / ADR 0004. Prova o contrato do binding (cofre + evento)
/// construindo o evento a partir do [`lina_secrets::CredentialRef`] devolvido pelo cofre.
#[test]
fn credential_stored_event_carries_reference_not_value() {
    let dir = std::env::temp_dir().join(format!(
        "lina-f40-cred-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = EventStore::open(&dir).expect("abre o event store");
    let vault = SecretVault::with_store("lina-space/ws-f40", MockStore::new());
    let needle = lina_secrets::generate_webhook_secret().expect("gera a agulha");

    // O binding: cofre.set_channel_credential → CredentialRef → append do evento (só a referência).
    let cred = vault
        .set_channel_credential("email", "api_key", &needle)
        .expect("guarda a credencial");
    store
        .append(&DomainEvent::CredentialStored {
            channel: cred.channel.clone(),
            scope: cred.scope.clone(),
            key_ref: cred.key_ref.clone(),
        })
        .expect("append do CredentialStored");

    // O payload serializado no log NÃO contém o valor — só a referência/escopo.
    let records = store.events().expect("lê o log");
    let stored = records
        .iter()
        .find(|r| r.kind == "CredentialStored")
        .expect("CredentialStored no log");
    let payload = serde_json::to_string(&stored.payload).expect("serializa o payload");
    assert!(
        !payload.contains(needle.as_str()),
        "o VALOR da credencial vazou no log: {payload}"
    );
    assert!(
        payload.contains("api_key"),
        "a referência (key_ref) está no log"
    );
    assert!(
        payload.contains("channel:email"),
        "o escopo está no log: {payload}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
