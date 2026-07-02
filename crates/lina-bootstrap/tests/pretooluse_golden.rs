//! W3-6 **AC-6.3** (headless, golden) — o hook `lina guard --pretooluse` lê o JSON do
//! `PreToolUse` no **stdin** e emite no **stdout APENAS** o JSON
//! `{"hookSpecificOutput":{permissionDecision, …}}`, **bem-formado e estável** (golden-file),
//! **nunca** stdout cru (mesma robustez do `SessionStart`, W3-2).
//!
//! Roda o BINÁRIO real (`CARGO_BIN_EXE_lina`), com `LINA_AUTONOMY=assistido` fixado para que o
//! golden seja determinístico independentemente do ambiente de quem roda o teste.

use std::io::Write;
use std::process::{Command, Stdio};

/// Golden-files versionados (a fonte da verdade do contrato de saída do hook).
const GOLDEN_RM_RF: &str = include_str!("golden/pretooluse_rm_rf_root.json");
const GOLDEN_CARGO_TEST: &str = include_str!("golden/pretooluse_cargo_test.json");

/// Alimenta `input` no stdin de `lina guard --pretooluse` (autonomia `assistido` fixa) e devolve
/// `(stdout, stderr, success)`.
fn run_pretooluse(input: &str) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_lina");
    // Hermético: isola `LINA_HOME` num dir temp SEM `workspace.json` → o gate fica LIGADO,
    // independentemente do `LINA_HOME` do ambiente (num terminal Lina real ele pode apontar para um
    // Espaço com `guard:off`, que curto-circuitaria o hook para `allow`). Assim o golden testa a
    // CLASSIFICAÇÃO do comando, não o toggle-mestre do Espaço (esse tem cobertura própria).
    let home = std::env::temp_dir().join(format!(
        "lina-golden-home-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(&home);
    let mut child = Command::new(exe)
        .args(["guard", "--pretooluse"])
        .env("LINA_AUTONOMY", "assistido")
        .env("LINA_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawnar lina guard --pretooluse");
    child
        .stdin
        .take()
        .expect("stdin do filho")
        .write_all(input.as_bytes())
        .expect("escrever no stdin");
    let out = child.wait_with_output().expect("aguardar o filho");
    (
        String::from_utf8(out.stdout).expect("stdout utf-8"),
        String::from_utf8(out.stderr).expect("stderr utf-8"),
        out.status.success(),
    )
}

/// O stdout deve ser SEMPRE um JSON único e válido com a forma do hook (jamais texto cru).
fn assert_well_formed_hook(stdout: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("stdout do hook deve ser JSON bem-formado; foi {stdout:?} ({e})")
    });
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"], "PreToolUse",
        "hookEventName canônico"
    );
    let d = v["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .expect("permissionDecision string");
    assert!(
        matches!(d, "allow" | "ask" | "deny"),
        "permissionDecision deve ser allow|ask|deny; foi {d}"
    );
    v
}

/// AC-6.3 (golden A): `rm -rf /` (Bash) → saída byte-estável e `permissionDecision != allow`.
#[test]
fn pretooluse_rm_rf_root_matches_golden_and_is_not_allow() {
    let input = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rm -rf /"}}"#;
    let (stdout, stderr, ok) = run_pretooluse(input);
    assert!(ok, "o hook sai com sucesso (fala pelo JSON, não pelo exit)");
    assert!(
        stderr.is_empty(),
        "caminho feliz não loga em stderr; stderr={stderr:?}"
    );
    // Estabilidade byte-a-byte (modulo o \n final do println): trava o contrato de saída.
    assert_eq!(
        stdout.trim_end(),
        GOLDEN_RM_RF.trim_end(),
        "saída do hook divergiu do golden-file"
    );
    let v = assert_well_formed_hook(&stdout);
    assert_ne!(
        v["hookSpecificOutput"]["permissionDecision"], "allow",
        "rm -rf / NUNCA pode ser allow"
    );
}

/// AC-6.3 (golden B): `cargo test` (Bash) → saída byte-estável e `allow`.
#[test]
fn pretooluse_cargo_test_matches_golden_and_is_allow() {
    let input = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test"}}"#;
    let (stdout, stderr, ok) = run_pretooluse(input);
    assert!(ok);
    assert!(stderr.is_empty(), "caminho feliz não loga em stderr");
    assert_eq!(
        stdout.trim_end(),
        GOLDEN_CARGO_TEST.trim_end(),
        "saída do hook divergiu do golden-file"
    );
    let v = assert_well_formed_hook(&stdout);
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
}

/// AC-6.3 (robustez): stdin com JSON ILEGÍVEL → o hook ainda emite JSON bem-formado (fail-safe
/// `ask`), NUNCA stdout cru, e sai com sucesso (o harness lê o JSON, não o exit code).
#[test]
fn pretooluse_malformed_input_is_failsafe_ask_never_raw_stdout() {
    let (stdout, _stderr, ok) = run_pretooluse("{ isto não é json");
    assert!(ok, "mesmo em erro o hook sai com sucesso e fala pelo JSON");
    let v = assert_well_formed_hook(&stdout);
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecision"], "ask",
        "entrada ilegível → fail-safe ask (jamais allow silencioso)"
    );
}

/// AC-6.3 (robustez): stdin VAZIO → fail-safe `ask` bem-formado.
#[test]
fn pretooluse_empty_stdin_is_failsafe_ask() {
    let (stdout, _stderr, ok) = run_pretooluse("");
    assert!(ok);
    let v = assert_well_formed_hook(&stdout);
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
}
