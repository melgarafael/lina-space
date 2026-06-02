//! W3-6 (AC-6.3) — **hook `PreToolUse` do Claude Code (tier 1)**: o gate de execução
//! verdadeiramente DURO. O harness do Claude Code garante que TODA chamada de tool
//! (Bash/Write/…) passa por este hook ANTES de executar.
//!
//! Fluxo: o hook recebe no **stdin** o JSON do `PreToolUse` (contém `tool_name` +
//! `tool_input`; para `Bash`, o comando vive em `tool_input.command`), extrai o comando
//! real, reusa o núcleo determinístico [`lina_core::check_action`] (classify+decide, **ZERO
//! LLM** — invariante #1) e emite no **stdout APENAS** o JSON:
//! ```json
//! {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow|ask|deny","permissionDecisionReason":"…"}}
//! ```
//!
//! **Robustez (mesma regra do `SessionStart`, W3-2):** este módulo NUNCA imprime texto cru
//! ou erro no stdout. Em qualquer falha (JSON ilegível, payload sem comando) ele devolve um
//! JSON **fail-safe `ask`** — a decisão volta para o humano, jamais um `allow` silencioso —
//! e o binário registra o diagnóstico em stderr.

use lina_core::{check_action, parse_autonomy, ActionClass, AutonomyLevel, Decision};
use serde::Serialize;

/// Nível default quando `LINA_AUTONOMY` está ausente/ilegível (espelha o default do produto,
/// `assistido` — design §1.1).
const DEFAULT_AUTONOMY: &str = "assistido";

/// JSON de saída do hook (`hookSpecificOutput`). Campos em ordem de DECLARAÇÃO — serde serializa
/// structs nessa ordem, garantindo um golden-file byte-estável (≠ `Value`, que ordena alfabético).
#[derive(Serialize)]
struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
}

#[derive(Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: String,
}

impl HookOutput {
    /// Monta a saída canônica. `decision` já é a string do hook (`allow`/`ask`/`deny`).
    fn new(decision: &'static str, reason: String) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PreToolUse",
                permission_decision: decision,
                permission_decision_reason: reason,
            },
        }
    }

    /// Serializa para a linha única que vai ao stdout. `to_string` de um struct não falha em
    /// caminho normal; o `unwrap_or_else` é uma rede determinística (sem `unwrap` de produção).
    fn render(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            // Último recurso: JSON `ask` literal e válido (jamais stdout cru).
            String::from(
                "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"ask\",\"permissionDecisionReason\":\"falha ao serializar a decisao do gate — confirmacao humana solicitada\"}}",
            )
        })
    }
}

/// Mapa [`Decision`] → `permissionDecision` do Claude Code (allow→allow, ask→ask, deny→deny).
fn decision_str(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Ask => "ask",
        Decision::Deny => "deny",
    }
}

/// Motivo legível (pt-br) — determinístico por `(classe, decisão)`, com o comando embutido para
/// dar contexto ao humano. Determinismo é o que torna o golden-file estável.
fn reason_for(cmd: &str, class: ActionClass, decision: Decision) -> String {
    match (class, decision) {
        (ActionClass::Routine, _) => {
            format!("comando '{cmd}': rotina local reversível — liberado automaticamente")
        }
        (ActionClass::GatedSoft, Decision::Allow) => {
            format!("comando '{cmd}': gated-soft afrouxado para automático no nível autônomo")
        }
        (ActionClass::GatedSoft, _) => format!(
            "comando '{cmd}': ação irreversível recuperável (gated-soft) — confirme antes de executar"
        ),
        (ActionClass::GatedHard, Decision::Deny) => {
            format!("comando '{cmd}': ação gated-hard negada")
        }
        (ActionClass::GatedHard, _) => format!(
            "comando '{cmd}': ação destrutiva ou com custódia de segredo (gated-hard) — confirmação humana obrigatória"
        ),
    }
}

/// **Função pura** do hook: dado o JSON do `PreToolUse` e a string de autonomia, devolve a
/// LINHA JSON de saída do hook. NUNCA entra em pânico e NUNCA devolve texto não-JSON.
///
/// - `Bash` → extrai `tool_input.command`, classifica e decide pela matriz.
/// - tool não-Bash (Write/Edit/…) → `allow` (ação local reversível; o gate mira o comando real).
/// - JSON ilegível ou comando ausente → fail-safe `ask` (decisão volta ao humano).
#[must_use]
pub fn pretooluse_output(input_json: &str, autonomy: &str) -> String {
    let level = parse_autonomy(autonomy).unwrap_or(AutonomyLevel::Assisted);

    let value: serde_json::Value = match serde_json::from_str(input_json) {
        Ok(v) => v,
        Err(_) => {
            return HookOutput::new(
                "ask",
                String::from(
                    "entrada do hook PreToolUse ilegível — confirmação humana solicitada por segurança",
                ),
            )
            .render();
        }
    };

    let tool_name = value.get("tool_name").and_then(|v| v.as_str());
    match tool_name {
        Some("Bash") => {
            let cmd = value
                .get("tool_input")
                .and_then(|ti| ti.get("command"))
                .and_then(|c| c.as_str())
                .map(str::trim)
                .filter(|c| !c.is_empty());
            match cmd {
                Some(cmd) => {
                    let verdict = check_action(cmd, level);
                    HookOutput::new(
                        decision_str(verdict.decision),
                        reason_for(cmd, verdict.class, verdict.decision),
                    )
                    .render()
                }
                None => HookOutput::new(
                    "ask",
                    String::from(
                        "tool Bash sem 'command' no payload — confirmação humana solicitada por segurança",
                    ),
                )
                .render(),
            }
        }
        Some(other) => HookOutput::new(
            "allow",
            format!(
                "ferramenta '{other}' não executa comando de shell — ação local reversível, liberada"
            ),
        )
        .render(),
        None => HookOutput::new(
            "ask",
            String::from(
                "payload do PreToolUse sem 'tool_name' — confirmação humana solicitada por segurança",
            ),
        )
        .render(),
    }
}

/// Nome da variável de ambiente de onde o hook lê o nível de autonomia (o app a injeta no PTY).
pub const AUTONOMY_ENV: &str = "LINA_AUTONOMY";

/// Lê a autonomia do ambiente (`LINA_AUTONOMY`) ou cai no default `assistido`.
#[must_use]
pub fn autonomy_from_env() -> String {
    std::env::var(AUTONOMY_ENV).unwrap_or_else(|_| DEFAULT_AUTONOMY.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(out: &str) -> serde_json::Value {
        serde_json::from_str(out).expect("saída do hook deve ser JSON bem-formado")
    }

    /// AC-6.3: `rm -rf /` (Bash) → JSON bem-formado com `permissionDecision` não-allow.
    #[test]
    fn rm_rf_root_bash_is_never_allow_and_well_formed() {
        let input = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rm -rf /"}}"#;
        let out = pretooluse_output(input, "assistido");
        let v = parse(&out);
        let d = v["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap();
        assert!(
            d == "ask" || d == "deny",
            "rm -rf / nunca pode ser allow; foi {d}"
        );
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"], "PreToolUse",
            "hookEventName deve ser PreToolUse"
        );
        assert!(
            !v["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .is_empty(),
            "deve haver um motivo"
        );
    }

    /// AC-6.3: `cargo test` → allow (rotina), sem ruído.
    #[test]
    fn cargo_test_bash_is_allow() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"cargo test"}}"#;
        let out = pretooluse_output(input, "assistido");
        let v = parse(&out);
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    /// `git push --force origin main` é gated-hard → ask em QUALQUER nível (autônomo não afrouxa).
    #[test]
    fn force_push_main_is_ask_even_autonomous() {
        let input =
            r#"{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}"#;
        let out = pretooluse_output(input, "autonomo");
        let v = parse(&out);
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    }

    /// gated-soft afrouxa para allow só no autônomo.
    #[test]
    fn feature_push_loosens_only_in_autonomous() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"git push origin feature/x"}}"#;
        assert_eq!(
            parse(&pretooluse_output(input, "assistido"))["hookSpecificOutput"]
                ["permissionDecision"],
            "ask"
        );
        assert_eq!(
            parse(&pretooluse_output(input, "autonomo"))["hookSpecificOutput"]
                ["permissionDecision"],
            "allow"
        );
    }

    /// Tool não-Bash (Write) → allow (ação local reversível).
    #[test]
    fn non_bash_tool_is_allow() {
        let input = r#"{"tool_name":"Write","tool_input":{"file_path":"/tmp/x","content":"y"}}"#;
        let v = parse(&pretooluse_output(input, "manual"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    /// Robustez: JSON ilegível → fail-safe `ask`, jamais pânico ou stdout cru.
    #[test]
    fn malformed_json_is_failsafe_ask() {
        let v = parse(&pretooluse_output("{ not json", "assistido"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    }

    /// Robustez: Bash sem `command` → fail-safe `ask`.
    #[test]
    fn bash_without_command_is_failsafe_ask() {
        let input = r#"{"tool_name":"Bash","tool_input":{}}"#;
        let v = parse(&pretooluse_output(input, "assistido"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    }

    /// Robustez: autonomia ilegível cai no default `assistido` (não entra em pânico).
    #[test]
    fn garbage_autonomy_falls_back_to_assisted() {
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"git push origin feature/x"}}"#;
        // feature push em assistido = ask; se caísse em autônomo seria allow.
        let v = parse(&pretooluse_output(input, "ludico-invalido"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    }
}
