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

// ───────────────────────────── paleta (espelha o canvas) ─────────────────────────────

const BG: u32 = 0x0a0e27;
const PANEL: u32 = 0x141a36;
const ACCENT: u32 = 0x7aa2f7;
const TEXT: u32 = 0xc8d3f5;
const MUTED: u32 = 0x5b658f;
const GREEN: u32 = 0x9ece6a;
const AMBER: u32 = 0xe0af68;

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
    /// Reduzir animações (a11y — acessível ao W4-6; aqui só o ajuste persistido).
    pub reduce_motion: bool,
    /// Pasta padrão de novos Espaços.
    pub default_cwd: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "escuro".into(),
            reduce_motion: false,
            default_cwd: String::new(),
        }
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
        };
        model.refresh_workspaces();
        model
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
    /// vivo e re-lista. (Abrir o canvas daquele Espaço é o hand-off seguinte — ver `.entrega`.)
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
        self.refresh_workspaces();
    }

    /// T7: alterna o tema (escuro↔claro) e persiste.
    pub fn toggle_theme(&mut self) {
        self.settings.theme = if self.settings.theme == "escuro" {
            "claro".into()
        } else {
            "escuro".into()
        };
        self.persist_settings();
    }

    /// T7: alterna "reduzir animações" e persiste.
    pub fn toggle_reduce_motion(&mut self) {
        self.settings.reduce_motion = !self.settings.reduce_motion;
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

/// Mostra o painel? Env-gated (`LINA_PERSIST_PANEL=1|true`) para não perturbar a demo do canvas.
#[must_use]
pub fn should_show() -> bool {
    matches!(
        std::env::var("LINA_PERSIST_PANEL").ok().as_deref(),
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
                    .text_color(rgb(MUTED))
                    .font_weight(FontWeight::BOLD)
                    .child(text!(title.to_string())),
            )
            .child(body)
            .into_any_element()
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<String>,
        bg: u32,
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
            .text_color(rgb(0xeef1ff))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _ev: &ClickEvent, window, cx| {
                on_click(view, window, cx);
            }))
            .child(text!(label))
            .into_any_element()
    }

    /// Badge "salvando…/Tudo salvo ✓" (rodapé/topo — sempre visível).
    fn save_badge(&self) -> AnyElement {
        let (label, color) = match self.model.save_indicator() {
            SaveIndicator::Saving => ("salvando…", AMBER),
            SaveIndicator::Saved => ("Tudo salvo ✓", GREEN),
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
        let dismissed = self.model.recovery_dismissed();
        let body = match self.model.recovery() {
            RecoveryStatus::InProgress => div()
                .px_4()
                .py_3()
                .rounded_md()
                .bg(rgb(AMBER))
                .text_color(rgb(0x11111b))
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
                    .bg(rgb(PANEL))
                    .border_2()
                    .border_color(rgb(GREEN))
                    .child(
                        div()
                            .text_color(rgb(GREEN))
                            .font_weight(FontWeight::BOLD)
                            .child(text!(title)),
                    )
                    .child(div().text_color(rgb(MUTED)).child(text!(names)))
                    .child(self.button(
                        "dismiss-recovery",
                        "Entendi, dispensar",
                        0x2a3152,
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
                .child(div().text_color(rgb(GREEN)).child(text!("✓ Tudo íntegro")))
                .child(self.button(
                    "check-integrity",
                    "Reverificar",
                    0x2a3152,
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
        let mut list = div().flex().flex_col().gap_2();
        for ws in self.model.workspaces() {
            // id estável e único por Espaço (a11y: evita colisão de id no `text!` do laço).
            let name = ws.name.clone();
            let name_click = name.clone();
            let dot = if ws.focused { ACCENT } else { 0x3a4566 };
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
                .bg(rgb(PANEL))
                .cursor_pointer()
                .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, _cx| {
                    v.model.switch_to(&name_click);
                }))
                .child(div().size(px(9.0)).rounded_full().bg(rgb(dot)))
                .child(
                    div()
                        .flex_1()
                        .text_color(rgb(TEXT))
                        .font_weight(FontWeight::BOLD)
                        .child(text!(name)),
                )
                .child(
                    div()
                        .text_color(rgb(MUTED))
                        .child(text!(format!("{} nó(s)", ws.nodes))),
                );
            let row = if ws.focused {
                row.child(div().text_color(rgb(ACCENT)).child(text!("• em foco")))
            } else {
                row
            };
            list = list.child(row);
        }
        if self.model.workspaces().is_empty() {
            list = list.child(div().text_color(rgb(MUTED)).child(text!(
                "Nenhum Espaço encontrado ainda — crie um pelo onboarding."
            )));
        }
        self.section("sec-workspaces", "Espaços (T6)", list.into_any_element())
    }

    /// T7 — Ajustes (toggles que persistem em settings.json).
    fn settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
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
                    .child(
                        div()
                            .w(px(160.0))
                            .text_color(rgb(TEXT))
                            .child(text!("Tema")),
                    )
                    .child(self.button(
                        "toggle-theme",
                        format!("{} (trocar)", s.theme),
                        0x2a3152,
                        cx,
                        |v, _w, _cx| v.model.toggle_theme(),
                    )),
            )
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
                            .text_color(rgb(TEXT))
                            .child(text!("Animações da interface")),
                    )
                    .child(self.button(
                        "toggle-reduce-motion",
                        if s.reduce_motion {
                            "desligadas"
                        } else {
                            "ligadas"
                        },
                        if s.reduce_motion { 0xe0af68 } else { 0x2a3152 },
                        cx,
                        |v, _w, _cx| v.model.toggle_reduce_motion(),
                    ))
                    .child(div().text_color(rgb(MUTED)).child(text!(
                        "(desligar reduz o movimento na tela — acessibilidade)"
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
                            .text_color(rgb(TEXT))
                            .child(text!("Pasta padrão")),
                    )
                    .child(div().flex_1().text_color(rgb(MUTED)).child(text!(
                        if s.default_cwd.is_empty() {
                            "(não definida)".to_string()
                        } else {
                            s.default_cwd.clone()
                        }
                    )))
                    .child(self.button(
                        "set-cwd-home",
                        "Usar a pasta de Documentos",
                        0x2a3152,
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
}

impl Render for PersistenceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        self.model.poll();

        div()
            .id("persistence-panel")
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
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
                                    .text_color(rgb(ACCENT))
                                    .child(text!("Espaços & Ajustes")),
                            )
                            .child(self.save_badge()),
                    )
                    .child(self.recovery_banner(cx))
                    .child(self.workspaces_list(cx))
                    .child(self.settings_panel(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::SharedModel;

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
