//! `onboarding` — **W4-1: onboarding T0→T3 + T1 check-up "Instalar para mim"**.
//!
//! Leva um não-técnico das boas-vindas (T0) à criação do 1º Espaço (T3) **sem jargão, só
//! cliques** — views vetoriais (sem grid de terminal nesta fase). O check-up (T1) detecta os
//! CLIs de IA no `PATH` via [`lina_core::discover_clis`] (W4-1 core); se nenhum, oferece
//! **"Instalar para mim"**, que roda o instalador num **PTY oculto** ([`lina_core::PtyManager`]) e
//! a **re-detecção** passa a mostrar o binário com versão (evento [`DomainEvent::DiscoveryIndexed`]).
//! O progresso é **retomável** (persistido em disco): fechar no meio e reabrir cai no passo onde parou.
//!
//! ## Split (igual ao resto do shell: lógica gpui-free + view fina)
//! - [`OnboardingModel`] + helpers (`install_recipe`, `run_install`, `Progress`) são **gpui-free e
//!   testáveis** — toda a lógica vive aqui.
//! - [`OnboardingView`] só renderiza o modelo e roteia cliques.
//!
//! ## Integração com `main.rs` (mínima, anti-colisão com o canvas/W4-2)
//! [`should_show`] decide (env-gated `LINA_ONBOARDING`) e [`open_window`] abre a janela. O canvas
//! (W4-2) é fiado em paralelo; o hand-off onboarding→canvas (após T3) é a integração seguinte —
//! aqui o fluxo fecha em "Espaço criado ✓" (ver `.entrega-w41.md`).
//!
//! **Invariantes (CLAUDE.md):** #1 (zero LLM — detecção é pattern-match no `PATH`), #2 (local-first
//! — só lê `PATH`/roda instalador local, nada sai da máquina), #6 (não-técnico-first: zero jargão,
//! nunca tela em branco, estado sempre salvo e visível; navegação sem becos).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use lina_cli_profiles::{InstallRecipe, Installers};

use gpui::{
    div, prelude::*, px, rgb, size, text, AnyElement, App, Bounds, ClickEvent, Context,
    FocusHandle, FontWeight, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use lina_core::{
    discover_clis, AlacrittyBackend, DiscoveredCli, DomainEvent, EventStore, PtyCommand,
    PtyManager, VtBackend, KNOWN_CLIS,
};
use serde::{Deserialize, Serialize};

// ───────────────────────────── paleta (espelha o canvas) ─────────────────────────────

const BG: u32 = 0x0a0e27;
const PANEL: u32 = 0x141a36;
const ACCENT: u32 = 0x7aa2f7;
const TEXT: u32 = 0xc8d3f5;
const MUTED: u32 = 0x5b658f;
const GREEN: u32 = 0x9ece6a;
const AMBER: u32 = 0xe0af68;
const RED: u32 = 0xf7768e;

// ═══════════════════════════════ máquina de passos (gpui-free) ═══════════════════════════════

/// Os passos do onboarding (T0→T3 + estado final). `Provider` (T2) é um passe-through leve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// T0 — boas-vindas.
    Welcome,
    /// T1 — check-up de CLIs + "Instalar para mim".
    Checkup,
    /// T2 — provedor/conta (passe-through).
    Provider,
    /// T3 — criar o 1º Espaço.
    CreateSpace,
    /// Concluído (Espaço criado ✓).
    Done,
}

impl Step {
    const ORDER: [Step; 5] = [
        Step::Welcome,
        Step::Checkup,
        Step::Provider,
        Step::CreateSpace,
        Step::Done,
    ];

    /// Índice 0..=4 (ordem de progresso) — base da persistência retomável e dos "dots".
    #[must_use]
    pub fn index(self) -> u8 {
        Self::ORDER.iter().position(|s| *s == self).unwrap_or(0) as u8
    }

    /// Passo a partir do índice persistido (clamp ao último; nunca falha).
    #[must_use]
    pub fn from_index(i: u8) -> Step {
        Self::ORDER
            .get(i as usize)
            .copied()
            .unwrap_or(Step::Welcome)
    }

    /// Próximo passo (satura em `Done`).
    #[must_use]
    pub fn next(self) -> Step {
        Self::from_index((self.index() + 1).min(4))
    }

    /// Passo anterior (satura em `Welcome`).
    #[must_use]
    pub fn prev(self) -> Step {
        Self::from_index(self.index().saturating_sub(1))
    }
}

// ═══════════════════════════════ persistência retomável (gpui-free) ═══════════════════════════════

/// Progresso persistido em `<dir>/onboarding.json` — o "passo onde parou" + escolhas. Reabrir o app
/// lê isto e retoma. (Os MARCOS — varredura de CLIs, criação do Espaço — vão para o event log como
/// `DiscoveryIndexed`/`WorkspaceCreated`; este marcador dá a granularidade fina do passo.)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Progress {
    /// Maior índice de passo alcançado (retoma aqui).
    pub step: u8,
    /// O usuário confirmou ter conta no provedor (T2).
    pub provider_ready: bool,
    /// Último CLI que o usuário pediu para instalar (T1).
    pub chosen_cli: Option<String>,
}

/// Caminho do marcador de progresso.
fn progress_path(dir: &Path) -> PathBuf {
    dir.join("onboarding.json")
}

/// Lê o progresso (best-effort: ausência/erro → `default`, começa do T0 — nunca falha).
#[must_use]
pub fn load_progress(dir: &Path) -> Progress {
    match std::fs::read_to_string(progress_path(dir)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Progress::default(),
    }
}

/// Grava o progresso (best-effort: erro é logado em stderr, não derruba o fluxo).
pub fn save_progress(dir: &Path, p: &Progress) {
    let path = progress_path(dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(p) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!(
                    "onboarding: não gravei o progresso em {}: {e}",
                    path.display()
                );
            }
        }
        Err(e) => eprintln!("onboarding: não serializei o progresso: {e}"),
    }
}

// ═══════════════════════════════ "Instalar para mim" (gpui-free) ═══════════════════════════════

/// Receitas de instalação por SO embutidas (config TOML, NÃO hardcoded em Rust — inv#3). Um dir
/// externo opcional `LINA_INSTALLERS` pode adicionar/sobrescrever CLIs sem recompilar (porta aberta).
const INSTALLERS_TOML: &str = include_str!("../../../profiles/installers/recipes.toml");

/// Tabela de receitas parseada uma única vez. TOML inválido → tabela vazia (o botão vira fallback
/// "instale manualmente"); nunca derruba o app.
fn installers() -> &'static Installers {
    static INSTALLERS: OnceLock<Installers> = OnceLock::new();
    INSTALLERS.get_or_init(|| {
        let mut table = Installers::from_toml_str(INSTALLERS_TOML, "profiles/installers/recipes.toml")
            .unwrap_or_else(|e| {
                eprintln!(
                    "onboarding: installers/recipes.toml inválido ({e}); 'Instalar para mim' indisponível"
                );
                Installers::default()
            });
        // Override externo OPCIONAL (sem recompilar — inv#3): `LINA_INSTALLERS=/caminho/recipes.toml`
        // adiciona/sobrescreve receitas por id. Best-effort: arquivo ausente/inválido é só logado.
        if let Some(path) = std::env::var_os("LINA_INSTALLERS") {
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|src| {
                    Installers::from_toml_str(&src, &path.to_string_lossy()).map_err(|e| e.to_string())
                }) {
                Ok(ext) => table.0.extend(ext.0), // entradas externas vencem as embutidas (mesmo id)
                Err(e) => eprintln!("onboarding: LINA_INSTALLERS ignorado ({e})"),
            }
        }
        table
    })
}

/// Estado do trabalho de instalação (mapeia os estados do design: detectando → instalando → ok →
/// falhou). `Verifying` = re-detectando após o instalador sair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    /// Nenhuma instalação em curso.
    Idle,
    /// Instalando — `line` é a última linha do instalador (progresso lido do grid).
    Installing { line: String },
    /// Instalador saiu; re-detectando no `PATH`.
    Verifying,
    /// CLI instalado e re-detectado (com versão, se reportada).
    Ok { version: Option<String> },
    /// Falha acionável (sem jargão) — o usuário pode tentar de novo / instalar manualmente.
    Failed { reason: String },
}

/// Resolve a RECEITA de instalação do CLI p/ o SO atual (puro — testável sem mexer no `env`/TOML
/// reais). Override `LINA_INSTALL_<ID>` (string de shell via `sh -c`) tem prioridade — flexível p/ o
/// fundador apontar um instalador alternativo e p/ os testes injetarem um instalador falso. Sem
/// override, vem da tabela `installers`. `None` = sem receita p/ este CLI/SO → fallback manual.
#[must_use]
pub fn install_recipe_with(
    cli_id: &str,
    env_override: Option<&str>,
    installers: &Installers,
) -> Option<InstallRecipe> {
    if let Some(s) = env_override {
        let s = s.trim();
        if !s.is_empty() {
            return Some(InstallRecipe {
                program: "sh".into(),
                args: vec!["-c".into(), s.to_string()],
                env: Default::default(),
                verify_paths: Vec::new(),
            });
        }
    }
    installers.recipe_for(cli_id).cloned()
}

/// Receita do CLI lendo o override `LINA_INSTALL_<ID>` do ambiente + a tabela embutida.
#[must_use]
pub fn install_recipe(cli_id: &str) -> Option<InstallRecipe> {
    let key = format!("LINA_INSTALL_{}", cli_id.to_ascii_uppercase());
    install_recipe_with(cli_id, std::env::var(&key).ok().as_deref(), installers())
}

/// Atualiza o estado compartilhado de instalação (best-effort sob poison).
fn set_install(state: &Arc<Mutex<InstallState>>, s: InstallState) {
    if let Ok(mut g) = state.lock() {
        *g = s;
    }
}

/// Roda o instalador num **PTY oculto** (terminal não exposto ao usuário): faz spawn do comando,
/// transmite a última linha do grid como progresso e, ao sair, chama `verify` (re-detecção no
/// `PATH`) — a fonte da verdade do sucesso é o CLI **aparecer** depois (não o exit code do npm).
/// `verify` é injetável: produção passa `discover_clis().find(id)`; testes injetam `discover_clis_in`.
pub fn run_install(
    recipe: InstallRecipe,
    state: Arc<Mutex<InstallState>>,
    verify: impl Fn() -> Option<DiscoveredCli> + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        set_install(
            &state,
            InstallState::Installing {
                line: "iniciando…".into(),
            },
        );

        let mut pty = PtyManager::new();
        const KEY: &str = "onboarding-install";
        let mut pcmd = PtyCommand::new(recipe.program).args(recipe.args);
        // `env` da receita garante não-interatividade (ex.: CODEX_NON_INTERACTIVE=1).
        for (k, v) in &recipe.env {
            pcmd = pcmd.env(k, v);
        }
        if let Err(e) = pty.spawn(KEY, pcmd, 100, 30) {
            set_install(
                &state,
                InstallState::Failed {
                    reason: format!("não consegui iniciar a instalação: {e}"),
                },
            );
            return;
        }
        let reader = match pty.clone_reader(KEY) {
            Ok(r) => r,
            Err(e) => {
                set_install(
                    &state,
                    InstallState::Failed {
                        reason: format!("não consegui ler o progresso da instalação: {e}"),
                    },
                );
                let _ = pty.kill(KEY, Duration::from_secs(1));
                return;
            }
        };

        // PTY oculto: as linhas do instalador alimentam um grid VT só para LER o progresso (não é
        // mostrado como terminal — só a última linha vira texto de status). EOF (`Ok(0)`) = saiu.
        let mut grid = AlacrittyBackend::new(100, 30);
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    grid.advance(&buf[..n]);
                    let line = grid.last_nonempty_line();
                    let line = if line.trim().is_empty() {
                        "instalando…".to_string()
                    } else {
                        line.trim().to_string()
                    };
                    set_install(&state, InstallState::Installing { line });
                }
            }
        }

        set_install(&state, InstallState::Verifying);
        let _ = pty.kill(KEY, Duration::from_secs(1));
        match verify() {
            Some(found) => set_install(
                &state,
                InstallState::Ok {
                    version: found.version,
                },
            ),
            None => set_install(
                &state,
                InstallState::Failed {
                    reason:
                        "a instalação terminou, mas o programa não apareceu — tente de novo ou \
                             instale manualmente"
                            .into(),
                },
            ),
        }
    })
}

// ═══════════════════════════════ o modelo (gpui-free) ═══════════════════════════════

/// Função de descoberta de CLIs (INJETÁVEL): produção usa [`lina_core::discover_clis`]; testes
/// injetam um `discover_clis_in` controlado — **sem rodar `--version` em binários reais**, então é
/// determinístico e não trava. (O `query_version` do core é síncrono e SEM timeout — um `--version`
/// pendurado travaria; por isso a descoberta roda **fora da thread de UI** — ver `redetect`. Um
/// timeout no `query_version` do core fecharia o furo de raiz — ver `.entrega-w41.md`.)
pub type DiscoverFn = Arc<dyn Fn() -> Vec<DiscoveredCli> + Send + Sync>;

/// Estado completo do onboarding, sem nenhum tipo de gpui. A view o possui e renderiza.
pub struct OnboardingModel {
    dir: PathBuf,
    store: Option<Arc<Mutex<EventStore>>>,
    step: Step,
    /// Snapshot compartilhado da última varredura (escrito pela thread de descoberta, lido pela view).
    detected: Arc<Mutex<Vec<DiscoveredCli>>>,
    /// `true` enquanto uma varredura roda em background (a view mostra "procurando…").
    discovering: Arc<AtomicBool>,
    discovery_handle: Option<thread::JoinHandle<()>>,
    /// Como descobrir CLIs (injetável p/ teste determinístico).
    discover: DiscoverFn,
    install: Arc<Mutex<InstallState>>,
    install_handle: Option<thread::JoinHandle<()>>,
    install_target: Option<String>,
    install_consumed: bool,
    provider_ready: bool,
}

impl OnboardingModel {
    /// Carrega o modelo com a descoberta REAL (`discover_clis`).
    pub fn load(dir: PathBuf) -> Self {
        Self::load_with(dir, Arc::new(discover_clis))
    }

    /// Como [`OnboardingModel::load`], mas com a descoberta INJETADA (testes passam uma varredura
    /// controlada que não roda `--version` em binários reais — determinística e sem travar).
    pub fn load_with(dir: PathBuf, discover: DiscoverFn) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let store = match EventStore::open(dir.join("events")) {
            Ok(s) => Some(Arc::new(Mutex::new(s))),
            Err(e) => {
                eprintln!("onboarding: sem persistência de eventos ({e}); fluxo segue em memória");
                None
            }
        };
        let progress = load_progress(&dir);
        let mut model = Self {
            dir,
            store,
            step: Step::from_index(progress.step),
            detected: Arc::new(Mutex::new(Vec::new())),
            discovering: Arc::new(AtomicBool::new(false)),
            discovery_handle: None,
            discover,
            install: Arc::new(Mutex::new(InstallState::Idle)),
            install_handle: None,
            install_target: progress.chosen_cli.clone(),
            install_consumed: true,
            provider_ready: progress.provider_ready,
        };
        model.redetect();
        model
    }

    /// Passo corrente.
    #[must_use]
    pub fn step(&self) -> Step {
        self.step
    }

    /// Estado atual da instalação (clone do compartilhado).
    #[must_use]
    pub fn install_state(&self) -> InstallState {
        self.install
            .lock()
            .map(|g| g.clone())
            .unwrap_or(InstallState::Idle)
    }

    /// O CLI alvo da instalação corrente (para a view destacar a linha certa).
    #[must_use]
    pub fn install_target(&self) -> Option<&str> {
        self.install_target.as_deref()
    }

    /// Snapshot dos CLIs detectados (clone do compartilhado).
    fn detected(&self) -> Vec<DiscoveredCli> {
        self.detected.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Versão detectada de um CLI (ou `None` se ausente).
    #[must_use]
    pub fn version_of(&self, id: &str) -> Option<String> {
        self.detected()
            .into_iter()
            .find(|c| c.id == id)
            .and_then(|c| c.version)
    }

    /// `true` se o CLI está presente no `PATH`.
    #[must_use]
    pub fn is_present(&self, id: &str) -> bool {
        self.detected().iter().any(|c| c.id == id)
    }

    /// `true` se NENHUM CLI foi encontrado (texto "nenhum CLI encontrado" no T1).
    #[must_use]
    pub fn nothing_found(&self) -> bool {
        self.detected().is_empty()
    }

    /// `true` enquanto a varredura roda (a view mostra "procurando…", não "nenhum CLI").
    #[must_use]
    pub fn is_discovering(&self) -> bool {
        self.discovering.load(Ordering::SeqCst)
    }

    /// Re-varre o `PATH` em **BACKGROUND** (não trava a UI nem mesmo se um `--version` pendurar) e
    /// LOGA `DiscoveryIndexed` ao terminar — é o que faz o binário recém-instalado "aparecer com
    /// versão". Idempotente: ignora se já há uma varredura em voo.
    pub fn redetect(&mut self) {
        if self.discovering.swap(true, Ordering::SeqCst) {
            return; // já há uma varredura em voo
        }
        let discover = Arc::clone(&self.discover);
        let detected = Arc::clone(&self.detected);
        let discovering = Arc::clone(&self.discovering);
        let store = self.store.clone();
        let handle = thread::spawn(move || {
            let clis = discover();
            if let Ok(mut d) = detected.lock() {
                *d = clis.clone();
            }
            if let Some(store) = store {
                if let Ok(mut g) = store.lock() {
                    if let Err(e) = g.append(&DomainEvent::DiscoveryIndexed { clis }) {
                        eprintln!("onboarding: falha ao apendar DiscoveryIndexed: {e}");
                    }
                }
            }
            discovering.store(false, Ordering::SeqCst);
        });
        self.discovery_handle = Some(handle);
    }

    /// Bloqueia até a varredura corrente terminar. **Test-only** (determinismo): a produção nunca
    /// bloqueia — a view reflete o snapshot por frame. (`allow(dead_code)`: usado só nos testes, mas
    /// mantido fora de `cfg(test)` para LER `discovery_handle` também no build de produção.)
    #[allow(dead_code)]
    pub fn block_on_discovery(&mut self) {
        if let Some(h) = self.discovery_handle.take() {
            let _ = h.join();
        }
    }

    /// Inicia "Instalar para mim" para `cli_id` (PTY oculto). Sem comando conhecido → falha acionável.
    pub fn start_install(&mut self, cli_id: &str) {
        // Não empilha instalações: ignora se já há uma em curso.
        if matches!(
            self.install_state(),
            InstallState::Installing { .. } | InstallState::Verifying
        ) {
            return;
        }
        let Some(recipe) = install_recipe(cli_id) else {
            set_install(
                &self.install,
                InstallState::Failed {
                    reason: format!(
                        "ainda não sei instalar '{cli_id}' neste sistema automaticamente — instale \
                         manualmente e clique em Verificar"
                    ),
                },
            );
            return;
        };
        self.install_target = Some(cli_id.to_string());
        self.install_consumed = false;
        self.save();
        // Feedback OTIMISTA: marca Installing já (a thread confirma em seguida) para que o 1º frame
        // após o clique mostre "⟳ iniciando…" no lugar do botão — sem isto há uma janela em que o
        // estado segue Idle e o usuário acha que "nada aconteceu".
        set_install(
            &self.install,
            InstallState::Installing {
                line: "iniciando…".into(),
            },
        );
        let id = cli_id.to_string();
        let verify_paths = recipe.verify_paths.clone();
        // `verify` reusa a descoberta injetada (consistente em teste/produção); roda na thread de
        // instalação. ANTES de descobrir, RE-HIDRATA o PATH: o instalador acabou de escrever seu dir
        // no rc do shell (um login shell novo já o enxerga) e unimos os `verify_paths` fixos da receita
        // (ex.: ~/.opencode/bin). Grava o PATH no processo → a verificação acha o binário recém-instalado
        // E o canvas já consegue spawná-lo sem reabrir o app. (Era o furo do "instalou mas não apareceu".)
        let discover = Arc::clone(&self.discover);
        let handle = run_install(recipe, Arc::clone(&self.install), move || {
            crate::refresh_path_after_install(&verify_paths);
            discover().into_iter().find(|c| c.id == id)
        });
        self.install_handle = Some(handle);
    }

    /// Chamado a cada frame: quando a instalação conclui (`Ok`), re-detecta UMA vez para refletir o
    /// novo binário (idempotente via `install_consumed`).
    pub fn poll_install(&mut self) {
        if self.install_consumed {
            return;
        }
        if matches!(self.install_state(), InstallState::Ok { .. }) {
            self.install_consumed = true;
            self.redetect();
        }
    }

    /// Re-detecção manual (botão "Verificar de novo") — útil após uma instalação manual.
    pub fn verify_now(&mut self) {
        self.redetect();
    }

    /// Avança para o próximo passo (persiste). T2→`provider_ready` é marcado por [`set_provider_ready`].
    pub fn advance(&mut self) {
        self.go(self.step.next());
    }

    /// Volta um passo (navegação sem becos: todo passo tem saída).
    pub fn back(&mut self) {
        self.go(self.step.prev());
    }

    /// Marca que o usuário tem conta no provedor (T2) e avança.
    pub fn set_provider_ready(&mut self, ready: bool) {
        self.provider_ready = ready;
        self.save();
        self.advance();
    }

    /// T3 — cria o 1º Espaço: loga `WorkspaceCreated` (preset não-setado; a galeria de Foco é W4-5)
    /// e conclui o onboarding.
    pub fn create_space(&mut self) {
        self.append(DomainEvent::WorkspaceCreated {
            name: "Meu primeiro Espaço".into(),
            focus_preset: String::new(),
        });
        self.go(Step::Done);
    }

    // ── internos ──

    fn go(&mut self, step: Step) {
        self.step = step;
        self.save();
    }

    /// Persiste o progresso no passo CORRENTE (o AC exige "reabrir cai no passo ONDE PAROU" — então
    /// `back()` REGRIDE o marcador; nada de high-water mark, que teleportaria o usuário à frente).
    fn save(&self) {
        let p = Progress {
            step: self.step.index(),
            provider_ready: self.provider_ready,
            chosen_cli: self.install_target.clone(),
        };
        save_progress(&self.dir, &p);
    }

    fn append(&self, event: DomainEvent) {
        if let Some(store) = &self.store {
            if let Ok(mut g) = store.lock() {
                if let Err(e) = g.append(&event) {
                    eprintln!("onboarding: falha ao apendar {}: {e}", event.kind());
                }
            }
        }
    }
}

// ═══════════════════════════════ entrada (main.rs) ═══════════════════════════════

/// Decide (PURO — testável sem env nem disco) se o onboarding deve aparecer, dado o passo persistido,
/// o override de ambiente e o modo demo. **PRODUÇÃO:** aparece na **1ª execução** (o usuário ainda não
/// concluiu, `step < Done`) e some depois — sem env nenhuma. **Override de dev:**
/// `LINA_ONBOARDING=1|force|true` força mostrar; `=0|false|off` força pular (útil pra testar o canvas).
/// **Demo** (canvas do fundador) pula por padrão (não estorva a apresentação).
#[must_use]
pub fn decide_show(progress_step: u8, env_override: Option<&str>, demo: bool) -> bool {
    match env_override.map(str::trim) {
        Some("1") | Some("force") | Some("true") => true,
        Some("0") | Some("false") | Some("off") => false,
        _ => !demo && progress_step < Step::Done.index(),
    }
}

/// Decide se o onboarding deve aparecer lendo o progresso persistido em `dir` + o env `LINA_ONBOARDING`.
/// Wrapper fino sobre [`decide_show`] (puro e testado): o boot só vê esta função.
#[must_use]
pub fn should_show(dir: &Path, demo: bool) -> bool {
    decide_show(
        load_progress(dir).step,
        std::env::var("LINA_ONBOARDING").ok().as_deref(),
        demo,
    )
}

/// Abre a janela do onboarding (chamada de dentro do `application().run` de `main.rs`). `dir` é o
/// diretório PERSISTENTE de estado do onboarding (progresso + log próprio) — em produção mora no
/// Application Support do usuário (ver `main.rs`), nunca em `temp`.
pub fn open_window(cx: &mut App, dir: PathBuf) {
    let bounds = Bounds::centered(None, size(px(920.0), px(640.0)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Lina Space — primeiros passos".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| OnboardingView::new(dir, window, cx)),
    );
    if let Err(e) = opened {
        eprintln!("onboarding: não abri a janela: {e}");
    }
}

// ═══════════════════════════════ a view gpui (fina) ═══════════════════════════════

/// View gpui do onboarding — só renderiza o [`OnboardingModel`] e roteia cliques.
pub struct OnboardingView {
    model: OnboardingModel,
    focus: FocusHandle,
}

impl OnboardingView {
    fn new(dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        // PULSO DE ANIMAÇÃO (mesmo mecanismo do canvas — ver `main.rs`). O gpui só redesenha quando a
        // view está "dirty"; o `request_animation_frame` do `render` SÓ se auto-sustenta enquanto
        // CHEGAM frames do SO, e o trabalho assíncrono do onboarding (varredura de CLIs + "Instalar
        // para mim") muda o estado a partir de uma thread GPUI-FREE — que NÃO acorda o event loop.
        // Sem este loop, o clique dispara a instalação (o `npm` roda no PTY oculto), mas a tela
        // CONGELA no frame do clique e o usuário vê "nada acontece". Aqui forçamos `cx.notify()`
        // enquanto há varredura/instalação em voo (mais o frame de assentamento, p/ desenhar o ✓/⚠
        // final), dormindo barato quando ocioso. Encerra sozinho quando a view é dropada (janela
        // fechada → `this.update` devolve `Err`).
        cx.spawn(async move |this, cx| {
            let mut was_busy = false;
            loop {
                let Ok(busy) = this.update(cx, |view, cx| {
                    view.model.poll_install();
                    let busy = matches!(
                        view.model.install_state(),
                        InstallState::Installing { .. } | InstallState::Verifying
                    ) || view.model.is_discovering();
                    // `busy || was_busy`: redesenha durante o trabalho E no 1º frame após ele terminar,
                    // p/ que o estado final (Ok ✓ / Failed ⚠ / lista re-detectada) realmente apareça.
                    if busy || was_busy {
                        cx.notify();
                    }
                    busy
                }) else {
                    break; // view dropada (app/janela fechando) → encerra o pulso.
                };
                was_busy = busy;
                cx.background_executor()
                    .timer(Duration::from_millis(if busy { 16 } else { 120 }))
                    .await;
            }
        })
        .detach();

        Self {
            model: OnboardingModel::load(dir),
            focus,
        }
    }

    /// Botão primário (ação de avançar / confirmar).
    fn primary_button(
        &self,
        id: &'static str,
        label: impl Into<String>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let label = label.into();
        div()
            .id(id)
            .px_5()
            .py_2()
            .rounded_md()
            .bg(rgb(0x2c7a4b))
            .text_color(rgb(0xeef1ff))
            .font_weight(FontWeight::BOLD)
            .cursor_pointer()
            .on_click(cx.listener(move |view, _ev: &ClickEvent, window, cx| {
                on_click(view, window, cx);
            }))
            .child(text!(label))
            .into_any_element()
    }

    /// Botão secundário (voltar / escolha alternativa).
    fn ghost_button(
        &self,
        id: &'static str,
        label: impl Into<String>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let label = label.into();
        div()
            .id(id)
            .px_4()
            .py_2()
            .rounded_md()
            .bg(rgb(0x2a3152))
            .text_color(rgb(TEXT))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _ev: &ClickEvent, window, cx| {
                on_click(view, window, cx);
            }))
            .child(text!(label))
            .into_any_element()
    }

    /// Cabeçalho de progresso (dots dos passos) — sempre visível: o usuário nunca se perde.
    fn step_dots(&self) -> AnyElement {
        let cur = self.model.step().index();
        let mut row = div().flex().flex_row().gap_2().items_center();
        for i in 0..4u8 {
            let on = i <= cur.min(3);
            row = row.child(
                div()
                    .w(px(if i == cur { 26.0 } else { 12.0 }))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(rgb(if on { ACCENT } else { 0x2a3152 })),
            );
        }
        row.into_any_element()
    }

    /// Caixa de título + subtítulo de um passo.
    fn heading(&self, title: &str, subtitle: &str) -> AnyElement {
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

    // ── conteúdo por passo ──

    fn render_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.heading(
                "Bem-vindo ao Lina Space",
                "Seu time de assistentes de IA, trabalhando junto numa tela só. Vamos configurar em 3 passos rápidos.",
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .child(self.primary_button("welcome-next", "Começar →", cx, |v, _w, _cx| {
                        v.model.advance();
                    })),
            )
            .into_any_element()
    }

    fn render_checkup(&self, cx: &mut Context<Self>) -> AnyElement {
        let install = self.model.install_state();
        let mut col = div().flex().flex_col().gap_5().child(self.heading(
            "Vamos ver o que você já tem",
            "Procuramos os assistentes de IA no seu computador. Se faltar algum, eu instalo para você.",
        ));

        // Banner de estado da varredura: "procurando…" enquanto roda (não trava a UI); só depois,
        // se vazia, "nenhum CLI encontrado" (critério do design) — nunca um falso "nenhum" durante a busca.
        if self.model.is_discovering() {
            col = col.child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(PANEL))
                    .text_color(rgb(ACCENT))
                    .child(text!("Procurando assistentes no seu computador…")),
            );
        } else if self.model.nothing_found() {
            col = col.child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(0x33202c))
                    .text_color(rgb(AMBER))
                    .child(text!(
                        "Nenhum assistente encontrado ainda — escolha um abaixo e clique \"Instalar para mim\"."
                    )),
            );
        }

        // Uma linha por CLI conhecido: presente (✓ versão) ou ausente ([Instalar para mim]).
        let mut list = div().flex().flex_col().gap_2();
        for id in KNOWN_CLIS {
            let id = *id;
            let present = self.model.is_present(id);
            let installing_this = self.model.install_target() == Some(id)
                && matches!(
                    install,
                    InstallState::Installing { .. } | InstallState::Verifying
                );

            // `.id(id)` (id único por CLI) é OBRIGATÓRIO p/ a11y: o `text!` do gpui gera o ElementId
            // por LOCALIZAÇÃO no fonte (não por conteúdo), então `text!` repetidos no MESMO ponto do
            // laço colidiriam no id do nó AccessKit (debug_assert! → pânico com leitor de tela ativo).
            // Um ancestral com id distinto por linha desambigua todos os `text!` descendentes
            // (mesmo padrão do `.id(("card", eid))` do canvas em main.rs).
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
                        .w(px(120.0))
                        .text_color(rgb(TEXT))
                        .font_weight(FontWeight::BOLD)
                        .child(text!(id.to_string())),
                );

            if present {
                let ver = self
                    .model
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
                    .child(self.install_button(id, cx));
            }
            list = list.child(row);
        }
        col = col.child(list);

        // Mensagem de falha acionável (sem jargão).
        if let InstallState::Failed { reason } = &install {
            col = col.child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(0x33202c))
                    .text_color(rgb(RED))
                    .child(text!(format!("⚠ {reason}"))),
            );
        }

        // Rodapé: Voltar · Verificar de novo · Continuar (sempre pode seguir — nunca um beco).
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    self.ghost_button("checkup-back", "← Voltar", cx, |v, _w, _cx| {
                        v.model.back();
                    }),
                )
                .child(self.ghost_button(
                    "checkup-verify",
                    "↻ Verificar de novo",
                    cx,
                    |v, _w, _cx| v.model.verify_now(),
                ))
                .child(div().flex_1())
                .child(
                    self.primary_button("checkup-next", "Continuar →", cx, |v, _w, _cx| {
                        v.model.advance();
                    }),
                ),
        );
        col.into_any_element()
    }

    /// Botão "Instalar para mim" de um CLI.
    fn install_button(&self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(id)
            .px_4()
            .py_2()
            .rounded_md()
            .bg(rgb(0x3d59c9))
            .text_color(rgb(0xeef1ff))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, cx| {
                view.model.start_install(id);
                cx.notify(); // desenha o feedback NA HORA; o pulso assume a animação do progresso
            }))
            .child(text!("Instalar para mim"))
            .into_any_element()
    }

    fn render_provider(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.heading(
                "Sua conta de IA",
                "Os assistentes usam sua conta no provedor (ex.: Claude). Você já tem uma?",
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(self.primary_button(
                        "provider-yes",
                        "Sim, já tenho conta →",
                        cx,
                        |v, _w, _cx| v.model.set_provider_ready(true),
                    ))
                    .child(self.ghost_button(
                        "provider-later",
                        "Configurar depois →",
                        cx,
                        |v, _w, _cx| v.model.set_provider_ready(false),
                    )),
            )
            .child(div().flex().flex_row().child(self.ghost_button(
                "provider-back",
                "← Voltar",
                cx,
                |v, _w, _cx| {
                    v.model.back();
                },
            )))
            .into_any_element()
    }

    fn render_create(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.heading(
                "Pronto para começar",
                "Vou criar seu primeiro Espaço — uma tela onde seus assistentes trabalham juntos.",
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        self.ghost_button("create-back", "← Voltar", cx, |v, _w, _cx| {
                            v.model.back();
                        }),
                    )
                    .child(div().flex_1())
                    .child(self.primary_button(
                        "create-space",
                        "✨ Criar meu Espaço",
                        cx,
                        |v, _w, _cx| v.model.create_space(),
                    )),
            )
            .into_any_element()
    }

    fn render_done(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .items_center()
            .child(div().text_size(px(40.0)).child(text!("✓")))
            .child(
                div()
                    .text_size(px(26.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(GREEN))
                    .child(text!("Espaço criado!")),
            )
            .child(
                div()
                    .text_size(px(15.0))
                    .text_color(rgb(MUTED))
                    .child(text!(
                    "Tudo pronto. Seu Espaço foi salvo — abra o canvas para começar a trabalhar."
                )),
            )
            // Handoff (inv#6 — sem becos sem saída): fechar esta janela revela o canvas, que já está
            // aberto por baixo. `window.remove_window()` encerra só a janela do onboarding, não o app.
            .child(self.primary_button(
                "open-canvas",
                "Abrir meu Espaço →",
                cx,
                |_v, window, _cx| window.remove_window(),
            ))
            .into_any_element()
    }
}

impl Render for OnboardingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Anima/poll: o trabalho de instalação roda numa thread; cada frame reflete o progresso e,
        // ao concluir, dispara a re-detecção.
        window.request_animation_frame();
        self.model.poll_install();

        let content = match self.model.step() {
            Step::Welcome => self.render_welcome(cx),
            Step::Checkup => self.render_checkup(cx),
            Step::Provider => self.render_provider(cx),
            Step::CreateSpace => self.render_create(cx),
            Step::Done => self.render_done(cx),
        };

        div()
            .id("onboarding")
            .track_focus(&self.focus)
            .size_full()
            // Conteúdo do check-up (heading + banner + N CLIs + rodapé) passa da altura da janela:
            // sem isto o excesso é só CLIPADO (rodapé "Continuar →" inalcançável). `size_full` dá a
            // altura limitada e o `.id("onboarding")` persiste o offset → o scroll engata.
            .overflow_y_scroll()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .flex()
            .flex_col()
            .items_center()
            .child(
                // Painel central com largura confortável de leitura.
                div()
                    .flex()
                    .flex_col()
                    // `div()` tem `flex_shrink: 1.0` por padrão: dentro do root (coluna de altura
                    // LIMITADA = janela), o taffy ENCOLHERIA este painel até a altura da janela →
                    // `content_size == bounds` → nada a rolar (era por isso que o scroll não engatava;
                    // só esticar a janela revelava os botões). `flex_shrink_0` mantém a altura NATURAL
                    // do conteúdo, que transborda e o `overflow_y_scroll` do root rola. (Idioma do
                    // Zed: context_menu/modal usam flex_shrink_0 + overflow_y_scroll.)
                    .flex_shrink_0()
                    .gap_8()
                    .w(px(680.0))
                    .mt(px(72.0))
                    .pb(px(40.0)) // respiro no fim do scroll: o último botão não cola na borda
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .text_color(rgb(ACCENT))
                                    .font_weight(FontWeight::BOLD)
                                    .child(text!("Lina Space")),
                            )
                            .child(div().flex_1())
                            .child(self.step_dots()),
                    )
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "lina-onb-{tag}-{}-{}",
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
    fn write_fake_cli(dir: &Path, id: &str, version_line: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(id);
        std::fs::write(&p, format!("#!/bin/sh\necho '{version_line}'\n")).expect("escrever cli");
        let mut perm = std::fs::metadata(&p).expect("meta").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&p, perm).expect("chmod");
        p
    }

    /// Máquina de passos: índices, next/prev saturam nas pontas, round-trip.
    #[test]
    fn step_machine_orders_and_saturates() {
        assert_eq!(Step::Welcome.index(), 0);
        assert_eq!(Step::Done.index(), 4);
        assert_eq!(Step::Welcome.prev(), Step::Welcome); // satura
        assert_eq!(Step::Done.next(), Step::Done); // satura
        assert_eq!(Step::Welcome.next(), Step::Checkup);
        assert_eq!(Step::CreateSpace.next(), Step::Done);
        for s in Step::ORDER {
            assert_eq!(Step::from_index(s.index()), s);
        }
        assert_eq!(Step::from_index(99), Step::Welcome); // clamp inválido
    }

    /// `decide_show` (puro): PRODUÇÃO mostra na 1ª execução (passo < Done) e some ao concluir; demo pula
    /// por padrão; o override de dev força mostrar/pular ignorando progresso e demo. É a lógica nova de
    /// "should_show por progresso, não por env" — testada sem tocar o `env` real (determinístico).
    #[test]
    fn decide_show_first_run_then_hidden() {
        let done = Step::Done.index();
        // 1ª execução: passo antes de Done, sem override, fora do demo → MOSTRA.
        assert!(decide_show(Step::Welcome.index(), None, false));
        assert!(decide_show(Step::CreateSpace.index(), None, false));
        // Concluído (Done persistido) → SOME (returning user cai direto no canvas).
        assert!(!decide_show(done, None, false));
        // Demo pula por padrão, mesmo na 1ª execução (não estorva o canvas do fundador).
        assert!(!decide_show(Step::Welcome.index(), None, true));
        // Override de dev vence tudo: força mostrar (mesmo concluído/demo) ou pular (mesmo 1ª execução).
        assert!(decide_show(done, Some("1"), true));
        assert!(decide_show(done, Some("force"), false));
        assert!(!decide_show(Step::Welcome.index(), Some("0"), false));
        assert!(!decide_show(Step::Welcome.index(), Some("false"), false));
        // Override desconhecido → ignora e cai na regra de progresso.
        assert!(decide_show(Step::Welcome.index(), Some("talvez"), false));
    }

    /// Progresso é retomável: grava e relê o passo/escolhas.
    #[test]
    fn progress_roundtrips_for_resume() {
        let tmp = TempDir::new("progress");
        assert_eq!(load_progress(tmp.path()), Progress::default());
        let p = Progress {
            step: 2,
            provider_ready: true,
            chosen_cli: Some("claude".into()),
        };
        save_progress(tmp.path(), &p);
        assert_eq!(load_progress(tmp.path()), p);
    }

    /// Resolução de receita (puro): override `sh -c` tem prioridade; a tabela embutida resolve os CLIs
    /// conhecidos p/ o SO atual; CLI desconhecido → None (fallback manual). Regressão do copilot:
    /// usa `@github/copilot` (binário `copilot`), NUNCA o deprecado `github-copilot-cli`.
    #[test]
    fn install_recipe_resolution_and_override() {
        let inst = installers();
        // Override vence tudo e roda via `sh -c`.
        let over = install_recipe_with("claude", Some("echo oi"), inst).expect("override");
        assert_eq!(over.program, "sh");
        assert_eq!(over.args, vec!["-c".to_string(), "echo oi".to_string()]);
        // Override vazio cai na tabela.
        assert_eq!(
            install_recipe_with("claude", Some("   "), inst),
            install_recipe_with("claude", None, inst)
        );
        // CLI conhecido → receita do SO atual com programa não-vazio.
        let claude = install_recipe_with("claude", None, inst).expect("claude tem receita");
        assert!(!claude.program.trim().is_empty());
        // CLI desconhecido → None (fallback manual).
        assert!(install_recipe_with("desconhecido", None, inst).is_none());
        // Regressão copilot: `@github/copilot`, nunca o pacote deprecado.
        let copilot = install_recipe_with("copilot", None, inst).expect("copilot tem receita");
        let joined = copilot.args.join(" ");
        assert!(joined.contains("@github/copilot"), "veio {joined:?}");
        assert!(!joined.contains("githubnext"));
    }

    /// O LOOP do critério, headless: "instalar" (fake) põe o binário no PATH e a re-detecção
    /// (`verify`) passa a achá-lo COM versão — exatamente o AC ("install + re-detecção mostra binário").
    #[cfg(unix)]
    #[test]
    fn run_install_then_redetect_finds_binary() {
        use lina_core::discover_clis_in;
        let bin = TempDir::new("instbin");
        let bin_path = bin.path().to_path_buf();
        // O "instalador" copia um claude falso (que responde --version) para o dir de PATH.
        let src = TempDir::new("instsrc");
        let fake = write_fake_cli(src.path(), "claude-src", "claude 4.2.1");
        let script = format!(
            "cp '{}' '{}/claude' && chmod +x '{}/claude'",
            fake.display(),
            bin_path.display(),
            bin_path.display()
        );
        let cmd = install_recipe_with("claude", Some(&script), installers()).expect("override install");

        let state = Arc::new(Mutex::new(InstallState::Idle));
        let verify_dir = bin_path.clone();
        let handle = run_install(cmd, Arc::clone(&state), move || {
            discover_clis_in(&verify_dir.display().to_string())
                .into_iter()
                .find(|c| c.id == "claude")
        });
        handle.join().expect("join install");

        let final_state = state.lock().expect("lock").clone();
        match final_state {
            InstallState::Ok { version } => {
                assert_eq!(version.as_deref(), Some("claude 4.2.1"));
                // o binário REALMENTE está no PATH simulado (prova do "which <cli>" do AC).
                assert!(bin_path.join("claude").exists());
            }
            other => panic!("esperava Ok após instalar+redetectar; veio {other:?}"),
        }
    }

    /// Instalador que NÃO instala nada → falha acionável (não Ok).
    #[cfg(unix)]
    #[test]
    fn run_install_failure_is_actionable() {
        use lina_core::discover_clis_in;
        let empty = TempDir::new("empty");
        let dir = empty.path().to_path_buf();
        let cmd = install_recipe_with("claude", Some("true"), installers()).expect("noop install");
        let state = Arc::new(Mutex::new(InstallState::Idle));
        let handle = run_install(cmd, Arc::clone(&state), move || {
            discover_clis_in(&dir.display().to_string())
                .into_iter()
                .find(|c| c.id == "claude")
        });
        handle.join().expect("join");
        let final_state = state.lock().expect("lock").clone();
        assert!(matches!(final_state, InstallState::Failed { .. }));
    }

    /// Descoberta INJETADA vazia (determinística, sem subprocesso real — não trava como o
    /// `discover_clis` real faria sobre os CLIs do host).
    fn empty_discover() -> DiscoverFn {
        Arc::new(Vec::<DiscoveredCli>::new)
    }

    /// `load_with` retoma o passo persistido e a 1ª varredura (em background) loga `DiscoveryIndexed`.
    #[test]
    fn model_resumes_step_and_logs_discovery() {
        let tmp = TempDir::new("model");
        save_progress(
            tmp.path(),
            &Progress {
                step: 2,
                provider_ready: false,
                chosen_cli: None,
            },
        );
        let mut model = OnboardingModel::load_with(tmp.path().to_path_buf(), empty_discover());
        assert_eq!(model.step(), Step::Provider, "retomou no passo persistido");
        model.block_on_discovery(); // espera a varredura de background concluir (determinismo)
        let store = EventStore::open(tmp.path().join("events")).expect("store");
        let n = store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "DiscoveryIndexed")
            .count();
        assert!(n >= 1, "load deve logar ao menos um DiscoveryIndexed");
    }

    /// A re-detecção roda FORA da thread de UI e popula o snapshot compartilhado (a view o lê por
    /// frame). Prova `is_present`/`version_of`/`nothing_found` sobre a descoberta injetada.
    #[test]
    fn redetect_off_thread_populates_detected() {
        let tmp = TempDir::new("redetect");
        let found = vec![DiscoveredCli {
            id: "claude".into(),
            version: Some("claude 4.2.1".into()),
            path: "/x/claude".into(),
        }];
        let disc: DiscoverFn = Arc::new(move || found.clone());
        let mut model = OnboardingModel::load_with(tmp.path().to_path_buf(), disc);
        model.block_on_discovery();
        assert!(model.is_present("claude"));
        assert_eq!(model.version_of("claude").as_deref(), Some("claude 4.2.1"));
        assert!(!model.nothing_found());
        assert!(!model.is_discovering(), "varredura concluída");
    }

    /// Retomabilidade FIEL ao AC: avançar até CreateSpace, voltar p/ Provider e "fechar" → reabrir
    /// cai em **Provider** (onde parou), NÃO em CreateSpace (mais longe alcançado). Regride o marcador.
    #[test]
    fn resume_lands_where_user_stopped_not_furthest() {
        let tmp = TempDir::new("resume-back");
        {
            let mut model = OnboardingModel::load_with(tmp.path().to_path_buf(), empty_discover());
            model.block_on_discovery();
            model.advance(); // Welcome → Checkup
            model.advance(); // Checkup → Provider
            model.advance(); // Provider → CreateSpace
            assert_eq!(model.step(), Step::CreateSpace);
            model.back(); // CreateSpace → Provider (usuário recuou antes de fechar)
            assert_eq!(model.step(), Step::Provider);
        } // model dropado = "fechou o app"
        let model = OnboardingModel::load_with(tmp.path().to_path_buf(), empty_discover());
        assert_eq!(
            model.step(),
            Step::Provider,
            "reabrir deve cair no passo onde parou (Provider), não no mais longe (CreateSpace)"
        );
    }

    /// `create_space` loga `WorkspaceCreated` e conclui.
    #[test]
    fn create_space_logs_workspace_and_finishes() {
        let tmp = TempDir::new("create");
        let mut model = OnboardingModel::load_with(tmp.path().to_path_buf(), empty_discover());
        model.block_on_discovery();
        model.create_space();
        assert_eq!(model.step(), Step::Done);
        let store = EventStore::open(tmp.path().join("events")).expect("store");
        assert!(store
            .events()
            .expect("events")
            .into_iter()
            .any(|r| r.kind == "WorkspaceCreated"));
    }
}
