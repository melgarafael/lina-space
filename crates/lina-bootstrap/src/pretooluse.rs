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

// ─────────────── R2c-2: guard de skill de orquestrador ESTRANGEIRO ───────────────
//
// A skill GLOBAL de outro orquestrador (Maestri) existe na máquina do fundador
// (power-user, 60+ skills globais standalone) e o Claude Code 2.1.x NÃO tem config de
// projeto p/ escondê-la nem deny por nome de skill. O env-scrub já matou `MAESTRI_*`
// (a skill agora SÓ FALHA), mas o modelo ainda a CARREGA e INVOCA. Aqui o gate
// PreToolUse intercepta a tool `Skill` e NEGA as do Maestri com uma mensagem de
// REDIRECT — o `deny` vira contexto que o modelo lê e corrige o rumo para os verbos
// `lina`. ENFORCEMENT real (a skill estrangeira para de funcionar de vez), não só
// doutrina; defesa em profundidade com env-scrub + ficha + gatilhos pt-br.
//
// **Doutrina de segurança (registrada):** o nome da skill no input NÃO é autoridade de
// identidade — é só o GATILHO do bloqueio defensivo (mesma família "campo escrito pelo
// agente não decide autorização", ADRs 0004/0006/0021). Por isso o bloqueio é
// CONSERVADOR e CIRÚRGICO: nega `maestri*` (alvo conhecido), libera todo o resto —
// NUNCA uma allow-list que mataria skills legítimas do papel. Payload sem nome legível
// ⇒ allow (não quebrar skill legítima por degradação), diferente do Bash (dúvida = ask).

/// Prefixos de skills de orquestradores estrangeiros (extensível — novos orquestradores
/// entram aqui). Uma skill cujo nome (sem namespace de plugin, lowercased) começa com um
/// destes — em fronteira de palavra — é bloqueada com redirect.
pub const FOREIGN_SKILL_PREFIXES: &[&str] = &["maestri"];

/// Nomes EXATOS conhecidos de skills do Maestri (documenta o alvo; permite, no futuro,
/// nomes que NÃO compartilhem um prefixo limpo). Hoje todos casam o prefixo também.
pub const FOREIGN_SKILL_EXACT: &[&str] = &[
    "maestri",
    "maestri-manager",
    "maestri-orchestrator",
    "maestri-portal",
];

/// `true` se `name` é uma skill de orquestrador estrangeiro. Normaliza defensivamente:
/// lowercased, sem namespace de plugin (`plugin:skill` → testa ambos), e o prefixo casa
/// só em FRONTEIRA (`maestri` puro ou seguido de `-`/`:`/`_`) — `"maestria-de-vendas"`
/// (palavra que contém "maestri") NÃO é o orquestrador. PURA — testável.
#[must_use]
pub fn is_foreign_skill(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    let bare = lower.rsplit(':').next().unwrap_or(&lower);
    let matches = |s: &str| {
        FOREIGN_SKILL_EXACT.contains(&s)
            || FOREIGN_SKILL_PREFIXES.iter().any(|p| {
                // Prefixo em fronteira de palavra: `<p>` puro OU `<p>` + separador.
                s.strip_prefix(p)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', ':', '_']))
            })
    };
    matches(&lower) || matches(bare)
}

/// Mensagem de REDIRECT do `deny` — o modelo lê isto e é empurrado para o caminho certo.
fn foreign_skill_redirect(name: &str) -> String {
    format!(
        "A skill \"{name}\" pertence a outro orquestrador e NAO funciona neste Espaco. \
         Para falar com outro terminal use os verbos lina: lina ask \"@Nome\" \"...\" / \
         lina handoff \"@Nome\" \"...\". Veja a skill lina-agent-bus."
    )
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

/// FIX-2 (dogfood): resultado estruturado do hook — o JSON de saída (idêntico ao de
/// [`pretooluse_output`], byte-estável) MAIS a decisão de gate que o caller (`run_pretooluse`) usa
/// para apendar `ActionGated{decision:"ask"}` no log e alimentar a fila de atenção. Mantém a decisão
/// PURA: quem faz IO (apender) é o bin, nunca esta função.
pub struct PretooluseResult {
    /// Linha JSON do `hookSpecificOutput` — vai ao stdout do hook.
    pub json: String,
    /// `Some` SÓ quando a decisão é `ask` sobre um comando Bash (o bloqueante que o humano vê) — é o
    /// que vira `ActionGated{decision:"ask"}`. `None` em allow/deny/skill/fail-safe (não enfileiram).
    pub gated_ask: Option<GatedAsk>,
}

/// FIX-2: o ASK do guard a registrar para a fila de atenção (sem o nó — o caller carimba o
/// `LINA_NODE_NAME` do env do PTY).
pub struct GatedAsk {
    /// Comando barrado (vira a copy leiga "vá ao terminal aprovar X").
    pub cmd: String,
    /// Classe canônica do gate (`gated-soft`/`gated-hard`).
    pub class: String,
}

/// **Função pura** do hook: dado o JSON do `PreToolUse` e a string de autonomia, devolve a
/// LINHA JSON de saída do hook. NUNCA entra em pânico e NUNCA devolve texto não-JSON. Casca fina
/// sobre [`pretooluse_result`] — preserva a assinatura e o golden-file existentes.
#[must_use]
pub fn pretooluse_output(input_json: &str, autonomy: &str) -> String {
    pretooluse_result(input_json, autonomy).json
}

/// Linha JSON de `allow` INCONDICIONAL do hook, no formato canônico do Claude Code. Usada quando o
/// gate está DESLIGADO por escolha do dono do Espaço (`workspace.json → guard:off`): uma chamada
/// residual do hook (settings antigo ainda com o comando) devolve `allow` na hora, sem classificar
/// nada. Reusa o mesmo serializador dos demais caminhos — zero drift de formato.
#[must_use]
pub fn allow_output(reason: &str) -> String {
    HookOutput::new("allow", reason.to_string()).render()
}

/// FIX-2: núcleo do hook — devolve o JSON E (quando `ask` sobre Bash) o `gated_ask` para a fila de
/// atenção. PURA (sem IO): o append do `ActionGated` é do caller. Matriz de decisão:
/// - `Bash` → extrai `tool_input.command`, classifica e decide pela matriz (ask → `gated_ask`).
/// - tool não-Bash (Write/Edit/…) → `allow` (ação local reversível; o gate mira o comando real).
/// - `Skill` → guard de orquestrador ESTRANGEIRO (Maestri) com redirect; o resto passa.
/// - JSON ilegível / comando ausente / sem `tool_name` → fail-safe `ask` (decisão volta ao humano;
///   sem comando legível, NÃO enfileira na fila — `gated_ask=None`, limitação conhecida v1).
#[must_use]
pub fn pretooluse_result(input_json: &str, autonomy: &str) -> PretooluseResult {
    let no_ask = |json: String| PretooluseResult {
        json,
        gated_ask: None,
    };
    let level = parse_autonomy(autonomy).unwrap_or(AutonomyLevel::Assisted);

    let value: serde_json::Value = match serde_json::from_str(input_json) {
        Ok(v) => v,
        Err(_) => {
            return no_ask(
                HookOutput::new(
                    "ask",
                    String::from(
                        "entrada do hook PreToolUse ilegível — confirmação humana solicitada por segurança",
                    ),
                )
                .render(),
            );
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
                    let json = HookOutput::new(
                        decision_str(verdict.decision),
                        reason_for(cmd, verdict.class, verdict.decision),
                    )
                    .render();
                    // FIX-2: SÓ o `ask` (bloqueante que o humano vê) enfileira na fila de atenção;
                    // allow/deny não. A classe acompanha p/ auditoria/copy.
                    let gated_ask = (verdict.decision == Decision::Ask).then(|| GatedAsk {
                        cmd: cmd.to_string(),
                        class: verdict.class.as_str().to_string(),
                    });
                    PretooluseResult { json, gated_ask }
                }
                None => no_ask(
                    HookOutput::new(
                        "ask",
                        String::from(
                            "tool Bash sem 'command' no payload — confirmação humana solicitada por segurança",
                        ),
                    )
                    .render(),
                ),
            }
        }
        // R2c-2: a tool `Skill` carrega/roda uma agent skill. Intercepta as de
        // orquestrador ESTRANGEIRO (Maestri) e NEGA com redirect; o resto (lina-agent-bus,
        // skills do papel) passa. O nome é o gatilho do bloqueio, NÃO autoridade.
        Some("Skill") => {
            // Defensivo: o campo exato do 2.1.x não é 100% confirmado — `skill` é o mais
            // provável; `name`/`skillName`/`skillId` cobrem variações sem regressão.
            let skill_name = value.get("tool_input").and_then(|ti| {
                ["skill", "name", "skillName", "skillId"]
                    .iter()
                    .find_map(|k| ti.get(*k).and_then(|v| v.as_str()))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            });
            match skill_name {
                Some(name) if is_foreign_skill(name) => {
                    no_ask(HookOutput::new("deny", foreign_skill_redirect(name)).render())
                }
                // Skill legítima OU nome ausente → allow. CIRÚRGICO: payload degradado
                // jamais mata uma skill do papel (a defesa é a denylist, não o fail-safe).
                _ => no_ask(
                    HookOutput::new(
                        "allow",
                        String::from(
                            "skill permitida neste Espaço (não é de orquestrador estrangeiro)",
                        ),
                    )
                    .render(),
                ),
            }
        }
        Some(other) => no_ask(
            HookOutput::new(
                "allow",
                format!(
                    "ferramenta '{other}' não executa comando de shell — ação local reversível, liberada"
                ),
            )
            .render(),
        ),
        None => no_ask(
            HookOutput::new(
                "ask",
                String::from(
                    "payload do PreToolUse sem 'tool_name' — confirmação humana solicitada por segurança",
                ),
            )
            .render(),
        ),
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

    // ─────────────── R2c-2: guard de skill de orquestrador ESTRANGEIRO ───────────────

    fn decision(out: &str) -> String {
        parse(out)["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn reason(out: &str) -> String {
        parse(out)["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A skill `maestri` (orquestrador estrangeiro) → **deny** com mensagem de REDIRECT
    /// que ENSINA o caminho certo (verbos `lina` + skill `lina-agent-bus`).
    #[test]
    fn foreign_skill_maestri_is_denied_with_redirect() {
        let input = r#"{"tool_name":"Skill","tool_input":{"skill":"maestri"}}"#;
        let out = pretooluse_output(input, "assistido");
        assert_eq!(decision(&out), "deny", "maestri é bloqueada");
        let r = reason(&out);
        assert!(r.contains("maestri"), "cita a skill negada: {r}");
        assert!(r.contains("lina ask"), "ensina o verbo lina: {r}");
        assert!(r.contains("lina-agent-bus"), "aponta a skill certa: {r}");
    }

    /// Variantes do Maestri (prefixo + nomes conhecidos) também são negadas.
    #[test]
    fn foreign_skill_maestri_variants_are_denied() {
        for name in [
            "maestri-orchestrator",
            "maestri-manager",
            "maestri-portal",
            "MAESTRI",           // case-insensitive
            "maestri:algum-sub", // namespaced defensivo
        ] {
            let input = format!(r#"{{"tool_name":"Skill","tool_input":{{"skill":"{name}"}}}}"#);
            assert_eq!(
                decision(&pretooluse_output(&input, "assistido")),
                "deny",
                "{name} deveria ser negada"
            );
        }
    }

    /// O campo do nome da skill é tratado defensivamente: `name` (além de `skill`)
    /// também dispara o bloqueio — o formato exato do 2.1.x não é 100% confirmado.
    #[test]
    fn foreign_skill_via_name_field_is_denied() {
        let input = r#"{"tool_name":"Skill","tool_input":{"name":"maestri-orchestrator"}}"#;
        assert_eq!(decision(&pretooluse_output(input, "assistido")), "deny");
    }

    /// Skill LEGÍTIMA do Lina (lina-agent-bus) → **allow** (a denylist é cirúrgica).
    #[test]
    fn lina_agent_bus_skill_is_allowed() {
        let input = r#"{"tool_name":"Skill","tool_input":{"skill":"lina-agent-bus"}}"#;
        assert_eq!(decision(&pretooluse_output(input, "assistido")), "allow");
    }

    /// Skill de PAPEL legítimo (frontend-design) → **allow** (nunca matar skill do papel).
    #[test]
    fn role_skill_frontend_design_is_allowed() {
        let input = r#"{"tool_name":"Skill","tool_input":{"skill":"frontend-design"}}"#;
        assert_eq!(decision(&pretooluse_output(input, "assistido")), "allow");
        // Palavra que CONTÉM "maestri" no meio não é um falso-positivo? Não há skill
        // assim, mas o match por boundary evita pegar "maestria-de-vendas".
        let input2 = r#"{"tool_name":"Skill","tool_input":{"skill":"maestria-de-vendas"}}"#;
        assert_eq!(
            decision(&pretooluse_output(input2, "assistido")),
            "allow",
            "boundary: 'maestria' (palavra) não é o orquestrador 'maestri'"
        );
    }

    /// Skill SEM nome legível (payload degradado) → **allow** (deny cirúrgico: payload
    /// ruim NUNCA mata uma skill legítima do papel; a defesa é a denylist, não o
    /// fail-safe — diferente do Bash, onde a dúvida vira `ask`).
    #[test]
    fn skill_without_name_is_allowed_not_blocked() {
        let input = r#"{"tool_name":"Skill","tool_input":{}}"#;
        assert_eq!(decision(&pretooluse_output(input, "assistido")), "allow");
    }

    /// **ZERO regressão do gate W3-6:** a cobertura nova de Skill não toca o gate de
    /// execução Bash — `rm -rf /` segue não-allow; `cargo test` segue allow.
    #[test]
    fn bash_execution_gate_unchanged_by_skill_guard() {
        let danger = r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf /"}}"#;
        assert_ne!(decision(&pretooluse_output(danger, "autonomo")), "allow");
        let routine = r#"{"tool_name":"Bash","tool_input":{"command":"cargo test"}}"#;
        assert_eq!(decision(&pretooluse_output(routine, "assistido")), "allow");
    }

    // ─────────────── FIX-2: o hook expõe o ASK para a fila de atenção ───────────────

    /// `pretooluse_result` devolve o MESMO JSON (golden-file/pureza intactos) e, quando a decisão é
    /// `ask` sobre Bash, um `gated_ask` com o comando + classe — o que o caller (`run_pretooluse`)
    /// apenda como `ActionGated{decision:"ask", node}`, fechando o loop até a fila de atenção.
    #[test]
    fn pretooluse_result_exposes_gated_ask_on_ask() {
        let input =
            r#"{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}"#;
        let r = pretooluse_result(input, "assistido");
        assert_eq!(
            r.json,
            pretooluse_output(input, "assistido"),
            "JSON idêntico ao da função pura (pureza/golden-file preservados)"
        );
        let g = r.gated_ask.expect("um ask gera gated_ask");
        assert_eq!(g.cmd, "git push --force origin main");
        assert!(!g.class.is_empty(), "classe preenchida");
    }

    /// `allow` (rotina) e `deny` (skill estrangeira) NÃO geram `gated_ask` — não bloqueiam o humano,
    /// logo não viram pendência da fila.
    #[test]
    fn pretooluse_result_no_gated_ask_on_allow_or_deny() {
        let allow = pretooluse_result(
            r#"{"tool_name":"Bash","tool_input":{"command":"cargo test"}}"#,
            "assistido",
        );
        assert!(allow.gated_ask.is_none(), "allow não enfileira atenção");
        let deny = pretooluse_result(
            r#"{"tool_name":"Skill","tool_input":{"skill":"maestri"}}"#,
            "assistido",
        );
        assert!(deny.gated_ask.is_none(), "deny não enfileira atenção");
    }
}
