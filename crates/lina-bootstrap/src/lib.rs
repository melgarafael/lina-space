//! **W3-2 · Bootstrap turno-0 — renderer da DOUTRINA CANÔNICA.** Dado o roster do workspace
//! (papéis via `lina-role-discovery`) + estado (vault, autonomia), GERA:
//! - `<workspace>/CLAUDE.md` (Claude Code, tier 1) + `AGENTS.md` (Codex) + `GEMINI.md` (Gemini),
//!   preenchendo os **13 placeholders** dos templates canônicos do Spec Writer
//!   (`assets/lina-doctrine/*.md`) — mesmo conteúdo, só o gatilho de bootstrap muda;
//! - o **bloco de estado vivo** que `lina whoami --bootstrap` imprime (e o JSON do hook
//!   `SessionStart` do Claude Code).
//!
//! A **doutrina canônica** (`assets/lina-doctrine/`) é a FONTE DA VERDADE — embutida via
//! `include_str!`. Puro Rust, **ZERO LLM**. Nomes de terminal são **sanitizados** antes de entrar
//! no markdown (sem newline/`{{`/backtick → estrutura dos 8 blocos à prova de injeção).

use std::path::Path;

use lina_role_discovery::{RegistryError, RoleAssignment, RoleRegistry};
use serde::{Deserialize, Serialize};

/// W3-6 (AC-6.3): lógica pura do hook `PreToolUse` do Claude Code (gate de execução tier 1).
pub mod pretooluse;
pub use pretooluse::{autonomy_from_env, pretooluse_output, AUTONOMY_ENV};

/// Templates CANÔNICOS embutidos (fonte da verdade do Spec Writer). Mesmos 13 placeholders; só o
/// gatilho de bootstrap muda por CLI. Um rebuild re-embute se o Spec Writer atualizar os arquivos.
const CLAUDE_TEMPLATE: &str = include_str!("../../../assets/lina-doctrine/CLAUDE.md");
const AGENTS_TEMPLATE: &str = include_str!("../../../assets/lina-doctrine/AGENTS.md");
const GEMINI_TEMPLATE: &str = include_str!("../../../assets/lina-doctrine/GEMINI.md");

/// Skill que entra SEMPRE (ensina o agente a falar com os colegas).
pub const ALWAYS_SKILL: &str = "lina-agent-bus";
/// Caminho default do plano compartilhado.
pub const DEFAULT_PLAN_PATH: &str = ".lina/plan.md";
/// Limite de tamanho de um nome de terminal (sanitização).
const MAX_NAME_LEN: usize = 64;

/// Nível de autonomia do workspace (doc §6 / design §1 bloco 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Autonomy {
    /// Não delega sozinho: só sugere.
    Manual,
    /// Propõe delegações; o Maestro confirma.
    Assisted,
    /// Delega e age sozinho dentro do orçamento.
    Autonomous,
}

impl Autonomy {
    /// Rótulo curto em pt-br (`manual`/`assistido`/`autonomo`) — casa com o vocabulário da doutrina.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Autonomy::Manual => "manual",
            Autonomy::Assisted => "assistido",
            Autonomy::Autonomous => "autonomo",
        }
    }

    /// Descrição do que o nível permite (linha do `whoami`).
    #[must_use]
    pub fn desc(&self) -> &'static str {
        match self {
            Autonomy::Manual => "você NÃO delega sozinho; só sugere e o usuário/Maestro executa.",
            Autonomy::Assisted => "proponha delegações; o Maestro confirma antes de disparar.",
            Autonomy::Autonomous => {
                "você delega e age sozinho dentro do orçamento; só ações irreversíveis pedem \
                 confirmação."
            }
        }
    }

    /// Teto de delegações/turno: **0 em manual**, **8** nos demais (defaults canônicos).
    #[must_use]
    pub fn max_delegations(&self) -> u32 {
        match self {
            Autonomy::Manual => 0,
            _ => 8,
        }
    }
}

/// Estado do workspace para o bootstrap de **UM** terminal. Serializável: o app escreve
/// `<cwd>/.lina/bootstrap.json`; o bin `lina whoami` lê de lá. O app REESCREVE a cada mudança de
/// roster (o arquivo é a âncora).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapInput {
    /// Nome deste terminal ("você") — ex.: `"@Dev Backend"`, `"Terminal A"`.
    pub terminal_name: String,
    /// Nomes de TODOS os terminais do workspace (inclui você; a ordem é a do canvas).
    pub roster: Vec<String>,
    /// Caminho do vault Obsidian.
    pub vault_path: String,
    /// Nível de autonomia atual do workspace.
    pub autonomy: Autonomy,
    /// Caminho do plano compartilhado (default `.lina/plan.md`).
    #[serde(default = "default_plan_path")]
    pub plan_path: String,
    /// Nome do workspace (default canônico — vem de `workspace.json` no futuro).
    #[serde(default = "default_workspace_name")]
    pub workspace_name: String,
    /// Label pt-br do preset de foco (default — vem de `workspace.json`).
    #[serde(default = "default_focus_preset")]
    pub focus_preset_label: String,
}

fn default_plan_path() -> String {
    DEFAULT_PLAN_PATH.to_string()
}
fn default_workspace_name() -> String {
    "Lina Space".to_string()
}
fn default_focus_preset() -> String {
    "geral".to_string()
}

impl BootstrapInput {
    /// Construtor com defaults canônicos para `plan_path`/`workspace_name`/`focus_preset_label`.
    #[must_use]
    pub fn new(
        terminal_name: impl Into<String>,
        roster: Vec<String>,
        vault_path: impl Into<String>,
        autonomy: Autonomy,
    ) -> Self {
        Self {
            terminal_name: terminal_name.into(),
            roster,
            vault_path: vault_path.into(),
            autonomy,
            plan_path: DEFAULT_PLAN_PATH.to_string(),
            workspace_name: default_workspace_name(),
            focus_preset_label: default_focus_preset(),
        }
    }

    /// Override do contexto do workspace (nome + label do preset).
    #[must_use]
    pub fn with_context(
        mut self,
        workspace_name: impl Into<String>,
        focus_preset_label: impl Into<String>,
    ) -> Self {
        self.workspace_name = workspace_name.into();
        self.focus_preset_label = focus_preset_label.into();
        self
    }
}

/// **Sanitiza um nome de terminal/colega** antes de entrar no markdown/template: remove caracteres
/// de controle (newline quebraria os 8 blocos), remove backtick (quebraria o code-span), neutraliza
/// os delimitadores `{{`/`}}` (evita placeholder não-substituído) e limita o tamanho. Defesa no
/// ÚLTIMO ponto antes da doutrina — independe de validação do chamador.
#[must_use]
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        // Remove controle (newline → quebraria os blocos) e os metacaracteres ESTRUTURAIS de
        // markdown/HTML (`#`/`*`/`[`/`]`/`|`/`<`/`>`/backtick) → o nome fica seguro em QUALQUER
        // posição da doutrina (header, tabela, code-span).
        .filter(|c| !c.is_control() && !matches!(c, '`' | '#' | '*' | '[' | ']' | '|' | '<' | '>'))
        .take(MAX_NAME_LEN)
        .collect();
    // Neutraliza os delimitadores de placeholder (evita {{x}} não-substituído / dupla substituição).
    cleaned.replace("{{", "{ {").replace("}}", "} }")
}

/// Frase de missão por papel (placeholder `role_mission`). Fallback genérico para papéis fora do
/// mapa (o role-discovery pode evoluir sem quebrar o renderer).
fn role_mission(role: &str) -> &'static str {
    match role {
        "MAESTRO" => "Você coordena o time: prioriza, distribui e mantém o plano; o que não é seu, delega ao colega certo.",
        "ARQUITETO" => "Você decide os contratos e a arquitetura ANTES dos outros implementarem (APIs, modelos, abordagem).",
        "BACKEND" => "Você implementa as APIs, o banco e a lógica de servidor do time.",
        "FRONTEND" => "Você implementa as telas e a interface do produto.",
        "LLM_ENGINEER" => "Você constrói o core e as integrações de IA do produto.",
        "DATA_ENGINEER" => "Você cuida dos pipelines e da modelagem de dados.",
        "UIUX_DESIGNER" => "Você desenha a interface e a experiência antes da implementação.",
        "WRITER" => "Você escreve a documentação, a copy e as specs do time.",
        "QA" => "Você valida e testa o que os colegas entregam — qualidade é o seu papel.",
        "BUG_FIXER" => "Você investiga e conserta os bugs que aparecem.",
        "CURADOR" => "Você cuida do conhecimento e do vault (e é o único que escreve em `Tino/`).",
        "AUTOMATOR" => "Você cria as automações, webhooks e scripts do time.",
        "DEVELOPER" => "Você desenvolve o que for preciso e delega o que for de outro papel.",
        _ => "Você colabora no time conforme o seu papel.",
    }
}

/// Frase curta "quando delegar a ele" por papel (coluna da `colleagues_table`).
fn role_blurb(role: &str) -> &'static str {
    match role {
        "MAESTRO" => "coordena o time/plano; delegue orquestração e prioridades",
        "ARQUITETO" => "decide contratos/abordagem antes de você implementar",
        "BACKEND" => "implementa APIs, banco e lógica de servidor",
        "FRONTEND" => "implementa telas e UI",
        "LLM_ENGINEER" => "constrói o core/IA e integrações de modelo",
        "DATA_ENGINEER" => "pipelines e modelagem de dados",
        "UIUX_DESIGNER" => "design de interface e experiência",
        "WRITER" => "documentação, copy e specs",
        "QA" => "valida e testa o que você entregou",
        "BUG_FIXER" => "investiga e conserta bugs",
        "CURADOR" => "curadoria de conhecimento e do vault",
        "AUTOMATOR" => "automações, webhooks e scripts",
        "DEVELOPER" => "desenvolvimento geral",
        _ => "colabora no time",
    }
}

/// Skills do papel + `lina-agent-bus` (sempre, sem duplicar) — usado no `whoami` (cujo formato NÃO
/// anexa a skill por conta própria).
fn skills_with_bus(a: &RoleAssignment) -> String {
    let mut skills: Vec<String> = a.skills.clone();
    if !skills.iter().any(|s| s == ALWAYS_SKILL) {
        skills.push(ALWAYS_SKILL.to_string());
    }
    skills.join(", ")
}

/// SÓ as skills do papel (sem `lina-agent-bus`) — para o placeholder `{{role_skills_list}}`, que o
/// template já segue com ", lina-agent-bus" (não duplicar).
fn skills_only(a: &RoleAssignment) -> String {
    a.skills
        .iter()
        .filter(|s| *s != ALWAYS_SKILL)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

/// Pastas graváveis do vault (placeholder `vault_writable_paths`) — default conservador derivado do
/// caminho do vault (o `vault.json` refina no futuro).
fn vault_writable_paths(vault: &str) -> String {
    format!("`{vault}/Lina/` (o resto do vault é leitura)")
}

/// Caminho da área "Tino" no vault (placeholder `vault_tino_path`).
fn vault_tino_path(vault: &str) -> String {
    format!("{vault}/Tino")
}

/// **Motor de bootstrap.** Detém o `RoleRegistry` (compila os regexes uma vez) e reusa.
pub struct Bootstrapper {
    registry: RoleRegistry,
}

impl Bootstrapper {
    /// Constrói com o registry default embutido (sem I/O).
    ///
    /// # Errors
    /// Propaga [`RegistryError`] se o YAML default não parsear (não deve ocorrer em produção).
    pub fn new() -> Result<Self, RegistryError> {
        Ok(Self {
            registry: RoleRegistry::with_defaults()?,
        })
    }

    /// Os colegas de `input.terminal_name`: todos do roster MENOS ele, com papel resolvido.
    fn colleagues(&self, input: &BootstrapInput) -> Vec<(String, RoleAssignment)> {
        input
            .roster
            .iter()
            .filter(|n| *n != &input.terminal_name)
            .map(|n| (n.clone(), self.registry.infer_role(n)))
            .collect()
    }

    /// Tabela de colegas em markdown (placeholder `colleagues_table`). Nomes em backticks
    /// (escapam markdown). Linha: `| @Nome | PAPEL | quando delegar |`.
    fn colleagues_table(&self, input: &BootstrapInput) -> String {
        let colleagues = self.colleagues(input);
        if colleagues.is_empty() {
            return "_(nenhum colega no workspace ainda — peça ao usuário para abrir mais terminais)_"
                .to_string();
        }
        let mut s = String::from("| Colega | Papel | Quando delegar a ele |\n|---|---|---|\n");
        for (name, role) in &colleagues {
            s.push_str(&format!(
                "| `{}` | {} | {} |\n",
                sanitize_name(name),
                role.role,
                role_blurb(&role.role)
            ));
        }
        s.trim_end().to_string()
    }

    /// Colegas inline (uma linha; bloco do `whoami`).
    fn colleagues_inline(&self, input: &BootstrapInput) -> String {
        let colleagues = self.colleagues(input);
        if colleagues.is_empty() {
            return "(nenhum colega ainda)".to_string();
        }
        colleagues
            .iter()
            .map(|(name, role)| {
                format!(
                    "{} ({}) — {}",
                    sanitize_name(name),
                    role.role,
                    role_blurb(&role.role)
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Preenche os **13 placeholders** canônicos de um `template`. O `{{placeholder}}` LITERAL do
    /// corpo (exemplo) NÃO está na lista → permanece (é exemplo, não preenchimento).
    fn render_template(&self, template: &str, input: &BootstrapInput) -> String {
        let me = self.registry.infer_role(&input.terminal_name);
        let name = sanitize_name(&input.terminal_name);
        let max_deleg = input.autonomy.max_delegations().to_string();
        template
            .replace("{{workspace_name}}", &sanitize_name(&input.workspace_name))
            .replace(
                "{{focus_preset_label}}",
                &sanitize_name(&input.focus_preset_label),
            )
            .replace("{{role_canonical}}", &me.role)
            .replace("{{terminal_name}}", &name)
            .replace("{{role_mission}}", role_mission(&me.role))
            .replace("{{role_skills_list}}", &skills_only(&me))
            .replace("{{colleagues_table}}", &self.colleagues_table(input))
            .replace("{{autonomy_level}}", input.autonomy.label())
            .replace("{{max_delegations_per_turn}}", &max_deleg)
            .replace("{{max_delegation_depth}}", "4")
            .replace("{{fanout_gate_threshold}}", "3")
            .replace(
                "{{vault_writable_paths}}",
                &vault_writable_paths(&input.vault_path),
            )
            .replace("{{vault_tino_path}}", &vault_tino_path(&input.vault_path))
    }

    /// `<workspace>/CLAUDE.md` (Claude Code, tier 1).
    #[must_use]
    pub fn doctrine(&self, input: &BootstrapInput) -> String {
        self.render_template(CLAUDE_TEMPLATE, input)
    }
    /// `<workspace>/AGENTS.md` (Codex, tier 2).
    #[must_use]
    pub fn agents_doctrine(&self, input: &BootstrapInput) -> String {
        self.render_template(AGENTS_TEMPLATE, input)
    }
    /// `<workspace>/GEMINI.md` (Gemini, tier 2).
    #[must_use]
    pub fn gemini_doctrine(&self, input: &BootstrapInput) -> String {
        self.render_template(GEMINI_TEMPLATE, input)
    }

    /// **Bloco de estado vivo** que `lina whoami --bootstrap` imprime (design §1).
    #[must_use]
    pub fn whoami(&self, input: &BootstrapInput) -> String {
        let me = self.registry.infer_role(&input.terminal_name);
        format!(
            "=== Lina Space BOOTSTRAP ===\n\
             Voce e o terminal \"{name}\" -> papel {role}.\n\
             Skills a carregar AGORA: {skills}.\n\
             VAULT OBSIDIAN: {vault}  (use `lina vault search \"termo\"`; voce JA tem acesso; nao peca o caminho)\n\
             COLEGAS NESTE WORKSPACE: {colegas}\n\
             PLANO COMPARTILHADO: {plan}  (use `lina plan read`)\n\
             AUTONOMIA: {auton} ({auton_desc})\n\
             PRIMEIRO PASSO OBRIGATORIO: 1) carregue skills  2) `lina handshake`  3) `lina plan read` (item @owner:voce?)\n\
             === FIM BOOTSTRAP ===",
            name = sanitize_name(&input.terminal_name),
            role = me.role,
            skills = skills_with_bus(&me),
            vault = input.vault_path,
            colegas = self.colleagues_inline(input),
            plan = input.plan_path,
            auton = input.autonomy.label(),
            auton_desc = input.autonomy.desc(),
        )
    }

    /// JSON do hook `SessionStart` do Claude Code: `hookSpecificOutput.additionalContext` com o
    /// bloco de estado vivo. **Nunca stdout cru**.
    #[must_use]
    pub fn whoami_hook_json(&self, input: &BootstrapInput) -> String {
        let payload = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": self.whoami(input),
            }
        });
        serde_json::to_string(&payload).unwrap_or_else(|_| String::from("{}"))
    }

    /// Escreve os arquivos de bootstrap de UM terminal no `dir` (cwd dele), pelos **dois canais**:
    /// - doutrina (canal 1, à prova de falhas): `CLAUDE.md` + `AGENTS.md` + `GEMINI.md` (um por CLI);
    /// - `.lina/bootstrap.json` (estado vivo serializado, lido por `lina whoami`);
    /// - `.claude/settings.json` (hook `SessionStart`, canal 2 — só Claude Code).
    ///
    /// # Errors
    /// Propaga erros de I/O (criar dirs / escrever arquivos).
    pub fn write_terminal_files(
        &self,
        dir: &Path,
        input: &BootstrapInput,
        lina_bin: &str,
    ) -> std::io::Result<()> {
        self.write_terminal_files_with_hooks(dir, input, lina_bin, None)
    }

    /// F1-1-3: como [`Self::write_terminal_files`], instalando TAMBÉM os hooks de
    /// observabilidade no settings quando o app fornece a fiação (porta do listener +
    /// token do nó). `None` = comportamento histórico byte-idêntico (CLI sem hooks /
    /// listener ausente — degradação por omissão, nada quebra).
    ///
    /// # Errors
    /// Propaga erros de I/O (criar dirs / escrever arquivos).
    pub fn write_terminal_files_with_hooks(
        &self,
        dir: &Path,
        input: &BootstrapInput,
        lina_bin: &str,
        wiring: Option<&HookWiring>,
    ) -> std::io::Result<()> {
        let lina = dir.join(".lina");
        let claude = dir.join(".claude");
        std::fs::create_dir_all(&lina)?;
        std::fs::create_dir_all(&claude)?;
        std::fs::write(dir.join("CLAUDE.md"), self.doctrine(input))?;
        std::fs::write(dir.join("AGENTS.md"), self.agents_doctrine(input))?;
        std::fs::write(dir.join("GEMINI.md"), self.gemini_doctrine(input))?;
        let json = serde_json::to_string_pretty(input).unwrap_or_else(|_| String::from("{}"));
        std::fs::write(lina.join("bootstrap.json"), json)?;
        std::fs::write(
            claude.join("settings.json"),
            hook_settings_json_with_observability(lina_bin, wiring),
        )?;
        Ok(())
    }
}

/// JSON do `<cwd>/.claude/settings.json` com os DOIS hooks do Claude Code:
/// - `SessionStart` → `<lina_bin> whoami --bootstrap` (estado vivo turno-0, W3-2);
/// - `PreToolUse` (matcher `Bash|Skill`) → `<lina_bin> guard --pretooluse`:
///   - **`Bash`** = o gate de execução DURO (W3-6 AC-6.3 — toda chamada `Bash` passa
///     pelo gate antes de rodar; classify+decide pela matriz, ZERO LLM);
///   - **`Skill`** (R2c-2) = o guard de skill ESTRANGEIRA — `Skill(maestri*)` é NEGADA
///     com redirect aos verbos `lina` (a skill global do Maestri existe na máquina do
///     fundador e o 2.1.x não tem deny por nome de skill; o `deny` do PreToolUse é o
///     enforcement). Skills legítimas (lina-agent-bus, do papel) passam.
///
/// O caminho do bin é **single-quoted** (paths podem ter espaços, ex.: `.../einstein workspace/...`)
/// com escape de aspas — neutraliza metacaracteres de shell. O matcher casa `Bash|Skill`
/// (regex do Claude Code); o `pretooluse_output` roteia por `tool_name` e devolve `allow`
/// para qualquer outra tool não-coberta (Write/Edit são `local-reversible`).
#[must_use]
pub fn hook_settings_json(lina_bin: &str) -> String {
    hook_settings_json_with_observability(lina_bin, None)
}

/// **F1-1-3 — fiação dos hooks de observabilidade** no spawn de um terminal: a porta do
/// listener loopback do app (`lina-hooks::HookListener`) + o TOKEN opaco de atribuição
/// deste nó (`register_node`). Primitivos de propósito — o bootstrap não depende do
/// crate do pipeline; quem liga as pontas é o app.
#[derive(Debug, Clone)]
pub struct HookWiring {
    /// Porta efêmera do listener (sempre `127.0.0.1` — inv#2; o host não é configurável).
    pub port: u16,
    /// Token opaco deste nó — a ÚNICA fonte de atribuição do POST (nunca o payload).
    pub token: String,
}

/// Os kinds de observabilidade instalados no settings — ESPELHO de
/// `lina_hooks::HookKind::ALL` (duplicado de propósito: o bootstrap não depende do
/// pipeline; mudou lá, mude aqui — o teste de settings denuncia divergência de URL).
const OBSERVABILITY_HOOK_KINDS: [&str; 6] = [
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "SubagentStart",
    "SubagentStop",
    "Stop",
];

/// **ADR 0025 — allowlist do permission-mode EXPLICITAMENTE pedido** (reverte o pin de
/// produção do R2b). Esta função só governa o caminho de pedido EXPLÍCITO (futuro mapa
/// por papel / gate-test): `default`, `plan` (papéis read-only), `acceptEdits` (opt-in)
/// e **`bypassPermissions`** passam — bypass É VÁLIDO **quando explicitamente pedido**,
/// porque a fonte é a escolha do humano (config do app / flag de teste), **nunca um campo
/// de agente** (não há caminho de auto-bypass do agente; ver `hook_settings_json_with_mode`).
/// Valores DESCONHECIDOS (`dontAsk`/`auto`/lixo) degradam para `default` (fail-safe).
/// `None` (sem pedido) → `default` defensivo, mas o caminho de PRODUÇÃO nem chama esta
/// função: ele **omite** a chave `permissions` para o `~/.claude` do usuário fluir.
#[must_use]
pub fn sanitized_permission_mode(requested: Option<&str>) -> &str {
    match requested.map(str::trim) {
        Some(m @ ("default" | "plan" | "acceptEdits" | "bypassPermissions")) => m,
        _ => "default",
    }
}

/// Como [`hook_settings_json`], com os hooks de OBSERVABILIDADE opcionais (F1-1-3):
/// `wiring = Some` adiciona a cada kind um handler **HTTP assíncrono** (13.10 achado 1:
/// handler HTTP do Claude Code, fev/2026; `async` roda em background sem bloquear o
/// turno do agente) → `http://127.0.0.1:<porta>/hook/<token>/<kind>`. Os handlers
/// `command` históricos (SessionStart turno-0 W3-2 + PreToolUse gate W3-6) permanecem
/// SEMPRE — observabilidade nunca substitui o gate. `wiring = None` (CLI sem hooks /
/// app sem listener) → MESMOS hooks do histórico (degradação por omissão).
///
/// **ADR 0025 (reverte R2b):** este é o caminho de PRODUÇÃO (`permission_mode == None`)
/// e por isso o settings **NÃO** fixa `permissions` — o `~/.claude/settings.json` do
/// usuário (incl. `bypassPermissions`, se ele escolheu) FLUI por merge. A segurança
/// durável do Lina é o guard W3-6 + custódia (ADR 0004/0006), **não** o permission-mode
/// do claude; o toast F1-1 só se aplica quando o usuário roda um modo que pede permissão.
#[must_use]
pub fn hook_settings_json_with_observability(
    lina_bin: &str,
    wiring: Option<&HookWiring>,
) -> String {
    hook_settings_json_with_mode(lina_bin, wiring, None)
}

/// Como [`hook_settings_json_with_observability`], com o permission-mode POR TERMINAL.
/// **ADR 0025:** `permission_mode == None` é PRODUÇÃO — **omite** a chave `permissions`
/// inteira (o `~/.claude` do usuário decide o defaultMode). `permission_mode == Some(m)`
/// é um pedido EXPLÍCITO (futuro mapa por papel / gate-test): fixa `defaultMode =`
/// [`sanitized_permission_mode`]`(Some(m))` — `bypassPermissions` é honrado **quando
/// explícito**; lixo degrada para `default`. O `m` NUNCA vem de campo de agente (vem da
/// config do app / flag de teste) → não há caminho de auto-bypass do agente.
#[must_use]
pub fn hook_settings_json_with_mode(
    lina_bin: &str,
    wiring: Option<&HookWiring>,
    permission_mode: Option<&str>,
) -> String {
    let quoted = format!("'{}'", lina_bin.replace('\'', "'\\''"));
    let session_cmd = format!("{quoted} whoami --bootstrap");
    let pretooluse_cmd = format!("{quoted} guard --pretooluse");
    let mut v = serde_json::json!({
        "hooks": {
            "SessionStart": [
                { "hooks": [ { "type": "command", "command": session_cmd } ] }
            ],
            // R2c-2: matcher alargado p/ `Bash|Skill` — o MESMO gate cobre o gate de
            // execução Bash (W3-6) E o guard de skill estrangeira (Skill(maestri*) → deny).
            "PreToolUse": [
                { "matcher": "Bash|Skill", "hooks": [ { "type": "command", "command": pretooluse_cmd } ] }
            ]
        }
    });
    // ADR 0025: só um pedido EXPLÍCITO (Some) escreve `permissions`; produção (None) omite
    // a chave para o `~/.claude/settings.json` do usuário fluir por merge. Os hooks (turno-0
    // + guard W3-6) já estão acima e ficam SEMPRE — a segurança durável não depende disto.
    if let Some(requested) = permission_mode {
        let mode = sanitized_permission_mode(Some(requested));
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "permissions".to_string(),
                serde_json::json!({ "defaultMode": mode }),
            );
        }
    }
    if let Some(w) = wiring {
        if let Some(hooks) = v["hooks"].as_object_mut() {
            for kind in OBSERVABILITY_HOOK_KINDS {
                let url = format!("http://127.0.0.1:{}/hook/{}/{kind}", w.port, w.token);
                let entry = serde_json::json!({
                    "hooks": [ { "type": "http", "url": url, "async": true } ]
                });
                match hooks.get_mut(kind).and_then(|k| k.as_array_mut()) {
                    // Kind já existe (PreToolUse do gate): ACRESCENTA a entry http.
                    Some(arr) => arr.push(entry),
                    None => {
                        hooks.insert(kind.to_string(), serde_json::json!([entry]));
                    }
                }
            }
        }
    }
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| String::from("{}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> BootstrapInput {
        BootstrapInput::new(
            name,
            vec!["@Dev Backend".into(), "@Arquiteto".into(), "@QA".into()],
            "/Users/dev/vault",
            Autonomy::Assisted,
        )
    }

    /// Os 8 marcadores de BLOCO da doutrina canônica, em ordem.
    const BLOCKS: [&str; 8] = [
        "<!-- ===== BLOCO 1 ",
        "<!-- ===== BLOCO 2 ",
        "<!-- ===== BLOCO 3 ",
        "<!-- ===== BLOCO 4 ",
        "<!-- ===== BLOCO 5 ",
        "<!-- ===== BLOCO 6 ",
        "<!-- ===== BLOCO 7 ",
        "<!-- ===== BLOCO 8 ",
    ];

    /// `{{...}}` restante depois de remover os EXEMPLOS LITERAIS da doutrina (`{{...}}` e
    /// `{{placeholder}}`) — esses são documentação no corpo, não placeholders de preenchimento.
    fn leftover_placeholders(md: &str) -> bool {
        md.replace("{{...}}", "")
            .replace("{{placeholder}}", "")
            .contains("{{")
    }

    /// **GATE W3-2 (canônico):** o CLAUDE.md tem os 8 BLOCOS na ORDEM, `role_canonical=BACKEND`, e
    /// TODOS os 13 placeholders resolvidos (nenhum `{{...}}` exceto o exemplo literal).
    #[test]
    fn canonical_doctrine_8_blocks_in_order_all_placeholders_resolved() {
        let bs = Bootstrapper::new().expect("registry");
        let md = bs.doctrine(&sample("@Dev Backend"));

        assert!(
            md.contains("papel: BACKEND") || md.contains("BACKEND"),
            "role resolvido"
        );
        assert!(
            !leftover_placeholders(&md),
            "nenhum {{...}} além do exemplo literal: {md}"
        );
        // 13 placeholders preenchidos (spot-check de valores):
        assert!(md.contains("Lina Space"), "workspace_name");
        assert!(
            md.contains("@Arquiteto") && md.contains("@QA"),
            "colleagues_table com colegas reais"
        );
        assert!(
            md.contains("lina-agent-bus"),
            "skills + lina-agent-bus (template anexa)"
        );
        assert!(
            md.contains("/Users/dev/vault/Tino"),
            "vault_tino_path derivado"
        );
        assert!(md.contains("assistido"), "autonomy_level");
        // 8 blocos na ordem.
        let mut last = 0usize;
        for (i, marker) in BLOCKS.iter().enumerate() {
            let pos = md
                .find(marker)
                .unwrap_or_else(|| panic!("BLOCO {} ausente", i + 1));
            assert!(pos >= last, "BLOCO {} fora de ordem", i + 1);
            last = pos;
        }
    }

    /// AGENTS.md e GEMINI.md (tier 2) também resolvem TODOS os placeholders (não têm o exemplo
    /// literal `{{placeholder}}`, então ZERO `{{...}}`).
    #[test]
    fn tier2_doctrines_have_no_unresolved_placeholders() {
        let bs = Bootstrapper::new().expect("registry");
        let input = sample("@Dev Backend");
        for md in [bs.agents_doctrine(&input), bs.gemini_doctrine(&input)] {
            assert!(
                !leftover_placeholders(&md),
                "tier-2 sem placeholder pendente"
            );
            assert!(md.contains("BACKEND") && md.contains("lina-agent-bus"));
        }
    }

    /// **R2c (bug de tela 21:40) — DOUTRINA anti-colisão de orquestrador:** as 3 fichas
    /// (Claude/Codex/Gemini) dizem explicitamente que skills/CLIs de OUTROS
    /// orquestradores (Maestri) NÃO funcionam e que a comunicação é SÓ pelos verbos
    /// `lina`; e NÃO contêm mais o `alias maestri` (que reforçava a skill estrangeira na
    /// máquina do fundador). Guarda contra regressão no re-render.
    #[test]
    fn doctrine_warns_against_foreign_orchestrators() {
        let bs = Bootstrapper::new().expect("registry");
        let input = sample("@Dev Backend");
        for md in [
            bs.doctrine(&input),
            bs.agents_doctrine(&input),
            bs.gemini_doctrine(&input),
        ] {
            assert!(
                md.contains("Maestri") && md.contains("NÃO funciona"),
                "a ficha alerta que orquestrador estrangeiro não funciona aqui"
            );
            assert!(
                md.contains("lina-agent-bus"),
                "a ficha aponta a skill oficial de A2A"
            );
            assert!(
                !md.contains("alias `maestri`"),
                "o 'alias maestri' (reforço do bug) foi removido"
            );
        }
    }

    /// **Injeção barrada:** um nome com newline + `## header` + `{{placeholder}}` NÃO quebra os 8
    /// blocos nem deixa `{{` (sanitização no renderer).
    #[test]
    fn malicious_names_cannot_break_structure_or_leak_placeholders() {
        let bs = Bootstrapper::new().expect("registry");
        let mut input = sample("Terminal\n## INJETADO {{vault_path");
        input.roster[0] = input.terminal_name.clone();
        let md = bs.doctrine(&input);
        assert!(
            !leftover_placeholders(&md),
            "nome com {{ não vaza placeholder"
        );
        assert!(
            !md.contains("## INJETADO"),
            "newline+header do nome não vira bloco"
        );
        // Continua tendo os 8 blocos canônicos na ordem.
        let mut last = 0;
        for m in BLOCKS {
            let p = md.find(m).expect("bloco presente");
            assert!(p >= last);
            last = p;
        }
    }

    /// `whoami` lista os COLEGAS REAIS (não o próprio terminal).
    #[test]
    fn whoami_lists_real_colleagues() {
        let bs = Bootstrapper::new().expect("registry");
        let out = bs.whoami(&sample("@Dev Backend"));
        assert!(out.starts_with("=== Lina Space BOOTSTRAP ==="));
        assert!(out.contains("@Dev Backend") && out.contains("BACKEND"));
        assert!(out.contains("@Arquiteto") && out.contains("ARQUITETO"));
        assert!(out.contains("@QA") && out.contains("(QA)"));
        let colegas = out.lines().find(|l| l.starts_with("COLEGAS")).unwrap_or("");
        assert!(
            !colegas.contains("@Dev Backend"),
            "não é colega de si mesmo"
        );
    }

    /// Renomear um colega REESCREVE a doutrina (diff): nome novo entra, antigo sai.
    #[test]
    fn renaming_a_colleague_changes_the_doctrine() {
        let bs = Bootstrapper::new().expect("registry");
        let before = bs.doctrine(&sample("@Dev Backend"));
        let mut renamed = sample("@Dev Backend");
        // Nome único que NÃO aparece nos exemplos do template (o template cita @QA no bloco TOM).
        for n in &mut renamed.roster {
            if n == "@QA" {
                *n = "@Zephyr".into();
            }
        }
        let after = bs.doctrine(&renamed);
        assert_ne!(before, after, "renomear muda a doutrina (diff observável)");
        assert!(!before.contains("@Zephyr"), "antes não tinha o nome novo");
        assert!(
            after.contains("@Zephyr"),
            "depois tem o nome novo na tabela de colegas"
        );
    }

    /// `max_delegations_per_turn` = 0 em manual, 8 nos demais.
    #[test]
    fn manual_autonomy_zeroes_delegation_budget() {
        let bs = Bootstrapper::new().expect("registry");
        let mut input = sample("@Dev Backend");
        input.autonomy = Autonomy::Manual;
        let md = bs.doctrine(&input);
        assert!(
            md.contains("**0**") || md.contains(" 0 "),
            "teto 0 em manual"
        );
        input.autonomy = Autonomy::Autonomous;
        assert!(bs.doctrine(&input).contains("**8**") || bs.doctrine(&input).contains(" 8 "));
    }

    /// `BootstrapInput` faz round-trip JSON; campos novos têm default ao desserializar sem eles.
    #[test]
    fn bootstrap_input_json_roundtrips_with_defaults() {
        let input = sample("@Dev Backend");
        let json = serde_json::to_string(&input).expect("serialize");
        let back: BootstrapInput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(input, back);
        let minimal =
            r#"{"terminal_name":"A","roster":["A"],"vault_path":"/v","autonomy":"manual"}"#;
        let parsed: BootstrapInput = serde_json::from_str(minimal).expect("deserialize minimal");
        assert_eq!(parsed.plan_path, DEFAULT_PLAN_PATH);
        assert_eq!(parsed.workspace_name, "Lina Space");
        assert_eq!(parsed.focus_preset_label, "geral");
    }

    /// O JSON do hook carrega o bloco em `additionalContext`.
    #[test]
    fn hook_json_wraps_the_block() {
        let bs = Bootstrapper::new().expect("registry");
        let v: serde_json::Value =
            serde_json::from_str(&bs.whoami_hook_json(&sample("@Dev Backend"))).expect("json");
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("ctx");
        assert!(ctx.contains("@Arquiteto"));
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
    }

    // ── F1-1-3: hooks de observabilidade (handlers HTTP → listener do lina-hooks) ──

    /// **R2c-2: o matcher PreToolUse cobre `Bash|Skill`** — o gate de execução (Bash) e
    /// o guard de skill estrangeira (Skill) na MESMA entry, apontando para o mesmo
    /// `guard --pretooluse`. Vale com e sem wiring de observabilidade.
    #[test]
    fn pretooluse_matcher_covers_bash_and_skill() {
        for wiring in [
            None,
            Some(HookWiring {
                port: 1,
                token: "t".into(),
            }),
        ] {
            let j = hook_settings_json_with_observability("/abs/lina", wiring.as_ref());
            let v: serde_json::Value = serde_json::from_str(&j).expect("json");
            let pre = v["hooks"]["PreToolUse"].as_array().expect("PreToolUse");
            let guarded = pre.iter().any(|e| {
                e["matcher"] == "Bash|Skill"
                    && e["hooks"].as_array().is_some_and(|hs| {
                        hs.iter().any(|h| {
                            h["command"]
                                .as_str()
                                .is_some_and(|c| c.contains("guard --pretooluse"))
                        })
                    })
            });
            assert!(
                guarded,
                "matcher Bash|Skill com o guard (wiring={:?}): {j}",
                wiring.is_some()
            );
        }
    }

    /// Sem fiação (porta/token ausentes — app sem listener, CLI sem hooks, testes
    /// antigos): o settings é BYTE-IDÊNTICO ao histórico — degradação por omissão.
    #[test]
    fn settings_without_wiring_is_unchanged() {
        let bin = "/usr/local/bin/lina";
        assert_eq!(
            hook_settings_json_with_observability(bin, None),
            hook_settings_json(bin),
            "sem wiring nada muda (compat com o gerado do W3-2/W3-6)"
        );
    }

    /// Com fiação: cada um dos 6 kinds de observabilidade ganha um handler **HTTP
    /// assíncrono** (13.10: handler HTTP do Claude Code, fev/2026; async não bloqueia o
    /// turno) apontando para `http://127.0.0.1:<porta>/hook/<token>/<kind>` — e os
    /// handlers `command` históricos (SessionStart turno-0 + PreToolUse gate W3-6)
    /// PERMANECEM intactos (observabilidade nunca substitui o gate).
    #[test]
    fn settings_with_wiring_adds_http_observability_hooks() {
        let wiring = HookWiring {
            port: 45678,
            token: "tok123abc".into(),
        };
        let j = hook_settings_json_with_observability("/usr/local/bin/lina", Some(&wiring));
        let v: serde_json::Value = serde_json::from_str(&j).expect("json válido");

        for kind in [
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "SubagentStart",
            "SubagentStop",
            "Stop",
        ] {
            let entries = v["hooks"][kind].as_array().unwrap_or_else(|| {
                panic!("kind {kind} deve existir no settings: {j}");
            });
            let has_http = entries.iter().any(|e| {
                e["hooks"].as_array().is_some_and(|hs| {
                    hs.iter().any(|h| {
                        h["type"] == "http"
                            && h["url"].as_str().is_some_and(|u| {
                                u == format!("http://127.0.0.1:45678/hook/tok123abc/{kind}")
                            })
                            && h["async"] == true
                    })
                })
            });
            assert!(has_http, "{kind} deve ter handler HTTP async: {j}");
        }

        // Os handlers históricos seguem lá: o gate é o guard, não o hook HTTP.
        // R2c-2: o matcher agora cobre `Bash|Skill` na MESMA entry (gate de execução +
        // guard de skill estrangeira) — o gate command do W3-6 segue intacto.
        let pre = v["hooks"]["PreToolUse"].as_array().expect("PreToolUse");
        let still_gated = pre.iter().any(|e| {
            e["matcher"] == "Bash|Skill"
                && e["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("guard --pretooluse"))
        });
        assert!(
            still_gated,
            "o gate command do W3-6 (matcher Bash|Skill) permanece: {j}"
        );
        assert!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .is_some_and(|c| c.contains("whoami --bootstrap")),
            "SessionStart turno-0 permanece"
        );
    }

    // ── ADR 0025 (reverte R2b): produção respeita o ~/.claude do usuário; modo só por pedido EXPLÍCITO ──

    /// **ADR 0025 (reverte R2b) — produção RESPEITA o `~/.claude` do usuário:** o settings
    /// que o bootstrap escreve em PRODUÇÃO (`permission_mode == None`) **NÃO** contém a
    /// chave `permissions` → o `defaultMode` do usuário (incl. `bypassPermissions`, se ele
    /// escolheu no próprio `~/.claude/settings.json`) FLUI por merge do Claude Code. E os
    /// hooks de segurança (SessionStart turno-0 + PreToolUse guard W3-6) seguem SEMPRE — a
    /// segurança durável é o guard/custódia, **independente** do permission-mode do claude.
    /// Vale com e sem fiação de hooks.
    #[test]
    fn production_settings_omit_permissions_so_user_choice_flows() {
        for wiring in [
            None,
            Some(HookWiring {
                port: 45678,
                token: "tok".into(),
            }),
        ] {
            let j = hook_settings_json_with_observability("/abs/lina", wiring.as_ref());
            let v: serde_json::Value = serde_json::from_str(&j).expect("json válido");
            assert!(
                v.get("permissions").is_none(),
                "produção NÃO fixa permissions — o defaultMode do usuário flui (wiring={:?}): {j}",
                wiring.is_some()
            );
            // O guard W3-6 (PreToolUse Bash|Skill) e o turno-0 (SessionStart) seguem SEMPRE.
            let pre = v["hooks"]["PreToolUse"].as_array().expect("PreToolUse");
            let guarded = pre.iter().any(|e| {
                e["matcher"] == "Bash|Skill"
                    && e["hooks"].as_array().is_some_and(|hs| {
                        hs.iter().any(|h| {
                            h["command"]
                                .as_str()
                                .is_some_and(|c| c.contains("guard --pretooluse"))
                        })
                    })
            });
            assert!(
                guarded,
                "o guard W3-6 (PreToolUse Bash|Skill) permanece mesmo sem permissions: {j}"
            );
            assert!(
                v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("whoami --bootstrap")),
                "SessionStart turno-0 permanece: {j}"
            );
        }
    }

    /// **ADR 0025 — modo EXPLÍCITO ≠ bypass silencioso herdado:** quando o app/teste pede
    /// um modo EXPLICITAMENTE (futuro mapa por papel / gate-test), a allowlist fixa-o — e
    /// `bypassPermissions` É VÁLIDO nesse caminho, porque é escolha EXPLÍCITA do humano,
    /// não a herança silenciosa que o R2b temia. `default/plan/acceptEdits` passam; valores
    /// DESCONHECIDOS (`dontAsk`/`auto`/lixo) degradam para `default` (fail-safe). Nenhum
    /// campo de agente alcança este parâmetro → não há caminho de auto-bypass do agente.
    #[test]
    fn permission_mode_explicit_request_honored_unknown_degrades() {
        // Modos válidos passam (incl. bypass EXPLÍCITO do humano).
        assert_eq!(sanitized_permission_mode(Some("default")), "default");
        assert_eq!(sanitized_permission_mode(Some("plan")), "plan");
        assert_eq!(
            sanitized_permission_mode(Some("acceptEdits")),
            "acceptEdits"
        );
        assert_eq!(
            sanitized_permission_mode(Some("bypassPermissions")),
            "bypassPermissions",
            "bypass EXPLÍCITO do humano é honrado (≠ bypass silencioso herdado que o R2b temia)"
        );
        assert_eq!(sanitized_permission_mode(Some("  plan  ")), "plan", "trim");
        // Desconhecido/lixo degrada para default (fail-safe).
        assert_eq!(sanitized_permission_mode(Some("dontAsk")), "default");
        assert_eq!(sanitized_permission_mode(Some("auto")), "default");
        assert_eq!(sanitized_permission_mode(Some("garbage")), "default");
        assert_eq!(
            sanitized_permission_mode(None),
            "default",
            "sem pedido → fallback defensivo default (produção nem chama: omite permissions)"
        );
        // JSON: pedido explícito fixa o modo (inclusive bypass)…
        let j = hook_settings_json_with_mode("/abs/lina", None, Some("bypassPermissions"));
        let v: serde_json::Value = serde_json::from_str(&j).expect("json");
        assert_eq!(v["permissions"]["defaultMode"], "bypassPermissions");
        let j = hook_settings_json_with_mode("/abs/lina", None, Some("plan"));
        let v: serde_json::Value = serde_json::from_str(&j).expect("json");
        assert_eq!(v["permissions"]["defaultMode"], "plan");
        // …lixo degrada para default…
        let j = hook_settings_json_with_mode("/abs/lina", None, Some("garbage"));
        let v: serde_json::Value = serde_json::from_str(&j).expect("json");
        assert_eq!(v["permissions"]["defaultMode"], "default");
        // …e PRODUÇÃO (None) NÃO fixa permissions (o ~/.claude do usuário decide).
        let j = hook_settings_json_with_mode("/abs/lina", None, None);
        let v: serde_json::Value = serde_json::from_str(&j).expect("json");
        assert!(
            v.get("permissions").is_none(),
            "None = produção: sem pin de permissions (o ~/.claude do usuário flui): {j}"
        );
    }

    /// **ADR 0025:** o arquivo escrito no disco por `write_terminal_files` (caminho de
    /// PRODUÇÃO) NÃO carrega bloco de `permissions` — o `~/.claude/settings.json` do
    /// usuário (defaultMode, model, MCP, comandos) flui por merge. Os hooks de segurança
    /// (turno-0 + guard W3-6) seguem intactos no MESMO arquivo.
    #[test]
    fn write_terminal_files_production_omits_permissions_keeps_hooks() {
        let bs = Bootstrapper::new().expect("registry");
        let dir = std::env::temp_dir().join(format!("lina-adr0025-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        bs.write_terminal_files(&dir, &sample("@Dev Backend"), "/abs/lina")
            .expect("escreve");
        let settings =
            std::fs::read_to_string(dir.join(".claude/settings.json")).expect("settings");
        let v: serde_json::Value = serde_json::from_str(&settings).expect("json");
        assert!(
            v.get("permissions").is_none(),
            "produção não fixa o modo — o settings do usuário decide: {settings}"
        );
        // Os hooks de segurança seguem intactos no MESMO arquivo (turno-0 + guard W3-6).
        assert!(settings.contains("whoami --bootstrap"), "turno-0 presente");
        assert!(
            settings.contains("guard --pretooluse"),
            "guard W3-6 presente"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// O comando do hook é **single-quoted** (espaços ok) e metacaracteres ficam literais.
    #[test]
    fn hook_settings_quotes_lina_bin_safely() {
        // Path com espaço (caso real do repo).
        let j = hook_settings_json("/Users/x/einstein workspace/target/debug/lina");
        assert!(j.contains("'/Users/x/einstein workspace/target/debug/lina' whoami --bootstrap"));
        // Metacaractere de shell: fica DENTRO das aspas (não vira comando separado).
        let j2 = hook_settings_json("lina; rm -rf /tmp");
        let v: serde_json::Value = serde_json::from_str(&j2).expect("json");
        let cmd = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("cmd");
        assert!(
            cmd.starts_with("'lina; rm -rf /tmp'"),
            "metacaractere single-quoted: {cmd}"
        );
    }

    /// `write_terminal_files` cria os 5 arquivos (3 doutrinas + bootstrap.json + hook).
    #[test]
    fn write_terminal_files_creates_all_files() {
        let bs = Bootstrapper::new().expect("registry");
        let dir = std::env::temp_dir().join(format!("lina-w32-wtf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        bs.write_terminal_files(&dir, &sample("@Dev Backend"), "/abs/lina")
            .expect("escreve");
        for f in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
            let c = std::fs::read_to_string(dir.join(f)).unwrap_or_else(|_| panic!("{f}"));
            assert!(
                c.contains("BACKEND") && !leftover_placeholders(&c),
                "{f} resolvido"
            );
        }
        let boot =
            std::fs::read_to_string(dir.join(".lina/bootstrap.json")).expect("bootstrap.json");
        let _: BootstrapInput = serde_json::from_str(&boot).expect("json válido");
        let settings =
            std::fs::read_to_string(dir.join(".claude/settings.json")).expect("settings");
        assert!(
            settings.contains("SessionStart")
                && settings.contains("'/abs/lina' whoami --bootstrap")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
