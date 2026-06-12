//! `persistence_ui` — **W4-4: "Tudo salvo ✓" + recuperação visível (T8) + Espaços (T6) + Ajustes (T7)**.
//!
//! O **chrome de confiança**: o usuário nunca duvida que o trabalho está salvo, vê a recuperação
//! pós-crash em vez de um silêncio assustador, troca de Espaço (T6) e ajusta preferências (T7).
//!
//! **NÃO reimplementa persistência** (é W0-5/W0-6, já no core). Só APRESENTA:
//! - **Salvo ✓**: deriva do `SharedModel.event_count` (já atualizado a cada append do core). Quando o
//!   contador sobe, o evento JÁ foi flushed (`append_jsonl` faz `flush`) → mostrar "✓" nunca mente; o
//!   "salvando…" é o transiente do(s) frame(s) em que o contador muda. (Um "salvando…" durante o
//!   fsync exigiria os WRITERS usarem `EventStore::append_with_flush`+`FlushState` e rotearem ao UI —
//!   mudança nos donos do bridge/pump; ver `.entrega-w44.md`.)
//! - **T8 recuperação**: REUSA o W0-6 — `EventStore::open_or_recover` (emite `Recovering`/`Recovered`
//!   via `UiHost`) + o artefato preservado `*.corrupt-*`. Apresenta, não recupera.
//! - **T6 Espaços**: projeção (`EventStore::project`) de cada workspace; troca loga `WorkspaceFocusSet`.
//! - **T7 Ajustes**: `settings.json` (persiste após reabrir). (Event-sourcing real exigiria um
//!   `SettingChanged` no core — ver `.entrega`.)
//!
//! Split igual ao resto do shell: [`PersistenceModel`] gpui-free e testável + [`PersistenceView`] fina.
//! Superfície: uma JANELA própria (env-gated `LINA_PERSIST_PANEL`), disjunta do canvas (não toco o
//! render do canvas, dono de outro terminal). Invariantes #6 (estado salvo e VISÍVEL; nunca silenciosa;
//! navegação sem becos) e #2 (local-first).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::{
    div, prelude::*, px, rgb, size, text, AnyElement, App, Bounds, ClickEvent, Context,
    FocusHandle, FontWeight, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use lina_core::{DomainEvent, EventStore};
use lina_host::{HostEvent, UiHost};
use serde::{Deserialize, Serialize};

use crate::bridge::{lock, Model};
use crate::theme;

/// Quantos frames o badge fica em "salvando…" após o contador subir (≈130ms a 60fps).
const SAVING_FRAMES: u8 = 8;

// ═══════════════════════════════ "Tudo salvo ✓" (gpui-free) ═══════════════════════════════

/// Estado do indicador de persistência (rodapé).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveIndicator {
    /// Uma escrita acabou de acontecer (transiente).
    Saving,
    /// Tudo durável no log — "Tudo salvo ✓".
    Saved,
}

// ═══════════════════════════════ T8 recuperação (gpui-free, REUSA W0-6) ═══════════════════════════════

/// Estado de recuperação a APRESENTAR (nunca silenciosa). `Recovered` lista o que foi restaurado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryStatus {
    /// Banco íntegro — nada a recuperar.
    Intact,
    /// Recuperação em andamento (sinal `Recovering` do W0-6).
    InProgress,
    /// Recuperado de um desligamento abrupto: itens restaurados + arquivo corrompido preservado.
    Recovered {
        restored: Vec<String>,
        corrupt_file: String,
    },
}

/// Host gravador: captura os `HostEvent` que o `open_or_recover` (W0-6) emite (`Recovering`/`Recovered`).
#[derive(Default)]
struct RecorderHost {
    events: Vec<HostEvent>,
}
impl UiHost for RecorderHost {
    fn on_event(&mut self, event: HostEvent) {
        self.events.push(event);
    }
}

/// Acha o artefato `*.corrupt-*` que o W0-6 PRESERVA ao recuperar (prova durável de que houve crash).
fn find_corrupt_file(events_dir: &Path) -> Option<String> {
    std::fs::read_dir(events_dir).ok()?.find_map(|e| {
        let name = e.ok()?.file_name().to_string_lossy().into_owned();
        name.contains(".corrupt-").then_some(name)
    })
}

/// Nomes dos nós restaurados na projeção de um store (o "lista nós/terminais restaurados" do AC).
fn restored_nodes(events_dir: &Path) -> Vec<String> {
    EventStore::open(events_dir)
        .ok()
        .and_then(|s| s.project().ok())
        .map(|st| st.nodes.values().filter_map(|n| n.name.clone()).collect())
        .unwrap_or_default()
}

/// **Apresenta** (não recupera) o estado de recuperação de um workspace: `recovering` vivo (do
/// `SharedModel`) tem prioridade; senão, o artefato `*.corrupt-*` revela uma recuperação passada.
#[must_use]
pub fn present_recovery(events_dir: &Path, recovering: bool) -> RecoveryStatus {
    if recovering {
        return RecoveryStatus::InProgress;
    }
    match find_corrupt_file(events_dir) {
        Some(corrupt_file) => RecoveryStatus::Recovered {
            restored: restored_nodes(events_dir),
            corrupt_file,
        },
        None => RecoveryStatus::Intact,
    }
}

/// **REUSA o W0-6**: roda `EventStore::open_or_recover` num workspace (verifica integridade e, se
/// corrompido, recupera do JSONL — código do core, NÃO reimplementado aqui) e devolve o status.
///
/// **Só para Espaços NÃO-vivos** (e testes): rodar isto no dir do Espaço VIVO renomearia o `.db` sob
/// a conexão que o canvas mantém (corrida — achado do red-team). A reverificação do vivo é READ-ONLY
/// (`PersistenceModel::check_integrity` → `present_recovery`). `allow(dead_code)`: usado nos testes e
/// reservado para uma ação futura de "recuperar um Espaço da lista T6 antes de abri-lo".
#[allow(dead_code)]
#[must_use]
pub fn run_recovery(events_dir: &Path) -> RecoveryStatus {
    let mut host = RecorderHost::default();
    match EventStore::open_or_recover(events_dir, &mut host) {
        Ok(_store) => {
            let recovered = host
                .events
                .iter()
                .any(|e| matches!(e, HostEvent::Recovered));
            if recovered {
                RecoveryStatus::Recovered {
                    restored: restored_nodes(events_dir),
                    corrupt_file: find_corrupt_file(events_dir).unwrap_or_default(),
                }
            } else {
                RecoveryStatus::Intact
            }
        }
        Err(e) => {
            eprintln!(
                "persistence_ui: open_or_recover falhou em {}: {e}",
                events_dir.display()
            );
            RecoveryStatus::Intact
        }
    }
}

// ═══════════════════════════════ T6 Espaços (gpui-free) ═══════════════════════════════

/// Um Espaço listado no Switcher (T6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub name: String,
    pub events_dir: PathBuf,
    pub focused: bool,
    pub nodes: usize,
}

/// O `events_dir` (`<ws>/.lina/events`) é um Espaço se tiver o log (db ou espelho JSONL).
fn is_workspace_store(events_dir: &Path) -> bool {
    events_dir.join("lina.db").exists() || events_dir.join("log.jsonl").exists()
}

/// Projeta o nome + nº de nós de um Espaço (ou `None` se não projeta).
fn project_entry(events_dir: PathBuf, focused: bool) -> Option<WorkspaceEntry> {
    let st = EventStore::open(&events_dir).ok()?.project().ok()?;
    Some(WorkspaceEntry {
        name: st.workspace_name.unwrap_or_else(|| "(sem nome)".into()),
        events_dir,
        focused,
        nodes: st.nodes.len(),
    })
}

/// Lista os Espaços: o `current` (vivo) + os Espaços encontrados varrendo `base` (cada subdir cujo
/// `<sub>/.lina/events` ou `<sub>/events` tem um store). Determinístico (ordenado por caminho).
#[must_use]
pub fn list_workspaces(base: &Path, current: &Path) -> Vec<WorkspaceEntry> {
    let mut out: Vec<WorkspaceEntry> = Vec::new();
    // O Espaço vivo primeiro (sempre presente, mesmo fora do `base`).
    if let Some(e) = project_entry(current.to_path_buf(), true) {
        out.push(e);
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        // Aceita tanto `<sub>/.lina/events` quanto `<sub>/events` quanto `<sub>` já sendo o events dir.
        for candidate in [d.join(".lina").join("events"), d.join("events"), d.clone()] {
            if is_workspace_store(&candidate)
                && candidate != current
                && !out.iter().any(|e| e.events_dir == candidate)
            {
                if let Some(e) = project_entry(candidate, false) {
                    out.push(e);
                }
                break;
            }
        }
    }
    out
}

// ═══════════════════════════════ T7 Ajustes (gpui-free) ═══════════════════════════════

/// Preferências do usuário, persistidas em `settings.json` (persiste após reabrir).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Tema visual (`"escuro"` | `"claro"`).
    pub theme: String,
    /// Acento curado do design system (F1-2-1; nome em `theme::ACCENTS`). `serde(default)`:
    /// `settings.json` antigos (sem o campo) seguem carregando sem resetar as demais escolhas.
    #[serde(default = "default_accent_setting")]
    pub accent: String,
    /// Reduzir animações (a11y — acessível ao W4-6; aqui só o ajuste persistido).
    pub reduce_motion: bool,
    /// Pasta padrão de novos Espaços.
    pub default_cwd: String,
    /// F1-1-7: mute do lembrete sonoro da Fila de Atenção (som 1×/30s com fila >0).
    /// `serde(default)` = som LIGADO (ON discreto, 13.13 achado 9); settings.json
    /// antigos (sem o campo) seguem carregando sem resetar as demais escolhas.
    #[serde(default)]
    pub attention_sound_muted: bool,
    /// F1-4-3: opt-out do restore por Espaço — `false` = «Abrir este Espaço sem religar os
    /// Agentes» (copy-f1-4 §4). `serde(default)` = restore LIGADO (o default do produto é o
    /// Espaço voltar vivo); settings.json antigos seguem carregando sem reset.
    #[serde(default = "default_restore_on_open")]
    pub restore_on_open: bool,
}

fn default_accent_setting() -> String {
    theme::DEFAULT_ACCENT.to_string()
}

fn default_restore_on_open() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "escuro".into(),
            accent: default_accent_setting(),
            reduce_motion: false,
            default_cwd: String::new(),
            attention_sound_muted: false,
            restore_on_open: true,
        }
    }
}

impl Settings {
    /// O par `(modo, acento)` destes ajustes, pronto p/ `theme::apply` (fonte única da conversão).
    #[must_use]
    pub fn theme_mode(&self) -> theme::Mode {
        theme::Mode::from_setting(&self.theme)
    }
}

fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// Lê os ajustes (best-effort: ausência/erro → default — nunca falha).
#[must_use]
pub fn load_settings(dir: &Path) -> Settings {
    std::fs::read_to_string(settings_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Grava os ajustes (best-effort; erro logado, não derruba).
pub fn save_settings(dir: &Path, s: &Settings) {
    let path = settings_path(dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(s) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!(
                    "persistence_ui: não gravei ajustes em {}: {e}",
                    path.display()
                );
            }
        }
        Err(e) => eprintln!("persistence_ui: não serializei ajustes: {e}"),
    }
}

// ═══════════════════════════════ o modelo (gpui-free) ═══════════════════════════════

/// Estado completo do chrome de persistência, sem gpui. A view o possui e renderiza.
pub struct PersistenceModel {
    /// Projeção do core compartilhada com o canvas (event_count, recovering, nós restaurados).
    shared: Model,
    /// Store do Espaço vivo (para logar `WorkspaceFocusSet` na troca de foco).
    store: Arc<Mutex<EventStore>>,
    /// `events` dir do Espaço vivo (settings.json mora ao lado; raiz do scan de recuperação).
    ws_dir: PathBuf,
    /// Pasta-base a varrer por Espaços (T6).
    workspaces_base: PathBuf,
    /// Onde os ajustes persistem (`<ws_dir>/settings.json`).
    settings_dir: PathBuf,
    settings: Settings,
    // ── indicador de salvamento ──
    last_event_count: u64,
    saving_ticks: u8,
    // ── recuperação (T8) ──
    recovery: RecoveryStatus,
    last_recovering: bool,
    /// `true` se uma recuperação ocorreu NESTA sessão (sinal `recovering` vivo true→false) — distingue
    /// "recuperado agora" de "há um artefato `*.corrupt-*` antigo no disco" (que persiste).
    recovered_now: bool,
    /// O usuário dispensou o banner de recuperação (navegação sem becos; não "mente para sempre").
    recovery_dismissed: bool,
    // ── Espaços (T6) ──
    workspaces: Vec<WorkspaceEntry>,
    // ── tema (F1-2-1) ──
    /// Resultado do último export/import de tema (mensagem leiga na tela; `None` = sem atividade).
    theme_io: Option<String>,
    // ── Plano (T7§P — F1-4-6) ──
    /// Estado da licença CACHEADO (recarregado em abrir/ativar/remover — pontos de
    /// gating; nunca por timer, critério 7 de F1-4-5).
    license: lina_license::LicenseState,
    /// O campo «cole aqui a chave…» do painel (mesmo estado headless do M9).
    plan_key: crate::license_ui::KeyEntry,
    /// PRO: `[ Trocar chave… ]` expandiu o campo de troca?
    plan_swap_open: bool,
    /// `[ Remover chave ]` aguardando a confirmação honesta?
    plan_remove_confirm: bool,
}

impl PersistenceModel {
    /// Monta o modelo a partir dos handles compartilhados do app.
    pub fn new(
        shared: Model,
        store: Arc<Mutex<EventStore>>,
        ws_dir: PathBuf,
        workspaces_base: PathBuf,
    ) -> Self {
        let settings_dir = ws_dir.clone();
        let settings = load_settings(&settings_dir);
        let recovering = lock(&shared).recovering;
        let last_event_count = lock(&shared).event_count;
        let mut model = Self {
            shared,
            store,
            ws_dir: ws_dir.clone(),
            workspaces_base,
            settings_dir,
            settings,
            last_event_count,
            saving_ticks: 0,
            recovery: present_recovery(&ws_dir, recovering),
            last_recovering: recovering,
            recovered_now: false,
            recovery_dismissed: false,
            workspaces: Vec::new(),
            theme_io: None,
            license: lina_license::LicenseState::load_default(),
            plan_key: crate::license_ui::KeyEntry::default(),
            plan_swap_open: false,
            plan_remove_confirm: false,
        };
        model.refresh_workspaces();
        model
    }

    // ───────────────────── Plano (T7§P — F1-4-6; tudo offline) ─────────────────────

    /// O que o painel mostra AGORA (projeção pura — critério 3: consistente com o
    /// `lina-license`). `used` = Espaços ativos do registry global.
    #[must_use]
    pub fn plan_view(&self) -> crate::license_ui::PlanPanel {
        let used = lina_core::default_registry_path()
            .and_then(|p| lina_core::WorkspaceRegistry::load(p).ok())
            .map_or(0, |r| r.active_count()) as u32;
        crate::license_ui::plan_panel(
            &self.license.effective(crate::license_ui::now_epoch_s()),
            used,
        )
    }

    #[must_use]
    pub fn plan_key(&self) -> &crate::license_ui::KeyEntry {
        &self.plan_key
    }
    #[must_use]
    pub fn plan_swap_open(&self) -> bool {
        self.plan_swap_open
    }
    #[must_use]
    pub fn plan_remove_confirm(&self) -> bool {
        self.plan_remove_confirm
    }
    pub fn plan_toggle_swap(&mut self) {
        self.plan_swap_open = !self.plan_swap_open;
        self.plan_remove_confirm = false;
    }
    pub fn plan_ask_remove(&mut self) {
        self.plan_remove_confirm = true;
    }
    pub fn plan_cancel_remove(&mut self) {
        self.plan_remove_confirm = false;
    }

    /// Clicar no campo cola DO CLIPBOARD (a janela T7 não roteia teclado — o gesto do
    /// leigo é exatamente "colar"). Substitui o conteúdo (previsível), normalizando
    /// espaços/quebras em silêncio (§2).
    pub fn plan_paste(&mut self, clipboard_text: &str) {
        self.plan_key.input.clear();
        self.plan_key.feedback = None;
        self.plan_key.type_str(clipboard_text);
    }

    /// `[[ Ativar ]]` do painel: chave oficial + caminho padrão; sucesso recarrega o
    /// estado (o painel vira PRO sem restart). Devolve o anúncio p/ leitor de tela.
    pub fn plan_activate(&mut self) -> Option<String> {
        let path = lina_license::default_license_path().unwrap_or_default();
        let ok = self.plan_key.activate(
            &path,
            &lina_license::Verifier::official(),
            crate::license_ui::now_epoch_s(),
        );
        if ok {
            self.license = lina_license::LicenseState::load_default();
            self.plan_swap_open = false;
        }
        self.plan_key.announcement()
    }

    /// `[[ Remover ]]` confirmado: apaga o license.json (volta a Free; Espaços abertos
    /// seguem — o limite vale para CRIAR novos) e recarrega o estado.
    pub fn plan_remove(&mut self) {
        let path = lina_license::default_license_path().unwrap_or_default();
        if let Err(e) = lina_license::deactivate(&path) {
            eprintln!("lina-gpui: T7§P — não removi a chave ({e})");
        }
        self.license = lina_license::LicenseState::load_default();
        self.plan_remove_confirm = false;
        self.plan_key = crate::license_ui::KeyEntry::default();
    }

    /// Chamado a cada frame: atualiza o indicador de salvamento e o estado de recuperação a partir
    /// do `SharedModel` (que o core mantém). Sem clock — conta frames.
    pub fn poll(&mut self) {
        let (count, recovering) = {
            let m = lock(&self.shared);
            (m.event_count, m.recovering)
        };
        // Indicador de salvamento: contador subiu → "salvando…" por alguns frames; senão "salvo ✓".
        if count > self.last_event_count {
            self.saving_ticks = SAVING_FRAMES;
            self.last_event_count = count;
        } else if self.saving_ticks > 0 {
            self.saving_ticks -= 1;
        }
        // Recuperação: re-apresenta quando o sinal `recovering` muda (entra/sai do modo recovery).
        if recovering != self.last_recovering {
            if recovering {
                // Entrou em recuperação NESTA sessão → reabre o banner (mesmo se dispensado antes).
                self.recovery_dismissed = false;
            } else {
                // Saiu de recuperação → foi "recuperado agora" (não um artefato antigo).
                self.recovered_now = true;
            }
            self.recovery = present_recovery(&self.ws_dir, recovering);
            self.last_recovering = recovering;
        }
    }

    /// O indicador atual ("salvando…" enquanto há ticks; senão "salvo ✓").
    #[must_use]
    pub fn save_indicator(&self) -> SaveIndicator {
        if self.saving_ticks > 0 {
            SaveIndicator::Saving
        } else {
            SaveIndicator::Saved
        }
    }

    /// Estado de recuperação a apresentar (T8).
    #[must_use]
    pub fn recovery(&self) -> &RecoveryStatus {
        &self.recovery
    }

    /// Espaços listados (T6).
    #[must_use]
    pub fn workspaces(&self) -> &[WorkspaceEntry] {
        &self.workspaces
    }

    /// Ajustes correntes (T7).
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Re-varre os Espaços (T6).
    pub fn refresh_workspaces(&mut self) {
        self.workspaces = list_workspaces(&self.workspaces_base, &self.ws_dir);
    }

    /// Recuperação ocorreu NESTA sessão (vs. um artefato `*.corrupt-*` antigo no disco).
    #[must_use]
    pub fn recovered_now(&self) -> bool {
        self.recovered_now
    }

    /// O banner de recuperação foi dispensado pelo usuário.
    #[must_use]
    pub fn recovery_dismissed(&self) -> bool {
        self.recovery_dismissed
    }

    /// Dispensa o banner de recuperação (navegação sem becos; reaparece numa NOVA recuperação).
    pub fn dismiss_recovery(&mut self) {
        self.recovery_dismissed = true;
    }

    /// **Reverificar (READ-ONLY)** — re-apresenta a integridade do Espaço VIVO sem recuperar. NÃO roda
    /// `open_or_recover` no dir vivo: isso RENOMEARIA o `.db` sob a conexão que o canvas mantém (corrida
    /// de dados). A recuperação destrutiva do Espaço vivo é do BOOT (`open_or_recover`, dono do canvas
    /// — ver `.entrega`); [`run_recovery`] (que recupera) fica para Espaços NÃO-vivos/teste.
    pub fn check_integrity(&mut self) {
        let recovering = lock(&self.shared).recovering;
        self.recovery = present_recovery(&self.ws_dir, recovering);
    }

    /// T6: troca o foco para `name` — loga `WorkspaceFocusSet` (evento committed do core) no store
    /// vivo, carimba o foco no PONTEIRO GLOBAL (`~/.lina/workspaces.json` — o próximo boot abre o
    /// Espaço escolhido, F1-4-1 fiação) e re-lista. (Abrir o canvas daquele Espaço SEM reiniciar é
    /// o hand-off seguinte — switch M8 "vivos"; ver `.entrega`.)
    pub fn switch_to(&mut self, name: &str) {
        {
            // `lock` (do bridge) recupera de poison — NÃO descarta a escrita silenciosamente.
            let mut g = lock(&self.store);
            if let Err(e) = g.append(&DomainEvent::WorkspaceFocusSet {
                workspace: name.to_string(),
            }) {
                eprintln!("persistence_ui: falha ao logar WorkspaceFocusSet: {e}");
            }
        }
        // Foco durável no ponteiro global — SÓ para alvo NÃO-vivo (o vivo já é o focado, e
        // re-registrá-lo abriria o store dele 2× sob a conexão do canvas). Best-effort: falha
        // loga e a troca vale só nesta sessão.
        if let Some(target) = self
            .workspaces
            .iter()
            .find(|w| w.name == name && !w.focused)
        {
            let root = crate::workspace_boot::ws_root_of_events_dir(&target.events_dir);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            match lina_core::default_registry_path()
                .ok_or_else(|| "sem HOME/USERPROFILE".to_string())
                .and_then(|p| lina_core::WorkspaceRegistry::load(p).map_err(|e| e.to_string()))
                .and_then(|mut reg| {
                    crate::workspace_boot::focus_target_workspace(&root, &mut reg, now_ms)
                }) {
                Ok(id) => eprintln!(
                    "persistence_ui: [WS] foco global carimbado em '{id}' — o próximo boot abre este Espaço"
                ),
                Err(e) => eprintln!(
                    "persistence_ui: [WS] não carimbei o foco global ({e}); a troca vale só nesta sessão"
                ),
            }
        }
        self.refresh_workspaces();
    }

    /// T7: alterna o tema (escuro↔claro), persiste e **aplica ao vivo** (F1-2-1: todas as janelas
    /// re-pintam no próximo frame, sem restart).
    pub fn toggle_theme(&mut self) {
        self.settings.theme = if self.settings.theme == "escuro" {
            "claro".into()
        } else {
            "escuro".into()
        };
        self.persist_settings();
        self.apply_theme();
    }

    /// F1-2-1: escolhe um dos 8 acentos curados, persiste e aplica ao vivo.
    pub fn set_accent(&mut self, name: &str) {
        self.settings.accent = name.to_string();
        self.persist_settings();
        self.apply_theme();
    }

    /// Aplica o tema dos ajustes correntes como tema VIVO do processo (fonte única: `theme::apply`).
    fn apply_theme(&self) {
        theme::apply(self.settings.theme_mode(), &self.settings.accent);
    }

    /// Onde o export/import de tema mora (`tema.json`, ao lado do `settings.json` — local-first).
    fn theme_file(&self) -> PathBuf {
        self.settings_dir.join("tema.json")
    }

    /// F1-2-1 (T7§A Avançado): exporta o tema corrente como JSON legível, 100% local.
    pub fn export_theme(&mut self) {
        let json = theme::export_json(self.settings.theme_mode(), &self.settings.accent);
        let path = self.theme_file();
        self.theme_io = Some(match std::fs::write(&path, json) {
            Ok(()) => format!("Tema exportado em {}", path.display()),
            // T7§A estados de erro: falha de disco é VISÍVEL, nunca silenciosa.
            Err(e) => format!("Não consegui exportar o tema ({e})"),
        });
    }

    /// F1-2-1 (T7§A Avançado): importa `tema.json`, aplica ao vivo e persiste. Arquivo
    /// inválido → mensagem leiga (copy do T7§A); campos de versão futura são ignorados.
    pub fn import_theme(&mut self) {
        let path = self.theme_file();
        let parsed = std::fs::read_to_string(&path)
            .map_err(|_| format!("Não achei um tema para importar em {}", path.display()))
            .and_then(|json| theme::parse_json(&json));
        self.theme_io = Some(match parsed {
            Ok((mode, accent)) => {
                self.settings.theme = mode.as_setting().to_string();
                self.settings.accent = accent;
                self.persist_settings();
                self.apply_theme();
                "Tema importado e aplicado ✓".to_string()
            }
            Err(msg) => msg,
        });
    }

    /// Mensagem do último export/import (mostrada na seção Aparência).
    #[must_use]
    pub fn theme_io_message(&self) -> Option<&str> {
        self.theme_io.as_deref()
    }

    /// T7: alterna "reduzir animações" e persiste.
    pub fn toggle_reduce_motion(&mut self) {
        self.settings.reduce_motion = !self.settings.reduce_motion;
        self.persist_settings();
    }

    /// F1-1-7: alterna o som da Fila de Atenção (mute) e persiste.
    pub fn toggle_attention_sound(&mut self) {
        self.settings.attention_sound_muted = !self.settings.attention_sound_muted;
        self.persist_settings();
    }

    /// T7: define a pasta padrão e persiste.
    pub fn set_default_cwd(&mut self, cwd: impl Into<String>) {
        self.settings.default_cwd = cwd.into();
        self.persist_settings();
    }

    fn persist_settings(&self) {
        save_settings(&self.settings_dir, &self.settings);
    }
}

// ═══════════════════════════════ entrada (main.rs) ═══════════════════════════════

/// AUTO-ABRE o painel no boot? **Override de dev** — em produção a janela abre SOB DEMANDA pela
/// engrenagem da barra, `⌘,` ou paleta (fix de tela F1-2-1 — antes o tema era inalcançável sem a
/// env, violando inv#6). O gate honra o flag canônico de dev `LINA_DEV` (F1-2-5: superfície limpa
/// por padrão; ferramentas de dev voltam com `LINA_DEV=1`) **e** o legado `LINA_PERSIST_PANEL`
/// (back-compat para roteiros de validação que já o usam). Fechado por padrão.
#[must_use]
pub fn should_show() -> bool {
    should_show_from(
        std::env::var("LINA_DEV").ok().as_deref(),
        std::env::var("LINA_PERSIST_PANEL").ok().as_deref(),
    )
}

/// Decisão PURA do gate (testável sem mexer no env do processo): abre se o flag canônico de dev OU
/// o legado específico forem verdadeiros.
#[must_use]
fn should_show_from(lina_dev: Option<&str>, persist_panel: Option<&str>) -> bool {
    is_dev_flag_on(lina_dev) || is_dev_flag_on(persist_panel)
}

/// Um flag de env de dev está "ligado"? Aceita `1|true|force` (mesma convenção de `LINA_DASH`/
/// `LINA_ONBOARDING`), tolerante a espaços. `None`/vazio/qualquer outro = desligado.
#[must_use]
fn is_dev_flag_on(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("force")
    )
}

/// Pasta-base a varrer por Espaços (T6): `LINA_WORKSPACES_DIR` ou o pai do dir do Espaço vivo.
fn workspaces_base(ws_dir: &Path) -> PathBuf {
    std::env::var_os("LINA_WORKSPACES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            ws_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| ws_dir.to_path_buf())
        })
}

/// Abre a janela do painel (chamada de dentro do `application().run` de `main.rs`).
pub fn open_window(cx: &mut App, shared: Model, store: Arc<Mutex<EventStore>>, ws_dir: PathBuf) {
    let base = workspaces_base(&ws_dir);
    let bounds = Bounds::centered(None, size(px(560.0), px(680.0)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Lina Space — Espaços & Ajustes".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| PersistenceView::new(shared, store, ws_dir, base, window, cx)),
    );
    if let Err(e) = opened {
        eprintln!("persistence_ui: não abri o painel: {e}");
    }
}

// ═══════════════════════════════ a view gpui (fina) ═══════════════════════════════

/// View gpui do painel — só renderiza o [`PersistenceModel`] e roteia cliques.
pub struct PersistenceView {
    model: PersistenceModel,
    focus: FocusHandle,
}

impl PersistenceView {
    fn new(
        shared: Model,
        store: Arc<Mutex<EventStore>>,
        ws_dir: PathBuf,
        base: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            model: PersistenceModel::new(shared, store, ws_dir, base),
            focus,
        }
    }

    /// `id` (único por seção) é OBRIGATÓRIO p/ a11y: `section()` é chamado 3× com `text!(title)` no
    /// MESMO ponto do fonte; sem um ancestral com id distinto, os títulos colidiriam no id de nó
    /// AccessKit (debug_assert! → pânico com leitor de tela). Lição do W4-1 (text! hasheia por
    /// localização no fonte, não por conteúdo).
    fn section(&self, id: &'static str, title: &str, body: AnyElement) -> AnyElement {
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(theme::active().text.muted))
                    .font_weight(FontWeight::BOLD)
                    .child(text!(title.to_string())),
            )
            .child(body)
            .into_any_element()
    }

    /// Botão com par `(bg, fg)` EXPLÍCITO em tokens — o chamador escolhe o par que o gate WCAG
    /// cobre (ex.: `raised`+`bright`, `warning`+`on_emphasis`), em vez de um fg fixo que viola AA
    /// sobre fills claros (violação pré-existente corrigida na migração F1-2-1).
    fn button(
        &self,
        id: &'static str,
        label: impl Into<String>,
        (bg, fg): (u32, u32),
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let label = label.into();
        div()
            .id(id)
            .px_4()
            .py_2()
            .rounded_md()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _ev: &ClickEvent, window, cx| {
                on_click(view, window, cx);
            }))
            .child(text!(label))
            .into_any_element()
    }

    /// Badge "salvando…/Tudo salvo ✓" (rodapé/topo — sempre visível).
    fn save_badge(&self) -> AnyElement {
        let th = theme::active();
        let (label, color) = match self.model.save_indicator() {
            SaveIndicator::Saving => ("salvando…", th.state.warning),
            SaveIndicator::Saved => ("Tudo salvo ✓", th.state.success),
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(div().size(px(8.0)).rounded_full().bg(rgb(color)))
            .child(div().text_color(rgb(color)).child(text!(label)))
            .into_any_element()
    }

    /// T8 — banner de recuperação (nunca silenciosa). Distingue "recuperado AGORA" de "há um backup
    /// `*.corrupt-*` preservado no disco" (que persiste) e é DISPENSÁVEL (não "mente para sempre").
    fn recovery_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        let th = theme::active();
        let dismissed = self.model.recovery_dismissed();
        let body = match self.model.recovery() {
            RecoveryStatus::InProgress => div()
                .px_4()
                .py_3()
                .rounded_md()
                .bg(rgb(th.state.warning))
                .text_color(rgb(th.text.on_emphasis))
                .child(text!("⟳ Recuperando seu trabalho…")),
            // Recuperado e ainda não dispensado: copy HONESTA (agora vs. artefato antigo) + Dispensar.
            RecoveryStatus::Recovered { restored, .. } if !dismissed => {
                let n = restored.len();
                let names = if restored.is_empty() {
                    "estado restaurado do registro".to_string()
                } else {
                    restored.join(", ")
                };
                let title = if self.model.recovered_now() {
                    format!("✓ Recuperado agora de um desligamento abrupto — {n} item(ns) restaurado(s)")
                } else {
                    format!("ⓘ Há um backup de recuperação preservado no disco — {n} item(ns) restaurado(s)")
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(th.surface.panel))
                    .border_2()
                    .border_color(rgb(th.state.success))
                    .child(
                        div()
                            .text_color(rgb(th.state.success))
                            .font_weight(FontWeight::BOLD)
                            .child(text!(title)),
                    )
                    .child(div().text_color(rgb(th.text.muted)).child(text!(names)))
                    .child(self.button(
                        "dismiss-recovery",
                        "Entendi, dispensar",
                        (th.surface.raised, th.text.bright),
                        cx,
                        |v, _w, _cx| v.model.dismiss_recovery(),
                    ))
            }
            // Íntegro OU recuperação já dispensada → estado tranquilo + reverificação READ-ONLY.
            _ => div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_color(rgb(th.state.success))
                        .child(text!("✓ Tudo íntegro")),
                )
                .child(self.button(
                    "check-integrity",
                    "Reverificar",
                    (th.surface.raised, th.text.bright),
                    cx,
                    |v, _w, _cx| v.model.check_integrity(),
                )),
        };
        self.section(
            "sec-recovery",
            "Integridade e recuperação (T8)",
            body.into_any_element(),
        )
    }

    /// T6 — lista de Espaços (cada um clicável → troca de foco).
    fn workspaces_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let th = theme::active();
        let mut list = div().flex().flex_col().gap_2();
        for ws in self.model.workspaces() {
            // id estável e único por Espaço (a11y: evita colisão de id no `text!` do laço).
            let name = ws.name.clone();
            let name_click = name.clone();
            let dot = if ws.focused {
                th.accent.primary
            } else {
                th.surface.raised_alt
            };
            let row = div()
                .id(gpui::SharedString::from(
                    ws.events_dir.display().to_string(),
                ))
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .rounded_md()
                .bg(rgb(th.surface.panel))
                .cursor_pointer()
                .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, _cx| {
                    v.model.switch_to(&name_click);
                }))
                .child(div().size(px(9.0)).rounded_full().bg(rgb(dot)))
                .child(
                    div()
                        .flex_1()
                        .text_color(rgb(th.text.primary))
                        .font_weight(FontWeight::BOLD)
                        .child(text!(name)),
                )
                .child(
                    div()
                        .text_color(rgb(th.text.muted))
                        .child(text!(format!("{} nó(s)", ws.nodes))),
                );
            let row = if ws.focused {
                row.child(
                    div()
                        .text_color(rgb(th.accent.primary))
                        .child(text!("• em foco")),
                )
            } else {
                row
            };
            list = list.child(row);
        }
        if self.model.workspaces().is_empty() {
            list = list.child(div().text_color(rgb(th.text.muted)).child(text!(
                "Nenhum Espaço encontrado ainda — crie um pelo onboarding."
            )));
        }
        self.section("sec-workspaces", "Espaços (T6)", list.into_any_element())
    }

    /// F1-2-1 · T7§A — Aparência: modo (escuro/claro, troca AO VIVO) + 8 acentos curados +
    /// exportar/importar tema (JSON local). A escolha persiste em `settings.json` (local-first).
    fn appearance_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let th = theme::active();
        let s = self.model.settings();
        // Pastilhas dos 8 acentos curados: cada uma pinta com o próprio valor NO MODO ATUAL; a
        // ativa ganha o anel de foco (token `focus.ring` — visível nos 2 temas, gate ≥3:1).
        let mode = s.theme_mode();
        let current = s.accent.clone();
        let mut swatches = div().flex().flex_row().items_center().gap_2();
        for (i, (name, scale)) in theme::ACCENTS.iter().enumerate() {
            let active = name.eq_ignore_ascii_case(&current);
            let name_click = (*name).to_string();
            let mut dot = div()
                .id(("accent", i))
                // W4-6 (a11y P0): pastilha de cor sem texto precisa de rótulo anunciável.
                .aria_label(format!("Cor de acento: {name}"))
                .size(px(22.0))
                .rounded_full()
                .bg(rgb(scale.pick(mode)))
                .cursor_pointer()
                .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                    v.model.set_accent(&name_click);
                    // Acorda TODAS as janelas (canvas incluso): sem isto, uma janela quiescente
                    // (loop de animação parado) só re-pintaria o tema novo no próximo input.
                    cx.refresh_windows();
                }));
            if active {
                dot = dot.border_2().border_color(rgb(th.focus.ring));
            }
            swatches = swatches.child(dot);
        }
        swatches = swatches.child(
            div()
                .text_color(rgb(th.text.muted))
                .child(text!(format!("acento: {current}"))),
        );

        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!("Modo")),
                    )
                    .child(self.button(
                        "toggle-theme",
                        format!("{} (trocar)", s.theme),
                        (th.surface.raised, th.text.bright),
                        cx,
                        |v, _w, cx| {
                            v.model.toggle_theme();
                            cx.refresh_windows(); // re-pinta TODAS as janelas (troca ao vivo)
                        },
                    ))
                    .child(
                        div()
                            .text_color(rgb(th.text.muted))
                            .child(text!("(aplica na hora, em todas as janelas)")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!("Cor de acento")),
                    )
                    .child(swatches),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!("▸ Avançado")),
                    )
                    .child(self.button(
                        "export-theme",
                        "Exportar tema",
                        (th.surface.raised, th.text.bright),
                        cx,
                        |v, _w, cx| {
                            v.model.export_theme();
                            cx.refresh_windows(); // mostra a mensagem de resultado na hora
                        },
                    ))
                    .child(self.button(
                        "import-theme",
                        "Importar tema",
                        (th.surface.raised, th.text.bright),
                        cx,
                        |v, _w, cx| {
                            v.model.import_theme();
                            cx.refresh_windows(); // tema importado aplica em todas as janelas
                        },
                    )),
            );
        if let Some(msg) = self.model.theme_io_message() {
            body = body.child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(msg.to_string())),
            );
        }
        self.section(
            "sec-appearance",
            "Aparência (T7§A)",
            body.into_any_element(),
        )
    }

    /// T7 — Ajustes (toggles que persistem em settings.json).
    fn settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let th = theme::active();
        let s = self.model.settings();
        let body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    // BUG 5: rótulo claro pelo ESTADO das animações (não o duplo-negativo "Reduzir
                    // animações: ligado"). Botão mostra ligadas/desligadas + cor de estado óbvia.
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!("Animações da interface")),
                    )
                    .child(self.button(
                        "toggle-reduce-motion",
                        if s.reduce_motion {
                            "desligadas"
                        } else {
                            "ligadas"
                        },
                        if s.reduce_motion {
                            (th.state.warning, th.text.on_emphasis)
                        } else {
                            (th.surface.raised, th.text.bright)
                        },
                        cx,
                        |v, _w, _cx| v.model.toggle_reduce_motion(),
                    ))
                    .child(div().text_color(rgb(th.text.muted)).child(text!(
                        "(desligar reduz o movimento na tela — acessibilidade)"
                    ))),
            )
            // F1-1-7: som da Fila de Atenção (1 toque suave a cada 30s enquanto algum
            // agente espera você). Rótulo pelo ESTADO (mesma doutrina do toggle acima).
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!("Som da Fila de Atenção")),
                    )
                    .child(self.button(
                        "toggle-attention-sound",
                        if s.attention_sound_muted {
                            "desligado"
                        } else {
                            "ligado"
                        },
                        if s.attention_sound_muted {
                            (th.state.warning, th.text.on_emphasis)
                        } else {
                            (th.surface.raised, th.text.bright)
                        },
                        cx,
                        |v, _w, _cx| v.model.toggle_attention_sound(),
                    ))
                    .child(div().text_color(rgb(th.text.muted)).child(text!(
                        "(um toque discreto a cada 30s enquanto um agente espera você)"
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!("Pasta padrão")),
                    )
                    .child(div().flex_1().text_color(rgb(th.text.muted)).child(text!(
                        if s.default_cwd.is_empty() {
                            "(não definida)".to_string()
                        } else {
                            s.default_cwd.clone()
                        }
                    )))
                    .child(self.button(
                        "set-cwd-home",
                        "Usar a pasta de Documentos",
                        (th.surface.raised, th.text.bright),
                        cx,
                        |v, _w, _cx| {
                            let docs = std::env::var("HOME")
                                .map(|h| format!("{h}/Documents"))
                                .unwrap_or_else(|_| "~/Documents".into());
                            v.model.set_default_cwd(docs);
                        },
                    )),
            );
        self.section("sec-settings", "Ajustes (T7)", body.into_any_element())
    }

    /// Campo «cole aqui a chave…» do T7§P: esta janela não roteia teclado (pré-existente),
    /// então o CLIQUE no campo cola do clipboard — o gesto real do leigo ("copiei do
    /// e-mail → colo aqui"). `[[ Ativar ]]` valida 100% offline e atualiza o painel.
    fn plan_key_field(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::license_ui as lic;
        let th = theme::active();
        let (shown, fg) = if self.model.plan_key().input.is_empty() {
            (
                format!("{} (clique para colar)", lic::COPY_KEY_PLACEHOLDER),
                th.text.muted,
            )
        } else {
            (self.model.plan_key().input.clone(), th.text.bright)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(th.text.secondary))
                    .child(text!(lic::COPY_KEY_LABEL)),
            )
            .child(
                div()
                    .id("plan-key-field")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(th.surface.border))
                    .bg(rgb(th.surface.card))
                    .text_color(rgb(fg))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, cx| {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            view.model.plan_paste(&text);
                        }
                        cx.notify();
                    }))
                    .child(text!(shown)),
            )
            .child(self.button(
                "plan-key-activate",
                format!("[[ {} ]]", lic::COPY_KEY_ACTIVATE),
                (
                    theme::active().accent.action,
                    theme::active().text.on_accent,
                ),
                cx,
                |view, _w, cx| {
                    let _ = view.model.plan_activate();
                    cx.notify();
                },
            ));
        match self.model.plan_key().feedback {
            Some(lic::KeyFeedback::Error {
                ref message,
                renewable,
            }) => {
                col = col.child(
                    div()
                        .text_color(rgb(th.state.warning))
                        .child(text!(message.clone())),
                );
                if renewable {
                    col = col
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(th.text.muted))
                                .child(text!(lic::COPY_RENEW_NOTE)),
                        )
                        .child(self.button(
                            "plan-renew-link",
                            "[ Renovar no site ↗ ]",
                            (theme::active().surface.raised, theme::active().text.bright),
                            cx,
                            |_view, _w, cx| cx.open_url(lic::STORE_URL),
                        ));
                }
            }
            Some(lic::KeyFeedback::Success {
                ref changed,
                ref valid_until,
            }) => {
                col = col.child(
                    div()
                        .text_color(rgb(th.state.success))
                        .child(text!(lic::COPY_SUCCESS_TITLE)),
                );
                col = col.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(th.text.secondary))
                        .child(text!(lic::COPY_SUCCESS_CHANGED)),
                );
                for (i, item) in changed.iter().enumerate() {
                    col = col.child(
                        div()
                            .id(("plan-changed", i))
                            .text_size(px(12.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!(item.clone())),
                    );
                }
                if let Some(v) = valid_until {
                    col = col.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(th.text.muted))
                            .child(text!(v.clone())),
                    );
                }
            }
            None => {}
        }
        col.into_any_element()
    }

    /// T7§P — Ajustes › Plano (F1-4-6): a face não-técnica da licença 100% offline.
    /// Free = duas colunas honestas + campo; PRO = validade/uso + trocar/remover;
    /// degradado = re-colagem graciosa. Copy congelada em `license_ui` (copy-f1-4 §3).
    fn plan_section(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::license_ui as lic;
        let th = theme::active();
        let mut body = div().flex().flex_col().gap_3();
        match self.model.plan_view() {
            lic::PlanPanel::Pro { valid_until, usage } => {
                body = body.child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(th.state.success))
                        .child(text!(lic::COPY_PLAN_PRO_TITLE)),
                );
                if let Some(v) = valid_until {
                    body = body.child(div().text_color(rgb(th.text.muted)).child(text!(v)));
                }
                body = body.child(div().text_color(rgb(th.text.primary)).child(text!(usage)));
                body = body.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(self.button(
                            "plan-swap",
                            format!("[ {} ]", lic::COPY_PLAN_SWAP),
                            (th.surface.raised, th.text.bright),
                            cx,
                            |view, _w, cx| {
                                view.model.plan_toggle_swap();
                                cx.notify();
                            },
                        ))
                        .child(self.button(
                            "plan-remove",
                            format!("[ {} ]", lic::COPY_PLAN_REMOVE),
                            (th.surface.raised, th.text.bright),
                            cx,
                            |view, _w, cx| {
                                view.model.plan_ask_remove();
                                cx.notify();
                            },
                        )),
                );
                if self.model.plan_swap_open() {
                    body = body
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(th.text.secondary))
                                .child(text!(lic::COPY_PLAN_SWAP_NOTE)),
                        )
                        .child(self.plan_key_field(cx));
                }
                if self.model.plan_remove_confirm() {
                    body = body.child(
                        div()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(th.state.warning))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(rgb(th.text.primary))
                                    .child(text!(lic::COPY_PLAN_REMOVE_CONFIRM)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(self.button(
                                        "plan-remove-cancel",
                                        format!("[ {} ]", lic::COPY_PLAN_REMOVE_CANCEL),
                                        (th.surface.raised, th.text.bright),
                                        cx,
                                        |view, _w, cx| {
                                            view.model.plan_cancel_remove();
                                            cx.notify();
                                        },
                                    ))
                                    .child(self.button(
                                        "plan-remove-do",
                                        format!("[[ {} ]]", lic::COPY_PLAN_REMOVE_DO),
                                        (th.state.warning, th.text.on_emphasis),
                                        cx,
                                        |view, _w, cx| {
                                            view.model.plan_remove();
                                            cx.notify();
                                        },
                                    )),
                            ),
                    );
                }
            }
            lic::PlanPanel::Repaste => {
                body = body
                    .child(
                        div()
                            .text_color(rgb(th.text.primary))
                            .child(text!(lic::COPY_PLAN_REPASTE)),
                    )
                    .child(self.plan_key_field(cx));
            }
            lic::PlanPanel::Free { expired_note } => {
                body = body.child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(th.text.bright))
                        .child(text!(lic::COPY_PLAN_FREE_TITLE)),
                );
                if let Some(note) = expired_note {
                    body = body.child(div().text_color(rgb(th.state.warning)).child(text!(note)));
                }
                // Duas colunas honestas (E1 do ux-flows; strings §3 congeladas).
                let mut have = div().flex().flex_col().gap_1().flex_1().child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(th.text.secondary))
                        .child(text!(lic::COPY_PLAN_HAVE_COL)),
                );
                for (i, item) in lic::COPY_PLAN_HAVE_ITEMS.iter().enumerate() {
                    have = have.child(
                        div()
                            .id(("plan-have", i))
                            .text_size(px(12.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!((*item).to_string())),
                    );
                }
                let unlock = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(th.text.secondary))
                            .child(text!(lic::COPY_PLAN_UNLOCK_COL)),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(th.text.primary))
                            .child(text!(lic::COPY_SUCCESS_SPACES)),
                    );
                body = body.child(div().flex().flex_row().gap_4().child(have).child(unlock));
                body = body.child(
                    div()
                        .text_color(rgb(th.text.primary))
                        .child(text!(lic::COPY_PLAN_ASK_KEY)),
                );
                body = body.child(self.plan_key_field(cx));
                body = body.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(th.text.muted))
                                .child(text!(lic::COPY_PLAN_BUY_PREFIX)),
                        )
                        .child(self.button(
                            "plan-buy",
                            format!("[ {} ]", lic::COPY_PLAN_BUY_LINK),
                            (th.surface.raised, th.text.bright),
                            cx,
                            |_view, _w, cx| cx.open_url(lic::STORE_URL),
                        )),
                );
            }
        }
        self.section("sec-plan", "Plano", body.into_any_element())
    }
}

impl Render for PersistenceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        self.model.poll();
        // F1-2-1: tokens vivos — trocar modo/acento na seção Aparência re-pinta ESTA janela (e o
        // canvas) no frame seguinte, sem restart.
        let th = theme::active();

        div()
            .id("persistence-panel")
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(th.surface.canvas))
            .text_color(rgb(th.text.primary))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .w(px(500.0))
                    .mt(px(40.0))
                    .ml(px(30.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(22.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(th.accent.primary))
                                    .child(text!("Espaços & Ajustes")),
                            )
                            .child(self.save_badge()),
                    )
                    .child(self.recovery_banner(cx))
                    .child(self.workspaces_list(cx))
                    .child(self.plan_section(cx))
                    .child(self.appearance_panel(cx))
                    .child(self.settings_panel(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::SharedModel;

    /// F1-2-5 (gate da flag de dev) — NÃO-VACUOSO: o painel de diagnóstico fica FECHADO por
    /// padrão (superfície limpa, inv#6) e só auto-abre com o flag canônico `LINA_DEV` ou o legado
    /// `LINA_PERSIST_PANEL`. Se alguém remover o gate (fazer o `should_show_from` retornar `true`
    /// incondicionalmente), a 1ª asserção FALHA. Puro: não toca o env do processo.
    #[test]
    fn dev_panel_closed_by_default_opens_only_with_flag() {
        // Sem nenhuma flag → fechado (o que o fundador vê no percurso T0→T8: sem resíduo de dev).
        assert!(!should_show_from(None, None), "fechado sem flag");
        assert!(
            !should_show_from(Some("0"), Some("0")),
            "fechado com flag falsa"
        );
        assert!(
            !should_show_from(Some("nope"), None),
            "valor desconhecido = desligado"
        );

        // Flag CANÔNICO de dev (F1-2-5: ferramentas de dev voltam com LINA_DEV=1).
        assert!(should_show_from(Some("1"), None), "LINA_DEV=1 abre");
        assert!(should_show_from(Some("true"), None), "LINA_DEV=true abre");
        assert!(
            should_show_from(Some(" force "), None),
            "tolera espaços ao redor"
        );

        // Flag LEGADO específico segue valendo (back-compat dos roteiros de validação).
        assert!(
            should_show_from(None, Some("1")),
            "LINA_PERSIST_PANEL=1 abre"
        );
        assert!(
            should_show_from(None, Some("force")),
            "LINA_PERSIST_PANEL=force abre"
        );
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "lina-persui-{tag}-{}-{}",
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

    /// Cria um Espaço (events store) com nome + 1 nó nomeado; devolve o events dir.
    fn seed_workspace(root: &Path, name: &str, node_name: &str) -> PathBuf {
        let events = root.join("events");
        let mut store = EventStore::open(&events).expect("open");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: name.into(),
                focus_preset: String::new(),
            })
            .expect("ws");
        let node = uuid::Uuid::now_v7();
        store
            .append(&DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
            })
            .expect("node");
        store
            .append(&DomainEvent::NodeRenamed {
                node,
                name: node_name.into(),
            })
            .expect("rename");
        events
    }

    fn shared_with(count: u64, recovering: bool) -> Model {
        let m = SharedModel {
            event_count: count,
            recovering,
            ..Default::default()
        };
        Arc::new(Mutex::new(m))
    }

    /// "salvando ✓": ao subir o event_count, o indicador vira `Saving` por alguns frames e
    /// volta a `Saved` — exatamente "editar → salvando → salvo ✓".
    #[test]
    fn save_indicator_flashes_saving_then_settles_saved() {
        let tmp = TempDir::new("save");
        let events = seed_workspace(tmp.path(), "App", "Term A");
        let shared = shared_with(3, false);
        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("open")));
        let mut model = PersistenceModel::new(
            shared.clone(),
            store,
            events.clone(),
            tmp.path().to_path_buf(),
        );
        assert_eq!(
            model.save_indicator(),
            SaveIndicator::Saved,
            "idle = salvo ✓"
        );

        // Uma "edição": o core incrementa o contador.
        lock(&shared).event_count = 4;
        model.poll();
        assert_eq!(
            model.save_indicator(),
            SaveIndicator::Saving,
            "subiu → salvando…"
        );

        // Sem novas escritas, assenta em salvo após SAVING_FRAMES.
        for _ in 0..SAVING_FRAMES {
            model.poll();
        }
        assert_eq!(
            model.save_indicator(),
            SaveIndicator::Saved,
            "assenta em salvo ✓"
        );
    }

    /// T6: lista o Espaço vivo + os Espaços encontrados na base, com nomes projetados.
    #[test]
    fn lists_live_and_discovered_workspaces() {
        let base = TempDir::new("base");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let _other = seed_workspace(&base.path().join("ws-outro"), "App Outro", "T2");

        let entries = list_workspaces(base.path(), &live);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"App Vivo"), "lista o vivo; veio {names:?}");
        assert!(
            names.contains(&"App Outro"),
            "lista o descoberto; veio {names:?}"
        );
        let live_entry = entries.iter().find(|e| e.name == "App Vivo").unwrap();
        assert!(live_entry.focused, "o vivo está em foco");
        assert_eq!(live_entry.nodes, 1);
    }

    /// T6: trocar de Espaço loga `WorkspaceFocusSet` (evento committed) no store vivo.
    #[test]
    fn switch_logs_workspace_focus_set() {
        let tmp = TempDir::new("switch");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("open")));
        let shared = shared_with(0, false);
        let mut model =
            PersistenceModel::new(shared, store, events.clone(), tmp.path().to_path_buf());
        model.switch_to("App Outro");

        let store = EventStore::open(&events).expect("reopen");
        let focused = store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "WorkspaceFocusSet")
            .count();
        assert_eq!(focused, 1, "uma troca de foco logada");
        assert_eq!(
            store
                .project()
                .expect("project")
                .focused_workspace
                .as_deref(),
            Some("App Outro")
        );
    }

    /// T7: alterar um ajuste persiste em settings.json (persiste após reabrir).
    #[test]
    fn settings_persist_across_reopen() {
        let _guard = crate::theme::THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new("settings");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("open")));
        let shared = shared_with(0, false);
        {
            let mut model =
                PersistenceModel::new(shared, store, events.clone(), tmp.path().to_path_buf());
            assert_eq!(model.settings().theme, "escuro");
            model.toggle_theme(); // escuro → claro
            model.toggle_reduce_motion(); // false → true
            model.set_default_cwd("/tmp/projetos");
        }
        // "reabrir": relê do disco.
        let reloaded = load_settings(&events);
        assert_eq!(reloaded.theme, "claro");
        assert!(reloaded.reduce_motion);
        assert_eq!(reloaded.default_cwd, "/tmp/projetos");
        theme::apply(theme::Mode::Dark, theme::DEFAULT_ACCENT); // não vaza estado global
    }

    /// F1-1-7: o mute do lembrete sonoro da Fila de Atenção PERSISTE (settings.json),
    /// nasce LIGADO (ON discreto — ux-flows E6) e um settings.json ANTIGO (sem o campo)
    /// segue carregando com o default (`serde(default)` — compat shape antigo).
    #[test]
    fn attention_sound_mute_persists_and_defaults_on() {
        let _guard = crate::theme::THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new("att-mute");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("open")));
        let shared = shared_with(0, false);
        {
            let mut model =
                PersistenceModel::new(shared, store, events.clone(), tmp.path().to_path_buf());
            assert!(
                !model.settings().attention_sound_muted,
                "default: som LIGADO (mute=false)"
            );
            model.toggle_attention_sound();
        }
        assert!(
            load_settings(&events).attention_sound_muted,
            "mute persistiu após 'reabrir'"
        );
        // settings.json de versão ANTERIOR (sem o campo) → default, sem resetar o resto.
        std::fs::write(
            events.join("settings.json"),
            r#"{"theme":"claro","reduce_motion":true,"default_cwd":"/x"}"#,
        )
        .expect("settings antigo");
        let old = load_settings(&events);
        assert!(!old.attention_sound_muted, "shape antigo → som ligado");
        assert_eq!(old.theme, "claro", "demais escolhas preservadas");
        theme::apply(theme::Mode::Dark, theme::DEFAULT_ACCENT); // não vaza estado global
    }

    /// **F1-2-1 critério 3 (persistência local-first)**: trocar o ACENTO pela UI simples persiste
    /// em `settings.json`; "fechar e reabrir" (reler do disco) devolve a escolha; e a mutação
    /// aplicou o tema VIVO do processo (o que as janelas leem no próximo frame). Zero rede: o
    /// caminho inteiro é `std::fs` local (nenhum cliente de rede neste módulo).
    #[test]
    fn accent_choice_survives_reopen_and_applies_live() {
        let _guard = crate::theme::THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new("accent");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("open")));
        let shared = shared_with(0, false);
        {
            let mut model =
                PersistenceModel::new(shared, store, events.clone(), tmp.path().to_path_buf());
            assert_eq!(model.settings().accent, theme::DEFAULT_ACCENT);
            model.set_accent("violeta");
            // aplicou AO VIVO (sem restart): o tema ativo já é o violeta.
            assert_eq!(
                theme::active().accent.primary,
                theme::Theme::build(theme::Mode::Dark, "violeta")
                    .accent
                    .primary
            );
        }
        // "reabrir o app": boot relê settings e aplica (mesmo caminho do main()).
        let reloaded = load_settings(&events);
        assert_eq!(
            reloaded.accent, "violeta",
            "o override sobrevive à reabertura"
        );
        theme::apply(reloaded.theme_mode(), &reloaded.accent);
        assert_eq!(
            theme::active().accent.primary,
            theme::Theme::build(theme::Mode::Dark, "violeta")
                .accent
                .primary
        );
        theme::apply(theme::Mode::Dark, theme::DEFAULT_ACCENT); // não vaza estado global
    }

    /// Compat: um `settings.json` da Fase 0 (SEM o campo `accent`) carrega sem resetar as demais
    /// escolhas do usuário — `serde(default)` preenche o acento default.
    #[test]
    fn old_settings_without_accent_still_load() {
        let tmp = TempDir::new("compat");
        let dir = tmp.path().join("events");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("settings.json"),
            r#"{"theme":"claro","reduce_motion":true,"default_cwd":"/x"}"#,
        )
        .expect("write");
        let s = load_settings(&dir);
        assert_eq!(s.theme, "claro", "escolhas antigas preservadas");
        assert!(s.reduce_motion);
        assert_eq!(s.accent, theme::DEFAULT_ACCENT, "acento ausente → default");
    }

    /// **F1-2-1 critério 3 (export/import)**: exportar gera um `tema.json` LEGÍVEL no disco;
    /// importar o aplica (tema vivo + settings persistidos) — 100% local, e arquivo inválido
    /// produz mensagem leiga sem mudar nada.
    #[test]
    fn theme_export_import_roundtrip_via_model() {
        let _guard = crate::theme::THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new("themeio");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("open")));
        let shared = shared_with(0, false);
        let mut model =
            PersistenceModel::new(shared, store, events.clone(), tmp.path().to_path_buf());

        model.toggle_theme(); // escuro → claro
        model.set_accent("teal");
        model.export_theme();
        let exported = std::fs::read_to_string(events.join("tema.json")).expect("tema.json");
        assert!(
            exported.contains(r#""modo": "claro""#),
            "legível: {exported}"
        );
        assert!(
            exported.contains(r#""acento": "teal""#),
            "legível: {exported}"
        );

        // outro estado → importar restaura o exportado (aplica vivo + persiste).
        model.toggle_theme(); // claro → escuro
        model.set_accent("rosa");
        model.import_theme();
        assert_eq!(model.settings().theme, "claro");
        assert_eq!(model.settings().accent, "teal");
        assert_eq!(theme::active().mode, theme::Mode::Light);
        assert_eq!(
            model.theme_io_message(),
            Some("Tema importado e aplicado ✓")
        );

        // arquivo inválido → mensagem leiga (copy T7§A) e NADA muda.
        std::fs::write(events.join("tema.json"), "não é um tema").expect("write lixo");
        model.import_theme();
        assert_eq!(
            model.theme_io_message(),
            Some("Este arquivo não parece um tema do Lina.")
        );
        assert_eq!(
            model.settings().accent,
            "teal",
            "import inválido não muda nada"
        );
        theme::apply(theme::Mode::Dark, theme::DEFAULT_ACCENT); // não vaza estado global
    }

    /// T8: REUSA o W0-6 — corrompe o db, `run_recovery` (que chama `open_or_recover`) recupera do
    /// JSONL e o status apresenta os nós restaurados + o arquivo corrompido preservado.
    #[test]
    fn recovery_reuses_w0_6_and_presents_restored_nodes() {
        let tmp = TempDir::new("recover");
        let events = seed_workspace(tmp.path(), "App", "Terminal A");
        // snapshot + checkpoint para materializar no .db antes de corromper.
        {
            let mut store = EventStore::open(&events).expect("open");
            store.take_snapshot().expect("snap");
        }
        // corrompe o .db (lixo no meio) — mesma lição do W0-6.
        let db = events.join("lina.db");
        corrupt_middle(&db);

        let status = run_recovery(&events);
        match status {
            RecoveryStatus::Recovered {
                restored,
                corrupt_file,
            } => {
                assert!(
                    restored.iter().any(|n| n == "Terminal A"),
                    "lista o nó restaurado; veio {restored:?}"
                );
                assert!(
                    corrupt_file.contains(".corrupt-"),
                    "arquivo corrompido preservado"
                );
            }
            other => panic!("esperava Recovered; veio {other:?}"),
        }
        // present_recovery (sem flag de recovering) reflete o mesmo via o artefato preservado.
        assert!(matches!(
            present_recovery(&events, false),
            RecoveryStatus::Recovered { .. }
        ));
        // recovering vivo tem prioridade (overlay "em andamento").
        assert_eq!(present_recovery(&events, true), RecoveryStatus::InProgress);
    }

    /// Espaço íntegro → `present_recovery` reporta `Intact` (sem ruído).
    #[test]
    fn intact_workspace_reports_intact() {
        let tmp = TempDir::new("intact");
        let events = seed_workspace(tmp.path(), "App", "T1");
        assert_eq!(present_recovery(&events, false), RecoveryStatus::Intact);
    }

    /// Banner de recuperação NÃO "mente para sempre": um artefato `*.corrupt-*` antigo apresenta
    /// `Recovered` mas `recovered_now=false` (não foi nesta sessão) e é DISPENSÁVEL. E `check_integrity`
    /// é READ-ONLY (re-apresenta sem recuperar/renomear — não corre com o store vivo).
    #[test]
    fn recovery_banner_is_dismissible_and_recheck_is_readonly() {
        let tmp = TempDir::new("dismiss");
        let events = seed_workspace(tmp.path(), "App", "T1");
        {
            let mut s = EventStore::open(&events).expect("open");
            s.take_snapshot().expect("snap");
        }
        corrupt_middle(&events.join("lina.db"));
        let _ = run_recovery(&events); // cria o artefato *.corrupt-* + recupera (reuso W0-6)

        let store = Arc::new(Mutex::new(EventStore::open(&events).expect("reopen")));
        let shared = shared_with(0, false);
        let mut model =
            PersistenceModel::new(shared, store, events.clone(), tmp.path().to_path_buf());
        // artefato no disco → Recovered, mas NÃO "agora" (nenhuma transição de recovering nesta sessão).
        assert!(matches!(model.recovery(), RecoveryStatus::Recovered { .. }));
        assert!(
            !model.recovered_now(),
            "artefato antigo ≠ recuperação desta sessão"
        );
        assert!(!model.recovery_dismissed());
        model.dismiss_recovery();
        assert!(
            model.recovery_dismissed(),
            "dispensável (navegação sem becos)"
        );

        // READ-ONLY: re-apresenta o mesmo Recovered, sem panic/corrida (não roda open_or_recover no vivo).
        model.check_integrity();
        assert!(matches!(model.recovery(), RecoveryStatus::Recovered { .. }));
    }

    /// Sobrescreve um trecho no MEIO do arquivo com lixo (0xEE) — corrupção localizada.
    fn corrupt_middle(path: &Path) {
        use std::io::{Seek, SeekFrom, Write};
        let len = std::fs::metadata(path).expect("metadata").len();
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("abrir p/ corromper");
        let start = len / 4;
        let span = (len / 2).max(1);
        f.seek(SeekFrom::Start(start)).expect("seek");
        f.write_all(&vec![0xEE_u8; span as usize]).expect("lixo");
        f.flush().expect("flush");
    }
}
