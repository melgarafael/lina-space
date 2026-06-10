//! **BUG-1 dogfood r1 (ADR 0026) — identidade por ENV DE SPAWN vence a ficha por-cwd.**
//!
//! O bug observado: o time inteiro foi spawnado com o MESMO cwd; o app reescreve
//! `<cwd>/.lina/bootstrap.json` a cada admissão → último escritor vence, e o MAESTRO
//! passou a se ver como "Automatizador" (whoami/handshake/outbox errados). O fix: o app
//! injeta `LINA_NODE_NAME`/`LINA_NODE_ID` no env do PTY (autoridade do app, por-processo)
//! e o CLI resolve `terminal_name` por env ANTES da ficha. Estes testes rodam o BINÁRIO
//! real (mesmo padrão de `handoff_cli.rs`) e provam as 3 exigências do despacho:
//! env vence ficha (controle: sem env → ficha) · outbox por-nó no subdir do env ·
//! compat de ficha ausente preservado.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempWs {
    home: PathBuf,
    cwd: PathBuf,
}

impl TempWs {
    /// `home` = LINA_HOME · `cwd` = diretório COMPARTILHADO com a ficha do ÚLTIMO escritor
    /// ("Automatizador") — o cenário real da colisão de identidade.
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("lina-ident-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("shared");
        let cwd = root.join("repo-compartilhado");
        std::fs::create_dir_all(&home).expect("criar LINA_HOME");
        std::fs::create_dir_all(cwd.join(".lina")).expect("criar cwd/.lina");
        let bootstrap = r#"{"terminal_name":"Automatizador","roster":["Automatizador","Bug Finder","QA"],"vault_path":"/tmp/none","autonomy":"assisted"}"#;
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

/// Roda o binário `lina` no cwd compartilhado. `node_env = Some(nome)` simula o env que o
/// app injeta no spawn do PTY; `None` limpa a var (isola do ambiente do test-runner).
fn run_lina(ws: &TempWs, args: &[&str], node_env: Option<&str>) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .current_dir(&ws.cwd)
        .env("LINA_HOME", &ws.home);
    match node_env {
        Some(name) => cmd.env("LINA_NODE_NAME", name),
        None => cmd.env_remove("LINA_NODE_NAME"),
    };
    cmd.output().expect("executar o binário lina")
}

/// **Exigência 1 do despacho:** com env setado e ficha divergente → identidade = env;
/// controle: sem env → ficha. Observável no `lina whoami` do binário real.
#[test]
fn whoami_identity_comes_from_spawn_env_over_clobbered_ficha() {
    let ws = TempWs::new("whoami");

    // O terminal do Bug Finder, cuja ficha foi sobrescrita pelo "Automatizador":
    let out = run_lina(&ws, &["whoami"], Some("Bug Finder"));
    assert!(out.status.success(), "whoami com env deve sair 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Voce e o terminal \"Bug Finder\""),
        "identidade = env do spawn, não a ficha sobrescrita: {stdout}"
    );
    assert!(
        !stdout.contains("Voce e o terminal \"Automatizador\""),
        "a ficha do último escritor NÃO define mais quem eu sou: {stdout}"
    );

    // Controle (terminal puro/compat): sem env, a ficha continua mandando.
    let out = run_lina(&ws, &["whoami"], None);
    assert!(out.status.success(), "whoami sem env deve sair 0 (compat)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Voce e o terminal \"Automatizador\""),
        "controle: sem env, vale a ficha: {stdout}"
    );
}

/// **Exigência 2:** o outbox POR-NÓ (origem autenticada do A2A) deposita no subdir do
/// NOME DO ENV — o `lina ask` do MAESTRO não cai mais na caixa do "Automatizador".
#[test]
fn ask_outbox_lands_in_env_named_subdir() {
    let ws = TempWs::new("outbox");

    let out = run_lina(&ws, &["ask", "@QA", "oi do bug finder"], Some("Bug Finder"));
    assert!(
        out.status.success(),
        "ask com env deve enfileirar (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let env_dir = ws.home.join("outbox").join("Bug Finder");
    let ficha_dir = ws.home.join("outbox").join("Automatizador");
    assert_eq!(
        std::fs::read_dir(&env_dir).map(Iterator::count).ok(),
        Some(1),
        "a mensagem mora no subdir do NOME DO ENV (origem inforjável)"
    );
    assert!(
        !ficha_dir.exists(),
        "nada vaza para o subdir do nome da ficha sobrescrita"
    );

    // Controle: sem env, o subdir continua sendo o da ficha (compat).
    let out = run_lina(&ws, &["ask", "@QA", "oi compat"], None);
    assert!(out.status.success(), "ask sem env (compat) deve enfileirar");
    assert_eq!(
        std::fs::read_dir(&ficha_dir).map(Iterator::count).ok(),
        Some(1),
        "controle: sem env, o canal é o subdir do nome da ficha"
    );
}

/// **Red-team r1 do ADR 0026 (doutrina):** verbo CUSTODIADO (`lina resume`, fila de broker)
/// com ficha AUSENTE **não** toma identidade do env — degrada para `agente-desconhecido`.
/// O env é exportável pelo processo do agente; em caminho de custódia ele não pode escolher
/// o subdir autenticado. NÃO-VACUOSO: reintroduzir o fallback de env faz o teste falhar.
#[test]
fn custodied_resume_without_ficha_ignores_env_identity() {
    let ws = TempWs::new("custody");
    let _ = std::fs::remove_file(ws.cwd.join(".lina/bootstrap.json"));

    let out = run_lina(&ws, &["resume"], Some("Mascarado"));
    assert!(
        out.status.success(),
        "resume registra o pedido mesmo sem ficha (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let broker_outbox = ws.home.join("broker").join("outbox");
    assert_eq!(
        std::fs::read_dir(broker_outbox.join("agente-desconhecido"))
            .map(Iterator::count)
            .ok(),
        Some(1),
        "sem ficha, o requester custodiado degrada para agente-desconhecido"
    );
    assert!(
        !broker_outbox.join("Mascarado").exists(),
        "o env NÃO escolhe o subdir da fila custodiada (doutrina: identidade exportável \
         não vira origem em verbo de autoridade)"
    );
}

/// **BUG-3 (integração, binário real):** com os papéis REAIS no `agents.json`, o whoami e o
/// handshake exibem o papel CANÔNICO — `Automatizador` é AUTOMATOR (não DEVELOPER da
/// inferência por nome) e `reviewer` vira REVIEWER nas duas superfícies (paridade).
#[test]
fn whoami_and_handshake_show_canonical_real_roles_from_agents_json() {
    let ws = TempWs::new("roles");
    let ficha = r#"{"terminal_name":"Automatizador","roster":["Automatizador","Bug Finder","Revisor"],"vault_path":"/tmp/none","autonomy":"assisted"}"#;
    std::fs::write(ws.cwd.join(".lina/bootstrap.json"), ficha).expect("ficha");
    let agents = r#"[{"name":"Automatizador","role":"AUTOMATOR","status":"Running"},{"name":"Bug Finder","role":"BUG_FIXER","status":"Running"},{"name":"Revisor","role":"reviewer","status":"Running"}]"#;
    std::fs::write(ws.home.join("agents.json"), agents).expect("agents.json");

    // whoami do Bug Finder (env): o PRÓPRIO papel e o dos colegas vêm do agents.json.
    let out = run_lina(&ws, &["whoami"], Some("Bug Finder"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Automatizador (AUTOMATOR)"),
        "colega exibe o papel REAL canônico, não DEVELOPER da inferência: {stdout}"
    );
    assert!(
        stdout.contains("Revisor (REVIEWER)"),
        "reviewer canoniza no whoami: {stdout}"
    );
    assert!(
        !stdout.contains("(reviewer)") && !stdout.contains("Automatizador (DEVELOPER)"),
        "nenhuma superfície vaza o cru/divergente: {stdout}"
    );

    // handshake: a MESMA forma canônica.
    let out = run_lina(&ws, &["handshake"], Some("Bug Finder"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Automatizador (AUTOMATOR)") && stdout.contains("Revisor (REVIEWER)"),
        "handshake exibe os MESMOS papéis canônicos do whoami (paridade): {stdout}"
    );
}

/// **Exigência 3 (compat):** ficha AUSENTE + env AUSENTE → o comportamento atual de
/// erro/orientação é preservado (exit 1 + mensagem apontando o arquivo; nada inventado).
#[test]
fn missing_ficha_without_env_keeps_error_and_orientation() {
    let ws = TempWs::new("compat");
    let _ = std::fs::remove_file(ws.cwd.join(".lina/bootstrap.json"));

    let out = run_lina(&ws, &["whoami"], None);
    assert!(
        !out.status.success(),
        "sem ficha e sem env, whoami continua falhando visível (nunca identidade default)"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(".lina/bootstrap.json"),
        "a orientação continua apontando a ficha ausente: {stderr}"
    );
}
