//! `dev_tools` — **tela "Ferramentas de desenvolvimento"** do onboarding (passo após o check-up).
//!
//! Detecta e instala as ferramentas que um não-técnico vai precisar quando começar a programar com os
//! assistentes: **git, GitHub CLI, Vercel CLI, Node.js e Python**. REUSA o mecanismo de instalação dos
//! assistentes de IA (`crate::onboarding`): receitas TOML por SO ([`Installers`]), o PTY oculto
//! ([`crate::onboarding::run_install`]) e a re-hidratação de PATH ([`crate::refresh_path_after_install`]).
//!
//! ## Split (igual ao resto do shell)
//! - [`DevToolsModel`] + helpers (descoberta, [`decide_plan`], [`install_recipe_for`]) são **gpui-free e
//!   testáveis** — toda a lógica vive aqui.
//! - [`DevToolsModel::render`] só desenha o modelo e roteia cliques pela view-pai ([`OnboardingView`]).
//!
//! ## Por que não dá pra reusar `discover_clis` direto
//! O `discover_clis` do core assume **id == nome do binário** (claude/codex/…). Aqui o id LÓGICO
//! (chave do TOML) difere do binário (`python` → `python3`) e do RÓTULO amigável (`gh` → "GitHub CLI").
//! Então a descoberta é genérica: itera [`DEV_TOOLS`] e usa [`lina_core::find_in_path`] +
//! [`lina_core::query_version`] (FORA da thread de UI — o `query_version` é síncrono e travaria).
//!
//! **Invariantes (CLAUDE.md):** #1 (zero LLM — detecção é pattern-match no PATH), #2 (local-first — só
//! lê o PATH/roda o instalador local), #3 (especificidade em TOML, não hardcoded), #6 (não-técnico-first:
//! zero jargão, nunca beco sem saída — "Continuar" nunca trava; estado sempre salvo e visível).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use gpui::{div, prelude::*, px, rgb, text, AnyElement, ClickEvent, Context, FontWeight, Window};

use lina_cli_profiles::{InstallRecipe, Installers, CURRENT_OS};
use lina_core::{find_in_path, query_version, DiscoveredCli};

use crate::onboarding::{install_recipe_with, run_install, InstallState, OnboardingView};

// ───────────────────────────── paleta (espelha o onboarding/canvas) ─────────────────────────────
// Mantidas locais (as do `onboarding` são module-private); os valores ESPELHAM a paleta de lá.
const PANEL: u32 = 0x141a36;
const ACCENT: u32 = 0x7aa2f7;
const TEXT: u32 = 0xc8d3f5;
const MUTED: u32 = 0x5b658f;
const GREEN: u32 = 0x9ece6a;
const AMBER: u32 = 0xe0af68;
const RED: u32 = 0xf7768e;

// ═══════════════════════════ catálogo de ferramentas (id ≠ binário ≠ rótulo) ═══════════════════════════

/// Uma ferramenta de desenvolvimento que a tela detecta/instala. `id` é a chave LÓGICA no
/// `dev-tools.toml` (e no snapshot de descoberta); `bin` é o nome do executável no PATH (≠ id no caso
/// do Python); `label` é o rótulo amigável (sem jargão, inv#6).
#[derive(Debug, Clone, Copy)]
pub struct DevTool {
    pub id: &'static str,
    pub bin: &'static str,
    pub label: &'static str,
}

/// As 5 ferramentas, em ordem amigável a dependências (Node antes de Vercel, que precisa dele).
pub const DEV_TOOLS: &[DevTool] = &[
    DevTool {
        id: "git",
        bin: "git",
        label: "Git",
    },
    DevTool {
        id: "gh",
        bin: "gh",
        label: "GitHub CLI",
    },
    DevTool {
        id: "node",
        bin: "node",
        label: "Node.js",
    },
    DevTool {
        id: "vercel",
        bin: "vercel",
        label: "Vercel CLI",
    },
    DevTool {
        id: "python",
        bin: "python3",
        label: "Python",
    },
];

/// A ferramenta de id lógico `id` (`None` se desconhecido).
#[must_use]
pub fn dev_tool(id: &str) -> Option<&'static DevTool> {
    DEV_TOOLS.iter().find(|t| t.id == id)
}

/// Rótulo amigável do id lógico (fallback: o próprio id, p/ nunca mostrar vazio).
#[must_use]
pub fn label_for(id: &str) -> String {
    dev_tool(id)
        .map(|t| t.label.to_string())
        .unwrap_or_else(|| id.to_string())
}

// ═══════════════════════════ receitas (TOML embutido, mesmo loader do onboarding) ═══════════════════════════

/// Receitas das ferramentas embutidas (config TOML, NÃO hardcoded — inv#3). Caminho relativo ao
/// arquivo-fonte (igual ao `recipes.toml` do onboarding).
const DEV_TOOLS_TOML: &str = include_str!("../../../profiles/installers/dev-tools.toml");

/// Tabela parseada uma única vez. TOML inválido → tabela vazia (os botões viram fallback manual);
/// nunca derruba o app.
pub fn dev_installers() -> &'static Installers {
    static INSTALLERS: OnceLock<Installers> = OnceLock::new();
    INSTALLERS.get_or_init(|| {
        Installers::from_toml_str(DEV_TOOLS_TOML, "profiles/installers/dev-tools.toml")
            .unwrap_or_else(|e| {
                eprintln!(
                    "dev_tools: dev-tools.toml inválido ({e}); 'Instalar para mim' indisponível"
                );
                Installers::default()
            })
    })
}

/// Receita de instalação do id lógico p/ o SO atual, com override `LINA_INSTALL_<ID>` (string de shell
/// via `sh -c`) — mesma convenção do onboarding (reusa [`install_recipe_with`]). `None` = sem receita
/// p/ este SO → fallback manual.
#[must_use]
pub fn install_recipe_for(id: &str) -> Option<InstallRecipe> {
    let key = format!("LINA_INSTALL_{}", id.to_ascii_uppercase());
    install_recipe_with(id, std::env::var(&key).ok().as_deref(), dev_installers())
}

// ═══════════════════════════ plano de instalação (puro, testável) ═══════════════════════════

/// Como instalar uma ferramenta, decidido a partir do SO + presença de pré-requisitos. A regra do
/// usuário: PREFERIR o silencioso; só abrir um terminal real quando o instalador precisar da senha
/// (sudo/UAC) ou de uma ferramenta-base ausente. Nunca rodar algo que pede senha num PTY oculto (penduraria).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPlan {
    /// Instala em silêncio no PTY oculto (caso do macOS com Homebrew — o fundador HOJE).
    Silent,
    /// Precisa da interação do usuário (senha do sudo / UAC) → abre um terminal real.
    Interactive,
    /// Falta uma ferramenta-base ANTES (ex.: Vercel precisa do Node.js) — guia, não tenta às cegas.
    NeedsFirst { needs: &'static str },
    /// Sem receita automática p/ este SO → instruções manuais.
    Manual,
}

/// `true` se o comando da receita exige interação humana: `sudo` (pede senha no apt) ou `winget` (UAC
/// no Windows). DERIVADO do comando porque o schema do [`InstallRecipe`] (no crate intocável,
/// `deny_unknown_fields`) não aceita um campo `requires_interaction` no TOML.
#[must_use]
pub fn recipe_needs_interaction(recipe: &InstallRecipe) -> bool {
    recipe_mentions(recipe, "winget") || recipe_mentions(recipe, "sudo")
}

/// `true` se `needle` aparece no programa ou em algum argumento da receita (ex.: "brew", "sudo").
fn recipe_mentions(recipe: &InstallRecipe, needle: &str) -> bool {
    recipe.program.contains(needle) || recipe.args.iter().any(|a| a.contains(needle))
}

/// Decide o [`InstallPlan`] de `id` no `os`, dada a receita resolvida e um teste de presença de binário
/// (`bin_present("node")`, `bin_present("brew")`, …). **PURO** — testável sem mexer no PATH real.
///
/// Ordem: (1) sem receita → `Manual`; (2) pré-requisito duro ausente (Vercel sem Node) → `NeedsFirst`;
/// (3) no macOS uma receita do brew sem o `brew` presente → `Interactive` (terminal real); (4) comando
/// que pede senha (sudo/winget) → `Interactive`; (5) o resto → `Silent`.
#[must_use]
pub fn decide_plan(
    id: &str,
    os: &str,
    recipe: Option<&InstallRecipe>,
    bin_present: impl Fn(&str) -> bool,
) -> InstallPlan {
    let Some(recipe) = recipe else {
        return InstallPlan::Manual;
    };
    // (2) Vercel é um pacote npm global → precisa do Node.js (que traz o npm) antes.
    if id == "vercel" && !bin_present("node") {
        return InstallPlan::NeedsFirst { needs: "node" };
    }
    // (3) No macOS, as receitas usam Homebrew; sem o `brew` o caminho silencioso falharia → terminal real.
    if os == "macos" && recipe_mentions(recipe, "brew") && !bin_present("brew") {
        return InstallPlan::Interactive;
    }
    // (4) Pede senha (sudo) / UAC (winget) → terminal real (nunca no PTY oculto).
    if recipe_needs_interaction(recipe) {
        return InstallPlan::Interactive;
    }
    // (5) Silencioso (macOS+brew, Node no Linux via tarball, npm/vercel com Node presente, …).
    InstallPlan::Silent
}

// ═══════════════════════════ abrir um terminal real (macOS) ═══════════════════════════

/// Reconstrói a linha de comando "como o usuário digitaria" a partir da receita. O caso comum é
/// `bash -c '<corpo>'` → devolve só o corpo; senão junta programa + args com aspas simples.
fn reconstruct_command(recipe: &InstallRecipe) -> String {
    if recipe.program == "bash" && recipe.args.len() == 2 && recipe.args[0] == "-c" {
        return recipe.args[1].clone();
    }
    std::iter::once(recipe.program.clone())
        .chain(recipe.args.iter().cloned())
        .map(|a| {
            if a.is_empty() || a.contains(char::is_whitespace) {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Abre uma janela do **Terminal do macOS** rodando a receita (caminho INTERATIVO: o usuário digita a
/// senha lá). Escreve um script `.command` temporário e o entrega ao Terminal via `open` — evita o
/// inferno de escapar aspas no AppleScript. Best-effort: devolve `Err` se não conseguir lançar.
#[cfg(target_os = "macos")]
pub fn open_in_terminal(recipe: &InstallRecipe) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let mut script = String::from("#!/bin/bash\n");
    // `env` da receita (não-interatividade etc.) vira export; no nosso TOML o env vem inline no comando,
    // mas honramos o campo p/ generalidade.
    for (k, v) in &recipe.env {
        script.push_str(&format!("export {k}={}\n", shell_single_quote(v)));
    }
    script.push_str(&reconstruct_command(recipe));
    script.push_str(
        "\necho\necho 'Pronto! Pode fechar esta janela e voltar ao Lina — clique em \"Verificar de novo\".'\n",
    );

    let path = std::env::temp_dir().join(format!(
        "lina-devtool-install-{}.command",
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&path)?;
        f.write_all(script.as_bytes())?;
        let mut perm = f.metadata()?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm)?;
    }
    std::process::Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(&path)
        .spawn()?;
    Ok(path)
}

/// Fora do macOS ainda não abrimos um terminal real (porta aberta — ver UX): devolve `Err` p/ o
/// caller cair na instrução "rode manualmente".
#[cfg(not(target_os = "macos"))]
pub fn open_in_terminal(_recipe: &InstallRecipe) -> std::io::Result<PathBuf> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "abrir terminal automaticamente só no macOS por enquanto",
    ))
}

/// Cita `s` entre aspas simples de shell (escapando aspas simples internas).
#[cfg(target_os = "macos")]
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ═══════════════════════════ avisos (estados que o `InstallState` não cobre) ═══════════════════════════

/// Avisos do caminho NÃO-silencioso (o [`InstallState`] reusado do onboarding só modela o PTY oculto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevNotice {
    /// Abrimos uma janela de Terminal p/ instalar `tool` (pode pedir a senha do Mac).
    TerminalOpened { tool: String },
    /// `tool` precisa de `needs` antes (ex.: Vercel precisa do Node.js).
    NeedsFirst { tool: String, needs: String },
    /// Não consegui abrir o Terminal — instrução p/ rodar o comando manualmente.
    RunManually { tool: String, command: String },
}

// ═══════════════════════════ descoberta genérica (id → binário → versão) ═══════════════════════════

/// Função de descoberta INJETÁVEL (testes injetam um snapshot controlado, sem rodar `--version` real).
pub type DiscoverFn = Arc<dyn Fn() -> Vec<DiscoveredCli> + Send + Sync>;

/// Descobre as [`DEV_TOOLS`] num PATH dado (puro/injetável): mapeia id→binário, acha no PATH e resolve
/// a versão. O `DiscoveredCli.id` carregado é o **id lógico** (git/gh/node/vercel/python).
#[must_use]
pub fn discover_dev_tools_in(path_env: &str) -> Vec<DiscoveredCli> {
    DEV_TOOLS
        .iter()
        .filter_map(|t| {
            let path = find_in_path(t.bin, path_env)?;
            let version = query_version(&path);
            Some(DiscoveredCli {
                id: t.id.to_string(),
                version,
                path: path.display().to_string(),
            })
        })
        .collect()
}

/// Descobre as ferramentas no PATH do processo (default de produção).
#[must_use]
pub fn discover_dev_tools() -> Vec<DiscoveredCli> {
    let path = std::env::var("PATH").unwrap_or_default();
    discover_dev_tools_in(&path)
}

// ═══════════════════════════ o modelo (gpui-free) ═══════════════════════════

/// PATH atual do processo (lido sob demanda; já hidratado no boot — ver `main.rs`).
fn current_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// Atualiza o estado compartilhado de instalação (best-effort sob poison).
fn set_install(state: &Arc<Mutex<InstallState>>, s: InstallState) {
    if let Ok(mut g) = state.lock() {
        *g = s;
    }
}

/// Estado da tela de ferramentas, sem nenhum tipo de gpui. Espelha o `OnboardingModel`: snapshot de
/// descoberta compartilhado (escrito por uma thread, lido por frame), instalação no PTY oculto reusada,
/// e os avisos do caminho interativo.
pub struct DevToolsModel {
    detected: Arc<Mutex<Vec<DiscoveredCli>>>,
    discovering: Arc<AtomicBool>,
    discovery_handle: Option<thread::JoinHandle<()>>,
    discover: DiscoverFn,
    install: Arc<Mutex<InstallState>>,
    install_handle: Option<thread::JoinHandle<()>>,
    install_target: Option<String>,
    install_consumed: bool,
    notice: Option<DevNotice>,
}

impl DevToolsModel {
    /// Modelo com a descoberta REAL ([`discover_dev_tools`]).
    pub fn new() -> Self {
        Self::new_with(Arc::new(discover_dev_tools))
    }

    /// Modelo com a descoberta INJETADA (testes passam um snapshot determinístico).
    pub fn new_with(discover: DiscoverFn) -> Self {
        let mut model = Self {
            detected: Arc::new(Mutex::new(Vec::new())),
            discovering: Arc::new(AtomicBool::new(false)),
            discovery_handle: None,
            discover,
            install: Arc::new(Mutex::new(InstallState::Idle)),
            install_handle: None,
            install_target: None,
            install_consumed: true,
            notice: None,
        };
        model.redetect();
        model
    }

    /// Estado atual da instalação (clone do compartilhado).
    #[must_use]
    pub fn install_state(&self) -> InstallState {
        self.install
            .lock()
            .map(|g| g.clone())
            .unwrap_or(InstallState::Idle)
    }

    /// O id lógico alvo da instalação corrente (p/ a view destacar a linha certa).
    #[must_use]
    pub fn install_target(&self) -> Option<&str> {
        self.install_target.as_deref()
    }

    /// Aviso corrente do caminho interativo (terminal aberto / pré-requisito / rode manualmente).
    #[must_use]
    pub fn notice(&self) -> Option<&DevNotice> {
        self.notice.as_ref()
    }

    /// Snapshot das ferramentas detectadas.
    fn detected(&self) -> Vec<DiscoveredCli> {
        self.detected.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Versão detectada de uma ferramenta (ou `None` se ausente).
    #[must_use]
    pub fn version_of(&self, id: &str) -> Option<String> {
        self.detected()
            .into_iter()
            .find(|c| c.id == id)
            .and_then(|c| c.version)
    }

    /// `true` se a ferramenta está presente no PATH.
    #[must_use]
    pub fn is_present(&self, id: &str) -> bool {
        self.detected().iter().any(|c| c.id == id)
    }

    /// `true` se TODAS as ferramentas já estão instaladas (banner "está tudo pronto").
    #[must_use]
    pub fn all_present(&self) -> bool {
        DEV_TOOLS.iter().all(|t| self.is_present(t.id))
    }

    /// `true` se NENHUMA foi encontrada ainda.
    #[must_use]
    pub fn nothing_found(&self) -> bool {
        self.detected().is_empty()
    }

    /// `true` enquanto a varredura roda (a view mostra "procurando…", não "nenhuma").
    #[must_use]
    pub fn is_discovering(&self) -> bool {
        self.discovering.load(Ordering::SeqCst)
    }

    /// Re-varre o PATH em **BACKGROUND** (não trava a UI nem se um `--version` pendurar). Idempotente:
    /// ignora se já há uma varredura em voo. Espelha o `redetect` do onboarding.
    pub fn redetect(&mut self) {
        if self.discovering.swap(true, Ordering::SeqCst) {
            return;
        }
        let discover = Arc::clone(&self.discover);
        let detected = Arc::clone(&self.detected);
        let discovering = Arc::clone(&self.discovering);
        let handle = thread::spawn(move || {
            let found = discover();
            if let Ok(mut d) = detected.lock() {
                *d = found;
            }
            discovering.store(false, Ordering::SeqCst);
        });
        self.discovery_handle = Some(handle);
    }

    /// Bloqueia até a varredura corrente terminar. **Test-only** (determinismo); a produção nunca bloqueia.
    #[allow(dead_code)]
    pub fn block_on_discovery(&mut self) {
        if let Some(h) = self.discovery_handle.take() {
            let _ = h.join();
        }
    }

    /// Re-detecção manual (botão "Verificar de novo"). Limpa o aviso anterior (o usuário agiu).
    pub fn verify_now(&mut self) {
        self.notice = None;
        self.redetect();
    }

    /// Inicia "Instalar para mim" para `id`, escolhendo o caminho pelo [`decide_plan`] sobre o PATH
    /// REAL: silencioso (PTY oculto, reusa [`run_install`]), interativo (terminal real) ou guia (pré-requisito).
    pub fn start_install(&mut self, id: &str) {
        let path = current_path();
        self.start_install_with(id, &|bin| find_in_path(bin, &path).is_some());
    }

    /// Núcleo de [`start_install`] com a presença de binário INJETADA — separa a decisão do PATH real
    /// (testável sem mutar o `env` global, que `cargo test` compartilha entre threads).
    fn start_install_with(&mut self, id: &str, bin_present: &dyn Fn(&str) -> bool) {
        // Não empilha instalações silenciosas.
        if matches!(
            self.install_state(),
            InstallState::Installing { .. } | InstallState::Verifying
        ) {
            return;
        }
        self.notice = None;

        let Some(recipe) = install_recipe_for(id) else {
            set_install(
                &self.install,
                InstallState::Failed {
                    reason: format!(
                        "ainda não sei instalar {} neste sistema automaticamente — instale \
                         manualmente e clique em Verificar de novo",
                        label_for(id)
                    ),
                },
            );
            return;
        };

        let plan = decide_plan(id, CURRENT_OS, Some(&recipe), bin_present);

        match plan {
            InstallPlan::Manual => set_install(
                &self.install,
                InstallState::Failed {
                    reason: format!(
                        "ainda não sei instalar {} neste sistema automaticamente — instale \
                         manualmente e clique em Verificar de novo",
                        label_for(id)
                    ),
                },
            ),
            InstallPlan::NeedsFirst { needs } => {
                self.notice = Some(DevNotice::NeedsFirst {
                    tool: label_for(id),
                    needs: label_for(needs),
                });
            }
            InstallPlan::Interactive => match open_in_terminal(&recipe) {
                Ok(_) => {
                    self.notice = Some(DevNotice::TerminalOpened {
                        tool: label_for(id),
                    });
                }
                Err(_) => {
                    self.notice = Some(DevNotice::RunManually {
                        tool: label_for(id),
                        command: reconstruct_command(&recipe),
                    });
                }
            },
            InstallPlan::Silent => self.start_silent(id, recipe),
        }
    }

    /// Caminho silencioso: PTY oculto (reusa [`run_install`]) + re-hidratação de PATH na verificação
    /// (reusa [`crate::refresh_path_after_install`]). Idêntico ao onboarding, com a descoberta de
    /// ferramentas no lugar da de assistentes.
    fn start_silent(&mut self, id: &str, recipe: InstallRecipe) {
        self.install_target = Some(id.to_string());
        self.install_consumed = false;
        // Feedback OTIMISTA: o 1º frame após o clique já mostra "⟳ iniciando…" (a thread confirma logo).
        set_install(
            &self.install,
            InstallState::Installing {
                line: "iniciando…".into(),
            },
        );
        let id = id.to_string();
        let verify_paths = recipe.verify_paths.clone();
        let discover = Arc::clone(&self.discover);
        let handle = run_install(recipe, Arc::clone(&self.install), move || {
            crate::refresh_path_after_install(&verify_paths);
            discover().into_iter().find(|c| c.id == id)
        });
        self.install_handle = Some(handle);
    }

    /// A cada frame: quando a instalação silenciosa conclui (`Ok`), re-detecta UMA vez (idempotente).
    pub fn poll_install(&mut self) {
        if self.install_consumed {
            return;
        }
        if matches!(self.install_state(), InstallState::Ok { .. }) {
            self.install_consumed = true;
            self.redetect();
        }
    }

    // ═══════════════════════════ a view (fina) ═══════════════════════════

    /// Desenha a tela espelhando o `render_checkup` do onboarding: cabeçalho + banner de estado + uma
    /// linha por ferramenta (ponto + rótulo + status + "Instalar para mim") + avisos + rodapé
    /// (Voltar · Verificar de novo · Continuar). Os cliques roteiam pela view-pai ([`OnboardingView`]).
    pub fn render(&self, _window: &mut Window, cx: &mut Context<OnboardingView>) -> AnyElement {
        let install = self.install_state();

        let mut col = div().flex().flex_col().gap_5().child(heading(
            "Ferramentas para você começar a programar",
            "Estas ajudam seus assistentes a guardar e publicar o que você criar. Se faltar alguma, eu instalo para você — você pode seguir sem elas a qualquer momento.",
        ));

        // Banner de estado da varredura (procurando / tudo pronto / nenhuma) — sem falso "nenhuma" durante a busca.
        if self.is_discovering() {
            col = col.child(banner(
                PANEL,
                ACCENT,
                "Procurando as ferramentas no seu computador…",
            ));
        } else if self.all_present() {
            col = col.child(banner(
                0x18301f,
                GREEN,
                "Tudo pronto! Você já tem todas as ferramentas. É só continuar.",
            ));
        } else if self.nothing_found() {
            col = col.child(banner(
                0x33202c,
                AMBER,
                "Ainda não encontrei nenhuma — escolha uma abaixo e clique \"Instalar para mim\".",
            ));
        }

        // Uma linha por ferramenta.
        let mut list = div().flex().flex_col().gap_2();
        for t in DEV_TOOLS {
            let id = t.id;
            let present = self.is_present(id);
            let installing_this = self.install_target() == Some(id)
                && matches!(
                    install,
                    InstallState::Installing { .. } | InstallState::Verifying
                );

            // `.id(id)` (único por ferramenta) é OBRIGATÓRIO p/ a11y: o `text!` gera o ElementId por
            // LOCALIZAÇÃO no fonte; `text!` repetidos no MESMO ponto do laço colidiriam no nó AccessKit
            // (pânico com leitor de tela). Um ancestral com id distinto por linha desambigua (idioma do
            // `render_checkup`).
            let mut row = div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .rounded_md()
                .bg(rgb(PANEL))
                .child(div().size(px(9.0)).rounded_full().bg(rgb(if present {
                    GREEN
                } else {
                    0x3a4566
                })))
                .child(
                    div()
                        .w(px(150.0))
                        .text_color(rgb(TEXT))
                        .font_weight(FontWeight::BOLD)
                        .child(text!(t.label.to_string())),
                );

            if present {
                let ver = self
                    .version_of(id)
                    .unwrap_or_else(|| "instalado".to_string());
                row = row.child(
                    div()
                        .flex_1()
                        .text_color(rgb(GREEN))
                        .child(text!(format!("✓ {ver}"))),
                );
            } else if installing_this {
                let line = match &install {
                    InstallState::Installing { line } => line.clone(),
                    InstallState::Verifying => "verificando…".to_string(),
                    _ => "instalando…".to_string(),
                };
                row = row.child(
                    div()
                        .flex_1()
                        .text_color(rgb(AMBER))
                        .child(text!(format!("⟳ {line}"))),
                );
            } else {
                row = row
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(MUTED))
                            .child(text!("não encontrado")),
                    )
                    .child(install_button(id, cx));
            }
            list = list.child(row);
        }
        col = col.child(list);

        // Aviso do caminho interativo (terminal aberto / pré-requisito / rode manualmente) — tranquilizador.
        if let Some(notice) = self.notice() {
            col = col.child(match notice {
                DevNotice::TerminalOpened { tool } => banner(
                    PANEL,
                    ACCENT,
                    &format!(
                        "Abri uma janela de Terminal para instalar {tool}. Ela pode pedir a senha do seu \
                         Mac — isso é normal e seguro. Quando terminar, volte aqui e clique \"Verificar de novo\"."
                    ),
                ),
                DevNotice::NeedsFirst { tool, needs } => banner(
                    PANEL,
                    ACCENT,
                    &format!("Para instalar {tool}, instale o {needs} primeiro (logo acima). Depois é só clicar em instalar de novo."),
                ),
                DevNotice::RunManually { tool, command } => banner(
                    0x33202c,
                    AMBER,
                    &format!("Não consegui abrir o instalador de {tool} sozinho. Você pode rodar este comando no seu terminal:\n{command}"),
                ),
            });
        }

        // Falha acionável da instalação silenciosa (sem jargão; nunca trava "Continuar").
        if let InstallState::Failed { reason } = &install {
            col = col.child(banner(
                0x33202c,
                RED,
                &format!(
                    "⚠ {reason}  ·  Nada quebrou — você pode tentar de novo ou seguir sem isso."
                ),
            ));
        }

        // Rodapé: Voltar · Verificar de novo · Continuar (sempre pode seguir — nunca um beco, inv#6).
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(ghost_button(
                    "devtools-back",
                    "← Voltar",
                    cx,
                    |onb, _w, _cx| onb.nav_back(),
                ))
                .child(ghost_button(
                    "devtools-verify",
                    "↻ Verificar de novo",
                    cx,
                    |onb, _w, cx| {
                        onb.dev_tools.verify_now();
                        cx.notify();
                    },
                ))
                .child(div().flex_1())
                .child(primary_button(
                    "devtools-next",
                    "Continuar →",
                    cx,
                    |onb, _w, _cx| onb.nav_continue(),
                )),
        );

        col.into_any_element()
    }
}

impl Default for DevToolsModel {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════ helpers de view (estilo espelhado do onboarding) ═══════════════════════════

/// Caixa de título + subtítulo (mesma proporção do `heading` do onboarding).
fn heading(title: &str, subtitle: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(28.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(text!(title.to_string())),
        )
        .child(
            div()
                .text_size(px(15.0))
                .text_color(rgb(MUTED))
                .child(text!(subtitle.to_string())),
        )
        .into_any_element()
}

/// Banner de uma linha (cor de fundo + cor de texto + mensagem). Cor NUNCA sozinha — sempre com texto (inv#6).
fn banner(bg: u32, fg: u32, msg: &str) -> AnyElement {
    div()
        .px_4()
        .py_3()
        .rounded_md()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .child(text!(msg.to_string()))
        .into_any_element()
}

/// Botão "Instalar para mim" de uma ferramenta (roteia o clique pela view-pai).
fn install_button(id: &'static str, cx: &mut Context<OnboardingView>) -> AnyElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(0x3d59c9))
        .text_color(rgb(0xeef1ff))
        .cursor_pointer()
        .on_click(cx.listener(move |onb, _ev: &ClickEvent, _w, cx| {
            onb.dev_tools.start_install(id);
            cx.notify(); // desenha o feedback na hora; o pulso assume a animação
        }))
        .child(text!("Instalar para mim"))
        .into_any_element()
}

/// Botão primário (avançar). Espelha o `primary_button` do onboarding (que é private lá).
fn primary_button(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<OnboardingView>,
    on_click: impl Fn(&mut OnboardingView, &mut Window, &mut Context<OnboardingView>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .px_5()
        .py_2()
        .rounded_md()
        .bg(rgb(0x2c7a4b))
        .text_color(rgb(0xeef1ff))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .on_click(cx.listener(move |onb, _ev: &ClickEvent, window, cx| on_click(onb, window, cx)))
        .child(text!(label))
        .into_any_element()
}

/// Botão secundário (voltar / verificar). Espelha o `ghost_button` do onboarding.
fn ghost_button(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<OnboardingView>,
    on_click: impl Fn(&mut OnboardingView, &mut Window, &mut Context<OnboardingView>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(0x2a3152))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .on_click(cx.listener(move |onb, _ev: &ClickEvent, window, cx| on_click(onb, window, cx)))
        .child(text!(label))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Tempdir único, removido no Drop (mesmo idioma dos testes do onboarding/cli_discovery).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "lina-devtools-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&p).expect("tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn write_fake_cli(dir: &Path, name: &str, version_line: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\necho '{version_line}'\n")).expect("escrever cli");
        let mut perm = std::fs::metadata(&p).expect("meta").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&p, perm).expect("chmod");
        p
    }

    /// O catálogo: id ≠ binário no Python; o rótulo é amigável e nunca vazio.
    #[test]
    fn catalog_maps_id_bin_and_label() {
        assert_eq!(dev_tool("python").map(|t| t.bin), Some("python3"));
        assert_eq!(dev_tool("git").map(|t| t.bin), Some("git"));
        assert_eq!(label_for("vercel"), "Vercel CLI");
        assert_eq!(label_for("gh"), "GitHub CLI");
        assert_eq!(label_for("node"), "Node.js");
        // id desconhecido → fallback no próprio id (nunca vazio).
        assert_eq!(label_for("zzz"), "zzz");
        assert!(dev_tool("zzz").is_none());
    }

    /// O `dev-tools.toml` REAL parseia e cobre as 5 ferramentas nos 3 SOs com `program` não-vazio.
    #[test]
    fn real_dev_tools_toml_is_valid_and_complete() {
        let inst = dev_installers();
        for id in ["git", "gh", "node", "vercel", "python"] {
            let p = inst
                .0
                .get(id)
                .unwrap_or_else(|| panic!("falta receita p/ {id}"));
            for os in ["macos", "linux", "windows"] {
                let r = p.for_os(os).unwrap_or_else(|| panic!("falta {id}.{os}"));
                assert!(!r.program.trim().is_empty(), "{id}.{os} sem program");
            }
        }
        // Vercel é npm (precisa de Node), nunca brew/sudo.
        let vercel = format!("{:?}", inst.0["vercel"]);
        assert!(vercel.contains("npm"), "vercel deve usar npm");
        // Python no macOS instala via brew (binário python3 resolvido pelo mapa).
        assert!(format!("{:?}", inst.0["python"].for_os("macos").unwrap()).contains("brew"));
    }

    /// Resolução de receita por SO via a tabela real + override `sh -c` (reusa `install_recipe_with`).
    #[test]
    fn recipe_resolution_per_os_and_override() {
        let inst = dev_installers();
        // Override vence tudo e roda via `sh -c`.
        let over = install_recipe_with("git", Some("echo oi"), inst).expect("override");
        assert_eq!(over.program, "sh");
        assert_eq!(over.args, vec!["-c".to_string(), "echo oi".to_string()]);
        // macOS: git via brew.
        let git_mac = inst.0["git"].for_os("macos").expect("git macos");
        assert!(git_mac.args.iter().any(|a| a.contains("brew install git")));
        // Linux: git via apt com sudo (→ interativo, ver decide_plan).
        let git_lin = inst.0["git"].for_os("linux").expect("git linux");
        assert!(git_lin.args.iter().any(|a| a.contains("sudo")));
        // Windows: git via winget.
        assert_eq!(
            inst.0["git"].for_os("windows").expect("git win").program,
            "winget"
        );
        // id desconhecido → None (fallback manual).
        assert!(install_recipe_with("zzz", None, inst).is_none());
    }

    /// `decide_plan` (puro): pré-requisito, brew ausente no Mac, sudo/winget e o caso silencioso.
    #[test]
    fn plan_chooses_silent_interactive_or_prereq() {
        let inst = dev_installers();
        let all_present = |_: &str| true;
        let none_present = |_: &str| false;

        // Vercel sem Node → NeedsFirst (não tenta às cegas).
        let vercel = inst.0["vercel"].for_os("macos").cloned();
        assert_eq!(
            decide_plan("vercel", "macos", vercel.as_ref(), |b| b != "node"),
            InstallPlan::NeedsFirst { needs: "node" }
        );
        // Vercel COM Node (macOS, npm, sem sudo) → silencioso.
        assert_eq!(
            decide_plan("vercel", "macos", vercel.as_ref(), all_present),
            InstallPlan::Silent
        );

        // git macOS com brew presente → silencioso; sem brew → interativo (terminal real).
        let git_mac = inst.0["git"].for_os("macos").cloned();
        assert_eq!(
            decide_plan("git", "macos", git_mac.as_ref(), all_present),
            InstallPlan::Silent
        );
        assert_eq!(
            decide_plan("git", "macos", git_mac.as_ref(), none_present),
            InstallPlan::Interactive
        );

        // git Linux (sudo no apt) → interativo, mesmo com tudo "presente".
        let git_lin = inst.0["git"].for_os("linux").cloned();
        assert_eq!(
            decide_plan("git", "linux", git_lin.as_ref(), all_present),
            InstallPlan::Interactive
        );

        // node Linux (tarball, SEM sudo) → silencioso (não pede senha).
        let node_lin = inst.0["node"].for_os("linux").cloned();
        assert_eq!(
            decide_plan("node", "linux", node_lin.as_ref(), all_present),
            InstallPlan::Silent
        );

        // node Windows (winget → UAC) → interativo.
        let node_win = inst.0["node"].for_os("windows").cloned();
        assert_eq!(
            decide_plan("node", "windows", node_win.as_ref(), all_present),
            InstallPlan::Interactive
        );

        // Sem receita → Manual.
        assert_eq!(
            decide_plan("git", "plan9", None, all_present),
            InstallPlan::Manual
        );
    }

    /// Descoberta genérica: acha pelo BINÁRIO (python3) e carrega o ID LÓGICO (python) com a versão.
    #[cfg(unix)]
    #[test]
    fn discovery_maps_binary_to_logical_id() {
        let dir = TempDir::new("disc");
        write_fake_cli(dir.path(), "python3", "Python 3.12.7");
        write_fake_cli(dir.path(), "git", "git version 2.44.0");
        let found = discover_dev_tools_in(&dir.path().display().to_string());
        // python achado pelo binário python3, mas carregado com o id lógico "python".
        let py = found
            .iter()
            .find(|c| c.id == "python")
            .expect("python detectado");
        assert_eq!(py.version.as_deref(), Some("Python 3.12.7"));
        assert!(py.path.ends_with("python3"));
        // git presente; node/vercel/gh ausentes.
        assert!(found.iter().any(|c| c.id == "git"));
        assert!(!found.iter().any(|c| c.id == "node"));
        assert_eq!(found.len(), 2);
    }

    /// O modelo reflete a descoberta injetada (is_present/version_of/all_present/nothing_found),
    /// rodando a varredura FORA da thread de UI.
    #[test]
    fn model_reflects_injected_discovery() {
        let snapshot = vec![DiscoveredCli {
            id: "git".into(),
            version: Some("git version 2.44.0".into()),
            path: "/x/git".into(),
        }];
        let disc: DiscoverFn = Arc::new(move || snapshot.clone());
        let mut model = DevToolsModel::new_with(disc);
        model.block_on_discovery();
        assert!(model.is_present("git"));
        assert_eq!(
            model.version_of("git").as_deref(),
            Some("git version 2.44.0")
        );
        assert!(!model.is_present("node"));
        assert!(!model.all_present());
        assert!(!model.nothing_found());
        assert!(!model.is_discovering());
    }

    /// `start_install` de Vercel sem Node injeta o aviso "instale o Node.js primeiro" (não dispara PTY).
    /// Usa `bin_present` INJETADO (não muta o `PATH` global, que `cargo test` compartilha entre threads).
    #[test]
    fn start_install_vercel_without_node_warns_prereq() {
        let mut model = DevToolsModel::new_with(Arc::new(Vec::<DiscoveredCli>::new));
        model.block_on_discovery();
        // node AUSENTE (e qualquer outro), determinístico — sem tocar o env.
        model.start_install_with("vercel", &|_| false);
        match model.notice() {
            Some(DevNotice::NeedsFirst { tool, needs }) => {
                assert_eq!(tool, "Vercel CLI");
                assert_eq!(needs, "Node.js");
            }
            other => panic!("esperava NeedsFirst; veio {other:?}"),
        }
        // Não entrou em instalação silenciosa.
        assert!(matches!(model.install_state(), InstallState::Idle));
    }

    /// `reconstruct_command`: `bash -c '<corpo>'` → só o corpo (caminho do terminal real no macOS).
    #[test]
    fn reconstruct_unwraps_bash_dash_c() {
        let r = InstallRecipe {
            program: "bash".into(),
            args: vec!["-c".into(), "brew install git".into()],
            env: Default::default(),
            verify_paths: vec![],
        };
        assert_eq!(reconstruct_command(&r), "brew install git");

        let w = InstallRecipe {
            program: "winget".into(),
            args: vec!["install".into(), "--id".into(), "Git.Git".into()],
            env: Default::default(),
            verify_paths: vec![],
        };
        assert_eq!(reconstruct_command(&w), "winget install --id Git.Git");
    }

    /// O LOOP do critério, headless (reusa `run_install` via override): "instalar" um git falso e a
    /// re-detecção genérica passa a achá-lo COM versão — prova de que o caminho silencioso reusa o
    /// mecanismo dos assistentes.
    #[cfg(unix)]
    #[test]
    fn silent_install_then_discovery_finds_tool() {
        let bin = TempDir::new("instbin");
        let bin_path = bin.path().to_path_buf();
        let src = TempDir::new("instsrc");
        let fake = write_fake_cli(src.path(), "git-src", "git version 2.44.0");
        let script = format!(
            "cp '{}' '{}/git' && chmod +x '{}/git'",
            fake.display(),
            bin_path.display(),
            bin_path.display()
        );
        let recipe =
            install_recipe_with("git", Some(&script), dev_installers()).expect("override install");

        let state = Arc::new(Mutex::new(InstallState::Idle));
        let verify_dir = bin_path.clone();
        let handle = run_install(recipe, Arc::clone(&state), move || {
            discover_dev_tools_in(&verify_dir.display().to_string())
                .into_iter()
                .find(|c| c.id == "git")
        });
        handle.join().expect("join install");

        let final_state = state.lock().expect("lock").clone();
        match final_state {
            InstallState::Ok { version } => {
                assert_eq!(version.as_deref(), Some("git version 2.44.0"));
                assert!(bin_path.join("git").exists());
            }
            other => panic!("esperava Ok após instalar+redetectar; veio {other:?}"),
        }
    }
}
