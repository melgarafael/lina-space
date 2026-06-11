//! W3-6 AC-6.2 (headless, observável) — o verbo `lina guard --check-action` decide
//! `allow`/`ask`/`deny` deterministicamente e apenda `ActionGated` ao event log SÓ quando
//! a decisão não é `allow`. Roda o BINÁRIO real (via `CARGO_BIN_EXE_lina`), num `LINA_HOME`
//! temporário e isolado por teste, e asserta a decisão (stdout) + o evento (event store).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use lina_core::EventStore;

/// Diretório `LINA_HOME` temporário e único; removido no `Drop` (best-effort).
struct TempHome(PathBuf);

impl TempHome {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p =
            std::env::temp_dir().join(format!("lina-w36a-{tag}-{}-{nanos}", std::process::id()));
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

/// Roda `lina guard --check-action --cmd <cmd> --autonomy <lvl>` com `LINA_HOME=home` e
/// devolve a decisão impressa (stdout, trimada). Falha o teste se o processo não suceder.
fn run_guard(home: &TempHome, cmd: &str, autonomy: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_lina");
    let out = Command::new(exe)
        .args([
            "guard",
            "--check-action",
            "--cmd",
            cmd,
            "--autonomy",
            autonomy,
        ])
        .env("LINA_HOME", home.path())
        .output()
        .expect("executar o binário lina");
    assert!(
        out.status.success(),
        "lina guard deveria sair com sucesso; status={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("stdout utf-8")
        .trim()
        .to_string()
}

/// Todos os `ActionGated` apendados ao event store de `home` (vazio se o store nem existe).
fn action_gated_events(home: &TempHome) -> Vec<(String, String, String)> {
    if !home.events_dir().join("lina.db").exists() {
        return Vec::new();
    }
    let store = EventStore::open(home.events_dir()).expect("abrir event store");
    store
        .events()
        .expect("ler eventos")
        .into_iter()
        .filter(|r| r.kind == "ActionGated")
        .map(|r| {
            let p = &r.payload;
            (
                p["cmd"].as_str().unwrap_or_default().to_string(),
                p["class"].as_str().unwrap_or_default().to_string(),
                p["decision"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// AC-6.2 (1): `git push --force origin main` em `autonomo` → `ask` (NUNCA allow) +
/// `ActionGated{class:"gated-hard", decision:"ask"}` no log.
#[test]
fn force_push_main_autonomous_is_ask_and_logs_gated_hard() {
    let home = TempHome::new("forcepush");
    let decision = run_guard(&home, "git push --force origin main", "autonomo");
    assert_eq!(decision, "ask", "force-push em main NUNCA pode ser allow");

    let events = action_gated_events(&home);
    assert_eq!(events.len(), 1, "exatamente 1 ActionGated apendado");
    let (cmd, class, dec) = &events[0];
    assert_eq!(cmd, "git push --force origin main");
    assert_eq!(class, "gated-hard");
    assert_eq!(dec, "ask");
}

/// AC-6.2 (2): `cargo test` → `allow`, SEM evento (routine não polui o log).
#[test]
fn cargo_test_is_allow_with_no_event() {
    let home = TempHome::new("cargotest");
    let decision = run_guard(&home, "cargo test", "assistido");
    assert_eq!(decision, "allow");
    assert!(
        action_gated_events(&home).is_empty(),
        "ação routine (allow) não deve apendar ActionGated"
    );
}

/// AC-6.2 (3): `git push origin feature/x` → `ask` em `assistido`; `allow` em `autonomo`
/// (afrouxa gated-soft). O caso `assistido` apenda evento; o `autonomo` (allow) não.
#[test]
fn feature_push_asks_in_assisted_and_allows_in_autonomous() {
    let home_a = TempHome::new("featassist");
    assert_eq!(
        run_guard(&home_a, "git push origin feature/x", "assistido"),
        "ask"
    );
    let ev = action_gated_events(&home_a);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].1, "gated-soft");
    assert_eq!(ev[0].2, "ask");

    let home_b = TempHome::new("featauto");
    assert_eq!(
        run_guard(&home_b, "git push origin feature/x", "autonomo"),
        "allow"
    );
    assert!(
        action_gated_events(&home_b).is_empty(),
        "gated-soft afrouxado para allow não apenda evento"
    );
}

/// AC-6.2 (4): `rm -rf /` → `ask`/`deny` (gated-hard), nunca allow; sempre apenda evento.
#[test]
fn rm_rf_root_is_never_allow_and_logs() {
    for lvl in ["manual", "assistido", "autonomo"] {
        let home = TempHome::new(&format!("rmrf-{lvl}"));
        let decision = run_guard(&home, "rm -rf /", lvl);
        assert_ne!(
            decision, "allow",
            "rm -rf / nunca pode ser allow (nível {lvl})"
        );

        let events = action_gated_events(&home);
        assert_eq!(
            events.len(),
            1,
            "rm -rf / deve apendar ActionGated (nível {lvl})"
        );
        assert_eq!(events[0].1, "gated-hard");
    }
}

// ─────────────── FIX-2: o caminho REAL do hook (`--pretooluse`) apenda p/ a fila ───────────────

/// Roda `lina guard --pretooluse` (o caminho que o app configura no `PreToolUse`) com `LINA_HOME`,
/// `LINA_NODE_NAME` e `LINA_AUTONOMY` no env e o JSON do hook no stdin; devolve o stdout trimado.
fn run_pretooluse_hook(home: &TempHome, node: &str, autonomy: &str, input_json: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut child = Command::new(exe)
        .args(["guard", "--pretooluse"])
        .env("LINA_HOME", home.path())
        .env("LINA_NODE_NAME", node)
        .env("LINA_AUTONOMY", autonomy)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lina guard --pretooluse");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(input_json.as_bytes())
        .expect("escrever stdin");
    let out = child.wait_with_output().expect("aguardar lina");
    assert!(
        out.status.success(),
        "guard --pretooluse deveria sair com sucesso; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("stdout utf-8")
        .trim()
        .to_string()
}

/// **FIX-2 (o fix do dogfooding): o hook decide `ask` E APENDA `ActionGated{ask, node}`.** Era o
/// buraco: o `--pretooluse` decidia `ask` (bloqueava o agente) mas NUNCA apendava o evento → a fila
/// de atenção jamais via o bloqueio. Agora o evento entra COM o `LINA_NODE_NAME` (a fila foca por
/// nome). Prova o circuito do binário ponta-a-ponta até o event log.
#[test]
fn pretooluse_ask_logs_action_gated_with_node() {
    let home = TempHome::new("pretooluse-ask");
    let input = r#"{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}"#;
    let out = run_pretooluse_hook(&home, "Bug Finder", "assistido", input);
    assert!(
        out.contains("\"permissionDecision\":\"ask\""),
        "o hook decide ask: {out}"
    );

    let store = EventStore::open(home.events_dir()).expect("abrir store");
    let gated: Vec<_> = store
        .events()
        .expect("eventos")
        .into_iter()
        .filter(|r| r.kind == "ActionGated")
        .collect();
    assert_eq!(
        gated.len(),
        1,
        "1 ActionGated apendado pelo caminho do hook"
    );
    let p = &gated[0].payload;
    assert_eq!(p["decision"].as_str(), Some("ask"));
    assert_eq!(
        p["node"].as_str(),
        Some("Bug Finder"),
        "carimba o NOME do nó (a fila foca por nome)"
    );
    assert_eq!(p["cmd"].as_str(), Some("git push --force origin main"));
}

/// **FIX-2: rotina (allow) pelo hook NÃO apenda** — não bloqueia o humano, logo não enfileira
/// atenção (anti-ruído: o log/fila só recebem o que exige você).
#[test]
fn pretooluse_allow_logs_nothing() {
    let home = TempHome::new("pretooluse-allow");
    let input = r#"{"tool_name":"Bash","tool_input":{"command":"cargo test"}}"#;
    let out = run_pretooluse_hook(&home, "Bug Finder", "assistido", input);
    assert!(
        out.contains("\"permissionDecision\":\"allow\""),
        "rotina = allow: {out}"
    );
    assert!(
        action_gated_events(&home).is_empty(),
        "allow não apenda ActionGated"
    );
}
