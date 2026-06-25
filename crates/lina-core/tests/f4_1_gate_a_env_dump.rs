//! **F4-1 · gate (a) — o critério INFORJÁVEL da onda (ADR 0004 · espelha F4-0-2/AC-0004.1).**
//!
//! A fronteira inegociável de F4-1: o `X-Api-Key`/token de sessão Waha vive no keyring
//! (`lina-secrets`, escopo `channel:whatsapp`), **NUNCA no env do PTY de um agente**. Sem o segredo no
//! env, o agente não consegue falar com o Waha sozinho — por construção (custódia, ADR 0004). É o
//! mesmo gate de F4-0-2, re-derivado no canal CONCRETO desta onda (whatsapp), porque é o transporte
//! vivo de F4-1 que tenta abrir o primeiro socket de saída do Lina.
//!
//! Prova por inspeção do processo-FILHO real: guarda a credencial no cofre, SPAWNA um PTY de agente do
//! MESMO jeito do app (`PtyCommand`/`PtyManager`), dumpa o env do filho (via `/usr/bin/env`) e grepa o
//! valor → **0 hits**. A prova-por-MUTAÇÃO (padrão-ouro) injeta o segredo no env de propósito e exige
//! que o dump o PEGUE (>0 hits) — senão o teste de 0-hits seria vácuo.
//!
//! Headless/CI: `openpty` cria o tty do filho; a leitura roda numa thread com teto de tempo — um filho
//! que travasse devolve string parcial, não pendura o teste (garantia "non-interactive não trava").
#![cfg(unix)]

use std::io::Read;
use std::time::Duration;

use lina_core::{PtyCommand, PtyManager};
use lina_secrets::{MockStore, SecretVault};

const COLS: u16 = 80;
const ROWS: u16 = 24;
/// Teto defensivo da leitura do env do filho (`env` sai em ms; isto só blinda contra pendurar em CI).
const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// Lê TODO o output do master do PTY `node` até EOF (o filho `env` sai) — numa thread com teto de
/// tempo: um filho que NÃO saísse devolveria string parcial em vez de pendurar o teste. Devolve o env
/// dumpado do processo-filho real.
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

/// Spawna um PTY de agente que dumpa o env do FILHO (`/usr/bin/env`) — o MESMO caminho de spawn do app
/// (`PtyCommand`/`PtyManager`) — aplicando `extra_env`. Devolve o env real do filho.
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

/// **GATE (a):** o `X-Api-Key` da sessão Waha guardado no cofre (escopo `channel:whatsapp`) NÃO
/// aparece em parte alguma do env de um agente spawnado (0 hits, inclusive em nenhuma `LINA_*`). O
/// valor vive só no keyring (out-of-process); o transporte o recebe por `execute(&secret)`, não do env.
#[test]
fn waha_api_key_never_appears_in_spawned_agent_env() {
    // Agulha aleatória (a credencial real do Waha) guardada SÓ no cofre — nunca tocada no env.
    let vault = SecretVault::with_store("lina-space/ws-f41", MockStore::new());
    let needle = lina_secrets::generate_webhook_secret().expect("gera a agulha");
    let cred = vault
        .set_channel_credential("whatsapp", "api_key", &needle)
        .expect("guarda o X-Api-Key no cofre");
    // O cofre tem o VALOR; a referência (o que viaja no log) não é o valor.
    assert_eq!(
        vault
            .get_channel_credential("whatsapp", "api_key")
            .expect("get")
            .as_deref(),
        Some(needle.as_str())
    );
    assert_ne!(
        cred.key_ref, needle,
        "key_ref é referência (a conta), nunca o valor do token"
    );

    // Spawna um agente do MESMO jeito do app: com o env de IDENTIDADE, SEM o segredo.
    let env_dump = spawn_agent_env_dump(
        "agente-f41-whatsapp",
        &[("LINA_NODE_ID", "n-teste"), ("LINA_AUTONOMY", "autonomo")],
    );

    // Sanidade ANTES do grep: o dump É mesmo o env do filho (as vars que injetamos aparecem) — senão
    // um dump vazio passaria o "0 hits" vacuamente. (A mutação abaixo blinda o outro lado.)
    assert!(
        env_dump.contains("LINA_NODE_ID=n-teste"),
        "o dump não trouxe o env do filho:\n{env_dump}"
    );

    // GATE (a): o token Waha NÃO está em parte alguma do env do filho.
    assert_eq!(
        env_dump.matches(needle.as_str()).count(),
        0,
        "o X-Api-Key do Waha VAZOU para o env do agente:\n{env_dump}"
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
        "GATE (a)/F4-1 OK · X-Api-Key Waha em 0 de {} vars do env do agente · identidade: {:?}",
        env_dump.lines().count(),
        lina_vars
    );
}

/// **Prova por MUTAÇÃO (padrão-ouro):** se o token Waha FOSSE injetado no env (o caminho proibido),
/// o dump o PEGARIA (>0 hits). Sem isto, um `env` que silenciasse daria falso-verde no teste acima.
#[test]
fn mutation_proves_env_dump_would_catch_a_waha_token_leak() {
    let needle = lina_secrets::generate_webhook_secret().expect("gera a agulha");
    // Injeta o segredo no env DE PROPÓSITO — o caminho que a custódia proíbe.
    let env_dump = spawn_agent_env_dump("agente-f41-mut", &[("WAHA_API_KEY", &needle)]);
    assert!(
        env_dump.matches(needle.as_str()).count() >= 1,
        "o dump de env DEVE pegar um segredo injetado — senão o teste de 0-hits é vácuo:\n{env_dump}"
    );
    assert!(
        env_dump.contains(&format!("WAHA_API_KEY={needle}")),
        "a var injetada aparece literal no dump:\n{env_dump}"
    );
}
