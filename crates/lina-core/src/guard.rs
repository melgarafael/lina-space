//! W3-6 — **núcleo determinístico do gate de execução** (autonomia × classe de ação).
//!
//! **Invariante #1 (`CLAUDE.md`):** o app NUNCA chama LLM. Este módulo é 100%
//! pattern-match + tabela. Dado um comando (string) e o nível de autonomia, ele:
//!   1. [`classify`] o comando em uma [`ActionClass`] (`routine`|`gated-soft`|`gated-hard`)
//!      por padrões embutidos (design §1.1);
//!   2. [`decide`] aplica a **matriz canônica** (nível × classe) → [`Decision`]
//!      (`allow`|`ask`|`deny`).
//!
//! O gate **não lê a intenção** do agente — ele intercepta o **comando real** (design
//! §1.3). Aqui vive só a lógica pura; a interceptação (hook `PreToolUse` / PATH-shim) e
//! a custódia de segredo são camadas externas (W3-6 restante), fora do escopo deste núcleo.
//!
//! ## Matriz canônica (design §1.1)
//! | Classe \ Nível | `manual` | `assistido` | `autônomo` |
//! |---|---|---|---|
//! | `routine`     | allow | allow | allow |
//! | `gated-soft`  | ask   | ask   | **allow** (afrouxado) |
//! | `gated-hard`  | ask   | ask   | **ask** (piso — nunca afrouxa) |
//!
//! `gated-hard` confirma em TODOS os níveis: autonomia não é licença para deletar prod,
//! pagar ou enviar a terceiros sem humano. `delegation`/`broadcast` são guardrails do
//! **router** (não deste gate).

use thiserror::Error;

use crate::router::AutonomyLevel;

/// Classe de uma ação, do ponto de vista do agente (design §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionClass {
    /// Local e reversível: ler/editar arquivo no workspace, build, teste, `cargo …`.
    Routine,
    /// Irreversível mas recuperável / escopo-feature: `git push` (branch feature),
    /// `rm` em path do workspace, `git commit --amend`.
    GatedSoft,
    /// Destrutivo OU com custódia de segredo: force-push / push em `main`/`master`,
    /// `rm -rf` fora do workspace, deploy/terraform em prod, pagamento, envio externo,
    /// `DROP`/migração destrutiva.
    GatedHard,
}

impl ActionClass {
    /// String canônica persistida no evento `ActionGated` (`"routine"` etc.).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ActionClass::Routine => "routine",
            ActionClass::GatedSoft => "gated-soft",
            ActionClass::GatedHard => "gated-hard",
        }
    }
}

/// Decisão do gate para uma ação sob um nível de autonomia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Executa sem confirmação.
    Allow,
    /// Pede confirmação humana (sim/não) antes de executar.
    Ask,
    /// Recusa sem oferecer confirmação. (Não usado pela matriz atual — `gated-hard` é
    /// `ask` (piso confirmável), não `deny`; existe para completude do contrato/hook.)
    Deny,
}

impl Decision {
    /// String canônica impressa pelo verbo e persistida no evento (`"allow"` etc.).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        }
    }
}

/// Veredito do gate: a classe inferida + a decisão sob a matriz. Ambos vão para o
/// evento `ActionGated` quando a decisão NÃO for `allow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateDecision {
    pub class: ActionClass,
    pub decision: Decision,
}

/// Erro de entrada do gate (nível de autonomia desconhecido).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuardError {
    #[error("nível de autonomia desconhecido: {0:?} (use manual|assistido|autonomo)")]
    UnknownAutonomy(String),
}

/// Traduz a string de autonomia (pt-br do `workspace.json` / CLI) para [`AutonomyLevel`].
/// Aceita também os sinônimos em inglês (espelho do enum do core) por robustez.
pub fn parse_autonomy(s: &str) -> Result<AutonomyLevel, GuardError> {
    // `to_lowercase` (Unicode, não ASCII) para casar entrada acentuada: `AUTÔNOMO` → `autônomo`.
    match s.trim().to_lowercase().as_str() {
        "manual" => Ok(AutonomyLevel::Manual),
        "assistido" | "assisted" => Ok(AutonomyLevel::Assisted),
        "autonomo" | "autônomo" | "autonomous" => Ok(AutonomyLevel::Autonomous),
        other => Err(GuardError::UnknownAutonomy(other.to_string())),
    }
}

// ───────────────────────────── ruleset embutido (design §1.1) ─────────────────────────────

/// Tokens-chave (igualdade exata de token, NÃO substring) que classificam um comando como
/// `gated-hard` por si só: deploy/infra de prod, pagamento, envio externo, e-mail.
/// Igualdade de token evita falsos positivos como `cat src/deploy.rs` (token `src/deploy.rs`).
const HARD_KEYWORD_TOKENS: &[&str] = &[
    "deploy", // `npm run deploy`, `lina do deploy`, …
    "payment", "pay", "charge", "stripe",  // pagamento / custódia de segredo
    "webhook", // envio a webhook de terceiro
    "sendmail", "msmtp", // e-mail externo
];

/// Subcomandos destrutivos de ferramentas de infra (`<tool> <verb>`).
const HARD_TOOL_VERBS: &[(&str, &[&str])] = &[
    ("terraform", &["apply", "destroy"]),
    ("kubectl", &["apply", "delete", "drain"]),
];

/// Classifica um comando em [`ActionClass`] por pattern-match determinístico.
///
/// **Prioridade (crítica para segurança):** `gated-hard` é testado ANTES de `gated-soft`
/// — um `git push --force origin main` casa os dois (é "git push" e é force/main); o piso
/// destrutivo precisa vencer para que `autônomo` jamais afrouxe um force-push.
///
/// Comando não reconhecido → `routine`: o pattern-match é a camada *soft* (admitidamente
/// incompleta — design §1.3); a garantia inquebrável é a custódia de segredo (W0-7).
/// Defaultar desconhecido para `ask` violaria "nunca tela em branco / local rotineiro segue".
#[must_use]
pub fn classify(cmd: &str) -> ActionClass {
    let lower = cmd.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();

    if is_gated_hard(&lower, &tokens) {
        ActionClass::GatedHard
    } else if is_gated_soft(&tokens) {
        ActionClass::GatedSoft
    } else {
        ActionClass::Routine
    }
}

/// Aplica a matriz canônica (design §1.1) — função pura, sem I/O.
#[must_use]
pub fn decide(class: ActionClass, autonomy: AutonomyLevel) -> Decision {
    match class {
        // Local-reversível: sempre permitido.
        ActionClass::Routine => Decision::Allow,
        // Soft: confirma em manual/assistido; `autônomo` AFROUXA para auto.
        ActionClass::GatedSoft => match autonomy {
            AutonomyLevel::Autonomous => Decision::Allow,
            AutonomyLevel::Manual | AutonomyLevel::Assisted => Decision::Ask,
        },
        // Hard: piso confirmável em TODOS os níveis (autônomo NUNCA afrouxa).
        ActionClass::GatedHard => Decision::Ask,
    }
}

/// Veredito completo: classifica e decide de uma vez.
#[must_use]
pub fn check_action(cmd: &str, autonomy: AutonomyLevel) -> GateDecision {
    let class = classify(cmd);
    GateDecision {
        class,
        decision: decide(class, autonomy),
    }
}

// ───────────────────────────── classificadores (pattern-match puro) ─────────────────────────────

fn is_gated_hard(lower: &str, tokens: &[&str]) -> bool {
    // 1) force-push: `git push` + flag de força.
    if is_git_push(tokens) && has_force_flag(tokens) {
        return true;
    }
    // 2) push em branch protegida (`main`/`master`).
    if is_git_push(tokens) && (tokens.contains(&"main") || tokens.contains(&"master")) {
        return true;
    }
    // 3) `rm -rf` em path FORA do workspace (absoluto/home/parent/glob raiz).
    if tokens.contains(&"rm") && rm_recursive_force(tokens) && has_dangerous_path(tokens) {
        return true;
    }
    // 4) tokens-chave de prod/pagamento/envio externo.
    if tokens.iter().any(|t| HARD_KEYWORD_TOKENS.contains(t)) {
        return true;
    }
    // 5) subcomandos destrutivos de infra (`terraform apply`, `kubectl delete`, …).
    if HARD_TOOL_VERBS
        .iter()
        .any(|(tool, verbs)| tokens.contains(tool) && verbs.iter().any(|v| tokens.contains(v)))
    {
        return true;
    }
    // 6) DROP destrutivo de banco.
    if lower.contains("drop table") || lower.contains("drop database") {
        return true;
    }
    // 7) chamada HTTP a host externo (curl/wget para fora de localhost) = envio externo.
    if (tokens.contains(&"curl") || tokens.contains(&"wget"))
        && lower.contains("http")
        && !lower.contains("localhost")
        && !lower.contains("127.0.0.1")
    {
        return true;
    }
    false
}

fn is_gated_soft(tokens: &[&str]) -> bool {
    // `git push` (já filtrado de force/main pela prioridade) → branch feature = soft.
    if is_git_push(tokens) {
        return true;
    }
    // `git commit --amend` reescreve história (recuperável) = soft.
    if tokens.contains(&"git") && tokens.contains(&"commit") && tokens.contains(&"--amend") {
        return true;
    }
    // `rm` em path do workspace (não-perigoso; o caso perigoso já foi gated-hard) = soft.
    if tokens.contains(&"rm") {
        return true;
    }
    false
}

/// `git push` em qualquer forma (`git push`, `git -C dir push`, `sudo git push`).
fn is_git_push(tokens: &[&str]) -> bool {
    tokens.contains(&"git") && tokens.contains(&"push")
}

/// Flag de força de um `git push` (`--force`, `-f`, `--force-with-lease`).
fn has_force_flag(tokens: &[&str]) -> bool {
    tokens
        .iter()
        .any(|t| *t == "--force" || *t == "-f" || t.starts_with("--force-with-lease"))
}

/// `rm` com recursivo E força — combina flags separadas (`-r -f`) e agrupadas (`-rf`/`-fr`).
fn rm_recursive_force(tokens: &[&str]) -> bool {
    let mut recursive = false;
    let mut force = false;
    for t in tokens {
        if !t.starts_with('-') {
            continue;
        }
        if *t == "--recursive" {
            recursive = true;
        }
        if *t == "--force" {
            force = true;
        }
        // flags curtas agrupadas: `-rf`, `-fr`, `-Rf`…
        if !t.starts_with("--") {
            let flags = &t[1..];
            if flags.contains('r') || flags.contains('R') {
                recursive = true;
            }
            if flags.contains('f') {
                force = true;
            }
        }
    }
    recursive && force
}

/// Há um alvo de path FORA do workspace: absoluto (`/…`), home (`~…`), parent (`..…`) ou
/// glob de raiz (`*`). Tokens de flag e o próprio `rm`/`sudo` são ignorados.
fn has_dangerous_path(tokens: &[&str]) -> bool {
    tokens.iter().any(|t| {
        if t.starts_with('-') || *t == "rm" || *t == "sudo" {
            return false;
        }
        t.starts_with('/') || t.starts_with('~') || t.starts_with("..") || *t == "*"
    })
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ---- classificação (pattern-match) ----

    #[test]
    fn force_push_to_main_is_gated_hard() {
        assert_eq!(
            classify("git push --force origin main"),
            ActionClass::GatedHard
        );
        assert_eq!(classify("git push -f origin main"), ActionClass::GatedHard);
        assert_eq!(
            classify("git push --force-with-lease origin feature/x"),
            ActionClass::GatedHard
        );
    }

    #[test]
    fn push_to_protected_branch_without_force_is_gated_hard() {
        assert_eq!(classify("git push origin main"), ActionClass::GatedHard);
        assert_eq!(classify("git push origin master"), ActionClass::GatedHard);
    }

    #[test]
    fn push_to_feature_branch_is_gated_soft() {
        assert_eq!(
            classify("git push origin feature/x"),
            ActionClass::GatedSoft
        );
        assert_eq!(classify("git push"), ActionClass::GatedSoft);
    }

    #[test]
    fn commit_amend_is_gated_soft() {
        assert_eq!(
            classify("git commit --amend -m fix"),
            ActionClass::GatedSoft
        );
    }

    #[test]
    fn rm_rf_outside_workspace_is_gated_hard() {
        assert_eq!(classify("rm -rf /"), ActionClass::GatedHard);
        assert_eq!(classify("rm -rf /etc"), ActionClass::GatedHard);
        assert_eq!(classify("sudo rm -rf /var/lib"), ActionClass::GatedHard);
        assert_eq!(classify("rm -rf ~/Documents"), ActionClass::GatedHard);
        assert_eq!(classify("rm -rf ../sibling"), ActionClass::GatedHard);
        assert_eq!(classify("rm -fr /"), ActionClass::GatedHard);
    }

    #[test]
    fn rm_in_workspace_is_gated_soft() {
        assert_eq!(classify("rm -rf build"), ActionClass::GatedSoft);
        assert_eq!(classify("rm ./tmp/file.txt"), ActionClass::GatedSoft);
        assert_eq!(classify("rm note.md"), ActionClass::GatedSoft);
    }

    #[test]
    fn prod_and_external_keywords_are_gated_hard() {
        assert_eq!(classify("terraform apply"), ActionClass::GatedHard);
        assert_eq!(
            classify("terraform destroy -auto-approve"),
            ActionClass::GatedHard
        );
        assert_eq!(classify("kubectl delete pod x"), ActionClass::GatedHard);
        assert_eq!(classify("npm run deploy"), ActionClass::GatedHard);
        assert_eq!(classify("stripe payments create"), ActionClass::GatedHard);
        assert_eq!(
            classify("curl -X POST https://api.terceiro.com/webhook"),
            ActionClass::GatedHard
        );
        assert_eq!(
            classify("psql -c 'DROP TABLE users'"),
            ActionClass::GatedHard
        );
    }

    #[test]
    fn routine_commands_are_routine() {
        assert_eq!(classify("cargo test"), ActionClass::Routine);
        assert_eq!(classify("cargo build --workspace"), ActionClass::Routine);
        assert_eq!(
            classify("cargo clippy -- -D warnings"),
            ActionClass::Routine
        );
        assert_eq!(classify("git status"), ActionClass::Routine);
        assert_eq!(classify("git diff"), ActionClass::Routine);
        assert_eq!(classify("git commit -m wip"), ActionClass::Routine);
        assert_eq!(classify("cat src/deploy.rs"), ActionClass::Routine); // 'deploy' como substring NÃO conta
        assert_eq!(classify("ls -la"), ActionClass::Routine);
        assert_eq!(
            classify("curl http://localhost:8080/health"),
            ActionClass::Routine
        ); // local
    }

    // ---- matriz (nível × classe) ----

    #[test]
    fn matrix_routine_allows_everywhere() {
        for lvl in [
            AutonomyLevel::Manual,
            AutonomyLevel::Assisted,
            AutonomyLevel::Autonomous,
        ] {
            assert_eq!(decide(ActionClass::Routine, lvl), Decision::Allow);
        }
    }

    #[test]
    fn matrix_gated_soft_confirms_except_autonomous() {
        assert_eq!(
            decide(ActionClass::GatedSoft, AutonomyLevel::Manual),
            Decision::Ask
        );
        assert_eq!(
            decide(ActionClass::GatedSoft, AutonomyLevel::Assisted),
            Decision::Ask
        );
        assert_eq!(
            decide(ActionClass::GatedSoft, AutonomyLevel::Autonomous),
            Decision::Allow,
            "autônomo afrouxa gated-soft para auto"
        );
    }

    #[test]
    fn matrix_gated_hard_confirms_at_every_level() {
        for lvl in [
            AutonomyLevel::Manual,
            AutonomyLevel::Assisted,
            AutonomyLevel::Autonomous,
        ] {
            assert_eq!(
                decide(ActionClass::GatedHard, lvl),
                Decision::Ask,
                "gated-hard é piso confirmável — autônomo NUNCA afrouxa"
            );
        }
    }

    // ---- AC-6.2 (núcleo) ----

    #[test]
    fn ac_6_2_force_push_main_is_ask_even_in_autonomous() {
        let v = check_action("git push --force origin main", AutonomyLevel::Autonomous);
        assert_eq!(v.class, ActionClass::GatedHard);
        assert_eq!(
            v.decision,
            Decision::Ask,
            "force-push em main NUNCA é allow"
        );
    }

    #[test]
    fn ac_6_2_cargo_test_is_allow() {
        let v = check_action("cargo test", AutonomyLevel::Manual);
        assert_eq!(v.class, ActionClass::Routine);
        assert_eq!(v.decision, Decision::Allow);
    }

    #[test]
    fn ac_6_2_feature_push_loosens_only_in_autonomous() {
        assert_eq!(
            check_action("git push origin feature/x", AutonomyLevel::Assisted).decision,
            Decision::Ask
        );
        assert_eq!(
            check_action("git push origin feature/x", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
    }

    #[test]
    fn ac_6_2_rm_rf_root_never_allow() {
        for lvl in [
            AutonomyLevel::Manual,
            AutonomyLevel::Assisted,
            AutonomyLevel::Autonomous,
        ] {
            assert_ne!(check_action("rm -rf /", lvl).decision, Decision::Allow);
        }
    }

    // ---- parse de autonomia ----

    #[test]
    fn parse_autonomy_accepts_ptbr_and_rejects_garbage() {
        assert_eq!(parse_autonomy("manual"), Ok(AutonomyLevel::Manual));
        assert_eq!(parse_autonomy("assistido"), Ok(AutonomyLevel::Assisted));
        assert_eq!(parse_autonomy("autonomo"), Ok(AutonomyLevel::Autonomous));
        assert_eq!(parse_autonomy("AUTÔNOMO"), Ok(AutonomyLevel::Autonomous));
        assert!(parse_autonomy("ludico").is_err());
    }
}
