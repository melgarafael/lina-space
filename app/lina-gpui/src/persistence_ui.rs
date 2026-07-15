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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
#[cfg(test)]
use std::sync::{Arc, Mutex};

use gpui::{
    div, prelude::*, px, rgb, size, text, AnyElement, App, Bounds, ClickEvent, Context,
    FocusHandle, FontWeight, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
#[cfg(test)]
use lina_core::DomainEvent;
use lina_core::{
    EventStore, StoreError, Workspace, WorkspaceEntry as RegistryEntry, WorkspaceError,
    WorkspaceRegistry,
};
use lina_host::{HeadlessUiHost, HostEvent, UiHost};
use rusqlite::OptionalExtension as _;
use serde::{Deserialize, Serialize};

use crate::bridge::{lock, Model};
use crate::theme;
use crate::ui::{Button, ButtonVariant, Input, RadiusExt};

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
    /// A integridade não pôde ser determinada; nunca apresentar isto como íntegro.
    Failed { detail: String },
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

/// Abre um store antes de montar o runtime. Banco saudável segue pelo caminho comum;
/// banco ausente/corrompido só é reconstruído quando há um JSONL completo e inequívoco.
/// O artefato preservado torna a recuperação observável na UI de persistência.
pub fn open_store_resilient(events_dir: &Path) -> Result<EventStore, String> {
    let mut host = RecorderHost::default();
    let (store, _projected) = EventStore::open_projected_or_recover(events_dir, &mut host)
        .map_err(|error| {
            format!(
                "não consegui abrir ou recuperar {} ({error})",
                events_dir.display()
            )
        })?;
    if host
        .events
        .iter()
        .any(|event| matches!(event, HostEvent::Recovered))
    {
        eprintln!(
            "persistence_ui: {} foi recuperado do espelho e o banco anterior foi preservado",
            events_dir.display()
        );
    }
    Ok(store)
}

/// Acha o artefato `*.corrupt-*` que o W0-6 PRESERVA ao recuperar (prova durável de que houve crash).
fn find_corrupt_file(events_dir: &Path) -> Result<Option<String>, String> {
    let entries = std::fs::read_dir(events_dir)
        .map_err(|error| format!("não consegui ler {}: {error}", events_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "não consegui ler uma entrada de {}: {error}",
                events_dir.display()
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(".corrupt-") {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Confere o SQLite vivo sem criar schema, reconciliar espelho ou tocar no WAL.
/// A leitura toca apenas o schema e as extremidades da chave primária; nunca varre o
/// histórico. A recuperação completa continua pertencendo a [`run_recovery`].
fn check_store_health_read_only(events_dir: &Path) -> Result<(), String> {
    let db_path = events_dir.join("lina.db");
    let metadata = std::fs::metadata(&db_path)
        .map_err(|error| format!("não consegui acessar {}: {error}", db_path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} não é um banco SQLite", db_path.display()));
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = rusqlite::Connection::open_with_flags(&db_path, flags).map_err(|error| {
        format!(
            "não consegui abrir {} para leitura: {error}",
            db_path.display()
        )
    })?;
    let has_events_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'events')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            format!(
                "não consegui conferir o schema de {}: {error}",
                db_path.display()
            )
        })?;
    if !has_events_table {
        return Err(format!(
            "{} não contém a tabela de eventos",
            db_path.display()
        ));
    }
    connection
        .prepare("SELECT seq, ts, kind, version, payload FROM events WHERE seq > ?1")
        .map_err(|error| {
            format!(
                "não consegui conferir o schema de {}: {error}",
                db_path.display()
            )
        })?;
    for query in [
        "SELECT seq FROM events ORDER BY seq ASC LIMIT 1",
        "SELECT seq FROM events ORDER BY seq DESC LIMIT 1",
    ] {
        let endpoint = connection
            .query_row(query, [], |row| row.get::<_, i64>(0))
            .optional()
            .map_err(|error| {
                format!(
                    "não consegui ler uma extremidade de {}: {error}",
                    db_path.display()
                )
            })?;
        if endpoint.is_some_and(|seq| seq < 1) {
            return Err(format!("{} contém sequência inválida", db_path.display()));
        }
    }
    Ok(())
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
    if let Err(detail) = check_store_health_read_only(events_dir) {
        return RecoveryStatus::Failed { detail };
    }
    match find_corrupt_file(events_dir) {
        Ok(Some(corrupt_file)) => RecoveryStatus::Recovered {
            restored: restored_nodes(events_dir),
            corrupt_file,
        },
        Ok(None) => RecoveryStatus::Intact,
        Err(detail) => RecoveryStatus::Failed { detail },
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
    match EventStore::open_projected_or_recover(events_dir, &mut host) {
        Ok((_store, projected)) => {
            let recovered = host
                .events
                .iter()
                .any(|e| matches!(e, HostEvent::Recovered));
            if recovered {
                match find_corrupt_file(events_dir) {
                    Ok(Some(corrupt_file)) => RecoveryStatus::Recovered {
                        restored: projected
                            .nodes
                            .values()
                            .filter_map(|node| node.name.clone())
                            .collect(),
                        corrupt_file,
                    },
                    Ok(None) => RecoveryStatus::Failed {
                        detail: "a recuperação terminou sem preservar o banco anterior".to_owned(),
                    },
                    Err(detail) => RecoveryStatus::Failed { detail },
                }
            } else {
                RecoveryStatus::Intact
            }
        }
        Err(e) => {
            eprintln!(
                "persistence_ui: abertura/recuperação falhou em {}: {e}",
                events_dir.display()
            );
            RecoveryStatus::Failed {
                detail: e.to_string(),
            }
        }
    }
}

// ═══════════════════════════════ T6 Espaços (gpui-free) ═══════════════════════════════

/// Um Espaço listado no Switcher (T6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub name: String,
    pub workspace_id: String,
    pub events_dir: PathBuf,
    pub focused: bool,
    pub archived: bool,
    pub nodes: usize,
}

/// Natureza de um problema encontrado durante a varredura. Só uma validação
/// determinística pode ser isolada; erro de leitura torna o snapshot inconclusivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceScanIssueKind {
    ReadFailure,
    InvalidWorkspace,
}

/// Problema estruturado do catálogo — callers nunca precisam inferir severidade
/// procurando texto de errno na mensagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceScanIssue {
    pub kind: WorkspaceScanIssueKind,
    pub message: String,
}

impl WorkspaceScanIssue {
    #[must_use]
    pub fn read_failure(message: impl Into<String>) -> Self {
        Self {
            kind: WorkspaceScanIssueKind::ReadFailure,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn invalid_workspace(message: impl Into<String>) -> Self {
        Self {
            kind: WorkspaceScanIssueKind::InvalidWorkspace,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn blocks_publication(&self) -> bool {
        self.kind == WorkspaceScanIssueKind::ReadFailure
    }
}

impl std::fmt::Display for WorkspaceScanIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkspaceScanIssue {}

/// Resultado da varredura. `InvalidWorkspace` isola só a entrada inválida;
/// qualquer `ReadFailure` impede o snapshot de substituir o last-good.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceScan {
    pub entries: Vec<WorkspaceEntry>,
    pub issues: Vec<WorkspaceScanIssue>,
    /// Projeções por raiz de Workspace, produzidas pela mesma leitura que validou
    /// as entradas. Consumidores reutilizam esta foto em vez de reabrir cada store.
    pub projected_states: BTreeMap<PathBuf, lina_core::ProjectedState>,
    /// Contador observável da rodada: um store deve ser projetado no máximo uma vez.
    pub projected_store_reads: usize,
    attempted_roots: std::collections::BTreeSet<PathBuf>,
    quarantined_roots: std::collections::BTreeSet<PathBuf>,
    preserve_loaded_registry: bool,
}

impl WorkspaceScan {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self
            .issues
            .iter()
            .any(WorkspaceScanIssue::blocks_publication)
    }

    #[must_use]
    pub fn issue_summary(&self) -> String {
        self.issues
            .iter()
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// O `events_dir` (`<ws>/.lina/events`) é um Espaço se tiver o log (db ou espelho JSONL).
fn is_workspace_store(events_dir: &Path) -> std::io::Result<bool> {
    let db = events_dir.join("lina.db");
    if db.try_exists()? {
        return Ok(true);
    }
    let jsonl = events_dir.join("log.jsonl");
    jsonl.try_exists()
}

fn store_projection_issue(
    events_dir: &Path,
    action: &str,
    error: StoreError,
    known_store: bool,
) -> WorkspaceScanIssue {
    let deterministically_malformed = match &error {
        StoreError::Io(error) => error.kind() == std::io::ErrorKind::InvalidData,
        StoreError::Json(_) => true,
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(sqlite_error, _)) => matches!(
            sqlite_error.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ),
        StoreError::Sqlite(_) => false,
    };
    let message = format!("{} {action}: {error}", events_dir.display());
    if deterministically_malformed && !known_store {
        WorkspaceScanIssue::invalid_workspace(message)
    } else {
        WorkspaceScanIssue::read_failure(message)
    }
}

fn missing_workspace_fact_issue(events_dir: &Path, missing_fact: &str) -> WorkspaceScanIssue {
    let message = format!(
        "{} está incompleto: faltou {missing_fact}",
        events_dir.display()
    );
    WorkspaceScanIssue::invalid_workspace(message)
}

/// Projeta os fatos autoritativos do store. Falha de open/projeção é leitura
/// inconclusiva. Fato obrigatório ausente é inválido determinístico; quando a raiz
/// já está no registry, o ponteiro last-good é preservado, mas não vira linha clicável.
fn project_entry(
    events_dir: PathBuf,
    focused: bool,
    include_archived: bool,
    known_store: bool,
) -> Result<Option<(WorkspaceEntry, lina_core::ProjectedState)>, WorkspaceScanIssue> {
    let mut host = HeadlessUiHost::new();
    let (_store, st) =
        EventStore::open_projected_or_recover(&events_dir, &mut host).map_err(|error| {
            store_projection_issue(&events_dir, "não abriu/projetou", error, known_store)
        })?;
    let workspace_id = st
        .workspace_id
        .as_deref()
        .filter(|workspace_id| !workspace_id.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| missing_workspace_fact_issue(&events_dir, "WorkspaceIdAssigned válido"))?;
    let name = st
        .workspace_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| missing_workspace_fact_issue(&events_dir, "WorkspaceCreated com nome"))?;
    // O fallback é re-derivável do log, inclusive o fato `archived`. Sem este
    // filtro, perder temporariamente o registry ressuscitaria Espaços arquivados.
    // O ativo continua visível mesmo num estado legado inconsistente.
    if st.archived && !focused && !include_archived {
        return Ok(None);
    }
    Ok(Some((
        WorkspaceEntry {
            name,
            workspace_id,
            events_dir,
            focused,
            archived: st.archived,
            nodes: st.nodes.len(),
        },
        st,
    )))
}

fn scan_workspaces(
    base: &Path,
    current: &Path,
    include_archived: bool,
    known_roots: &std::collections::BTreeSet<PathBuf>,
    base_failure_is_inconclusive: bool,
) -> Result<WorkspaceScan, WorkspaceScanIssue> {
    let mut out: Vec<WorkspaceEntry> = Vec::new();
    let mut issues = Vec::new();
    let mut projected_states = BTreeMap::new();
    let mut projected_store_reads = 0;
    let mut attempted_roots = std::collections::BTreeSet::new();
    let mut preserve_loaded_registry = false;
    // O Espaço vivo vem primeiro, mas a varredura pré-boot nunca materializa um
    // EventStore vazio só para descobrir se o default existe.
    match is_workspace_store(current) {
        Ok(true) => {
            projected_store_reads += 1;
            attempted_roots.insert(crate::workspace_boot::ws_root_of_events_dir(current));
            match project_entry(current.to_path_buf(), true, include_archived, true) {
                Ok(Some((entry, state))) => {
                    projected_states.insert(
                        crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir),
                        state,
                    );
                    out.push(entry);
                }
                Ok(None) => {}
                Err(mut issue) => {
                    preserve_loaded_registry |= issue.kind
                        == WorkspaceScanIssueKind::InvalidWorkspace
                        && known_roots
                            .contains(&crate::workspace_boot::ws_root_of_events_dir(current));
                    issue.message = format!(
                        "não consegui reler o Espaço ativo; ele continua visível pelo runtime: {}",
                        issue.message
                    );
                    issues.push(issue);
                }
            }
        }
        Ok(false) => {}
        Err(error) => {
            issues.push(WorkspaceScanIssue::read_failure(format!(
                "não consegui verificar o Espaço ativo {}: {error}",
                current.display()
            )));
        }
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    let entries = match std::fs::read_dir(base) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if base_failure_is_inconclusive {
                issues.push(WorkspaceScanIssue::read_failure(format!(
                    "não consegui confirmar a base conhecida {}: {error}",
                    base.display()
                )));
            }
            None
        }
        Err(error) => {
            if base_failure_is_inconclusive {
                issues.push(WorkspaceScanIssue::read_failure(format!(
                    "não consegui confirmar a base conhecida {}: {error}",
                    base.display()
                )));
                None
            } else {
                return Err(WorkspaceScanIssue::read_failure(format!(
                    "não consegui ler {}: {error}",
                    base.display()
                )));
            }
        }
    };
    if let Some(entries) = entries {
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    issues.push(WorkspaceScanIssue::read_failure(format!(
                        "uma entrada de {} não pôde ser lida: {error}",
                        base.display()
                    )));
                    continue;
                }
            };
            let metadata = match std::fs::metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) => {
                    issues.push(WorkspaceScanIssue::read_failure(format!(
                        "não consegui identificar {}: {error}",
                        entry.path().display()
                    )));
                    continue;
                }
            };
            if metadata.is_dir() {
                if crate::workspace_boot::is_reserved_creation_path(&entry.path()) {
                    issues.push(WorkspaceScanIssue::invalid_workspace(format!(
                        "{} é uma preparação pendente e ficou oculta do catálogo",
                        entry.path().display()
                    )));
                } else {
                    dirs.push(entry.path());
                }
            }
        }
    }
    dirs.sort();
    for d in dirs {
        // Aceita tanto `<sub>/.lina/events` quanto `<sub>/events` quanto `<sub>` já sendo o events dir.
        for candidate in [d.join(".lina").join("events"), d.join("events"), d.clone()] {
            let is_store = match is_workspace_store(&candidate) {
                Ok(is_store) => is_store,
                Err(error) => {
                    issues.push(WorkspaceScanIssue::read_failure(format!(
                        "não consegui verificar {}: {error}",
                        candidate.display()
                    )));
                    continue;
                }
            };
            if is_store && candidate != current && !out.iter().any(|e| e.events_dir == candidate) {
                let root = crate::workspace_boot::ws_root_of_events_dir(&candidate);
                let known_store = known_roots.contains(&root);
                projected_store_reads += 1;
                attempted_roots.insert(root.clone());
                match project_entry(candidate, false, include_archived, known_store) {
                    Ok(Some((entry, state))) => {
                        projected_states.insert(
                            crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir),
                            state,
                        );
                        out.push(entry);
                    }
                    Ok(None) => {}
                    Err(issue) => {
                        preserve_loaded_registry |=
                            known_store && issue.kind == WorkspaceScanIssueKind::InvalidWorkspace;
                        issues.push(issue);
                    }
                }
                break;
            }
        }
    }
    let mut scan = WorkspaceScan {
        entries: out,
        issues,
        projected_states,
        projected_store_reads,
        attempted_roots,
        quarantined_roots: std::collections::BTreeSet::new(),
        preserve_loaded_registry,
    };
    quarantine_duplicate_workspace_ids(&mut scan, known_roots);
    Ok(scan)
}

fn quarantine_duplicate_workspace_ids(
    scan: &mut WorkspaceScan,
    known_roots: &std::collections::BTreeSet<PathBuf>,
) {
    let identities = scan.entries.iter().map(|entry| {
        (
            entry.workspace_id.clone(),
            crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir),
        )
    });
    let (quarantined, preserve_loaded_registry) =
        duplicate_workspace_id_quarantine(identities, known_roots);
    scan.preserve_loaded_registry |= preserve_loaded_registry;
    apply_duplicate_workspace_id_quarantine(scan, quarantined);
}

fn duplicate_workspace_id_quarantine(
    identities: impl IntoIterator<Item = (String, PathBuf)>,
    known_roots: &std::collections::BTreeSet<PathBuf>,
) -> (Vec<(String, PathBuf)>, bool) {
    let mut roots_by_id = BTreeMap::<String, std::collections::BTreeSet<PathBuf>>::new();
    for (workspace_id, root) in identities {
        roots_by_id.entry(workspace_id).or_default().insert(root);
    }
    let mut quarantined = Vec::new();
    let mut preserve_loaded_registry = false;
    for (workspace_id, roots) in roots_by_id {
        if roots.len() < 2 {
            continue;
        }
        let known: Vec<PathBuf> = roots
            .iter()
            .filter(|root| known_roots.contains(*root))
            .cloned()
            .collect();
        if known.len() == 1 {
            quarantined.extend(
                roots
                    .into_iter()
                    .filter(|root| root != &known[0])
                    .map(|root| (workspace_id.clone(), root)),
            );
        } else {
            preserve_loaded_registry |= known.len() > 1;
            quarantined.extend(roots.into_iter().map(|root| (workspace_id.clone(), root)));
        }
    }
    (quarantined, preserve_loaded_registry)
}

fn apply_duplicate_workspace_id_quarantine(
    scan: &mut WorkspaceScan,
    quarantined: Vec<(String, PathBuf)>,
) -> std::collections::BTreeSet<PathBuf> {
    let quarantined_roots: std::collections::BTreeSet<PathBuf> =
        quarantined.iter().map(|(_, root)| root.clone()).collect();
    for root in &quarantined_roots {
        scan.projected_states.remove(root);
        scan.quarantined_roots.insert(root.clone());
    }
    scan.entries.retain(|entry| {
        !quarantined_roots.contains(&crate::workspace_boot::ws_root_of_events_dir(
            &entry.events_dir,
        ))
    });
    scan.issues
        .extend(quarantined.into_iter().map(|(id, root)| {
            WorkspaceScanIssue::invalid_workspace(format!(
                "{} foi isolado: workspace_id duplicado ({id})",
                root.display()
            ))
        }));
    quarantined_roots
}

/// Lista os Espaços visíveis: arquivados fora do foco continuam ocultos, mas seus
/// fatos nunca são confundidos com falha de leitura.
#[cfg(test)]
pub fn list_workspaces(base: &Path, current: &Path) -> Result<WorkspaceScan, WorkspaceScanIssue> {
    scan_workspaces(
        base,
        current,
        false,
        &std::collections::BTreeSet::new(),
        false,
    )
}

/// Registry carregado junto do scan que o validou/reconstruiu.
pub struct WorkspaceRegistryCatalog {
    pub registry: WorkspaceRegistry,
    /// Entradas já projetadas e conferidas nesta mesma leitura. Consumidores não
    /// reabrem os stores só para repetir a validação.
    pub verified_entries: Vec<RegistryEntry>,
    /// Raízes cuja identidade permaneceu realmente ambígua e não podem reaparecer
    /// pelo fallback do scan.
    pub quarantined_roots: std::collections::BTreeSet<PathBuf>,
    pub scan: WorkspaceScan,
    pub rebuilt: bool,
}

fn rederive_registry_pointer(
    pointer: &RegistryEntry,
) -> Result<(RegistryEntry, lina_core::ProjectedState), WorkspaceScanIssue> {
    if crate::workspace_boot::is_reserved_creation_path(&pointer.path) {
        return Err(WorkspaceScanIssue::invalid_workspace(format!(
            "{} é uma preparação pendente e ficou oculta do catálogo",
            pointer.path.display()
        )));
    }
    let events_dir = Workspace::events_dir(&pointer.path);
    match is_workspace_store(&events_dir) {
        Ok(true) => {}
        Ok(false) => {
            return Err(WorkspaceScanIssue::read_failure(format!(
                "não consegui confirmar o store conhecido em {}",
                pointer.path.display()
            )));
        }
        Err(error) => {
            return Err(WorkspaceScanIssue::read_failure(format!(
                "não consegui verificar {}: {error}",
                pointer.path.display()
            )));
        }
    }
    let (projected, state) = project_entry(events_dir, false, true, true)?
        .ok_or_else(|| WorkspaceScanIssue::invalid_workspace("store arquivado não projetou"))?;
    Ok((
        RegistryEntry {
            id: projected.workspace_id,
            name: projected.name,
            path: pointer.path.clone(),
            last_focus: pointer.last_focus,
            archived: projected.archived,
        },
        state,
    ))
}

fn reconciled_registry_entries(
    loaded: Option<&WorkspaceRegistry>,
    scan: &mut WorkspaceScan,
) -> (Vec<RegistryEntry>, std::collections::BTreeSet<PathBuf>) {
    let mut candidates = Vec::new();
    let mut blocked_roots = scan.quarantined_roots.clone();
    let mut focus_by_id = BTreeMap::<String, u64>::new();
    let scanned_by_root: BTreeMap<PathBuf, RegistryEntry> = scan
        .entries
        .iter()
        .map(|entry| {
            let root = crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir);
            (
                root.clone(),
                RegistryEntry {
                    id: entry.workspace_id.clone(),
                    name: entry.name.clone(),
                    path: root,
                    last_focus: 0,
                    archived: entry.archived,
                },
            )
        })
        .collect();

    if let Some(registry) = loaded {
        for pointer in registry.entries() {
            focus_by_id
                .entry(pointer.id.clone())
                .and_modify(|focus| *focus = (*focus).max(pointer.last_focus))
                .or_insert(pointer.last_focus);
            if scan.quarantined_roots.contains(&pointer.path) {
                continue;
            }
            let derived = match scanned_by_root.get(&pointer.path).cloned() {
                Some(derived) => Ok(derived),
                None if scan.attempted_roots.contains(&pointer.path) => {
                    blocked_roots.insert(pointer.path.clone());
                    continue;
                }
                None => {
                    scan.projected_store_reads += 1;
                    rederive_registry_pointer(pointer).map(|(derived, state)| {
                        scan.projected_states.insert(pointer.path.clone(), state);
                        derived
                    })
                }
            };
            match derived {
                Ok(mut derived) => {
                    derived.last_focus = pointer.last_focus;
                    candidates.push(derived);
                }
                Err(issue) => {
                    if issue.kind == WorkspaceScanIssueKind::InvalidWorkspace {
                        scan.preserve_loaded_registry = true;
                        scan.quarantined_roots.insert(pointer.path.clone());
                    }
                    blocked_roots.insert(pointer.path.clone());
                    scan.issues.push(issue);
                }
            }
        }
    }

    for entry in &scan.entries {
        let root = crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir);
        if blocked_roots.contains(&root) {
            continue;
        }
        if let Some(existing) = candidates
            .iter_mut()
            .find(|candidate| candidate.id == entry.workspace_id && candidate.path == root)
        {
            existing.name.clone_from(&entry.name);
            existing.archived = entry.archived;
            continue;
        }
        candidates.push(RegistryEntry {
            id: entry.workspace_id.clone(),
            name: entry.name.clone(),
            path: root,
            last_focus: focus_by_id
                .get(&entry.workspace_id)
                .copied()
                .unwrap_or_default(),
            archived: entry.archived,
        });
    }

    let known_roots: std::collections::BTreeSet<PathBuf> = loaded
        .into_iter()
        .flat_map(|registry| registry.entries().iter())
        .map(|entry| entry.path.clone())
        .collect();
    let candidate_identities = candidates
        .iter()
        .map(|entry| (entry.id.clone(), entry.path.clone()));
    let (quarantined, preserve_loaded_registry) =
        duplicate_workspace_id_quarantine(candidate_identities, &known_roots);
    scan.preserve_loaded_registry |= preserve_loaded_registry;
    let duplicate_roots = apply_duplicate_workspace_id_quarantine(scan, quarantined);
    blocked_roots.extend(duplicate_roots.iter().cloned());
    candidates.retain(|entry| !duplicate_roots.contains(&entry.path));

    let mut id_counts = BTreeMap::<String, usize>::new();
    let mut path_counts = BTreeMap::<PathBuf, usize>::new();
    for entry in &candidates {
        *id_counts.entry(entry.id.clone()).or_default() += 1;
        *path_counts.entry(entry.path.clone()).or_default() += 1;
    }
    let mut verified = Vec::new();
    for entry in candidates {
        if id_counts.get(&entry.id).copied().unwrap_or_default() > 1
            || path_counts.get(&entry.path).copied().unwrap_or_default() > 1
        {
            blocked_roots.insert(entry.path.clone());
            scan.issues
                .push(WorkspaceScanIssue::invalid_workspace(format!(
                    "{} foi isolado: identidade ou caminho duplicado no catálogo",
                    entry.path.display()
                )));
        } else {
            verified.push(entry);
        }
    }

    if loaded.is_none()
        && !verified
            .iter()
            .any(|entry| !entry.archived && entry.last_focus > 0)
    {
        let preferred_id = scan
            .entries
            .iter()
            .find(|entry| entry.focused && !entry.archived)
            .or_else(|| scan.entries.iter().find(|entry| !entry.archived))
            .map(|entry| entry.workspace_id.as_str());
        if let Some(entry) = verified
            .iter_mut()
            .find(|entry| Some(entry.id.as_str()) == preferred_id)
        {
            entry.last_focus = 1;
        }
    }
    (verified, blocked_roots)
}

/// Carrega o ponteiro global e reconcilia somente ausência ou conteúdo stale válido.
/// Parse inválido é inconclusivo e fica byte-idêntico para não apagar ponteiros externos.
pub fn load_or_rebuild_workspace_registry(
    registry_path: &Path,
    workspaces_base: &Path,
    current_events_dir: &Path,
) -> Result<WorkspaceRegistryCatalog, String> {
    load_or_rebuild_workspace_registry_attempt(
        registry_path,
        workspaces_base,
        current_events_dir,
        true,
    )
}

fn load_or_rebuild_workspace_registry_attempt(
    registry_path: &Path,
    workspaces_base: &Path,
    current_events_dir: &Path,
    retry_after_concurrent_change: bool,
) -> Result<WorkspaceRegistryCatalog, String> {
    let registry_snapshot = match std::fs::read(registry_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "não consegui fotografar o registry {}: {error}",
                registry_path.display()
            ));
        }
    };
    let registry_exists = registry_snapshot.is_some();
    let (mut loaded_registry, rebuild_required) = match WorkspaceRegistry::load(registry_path) {
        Ok(registry) => (Some(registry), !registry_exists),
        Err(error @ WorkspaceError::Json(_)) => {
            return Err(format!(
                "o registry {} está inválido; catálogo inconclusivo e bytes preservados: {error}",
                registry_path.display()
            ));
        }
        Err(WorkspaceError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Err(format!(
                "o registry {} está inválido; catálogo inconclusivo e bytes preservados: {error}",
                registry_path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "não consegui ler o registry {}: {error}",
                registry_path.display()
            ));
        }
    };
    let pending_focus_issue = loaded_registry.as_mut().and_then(|registry| {
        registry.recover_pending_focus().err().map(|error| {
            WorkspaceScanIssue::read_failure(format!(
                "não consegui reconciliar a troca de Espaço interrompida; o journal foi preservado: {error}"
            ))
        })
    });

    let known_roots: std::collections::BTreeSet<PathBuf> = loaded_registry
        .as_ref()
        .map(|registry| {
            registry
                .entries()
                .iter()
                .map(|entry| entry.path.clone())
                .collect()
        })
        .unwrap_or_default();
    let base_failure_is_inconclusive =
        registry_exists && (rebuild_required || !known_roots.is_empty());
    let mut scan = scan_workspaces(
        workspaces_base,
        current_events_dir,
        true,
        &known_roots,
        base_failure_is_inconclusive,
    )
    .map_err(|issue| issue.message)?;
    scan.issues.extend(pending_focus_issue);
    let (verified_entries, quarantined_roots) =
        reconciled_registry_entries(loaded_registry.as_ref(), &mut scan);
    if scan.preserve_loaded_registry {
        let registry = loaded_registry.ok_or_else(|| {
            "identidades conhecidas ficaram ambíguas sem registry last-good".to_owned()
        })?;
        let catalog_entries = if scan.is_complete() {
            verified_entries
        } else {
            registry
                .entries()
                .iter()
                .filter(|entry| !scan.quarantined_roots.contains(&entry.path))
                .cloned()
                .collect()
        };
        return Ok(WorkspaceRegistryCatalog {
            registry,
            verified_entries: catalog_entries,
            quarantined_roots,
            scan,
            rebuilt: false,
        });
    }
    if !scan.is_complete() {
        let registry = loaded_registry.ok_or_else(|| {
            format!(
                "o registry precisa ser reconstruído, mas a varredura ficou inconclusiva: {}",
                scan.issue_summary()
            )
        })?;
        let last_good_entries = registry.entries().to_vec();
        return Ok(WorkspaceRegistryCatalog {
            registry,
            verified_entries: last_good_entries,
            quarantined_roots,
            scan,
            rebuilt: false,
        });
    }

    let reconciliation_required = rebuild_required
        || loaded_registry
            .as_ref()
            .is_some_and(|registry| registry.entries() != verified_entries.as_slice());
    let catalog_entries = verified_entries.clone();
    let mut registry = if reconciliation_required {
        match WorkspaceRegistry::rebuild_from_verified_entries_if_unchanged(
            registry_path,
            verified_entries,
            registry_snapshot.as_deref(),
        ) {
            Ok(registry) => registry,
            Err(WorkspaceError::RegistryChangedDuringRebuild { .. })
                if retry_after_concurrent_change =>
            {
                return load_or_rebuild_workspace_registry_attempt(
                    registry_path,
                    workspaces_base,
                    current_events_dir,
                    false,
                );
            }
            Err(error @ WorkspaceError::RegistryChangedDuringRebuild { .. }) => {
                return Err(format!(
                    "o registry mudou durante a reconciliação; catálogo inconclusivo e nenhum byte foi sobrescrito: {error}"
                ));
            }
            Err(error) => {
                return Err(format!("não consegui reconstruir o registry: {error}"));
            }
        }
    } else {
        loaded_registry
            .ok_or_else(|| "registry carregado desapareceu durante a leitura".to_owned())?
    };
    if rebuild_required {
        registry.recover_pending_focus().map_err(|error| {
            format!("não consegui reconciliar a troca de Espaço interrompida: {error}")
        })?;
    }

    let rebuilt = reconciliation_required;
    Ok(WorkspaceRegistryCatalog {
        registry,
        verified_entries: catalog_entries,
        quarantined_roots,
        scan,
        rebuilt,
    })
}

// ═══════════════════════════════ T7 Ajustes (gpui-free) ═══════════════════════════════

/// Preferências do usuário, persistidas em `settings.json` (persiste após reabrir).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Tema visual (`"sistema"` | `"escuro"` | `"claro"`) — `"sistema"` segue a aparência do SO
    /// (F2-1-4; decisão F2-0-D nº2: o chrome segue o sistema é o DEFAULT do produto).
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
    /// F2-1-4: ajustes parciais por token (tema "Avançado") — persistem entre sessões AQUI
    /// (fonte única; `tema.json` é só veículo de export/import). Vazio = sem refinamento.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub theme_overrides: BTreeMap<String, String>,
    /// F2-1-4: marcador da migração única `escuro`→`sistema` (o default antigo "escuro" quase
    /// nunca foi escolha; a decisão F2-0-D nº2 fez "sistema" o default do produto). `false` em
    /// settings antigos (`serde(default)`) dispara a migração UMA vez em [`load_settings`];
    /// quem re-escolher "escuro" depois fica em "escuro" para sempre.
    #[serde(default)]
    pub theme_migrated_to_system: bool,
    /// F2-2-4 (costura): MRU da paleta DURÁVEL entre sessões (keys de `palette::Command::key()`,
    /// frente = mais recente; teto/dedup são do modelo `palette.rs`). O shell injeta no `open`
    /// e re-grava a cada escolha. Aditivo: settings antigos carregam vazio (ranking degrada
    /// graciosamente sem MRU — tiers alias/fuzzy seguem).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette_mru: Vec<String>,
    /// F3-1-7: metas que o usuário FECHOU (dispensou) do canvas — DURÁVEL entre sessões (por `goal_id`).
    /// Preferência de VISUALIZAÇÃO (a meta segue no event log, intacta e auditável); o card só não é
    /// mostrado. Aditivo: settings antigos carregam vazio (nada dispensado — todo card aparece).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissed_goals: Vec<String>,
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
            theme: "sistema".into(),
            accent: default_accent_setting(),
            reduce_motion: false,
            default_cwd: String::new(),
            attention_sound_muted: false,
            restore_on_open: true,
            theme_overrides: BTreeMap::new(),
            // Instalação nova já nasce em "sistema" — nada a migrar (e o load não re-grava).
            theme_migrated_to_system: true,
            palette_mru: Vec::new(),
            dismissed_goals: Vec::new(),
        }
    }
}

impl Settings {
    /// A PREFERÊNCIA de modo destes ajustes (F2-1-4) — `"sistema"` incluso; é o que
    /// `theme::apply_setting` resolve contra a aparência viva do SO (único caminho de parse;
    /// o `theme_mode()` legado saiu junto com o último consumidor, na costura r2a).
    #[must_use]
    pub fn mode_setting(&self) -> theme::ModeSetting {
        theme::ModeSetting::from_setting(&self.theme)
    }
}

fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// Lê os ajustes (best-effort: ausência/erro → default — nunca falha).
///
/// **Migração única F2-1-4 (decisão F2-0-D nº2):** num `settings.json` pré-F2 (sem o marcador),
/// `"escuro"` era o default de fábrica que quase ninguém escolheu — vira `"sistema"`; `"claro"`
/// foi escolha explícita (o default era escuro) e FICA. O marcador gravado impede re-migrar quem
/// re-escolher "escuro" depois.
#[must_use]
pub fn load_settings(dir: &Path) -> Settings {
    let mut s: Settings = std::fs::read_to_string(settings_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if !s.theme_migrated_to_system {
        if s.theme.trim().eq_ignore_ascii_case("escuro") {
            s.theme = "sistema".into();
        }
        s.theme_migrated_to_system = true;
        save_settings(dir, &s);
    }
    s
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
    /// `events` dir do Espaço vivo (settings.json mora ao lado; raiz do scan de recuperação).
    ws_dir: PathBuf,
    /// Pasta-base a varrer por Espaços (T6).
    workspaces_base: PathBuf,
    /// Ponteiro global do catálogo. `None` mantém o snapshot e mostra aviso.
    registry_path: Option<PathBuf>,
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
    /// Falha/entrada isolada da última varredura. A lista continua sendo o último
    /// snapshot completo quando a enumeração da base falha.
    workspaces_warning: Option<String>,
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
    pub fn new(shared: Model, ws_dir: PathBuf, workspaces_base: PathBuf) -> Self {
        Self::with_registry_path(
            shared,
            ws_dir,
            workspaces_base,
            lina_core::default_registry_path(),
        )
    }

    fn with_registry_path(
        shared: Model,
        ws_dir: PathBuf,
        workspaces_base: PathBuf,
        registry_path: Option<PathBuf>,
    ) -> Self {
        let settings_dir = ws_dir.clone();
        let settings = load_settings(&settings_dir);
        let recovering = lock(&shared).recovering;
        let last_event_count = lock(&shared).event_count;
        let mut model = Self {
            shared,
            ws_dir: ws_dir.clone(),
            workspaces_base,
            registry_path,
            settings_dir,
            settings,
            last_event_count,
            saving_ticks: 0,
            recovery: present_recovery(&ws_dir, recovering),
            last_recovering: recovering,
            recovered_now: false,
            recovery_dismissed: false,
            workspaces: Vec::new(),
            workspaces_warning: None,
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

    /// Aviso observável da última varredura, sem substituir a lista por vazio.
    #[must_use]
    pub fn workspaces_warning(&self) -> Option<&str> {
        self.workspaces_warning.as_deref()
    }

    /// Ajustes correntes (T7).
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Atualiza a lista pelo mesmo catálogo global usado pelo canvas/sidebar.
    pub fn refresh_workspaces(&mut self) {
        let Some(registry_path) = self.registry_path.as_deref() else {
            self.workspaces_warning = Some(
                "Não consegui atualizar a lista agora. Mantive a última versão completa."
                    .to_owned(),
            );
            return;
        };
        let catalog = match load_or_rebuild_workspace_registry(
            registry_path,
            &self.workspaces_base,
            &self.ws_dir,
        ) {
            Ok(catalog) => catalog,
            Err(error) => {
                eprintln!("persistence_ui: catálogo de Espaços não atualizado: {error}");
                self.workspaces_warning = Some(
                    "Não consegui atualizar a lista agora. Mantive a última versão completa."
                        .to_owned(),
                );
                return;
            }
        };

        let current_root = crate::workspace_boot::ws_root_of_events_dir(&self.ws_dir);
        let scanned_events_by_root: BTreeMap<PathBuf, PathBuf> = catalog
            .scan
            .entries
            .iter()
            .map(|entry| {
                (
                    crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir),
                    entry.events_dir.clone(),
                )
            })
            .collect();
        let previous_by_root: BTreeMap<PathBuf, &WorkspaceEntry> = self
            .workspaces
            .iter()
            .map(|entry| {
                (
                    crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir),
                    entry,
                )
            })
            .collect();
        let rows: Vec<WorkspaceEntry> = catalog
            .verified_entries
            .iter()
            .filter(|entry| !entry.archived || entry.path == current_root)
            .map(|entry| {
                let previous = previous_by_root.get(&entry.path).copied();
                WorkspaceEntry {
                    name: entry.name.clone(),
                    workspace_id: entry.id.clone(),
                    events_dir: scanned_events_by_root
                        .get(&entry.path)
                        .cloned()
                        .or_else(|| previous.map(|entry| entry.events_dir.clone()))
                        .unwrap_or_else(|| Workspace::events_dir(&entry.path)),
                    focused: entry.path == current_root,
                    archived: entry.archived,
                    nodes: catalog
                        .scan
                        .projected_states
                        .get(&entry.path)
                        .map(|state| state.nodes.len())
                        .or_else(|| previous.map(|entry| entry.nodes))
                        .unwrap_or_default(),
                }
            })
            .collect();
        let issue_summary = catalog.scan.issue_summary();
        if catalog.scan.is_complete() {
            self.workspaces = rows;
            self.workspaces_warning = (!catalog.scan.issues.is_empty()).then(|| {
                eprintln!("persistence_ui: stores isolados no catálogo: {issue_summary}");
                "Alguns Espaços precisam de atenção; os demais continuam disponíveis.".to_owned()
            });
        } else {
            if self.workspaces.is_empty() {
                self.workspaces = rows;
            }
            eprintln!(
                "persistence_ui: snapshot anterior preservado; catálogo inconclusivo: {issue_summary}"
            );
            self.workspaces_warning = Some(
                "Não consegui confirmar todos os Espaços conhecidos. Mantive a última versão completa."
                    .to_owned(),
            );
        }
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

    /// T7 + F2-1-4: cicla o modo (sistema → escuro → claro → sistema), persiste e **aplica ao
    /// vivo** (todas as janelas re-pintam no próximo frame, sem restart).
    pub fn toggle_theme(&mut self) {
        self.settings.theme = match self.settings.mode_setting() {
            theme::ModeSetting::Sistema => "escuro",
            theme::ModeSetting::Escuro => "claro",
            theme::ModeSetting::Claro => "sistema",
        }
        .into();
        self.persist_settings();
        self.apply_theme();
    }

    /// F1-2-1: escolhe um dos 8 acentos curados, persiste e aplica ao vivo.
    pub fn set_accent(&mut self, name: &str) {
        self.settings.accent = name.to_string();
        self.persist_settings();
        self.apply_theme();
    }

    /// Aplica o tema dos ajustes correntes como tema VIVO do processo — pela PREFERÊNCIA
    /// (F2-1-4): `theme::apply_setting` resolve `"sistema"` contra a aparência viva do SO.
    fn apply_theme(&self) {
        theme::apply_setting(self.settings.mode_setting(), &self.settings.accent);
    }

    /// Onde o export/import de tema mora (`tema.json`, ao lado do `settings.json` — local-first).
    fn theme_file(&self) -> PathBuf {
        self.settings_dir.join("tema.json")
    }

    /// F1-2-1 (T7§A Avançado): exporta o tema corrente como JSON legível, 100% local — grava a
    /// PREFERÊNCIA (`"sistema"` sai como `"sistema"`) + ajustes vivos.
    pub fn export_theme(&mut self) {
        let json = theme::export_json(self.settings.mode_setting(), &self.settings.accent);
        let path = self.theme_file();
        self.theme_io = Some(match std::fs::write(&path, json) {
            Ok(()) => format!("Tema exportado em {}", path.display()),
            // T7§A estados de erro: falha de disco é VISÍVEL, nunca silenciosa.
            Err(e) => format!("Não consegui exportar o tema ({e})"),
        });
    }

    /// F1-2-1 + F2-1-4 (T7§A Avançado): importa `tema.json` COMPLETO — preferência de modo
    /// (incl. `"sistema"`), acento, reduzir-animações e ajustes por token — aplica ao vivo e
    /// persiste. Arquivo inválido → mensagem leiga; campos de versão futura são ignorados;
    /// seções ausentes não mexem no que o usuário já tinha (aplica o que entende), EXCETO
    /// `ajustes`: presente-ou-ausente é o estado inteiro do refinamento (import atômico).
    pub fn import_theme(&mut self) {
        let path = self.theme_file();
        let parsed = std::fs::read_to_string(&path)
            .map_err(|_| format!("Não achei um tema para importar em {}", path.display()))
            .and_then(|json| theme::parse_prefs(&json));
        self.theme_io = Some(match parsed {
            Ok(prefs) => {
                self.settings.theme = theme::ModeSetting::from_setting(&prefs.modo)
                    .as_setting()
                    .to_string();
                self.settings.accent = prefs.acento_normalizado();
                if let Some(mov) = prefs.movimento {
                    self.settings.reduce_motion = mov.reduzir;
                    theme::set_reduce_motion(mov.reduzir);
                }
                self.settings.theme_overrides = prefs.ajustes.unwrap_or_default();
                theme::set_overrides(self.settings.theme_overrides.clone());
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

    /// T7 + F2-1-3: alterna "reduzir animações", persiste E aplica no tema vivo (o ajuste agora
    /// está FIADO: `MotionTokens::effective` zera as durações de todo consumidor de movimento).
    pub fn toggle_reduce_motion(&mut self) {
        self.settings.reduce_motion = !self.settings.reduce_motion;
        theme::set_reduce_motion(self.settings.reduce_motion);
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

/// Pasta-base a varrer por Espaços (T6): `LINA_WORKSPACES_DIR` ou o pai da raiz
/// real do Espaço vivo (`<root>/.lina/events` não pode virar `<root>/.lina`).
fn workspaces_base(ws_dir: &Path) -> PathBuf {
    std::env::var_os("LINA_WORKSPACES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::workspace_boot::ws_root_of_events_dir(ws_dir)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| crate::workspace_boot::ws_root_of_events_dir(ws_dir))
        })
}

/// Única porta de troca da janela de Ajustes. A callback pertence ao canvas e
/// delega ao mesmo runtime/journal da sidebar; `None` torna a lista read-only.
pub type WorkspaceSwitchCallback = Rc<dyn Fn(PathBuf, &mut Window, &mut Context<PersistenceView>)>;

/// Abre a janela do painel (chamada de dentro do `application().run` de `main.rs`).
pub fn open_window(
    cx: &mut App,
    shared: Model,
    ws_dir: PathBuf,
    on_switch: Option<WorkspaceSwitchCallback>,
) {
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
        |window, cx| cx.new(|cx| PersistenceView::new(shared, ws_dir, base, on_switch, window, cx)),
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
    on_switch: Option<WorkspaceSwitchCallback>,
}

fn request_workspace_switch(target: &WorkspaceEntry, mut dispatch: impl FnMut(PathBuf)) -> bool {
    if target.focused {
        return false;
    }
    dispatch(crate::workspace_boot::ws_root_of_events_dir(
        &target.events_dir,
    ));
    true
}

impl PersistenceView {
    fn new(
        shared: Model,
        ws_dir: PathBuf,
        base: PathBuf,
        on_switch: Option<WorkspaceSwitchCallback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            model: PersistenceModel::new(shared, ws_dir, base),
            focus,
            on_switch,
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
        let label: String = label.into();
        // Consolidado no Button do catálogo (F2-2-1): ganha role(Button)+aria de graça (a11y que
        // o helper inline não tinha), mantendo o par (bg,fg) WCAG-coberto que o call-site escolhe.
        Button::new(id, label)
            .variant(ButtonVariant::Custom(bg, fg))
            .on_click(cx.listener(move |view, _ev: &ClickEvent, window, cx| {
                on_click(view, window, cx);
            }))
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
                .rounded_chrome()
                .bg(rgb(th.state.warning))
                .text_color(rgb(th.text.on_emphasis))
                .child(text!("⟳ Recuperando seu trabalho…")),
            RecoveryStatus::Failed { .. } => div()
                .flex()
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .rounded_chrome()
                .bg(rgb(th.surface.panel))
                .border_2()
                .border_color(rgb(th.state.warning))
                .child(
                    div()
                        .text_color(rgb(th.state.warning))
                        .font_weight(FontWeight(f32::from(th.typography.weight.bold)))
                        .child(text!("Não consegui verificar seus dados")),
                )
                .child(
                    div()
                        .text_color(rgb(th.text.muted))
                        .child(text!("Tente reverificar; seus arquivos foram preservados.")),
                ),
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
                    .rounded_chrome()
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
        if let Some(warning) = self.model.workspaces_warning() {
            list = list.child(
                div()
                    .text_color(rgb(th.state.warning))
                    .child(text!(format!("⚠ {warning}"))),
            );
        }
        for ws in self.model.workspaces() {
            // id estável e único por Espaço (a11y: evita colisão de id no `text!` do laço).
            let name = ws.name.clone();
            let switch_target = ws.clone();
            let on_switch = self.on_switch.clone();
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
                .rounded_content()
                .bg(rgb(th.surface.panel))
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
            let row = if let Some(on_switch) = on_switch.filter(|_| !ws.focused) {
                row.cursor_pointer().on_click(cx.listener(
                    move |_view, _event: &ClickEvent, window, cx| {
                        request_workspace_switch(&switch_target, |root| {
                            on_switch(root, window, cx);
                        });
                    },
                ))
            } else {
                row
            };
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

    /// F1-2-1 · T7§A + F2-1-4 — Aparência: modo (sistema/escuro/claro, troca AO VIVO) + 8
    /// acentos curados + exportar/importar tema (JSON local). Persiste em `settings.json`.
    fn appearance_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let th = theme::active();
        let s = self.model.settings();
        // Pastilhas dos 8 acentos curados: pintam no modo RESOLVIDO vivo (com "sistema", é o
        // modo que o SO ditou); a ativa ganha o anel de foco (`focus.ring`, gate ≥3:1).
        let mode = th.mode;
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
                        // Rótulo LEIGO pela preferência (zero jargão): "sistema" vira uma frase.
                        format!(
                            "{} (trocar)",
                            match s.mode_setting() {
                                theme::ModeSetting::Sistema => "segue o computador",
                                theme::ModeSetting::Escuro => "escuro",
                                theme::ModeSetting::Claro => "claro",
                            }
                        ),
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
        let key = self.model.plan_key().input.clone();
        let placeholder = format!("{} (clique para colar)", lic::COPY_KEY_PLACEHOLDER);
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
                // F2-2-1: campo simulado consolidado no Input (paste_only) — o clique cola do
                // clipboard (o gesto leigo "copiei do e-mail → colo aqui"). Ganha aria_label; cor da
                // borda/texto/placeholder vêm do tema por dentro (placeholder apagado, valor primário).
                Input::new("plan-key-field", key)
                    .placeholder(placeholder)
                    .paste_only()
                    .aria("Chave do Lina PRO — clique para colar")
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, cx| {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            view.model.plan_paste(&text);
                        }
                        cx.notify();
                    })),
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
                            .rounded_chrome()
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

        // Doutrina de overflow (M1): esta janela é fixa (560×680, redimensionável), então o
        // conteúdo raiz segue as 3 regiões — header FIXO · corpo ROLÁVEL · (sem footer). O ✕ é o
        // botão de fechar da titlebar do SO (não há ✕ in-content). Sem isto, as seções
        // (Espaços + Plano + Aparência + Ajustes) cresciam além da janela e o fim ficava
        // inacessível (sem scroll). A linha que destrava o scroll no flex-col é `min_h(px(0.))`.
        // Largura/margem da coluna ficam num ÚNICO wrapper (não duplicadas no header e no corpo).
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
                    // Coluna que preenche a altura; `min_h(0)` deixa o corpo rolar em vez de estourar.
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .gap_6()
                    .w(px(500.0))
                    .mt(px(40.0))
                    .ml(px(30.0))
                    // ── HEADER FIXO: título + selo "salvo"; nunca rola (sempre visível). ──
                    .child(
                        div()
                            .flex_shrink_0()
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
                    // ── CORPO ROLÁVEL: o excedente das seções rola aqui dentro, não na janela. ──
                    .child(
                        div()
                            .id("persistence-body")
                            .flex_1()
                            .min_h(px(0.))
                            .w_full()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(self.recovery_banner(cx))
                            .child(self.workspaces_list(cx))
                            .child(self.plan_section(cx))
                            .child(self.appearance_panel(cx))
                            .child(self.settings_panel(cx)),
                    ),
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
        store
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: uuid::Uuid::now_v7().to_string(),
            })
            .expect("workspace id");
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
        let mut model = PersistenceModel::with_registry_path(
            shared.clone(),
            events.clone(),
            tmp.path().to_path_buf(),
            Some(tmp.path().join("workspaces.json")),
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

        let scan = list_workspaces(base.path(), &live).expect("varredura completa");
        let entries = scan.entries;
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

    #[test]
    fn workspace_reliability_complete_creation_staging_is_hidden_from_catalog() {
        let base = TempDir::new("hidden-complete-staging");
        let live_root = base.path().join("live");
        let staging_root = base
            .path()
            .join(format!(".lina-create-{}", uuid::Uuid::now_v7()));
        drop(Workspace::create(&live_root, "Vivo", "blank", None).expect("live"));
        drop(Workspace::create(&staging_root, "Preparação", "blank", None).expect("staging"));

        let scan = list_workspaces(base.path(), &Workspace::events_dir(&live_root))
            .expect("scan continua disponível");

        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].name, "Vivo");
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.message.contains("preparação")));
        let report = crate::workspace_boot::recover_pending_workspace_creations(
            base.path(),
            &base.path().join("workspaces.json"),
        )
        .expect("orphan é reportado sem bloquear catálogo saudável");
        assert_eq!(report.issues.len(), 1);
        assert!(staging_root.exists(), "sem journal, recovery não apaga");
    }

    /// Uma falha de enumeração não pode virar `Ok([])`: isso faria o caller
    /// apagar uma lista válida. Arquivo regular como `base` injeta `NotADirectory`
    /// de forma portável, sem depender de permissões do usuário do teste.
    #[test]
    fn workspace_reliability_scan_read_error_is_not_an_empty_success() {
        let tmp = TempDir::new("scan-read-error");
        let live = seed_workspace(&tmp.path().join("ws-live"), "App Vivo", "T1");
        let not_a_directory = tmp.path().join("base-invalida");
        std::fs::write(&not_a_directory, b"nao e diretorio").expect("fixture");

        let error = list_workspaces(&not_a_directory, &live)
            .expect_err("read_dir precisa continuar sendo erro observável");
        assert_eq!(error.kind, WorkspaceScanIssueKind::ReadFailure);
        assert!(
            error.message.contains("base-invalida"),
            "o diagnóstico identifica a base que falhou: {error}"
        );
    }

    #[test]
    fn workspace_reliability_missing_identity_or_name_is_always_invalid_domain_state() {
        let tmp = TempDir::new("missing-facts-context");
        let missing_id = Workspace::events_dir(&tmp.path().join("missing-id"));
        EventStore::open(&missing_id)
            .expect("missing-id store")
            .append(&DomainEvent::WorkspaceCreated {
                name: "Conhecido".to_owned(),
                focus_preset: "blank".to_owned(),
            })
            .expect("created fact");
        let missing_name = Workspace::events_dir(&tmp.path().join("missing-name"));
        EventStore::open(&missing_name)
            .expect("missing-name store")
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: "known-id".to_owned(),
            })
            .expect("id fact");

        for events_dir in [&missing_id, &missing_name] {
            let known = project_entry(events_dir.clone(), false, true, true)
                .expect_err("fato obrigatório ausente é inválido mesmo se conhecido");
            assert_eq!(known.kind, WorkspaceScanIssueKind::InvalidWorkspace);
            let unknown = project_entry(events_dir.clone(), false, true, false)
                .expect_err("fato ausente novo é inválido determinístico");
            assert_eq!(unknown.kind, WorkspaceScanIssueKind::InvalidWorkspace);
        }
    }

    #[test]
    fn workspace_reliability_known_partial_workspace_preserves_registry_and_warns() {
        let tmp = TempDir::new("known-partial-last-good");
        let workspaces_base = tmp.path().join("workspaces");
        let healthy_root = workspaces_base.join("healthy");
        let partial_root = workspaces_base.join("known-partial");
        drop(Workspace::create(&healthy_root, "Saudável", "blank", None).expect("healthy"));
        let partial_events = Workspace::events_dir(&partial_root);
        EventStore::open(&partial_events)
            .expect("partial store")
            .append(&DomainEvent::WorkspaceCreated {
                name: "Parcial conhecido".to_owned(),
                focus_preset: "blank".to_owned(),
            })
            .expect("created only");
        let healthy = WorkspaceRegistry::rederive_entry(&healthy_root).expect("healthy entry");
        let partial = RegistryEntry {
            id: "known-partial-id".to_owned(),
            name: "Parcial conhecido".to_owned(),
            path: partial_root.clone(),
            last_focus: 0,
            archived: false,
        };
        let registry_path = tmp.path().join("catalog/workspaces.json");
        std::fs::create_dir_all(registry_path.parent().expect("catalog dir")).expect("catalog dir");
        let last_good = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "workspaces": [healthy, partial]
        }))
        .expect("registry json");
        std::fs::write(&registry_path, &last_good).expect("last-good");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            &workspaces_base,
            &Workspace::events_dir(&healthy_root),
        )
        .expect("last-good continua publicável");

        assert!(
            catalog.scan.is_complete(),
            "invalidade determinística não bloqueia os saudáveis nem nova criação"
        );
        assert_eq!(catalog.registry.entries().len(), 2);
        assert_eq!(catalog.verified_entries.len(), 1);
        assert_eq!(catalog.verified_entries[0].path, healthy_root);
        assert!(catalog
            .verified_entries
            .iter()
            .all(|entry| entry.path != partial_root));
        assert!(
            catalog.quarantined_roots.contains(&partial_root),
            "a raiz isolada continua endereçável para uma futura ação de retomar/arquivar"
        );
        assert!(catalog.scan.issues.iter().any(|issue| issue.kind
            == WorkspaceScanIssueKind::InvalidWorkspace
            && issue.message.contains("WorkspaceIdAssigned")));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), last_good);

        let model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            Workspace::events_dir(&healthy_root),
            workspaces_base,
            Some(registry_path),
        );
        assert_eq!(model.workspaces().len(), 1, "parcial não vira row normal");
        assert_eq!(
            crate::workspace_boot::ws_root_of_events_dir(&model.workspaces()[0].events_dir),
            healthy_root
        );
        assert_eq!(
            model.workspaces_warning(),
            Some("Alguns Espaços precisam de atenção; os demais continuam disponíveis.")
        );
    }

    #[test]
    fn workspace_reliability_missing_known_base_preserves_last_good_rows_and_bytes() {
        let tmp = TempDir::new("missing-known-base");
        let workspaces_base = tmp.path().join("workspaces");
        let first_root = workspaces_base.join("first");
        let second_root = workspaces_base.join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        let second = WorkspaceRegistry::rederive_entry(&second_root).expect("second entry");
        let registry_path = tmp.path().join("catalog/workspaces.json");
        std::fs::create_dir_all(registry_path.parent().expect("catalog dir")).expect("catalog dir");
        let last_good = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "workspaces": [first, second]
        }))
        .expect("registry json");
        std::fs::write(&registry_path, &last_good).expect("last-good");
        std::fs::rename(&workspaces_base, tmp.path().join("workspaces-renamed"))
            .expect("base renamed");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            &workspaces_base,
            &Workspace::events_dir(&first_root),
        )
        .expect("last-good continua disponível");

        assert!(!catalog.scan.is_complete());
        assert_eq!(catalog.verified_entries.len(), 2);
        assert_eq!(catalog.registry.entries().len(), 2);
        assert_eq!(std::fs::read(&registry_path).expect("registry"), last_good);

        let model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            Workspace::events_dir(&first_root),
            workspaces_base,
            Some(registry_path),
        );
        let names: std::collections::BTreeSet<&str> = model
            .workspaces()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["Primeiro", "Segundo"])
        );
        assert!(model.workspaces_warning().is_some());
    }

    #[test]
    fn workspace_reliability_corrupt_registry_and_missing_base_never_publish_or_rewrite() {
        let tmp = TempDir::new("corrupt-registry-missing-base");
        let registry_path = tmp.path().join("catalog/workspaces.json");
        std::fs::create_dir_all(registry_path.parent().expect("catalog dir")).expect("catalog dir");
        let corrupt = b"{ registry ainda corrompido";
        std::fs::write(&registry_path, corrupt).expect("corrupt registry");
        let missing_base = tmp.path().join("missing-workspaces");

        let error = match load_or_rebuild_workspace_registry(
            &registry_path,
            &missing_base,
            &Workspace::events_dir(&missing_base.join("current")),
        ) {
            Ok(_) => panic!("catálogo inconclusivo bloqueia publicação"),
            Err(error) => error,
        };

        assert!(error.contains("inconclusiv"));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), corrupt);
    }

    #[test]
    fn workspace_reliability_settings_base_lists_sibling_roots() {
        let tmp = TempDir::new("settings-sibling-roots");
        let first_root = tmp.path().join("first");
        let second_root = tmp.path().join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let first_events = Workspace::events_dir(&first_root);
        let model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            first_events.clone(),
            workspaces_base(&first_events),
            Some(tmp.path().join("workspaces.json")),
        );

        let names: std::collections::BTreeSet<&str> = model
            .workspaces()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["Primeiro", "Segundo"])
        );
    }

    #[test]
    fn workspace_reliability_settings_lists_external_registry_root() {
        let tmp = TempDir::new("settings-external-root");
        let workspaces_base = tmp.path().join("managed");
        let current_root = workspaces_base.join("current");
        let external_root = tmp.path().join("external");
        drop(Workspace::create(&current_root, "Atual", "blank", None).expect("current"));
        drop(Workspace::create(&external_root, "Externo", "blank", None).expect("external"));
        let registry_path = tmp.path().join("catalog/workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(WorkspaceRegistry::rederive_entry(&current_root).expect("current entry"));
        registry.upsert(WorkspaceRegistry::rederive_entry(&external_root).expect("external entry"));
        registry.save().expect("registry save");

        let model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            Workspace::events_dir(&current_root),
            workspaces_base,
            Some(registry_path),
        );

        let roots: std::collections::BTreeSet<PathBuf> = model
            .workspaces()
            .iter()
            .map(|entry| crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir))
            .collect();
        assert_eq!(
            roots,
            std::collections::BTreeSet::from([current_root, external_root])
        );
    }

    /// O consumidor do scan mantém de fato o snapshot anterior; não basta a
    /// função baixa retornar `Err` se a camada de UI ainda o converter em vazio.
    #[test]
    fn workspace_reliability_model_keeps_last_good_after_read_error() {
        let base = TempDir::new("model-last-good");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let _other = seed_workspace(&base.path().join("ws-other"), "App Outro", "T2");
        let mut model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            live.clone(),
            base.path().to_path_buf(),
            Some(base.path().join("workspaces.json")),
        );
        let last_good = model.workspaces().to_vec();
        assert_eq!(last_good.len(), 2, "fixture começa com snapshot completo");

        let not_a_directory = base.path().join("base-invalida");
        std::fs::write(&not_a_directory, b"nao e diretorio").expect("fixture");
        model.workspaces_base = not_a_directory;
        model.refresh_workspaces();

        assert_eq!(model.workspaces(), last_good.as_slice());
        assert!(
            model.workspaces_warning().is_some(),
            "o erro preservado também chega à camada renderizável"
        );
    }

    #[test]
    fn workspace_reliability_known_store_parse_failure_keeps_last_good() {
        let base = TempDir::new("model-known-parse-last-good");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let other = seed_workspace(&base.path().join("ws-other"), "App Outro", "T2");
        let mut model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            live.clone(),
            base.path().to_path_buf(),
            Some(base.path().join("workspaces.json")),
        );
        let last_good = model.workspaces().to_vec();
        assert_eq!(last_good.len(), 2, "fixture começa completa");

        let connection = rusqlite::Connection::open(other.join("lina.db")).expect("sqlite");
        connection
            .execute(
                "UPDATE events SET payload = ?1 WHERE seq = 1",
                ["{ payload inválido"],
            )
            .expect("injetar falha determinística de parse");
        drop(connection);
        std::fs::remove_file(other.join("log.recovery.json"))
            .expect("tornar a recuperação semanticamente inconclusiva");

        model.refresh_workspaces();

        assert_eq!(
            model.workspaces(),
            last_good.as_slice(),
            "uma entrada antes válida não desaparece durante refresh inválido"
        );
        assert_eq!(
            model.workspaces_warning(),
            Some("Não consegui confirmar todos os Espaços conhecidos. Mantive a última versão completa.")
        );
    }

    #[test]
    fn workspace_reliability_known_parse_failure_preserves_real_registry_snapshot() {
        let base = TempDir::new("registry-known-parse-last-good");
        let live_root = base.path().join("live");
        let known_root = base.path().join("known");
        drop(Workspace::create(&live_root, "Vivo", "blank", None).expect("live"));
        drop(Workspace::create(&known_root, "Conhecido", "blank", None).expect("known"));

        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let mut live_entry = WorkspaceRegistry::rederive_entry(&live_root).expect("live entry");
        live_entry.last_focus = 10;
        registry.upsert(live_entry);
        registry.upsert(WorkspaceRegistry::rederive_entry(&known_root).expect("known entry"));
        registry.save().expect("last-good registry");
        let last_good_bytes = std::fs::read(&registry_path).expect("snapshot bytes");

        let known_events = Workspace::events_dir(&known_root);
        let connection = rusqlite::Connection::open(known_events.join("lina.db")).expect("sqlite");
        connection
            .execute(
                "UPDATE events SET payload = ?1 WHERE seq = 1",
                ["{ payload inválido"],
            )
            .expect("corromper payload conhecido");
        drop(connection);
        std::fs::remove_file(known_events.join("log.recovery.json"))
            .expect("impedir recovery sem certificado");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&live_root),
        )
        .expect("registry last-good continua disponível");

        assert!(
            !catalog.scan.is_complete(),
            "criação/rewrite ficam bloqueados"
        );
        assert!(!catalog.rebuilt);
        assert_eq!(catalog.registry.entries().len(), 2);
        assert_eq!(
            catalog.verified_entries.len(),
            2,
            "sidebar recebe last-good integral"
        );
        assert!(catalog.quarantined_roots.contains(&known_root));
        assert_eq!(
            crate::workspace_boot::pick_production_root(&catalog.registry, live_root.clone())
                .expect("o Espaço saudável continua selecionável"),
            live_root
        );
        assert_eq!(
            std::fs::read(&registry_path).expect("registry preservado"),
            last_good_bytes
        );
        assert_eq!(
            WorkspaceRegistry::load(&registry_path)
                .expect("registry ainda legível")
                .entries()
                .len(),
            2
        );
    }

    #[test]
    fn workspace_reliability_new_domain_invalid_store_isolated_in_real_rebuild() {
        let base = TempDir::new("registry-new-domain-invalid");
        let live_root = base.path().join("live");
        let partial_root = base.path().join("partial");
        drop(Workspace::create(&live_root, "Vivo", "blank", None).expect("live"));
        EventStore::open(Workspace::events_dir(&partial_root))
            .expect("partial store")
            .append(&DomainEvent::WorkspaceCreated {
                name: "Parcial".to_owned(),
                focus_preset: "blank".to_owned(),
            })
            .expect("partial event");
        let registry_path = base.path().join("workspaces.json");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&live_root),
        )
        .expect("invalidade de domínio nova isola só a entrada");

        assert!(catalog.scan.is_complete());
        assert!(catalog.scan.issues.iter().any(|issue| {
            issue.kind == WorkspaceScanIssueKind::InvalidWorkspace
                && issue.message.contains("WorkspaceIdAssigned")
        }));
        assert_eq!(catalog.registry.entries().len(), 1);
        assert_eq!(catalog.registry.entries()[0].path, live_root);
        assert!(catalog
            .registry
            .entries()
            .iter()
            .all(|entry| entry.path != partial_root));
    }

    #[test]
    fn workspace_reliability_pending_focus_failure_keeps_journal_and_healthy_candidate() {
        let base = TempDir::new("pending-focus-broken-target");
        let healthy_root = base.path().join("healthy");
        let broken_root = base.path().join("broken");
        drop(Workspace::create(&healthy_root, "Saudável", "blank", None).expect("healthy"));
        drop(Workspace::create(&broken_root, "Quebrado", "blank", None).expect("broken"));
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let mut healthy = WorkspaceRegistry::rederive_entry(&healthy_root).expect("healthy entry");
        healthy.last_focus = 10;
        let mut broken = WorkspaceRegistry::rederive_entry(&broken_root).expect("broken entry");
        broken.last_focus = 20;
        registry.upsert(healthy);
        registry.upsert(broken.clone());
        registry.save().expect("last-good registry");
        let pending = lina_core::PendingWorkspaceFocus {
            focus_id: uuid::Uuid::now_v7().to_string(),
            target: broken,
            focused_at_ms: 30,
        };
        registry.prepare_focus(&pending).expect("prepare focus");
        EventStore::open(Workspace::events_dir(&broken_root))
            .expect("broken store before corruption")
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: pending.target.name.clone(),
                focus_id: Some(pending.focus_id.clone()),
            })
            .expect("commit point");

        let broken_events = Workspace::events_dir(&broken_root);
        let connection =
            rusqlite::Connection::open(broken_events.join("lina.db")).expect("sqlite broken");
        connection
            .execute(
                "UPDATE events SET payload = ?1 WHERE seq = 1",
                ["{ payload inválido"],
            )
            .expect("corrupt known target");
        drop(connection);
        std::fs::remove_file(broken_events.join("log.recovery.json"))
            .expect("disable uncertified recovery");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&healthy_root),
        )
        .expect("last-good permite seguir com candidato saudável");

        assert!(!catalog.scan.is_complete());
        assert!(catalog.registry.pending_focus_path().is_file());
        assert_eq!(catalog.registry.entries().len(), 2);
        assert_eq!(
            crate::workspace_boot::pick_production_root(&catalog.registry, healthy_root.clone())
                .expect("picker pula alvo quebrado"),
            healthy_root
        );
        assert!(catalog.scan.issues.iter().any(|issue| {
            issue.kind == WorkspaceScanIssueKind::ReadFailure
                && issue.message.contains("journal foi preservado")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_reliability_model_keeps_last_good_after_entry_io_failure() {
        use std::os::unix::fs::symlink;

        let base = TempDir::new("model-entry-io-last-good");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let _other = seed_workspace(&base.path().join("ws-other"), "App Outro", "T2");
        let mut model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            live.clone(),
            base.path().to_path_buf(),
            Some(base.path().join("workspaces.json")),
        );
        let last_good = model.workspaces().to_vec();
        symlink(
            base.path().join("destino-ausente"),
            base.path().join("entrada-transitoria"),
        )
        .expect("broken symlink");

        model.refresh_workspaces();

        assert_eq!(model.workspaces(), last_good.as_slice());
        assert_eq!(
            model.workspaces_warning(),
            Some("Não consegui confirmar todos os Espaços conhecidos. Mantive a última versão completa.")
        );
    }

    #[test]
    fn workspace_reliability_invalid_registry_preserves_bytes_and_external_last_good() {
        let base = TempDir::new("registry-invalid-external-last-good");
        let workspaces_base = base.path().join("managed");
        let live_root = workspaces_base.join("ws-live");
        let external_root = base.path().join("external");
        drop(Workspace::create(&live_root, "App Vivo", "blank", None).expect("live"));
        drop(Workspace::create(&external_root, "Externo", "blank", None).expect("external"));
        let live = Workspace::events_dir(&live_root);
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(WorkspaceRegistry::rederive_entry(&live_root).expect("live entry"));
        registry.upsert(WorkspaceRegistry::rederive_entry(&external_root).expect("external entry"));
        registry.save().expect("last-good registry");
        let mut model = PersistenceModel::with_registry_path(
            shared_with(0, false),
            live.clone(),
            workspaces_base.clone(),
            Some(registry_path.clone()),
        );
        let last_good = model.workspaces().to_vec();
        assert_eq!(last_good.len(), 2, "fixture inclui ponteiro externo");
        let corrupt = b"{ registry quebrado";
        std::fs::write(&registry_path, corrupt).expect("corrupt registry");

        model.refresh_workspaces();

        assert_eq!(model.workspaces(), last_good.as_slice());
        assert!(model.workspaces().iter().any(|entry| {
            crate::workspace_boot::ws_root_of_events_dir(&entry.events_dir) == external_root
        }));
        assert_eq!(
            model.workspaces_warning(),
            Some("Não consegui atualizar a lista agora. Mantive a última versão completa.")
        );
        assert_eq!(std::fs::read(&registry_path).expect("registry"), corrupt);
        let error =
            match load_or_rebuild_workspace_registry(&registry_path, &workspaces_base, &live) {
                Ok(_) => panic!("parse inválido é inconclusivo, nunca ausência reconstruível"),
                Err(error) => error,
            };
        assert!(error.contains("registry") && error.contains("inconclusivo"));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), corrupt);
    }

    #[test]
    fn workspace_reliability_valid_stale_registry_absorbs_scan_only_store() {
        let base = TempDir::new("registry-stale-union");
        let first_root = base.path().join("first");
        let second_root = base.path().join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("empty registry");
        registry
            .upsert(WorkspaceRegistry::rederive_entry(&first_root).expect("first registry entry"));
        registry.save().expect("stale registry");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&first_root),
        )
        .expect("scan completo reconcilia registry válido");

        assert!(catalog.rebuilt, "o ponteiro stale foi regravado");
        assert_eq!(catalog.registry.entries().len(), 2);
        let names: std::collections::BTreeSet<&str> = catalog
            .registry
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["Primeiro", "Segundo"])
        );
        assert_eq!(
            WorkspaceRegistry::load(&registry_path)
                .expect("reload")
                .entries()
                .len(),
            2
        );
    }

    #[test]
    fn workspace_reliability_registry_cas_refuses_stale_rebuild_snapshot() {
        let base = TempDir::new("registry-cas-stale-snapshot");
        let first_root = base.path().join("first");
        let concurrent_root = base.path().join("concurrent");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(
            Workspace::create(&concurrent_root, "Concorrente", "blank", None).expect("concurrent"),
        );
        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        let concurrent =
            WorkspaceRegistry::rederive_entry(&concurrent_root).expect("concurrent entry");
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(first.clone());
        registry.save().expect("primeira foto");
        let stale_snapshot = std::fs::read(&registry_path).expect("snapshot stale");

        let mut concurrent_registry =
            WorkspaceRegistry::load(&registry_path).expect("concurrent registry");
        concurrent_registry.upsert(concurrent);
        concurrent_registry.save().expect("publicação concorrente");
        let concurrent_bytes = std::fs::read(&registry_path).expect("bytes concorrentes");

        let error = match WorkspaceRegistry::rebuild_from_verified_entries_if_unchanged(
            &registry_path,
            vec![first],
            Some(&stale_snapshot),
        ) {
            Ok(_) => panic!("foto stale não pode sobrescrever publicação concorrente"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            WorkspaceError::RegistryChangedDuringRebuild { .. }
        ));
        assert_eq!(
            std::fs::read(&registry_path).expect("registry final"),
            concurrent_bytes
        );
        assert_eq!(
            WorkspaceRegistry::load(&registry_path)
                .expect("registry concorrente intacto")
                .entries()
                .len(),
            2
        );
    }

    #[test]
    fn workspace_reliability_unique_pointer_mismatch_is_repaired_from_store() {
        let base = TempDir::new("registry-repair-unique-mismatch");
        let root = base.path().join("workspace");
        drop(Workspace::create(&root, "Nome real", "blank", None).expect("workspace"));
        let canonical = WorkspaceRegistry::rederive_entry(&root).expect("canonical");
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(RegistryEntry {
            id: "id-forjado".into(),
            name: "Nome stale".into(),
            path: root.clone(),
            last_focus: 42,
            archived: true,
        });
        registry.save().expect("stale registry");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&root),
        )
        .expect("mismatch único é reparável");

        assert!(catalog.rebuilt, "registry stale foi regravado");
        let repaired = catalog.registry.entries().first().expect("entry");
        assert_eq!(repaired.id, canonical.id);
        assert_eq!(repaired.name, "Nome real");
        assert!(!repaired.archived, "archive vem do store");
        assert_eq!(repaired.last_focus, 42, "foco do ponteiro é preservado");
        let persisted = WorkspaceRegistry::load(&registry_path).expect("reload");
        assert_eq!(persisted.entries(), catalog.registry.entries());
    }

    #[test]
    fn workspace_reliability_ambiguity_isolated_without_blocking_healthy_repair() {
        let base = TempDir::new("registry-mixed-ambiguity");
        let ambiguous_root = base.path().join("ambiguous");
        let healthy_root = base.path().join("healthy");
        drop(Workspace::create(&ambiguous_root, "Ambíguo", "blank", None).expect("ambiguous"));
        drop(Workspace::create(&healthy_root, "Saudável", "blank", None).expect("healthy"));
        let ambiguous =
            WorkspaceRegistry::rederive_entry(&ambiguous_root).expect("ambiguous canonical");
        let healthy = WorkspaceRegistry::rederive_entry(&healthy_root).expect("healthy canonical");
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(ambiguous);
        registry.upsert(RegistryEntry {
            id: "segundo-id".into(),
            name: "Duplicado".into(),
            path: ambiguous_root.clone(),
            last_focus: 100,
            archived: false,
        });
        registry.upsert(RegistryEntry {
            id: "healthy-stale-id".into(),
            name: "Nome stale".into(),
            path: healthy_root.clone(),
            last_focus: 50,
            archived: true,
        });
        registry.save().expect("mixed registry");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&healthy_root),
        )
        .expect("ambiguidade local não bloqueia reparo saudável");

        assert!(catalog.rebuilt);
        assert!(catalog.quarantined_roots.contains(&ambiguous_root));
        assert_eq!(catalog.verified_entries.len(), 1);
        let repaired = catalog
            .registry
            .entries()
            .first()
            .expect("healthy repaired");
        assert_eq!(repaired.id, healthy.id);
        assert_eq!(repaired.name, healthy.name);
        assert_eq!(repaired.path, healthy_root);
        assert_eq!(repaired.last_focus, 50);
        assert!(!repaired.archived);
        assert!(WorkspaceRegistry::rederive_entry(&ambiguous_root).is_ok());
        assert!(WorkspaceRegistry::rederive_entry(&healthy_root).is_ok());
        let persisted = WorkspaceRegistry::load(&registry_path).expect("reload");
        assert_eq!(persisted.entries(), catalog.registry.entries());
    }

    #[test]
    fn workspace_reliability_known_uuid_provenance_beats_new_clone() {
        let base = TempDir::new("registry-known-id-beats-clone");
        let known_root = base.path().join("known");
        let clone_root = base.path().join("clone");
        drop(Workspace::create(&known_root, "Conhecido", "blank", None).expect("known"));
        drop(Workspace::create(&clone_root, "Clone", "blank", None).expect("clone"));
        let known = WorkspaceRegistry::rederive_entry(&known_root).expect("known entry");
        EventStore::open(Workspace::events_dir(&clone_root))
            .expect("clone store")
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: known.id.clone(),
            })
            .expect("clone uuid");
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(known.clone());
        registry.save().expect("last-good");
        let last_good = std::fs::read(&registry_path).expect("last-good bytes");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&known_root),
        )
        .expect("proveniência resolve clone novo");

        assert!(!catalog.rebuilt);
        assert_eq!(catalog.registry.entries(), std::slice::from_ref(&known));
        assert_eq!(catalog.verified_entries, vec![known]);
        assert!(catalog.quarantined_roots.contains(&clone_root));
        assert!(!catalog.quarantined_roots.contains(&known_root));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), last_good);
    }

    #[test]
    fn workspace_reliability_external_known_uuid_provenance_beats_new_clone() {
        let base = TempDir::new("registry-external-known-id-beats-clone");
        let workspaces_base = base.path().join("managed");
        let known_root = base.path().join("external-known");
        let clone_root = workspaces_base.join("clone");
        drop(Workspace::create(&known_root, "Conhecido", "blank", None).expect("known"));
        drop(Workspace::create(&clone_root, "Clone", "blank", None).expect("clone"));
        let known = WorkspaceRegistry::rederive_entry(&known_root).expect("known entry");
        EventStore::open(Workspace::events_dir(&clone_root))
            .expect("clone store")
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: known.id.clone(),
            })
            .expect("clone uuid");
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(known.clone());
        registry.save().expect("last-good");
        let last_good = std::fs::read(&registry_path).expect("last-good bytes");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            &workspaces_base,
            &Workspace::events_dir(&clone_root),
        )
        .expect("proveniência externa resolve clone novo");

        assert!(!catalog.rebuilt);
        assert_eq!(catalog.registry.entries(), std::slice::from_ref(&known));
        assert_eq!(catalog.verified_entries, vec![known]);
        assert!(catalog.quarantined_roots.contains(&clone_root));
        assert!(!catalog.quarantined_roots.contains(&known_root));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), last_good);
    }

    #[test]
    fn workspace_reliability_two_known_roots_with_same_uuid_are_both_isolated_without_rewrite() {
        let base = TempDir::new("registry-two-known-same-id");
        let first_root = base.path().join("first");
        let second_root = base.path().join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        EventStore::open(Workspace::events_dir(&second_root))
            .expect("second store")
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: first.id.clone(),
            })
            .expect("duplicate uuid");
        let second = WorkspaceRegistry::rederive_entry(&second_root).expect("second entry");
        let registry_path = base.path().join("workspaces.json");
        let last_good = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "workspaces": [first, second]
        }))
        .expect("registry json");
        std::fs::write(&registry_path, &last_good).expect("ambiguous last-good");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            base.path(),
            &Workspace::events_dir(&first_root),
        )
        .expect("ambiguidade conhecida é isolada sem apagar ponteiros");

        assert!(!catalog.rebuilt);
        assert_eq!(catalog.registry.entries().len(), 2);
        assert!(catalog.verified_entries.is_empty());
        assert!(catalog.quarantined_roots.contains(&first_root));
        assert!(catalog.quarantined_roots.contains(&second_root));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), last_good);
    }

    #[test]
    fn workspace_reliability_two_external_known_roots_with_same_uuid_preserve_last_good() {
        let base = TempDir::new("registry-two-external-known-same-id");
        let workspaces_base = base.path().join("managed");
        std::fs::create_dir_all(&workspaces_base).expect("managed base");
        let first_root = base.path().join("external-first");
        let second_root = base.path().join("external-second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        EventStore::open(Workspace::events_dir(&second_root))
            .expect("second store")
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: first.id.clone(),
            })
            .expect("duplicate uuid");
        let second = WorkspaceRegistry::rederive_entry(&second_root).expect("second entry");
        let registry_path = base.path().join("workspaces.json");
        let last_good = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "workspaces": [first, second]
        }))
        .expect("registry json");
        std::fs::write(&registry_path, &last_good).expect("ambiguous last-good");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            &workspaces_base,
            &Workspace::events_dir(&workspaces_base.join("missing-current")),
        )
        .expect("ambiguidade externa conhecida preserva o last-good");

        assert!(!catalog.rebuilt);
        assert_eq!(catalog.registry.entries().len(), 2);
        assert!(catalog.verified_entries.is_empty());
        assert!(catalog.quarantined_roots.contains(&first_root));
        assert!(catalog.quarantined_roots.contains(&second_root));
        assert_eq!(std::fs::read(&registry_path).expect("registry"), last_good);
    }

    #[test]
    fn workspace_reliability_registry_pointer_outside_base_recovers_certified_jsonl() {
        let base = TempDir::new("registry-outside-base-recovery");
        let workspaces_base = base.path().join("managed");
        let current_root = workspaces_base.join("current");
        let external_root = base.path().join("external");
        drop(Workspace::create(&current_root, "Atual", "blank", None).expect("current"));
        let external =
            Workspace::create(&external_root, "Externo", "blank", None).expect("external");
        let mut external_entry = external.registry_entry().expect("external entry");
        let external_id = external_entry.id.clone();
        external_entry.last_focus = 10;
        drop(external);
        let registry_path = base.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(external_entry);
        registry.save().expect("registry with external pointer");
        let external_events = Workspace::events_dir(&external_root);
        assert!(external_events.join("log.jsonl").is_file());
        std::fs::remove_file(external_events.join("lina.db")).expect("remove external db");

        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            &workspaces_base,
            &Workspace::events_dir(&current_root),
        )
        .expect("pointer externo recuperável continua verificável");

        assert!(
            external_events.join("lina.db").is_file(),
            "DB foi reconstruído"
        );
        assert!(catalog
            .registry
            .entries()
            .iter()
            .any(|entry| entry.id == external_id && entry.path == external_root));
        assert!(
            catalog.scan.projected_states.contains_key(&external_root),
            "ponteiro externo reutiliza a projeção que o validou"
        );
        let mut mini_cache = crate::dashboard::MiniStatusCache::default();
        mini_cache.refresh_from_projections(
            std::slice::from_ref(&external_events),
            &catalog.scan.projected_states,
            &[],
            "2099-01-01",
            1,
        );
        assert!(mini_cache
            .get(&external_events)
            .is_some_and(|entry| !entry.status.unreachable));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_reliability_registry_rebuild_waits_for_complete_scan() {
        use std::os::unix::fs::symlink;

        let base = TempDir::new("registry-rebuild-read-failure");
        let live_root = base.path().join("ws-live");
        drop(Workspace::create(&live_root, "App Vivo", "blank", None).expect("live"));
        let live = Workspace::events_dir(&live_root);
        symlink(
            base.path().join("destino-ausente"),
            base.path().join("entrada-transitoria"),
        )
        .expect("broken symlink");
        let registry_path = base.path().join("workspaces.json");
        let catalog = load_or_rebuild_workspace_registry(&registry_path, base.path(), &live)
            .expect("snapshot vazio permanece disponível sem publicar arquivo");

        assert!(!catalog.scan.is_complete());
        assert!(!catalog.rebuilt);
        assert!(catalog.registry.entries().is_empty());
        assert!(!registry_path.exists(), "scan falho não cria registry");
    }

    /// Um store inválido no meio da base é isolado e advertido; ele não esconde
    /// o Espaço vivo nem outro store saudável.
    #[test]
    fn workspace_reliability_bad_workspace_is_isolated_from_healthy_ones() {
        let base = TempDir::new("scan-isolation");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let _healthy = seed_workspace(&base.path().join("ws-ok"), "App Saudável", "T2");
        let bad_events = base.path().join("ws-ruim").join("events");
        std::fs::create_dir_all(&bad_events).expect("bad dir");
        std::fs::write(bad_events.join("lina.db"), b"nao e sqlite").expect("bad db");

        let scan = list_workspaces(base.path(), &live).expect("base ainda é enumerável");
        let names: Vec<&str> = scan
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert!(names.contains(&"App Vivo"), "ativo preservado: {names:?}");
        assert!(
            names.contains(&"App Saudável"),
            "entrada ruim não esconde a saudável: {names:?}"
        );
        assert_eq!(scan.issues.len(), 1, "um aviso aponta o store isolado");
        assert_eq!(
            scan.issues[0].kind,
            WorkspaceScanIssueKind::InvalidWorkspace
        );
        assert!(
            scan.is_complete(),
            "corrupção determinística fica em quarentena"
        );
        assert!(scan.issues[0].message.contains("ws-ruim"));
    }

    #[test]
    fn workspace_reliability_io_classification_is_structured_not_textual() {
        let path = Path::new("/catalogo/workspace");
        let permission = store_projection_issue(
            path,
            "não abriu",
            StoreError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            false,
        );
        let new_invalid_data = store_projection_issue(
            path,
            "não abriu",
            StoreError::Io(std::io::Error::from(std::io::ErrorKind::InvalidData)),
            false,
        );
        let known_invalid_data = store_projection_issue(
            path,
            "não abriu",
            StoreError::Io(std::io::Error::from(std::io::ErrorKind::InvalidData)),
            true,
        );

        assert_eq!(permission.kind, WorkspaceScanIssueKind::ReadFailure);
        assert_eq!(
            new_invalid_data.kind,
            WorkspaceScanIssueKind::InvalidWorkspace
        );
        assert_eq!(known_invalid_data.kind, WorkspaceScanIssueKind::ReadFailure);
        #[cfg(unix)]
        {
            let emfile = store_projection_issue(
                path,
                "não abriu",
                StoreError::Io(std::io::Error::from_raw_os_error(libc::EMFILE)),
                false,
            );
            assert_eq!(emfile.kind, WorkspaceScanIssueKind::ReadFailure);
        }
    }

    /// Reproduz a assinatura dos TomikOS parciais: `WorkspaceCreated` existe,
    /// mas a identidade durável nunca foi atribuída. O scan não pode promovê-lo
    /// a Espaço normal nem esconder os completos ao redor.
    #[test]
    fn workspace_reliability_partial_without_workspace_id_is_isolated() {
        let base = TempDir::new("scan-partial-id");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let partial_events = base.path().join("ws-parcial").join("events");
        EventStore::open(&partial_events)
            .expect("partial store")
            .append(&DomainEvent::WorkspaceCreated {
                name: "TomikOS parcial".into(),
                focus_preset: "blank".into(),
            })
            .expect("partial WorkspaceCreated");

        let scan = list_workspaces(base.path(), &live).expect("base enumerável");
        assert!(scan.entries.iter().any(|entry| entry.name == "App Vivo"));
        assert!(
            scan.entries
                .iter()
                .all(|entry| entry.name != "TomikOS parcial"),
            "store sem identidade não vira linha normal"
        );
        assert!(
            scan.issues
                .iter()
                .any(|issue| issue.message.contains("WorkspaceIdAssigned")),
            "a quarentena explica qual fronteira de criação faltou"
        );
    }

    #[test]
    fn workspace_reliability_empty_id_and_missing_created_are_not_workspaces() {
        let base = TempDir::new("scan-invalid-completeness");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");

        let empty_id_events = base.path().join("ws-empty-id").join("events");
        let mut empty_id = EventStore::open(&empty_id_events).expect("empty-id store");
        empty_id
            .append(&DomainEvent::WorkspaceCreated {
                name: "Identidade vazia".into(),
                focus_preset: "blank".into(),
            })
            .expect("created");
        empty_id
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: String::new(),
            })
            .expect("id vazio");
        drop(empty_id);

        let missing_created_events = base.path().join("ws-no-created").join("events");
        EventStore::open(&missing_created_events)
            .expect("no-created store")
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: uuid::Uuid::now_v7().to_string(),
            })
            .expect("id sem nome");

        let scan = list_workspaces(base.path(), &live).expect("base enumerável");
        assert_eq!(
            scan.entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["App Vivo"]
        );
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.message.contains("WorkspaceIdAssigned válido")));
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.message.contains("WorkspaceCreated com nome")));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_reliability_one_broken_entry_becomes_warning_not_scan_failure() {
        use std::os::unix::fs::symlink;

        let base = TempDir::new("scan-broken-entry");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let _healthy = seed_workspace(&base.path().join("ws-ok"), "App Saudável", "T2");
        let broken = base.path().join("atalho-quebrado");
        symlink(base.path().join("destino-ausente"), &broken).expect("symlink quebrado");

        let scan = list_workspaces(base.path(), &live)
            .expect("uma entrada quebrada não invalida a enumeração");
        let names: std::collections::BTreeSet<&str> = scan
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["App Saudável", "App Vivo"])
        );
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.message.contains("atalho-quebrado")));
        assert!(
            !scan.is_complete(),
            "falha de metadata é leitura inconclusiva, não quarentena determinística"
        );
    }

    /// A varredura de recuperação também re-deriva `archived` do log. Assim,
    /// um registry ausente/novo não faz um Espaço arquivado reaparecer na lista.
    #[test]
    fn workspace_reliability_scan_never_resurrects_archived_workspace() {
        let base = TempDir::new("scan-archived");
        let live = seed_workspace(&base.path().join("ws-live"), "App Vivo", "T1");
        let archived = seed_workspace(&base.path().join("ws-archived"), "Arquivado", "T2");
        EventStore::open(&archived)
            .expect("open archived")
            .append(&DomainEvent::WorkspaceArchived)
            .expect("archive");

        let scan = list_workspaces(base.path(), &live).expect("varredura completa");
        assert!(
            scan.entries.iter().all(|entry| entry.name != "Arquivado"),
            "o fato archived vem do store e precisa sobreviver sem registry"
        );
        assert!(scan.entries.iter().any(|entry| entry.name == "App Vivo"));
    }

    const RESTART_SOAK_CHILD: &str = "LINA_WORKSPACE_RESTART_SOAK_CHILD";

    fn restart_soak_roots(home: &Path) -> [PathBuf; 3] {
        let base = home.join("espacos");
        [base.join("alfa"), base.join("beta"), base.join("gama")]
    }

    fn restart_soak_shared() -> crate::runtime::SharedInfra {
        crate::runtime::SharedInfra {
            pty: Arc::new(Mutex::new(lina_core::PtyManager::new())),
            cmd_factory: Arc::new(|_| lina_core::PtyCommand::new("cat")),
            cols: 80,
            rows: 24,
        }
    }

    fn stop_restart_soak(runtimes: &crate::runtime::Runtimes) {
        crate::runtime::stop_all(runtimes);
        for runtime in lock(runtimes).map.values() {
            runtime.nodes.shutdown_terminals_after_failed_boot();
        }
    }

    fn run_restart_soak_child(phase: &str) {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME isolado"));
        let roots = restart_soak_roots(&home);
        match phase {
            "seed" => {
                for (root, name) in roots.iter().zip(["Alfa", "Beta", "Gama"]) {
                    drop(
                        lina_core::Workspace::create(root, name, "blank", None)
                            .expect("workspace completo"),
                    );
                }
                let runtime = crate::runtime::boot_ws_runtime(
                    roots[0].clone(),
                    &restart_soak_shared(),
                    false,
                )
                .expect("boot inicial real");
                crate::runtime::register_ready_runtime(&runtime)
                    .expect("publicar primeiro runtime pronto");
                let runtimes = crate::runtime::runtimes_with_boot(runtime);
                stop_restart_soak(&runtimes);
            }
            "undo" => {
                let beta_events = lina_core::Workspace::events_dir(&roots[1]);
                let before = EventStore::open(&beta_events)
                    .expect("beta")
                    .event_count()
                    .expect("count beta");
                let mut pending = Some(crate::sidebar::ArchiveToast::new(
                    roots[1].clone(),
                    "Beta".to_owned(),
                    lina_core::now_ms(),
                    false,
                ));
                assert_eq!(pending.as_ref().expect("toast vivo").root, roots[1]);
                let undone = crate::sidebar::undo_archive_toast(&mut pending)
                    .expect("o mesmo seam da UI remove a intenção");
                assert_eq!(undone.root, roots[1]);
                assert!(pending.is_none());
                assert_eq!(
                    EventStore::open(&beta_events)
                        .expect("beta depois do Desfazer")
                        .event_count()
                        .expect("count beta depois"),
                    before,
                    "Desfazer não pode gravar fato escondido"
                );
            }
            "archive" => {
                let registry_path = lina_core::default_registry_path().expect("registry isolado");
                let registry =
                    lina_core::WorkspaceRegistry::load(&registry_path).expect("registry");
                assert_ne!(
                    registry.focused().map(|entry| entry.path.as_path()),
                    Some(roots[1].as_path()),
                    "a UI proíbe arquivar o Espaço ativo"
                );
                let toast = crate::sidebar::ArchiveToast::new(
                    roots[1].clone(),
                    "Beta".to_owned(),
                    lina_core::now_ms(),
                    false,
                );
                crate::commit_archive_toast(&toast, Some(&registry_path), None)
                    .expect("o mesmo commit do timeout arquiva Beta");
            }
            "reject-archived" => {
                let registry_path = lina_core::default_registry_path().expect("registry isolado");
                let registry =
                    lina_core::WorkspaceRegistry::load(&registry_path).expect("registry");
                let active = registry.focused().expect("foco durável").path.clone();
                let shared = restart_soak_shared();
                let runtime = crate::runtime::boot_ws_runtime(active.clone(), &shared, false)
                    .expect("boot depois do archive");
                let runtimes = crate::runtime::runtimes_with_boot(runtime);
                let error =
                    crate::runtime::activate_workspace(&runtimes, roots[1].clone(), &shared)
                        .expect_err("arquivado não pode ser reativado");
                assert!(error.contains("arquivado"), "erro visível: {error}");
                assert_eq!(lock(&runtimes).active, active);
                stop_restart_soak(&runtimes);
            }
            phase if phase.starts_with("switch:") => {
                let target_index = phase
                    .split_once(':')
                    .and_then(|(_, index)| index.parse::<usize>().ok())
                    .filter(|index| *index < roots.len())
                    .expect("target do switch");
                let registry_path = lina_core::default_registry_path().expect("registry isolado");
                let registry =
                    lina_core::WorkspaceRegistry::load(&registry_path).expect("registry");
                let active = registry.focused().expect("foco durável").path.clone();
                let shared = restart_soak_shared();
                let runtime = crate::runtime::boot_ws_runtime(active, &shared, false)
                    .expect("boot real no novo processo");
                let runtimes = crate::runtime::runtimes_with_boot(runtime);
                crate::runtime::activate_workspace(&runtimes, roots[target_index].clone(), &shared)
                    .expect("troca real no novo processo");
                assert_eq!(lock(&runtimes).active, roots[target_index]);
                stop_restart_soak(&runtimes);
            }
            _ => panic!("fase desconhecida do restart soak: {phase}"),
        }
    }

    fn spawn_restart_soak_phase(home: &Path, phase: &str) {
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args([
                "--exact",
                "persistence_ui::tests::workspace_reliability_switch_archive_restart_soak",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(RESTART_SOAK_CHILD, phase)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("LINA_NO_RESTORE", "1")
            .env_remove("LINA_WS_ROOT")
            .env_remove("LINA_DEMO")
            .status()
            .expect("subprocesso do restart soak");
        assert!(status.success(), "fase {phase} falhou: {status}");
    }

    fn assert_restart_soak_catalog(home: &Path, expected: &[&str]) {
        let roots = restart_soak_roots(home);
        let registry_path = home.join(".lina").join("workspaces.json");
        let registry =
            lina_core::WorkspaceRegistry::load(&registry_path).expect("registry reaberto");
        let active = registry.focused().expect("um foco durável").path.clone();
        let catalog = load_or_rebuild_workspace_registry(
            &registry_path,
            &home.join("espacos"),
            &lina_core::Workspace::events_dir(&active),
        )
        .expect("catálogo reaberto");
        assert!(catalog.scan.issues.is_empty(), "catálogo íntegro");

        let positions: std::collections::BTreeMap<(String, PathBuf), usize> = catalog
            .registry
            .entries()
            .iter()
            .enumerate()
            .map(|(position, entry)| ((entry.id.clone(), entry.path.clone()), position))
            .collect();
        let verified: Vec<crate::sidebar::VerifiedRegistryEntry> = catalog
            .verified_entries
            .iter()
            .cloned()
            .map(|entry| crate::sidebar::VerifiedRegistryEntry {
                shortcut_index: positions
                    .get(&(entry.id.clone(), entry.path.clone()))
                    .and_then(|position| (*position < 9).then_some(*position + 1)),
                entry,
            })
            .collect();
        let event_dirs: Vec<PathBuf> = verified
            .iter()
            .map(|entry| crate::sidebar::events_dir_of(&entry.entry.path))
            .collect();
        let mut cache = crate::dashboard::MiniStatusCache::default();
        cache.refresh_from_projections(
            &event_dirs,
            &catalog.scan.projected_states,
            &[],
            "2026-07-11",
            1,
        );
        let rows = crate::sidebar::build_rows(&verified, &catalog.scan.entries, &active, &cache);
        let row_names: std::collections::BTreeSet<&str> =
            rows.iter().map(|row| row.name.as_str()).collect();
        let expected_names: std::collections::BTreeSet<&str> = expected.iter().copied().collect();
        assert_eq!(row_names, expected_names, "sidebar e catálogo convergem");
        assert_eq!(rows.iter().filter(|row| row.focused).count(), 1);
        assert!(rows.iter().any(|row| row.focused && row.ws_root == active));
        assert!(roots.iter().any(|root| root == &active));
    }

    /// Gate H: cada troca ocorre pelo runtime real dentro de um processo novo com o
    /// mesmo HOME isolado. Desfazer e confirmar atravessam os mesmos seams chamados
    /// pela UI; um restart seguinte recusa o alvo sem apagar Alfa/Gama.
    #[test]
    fn workspace_reliability_switch_archive_restart_soak() {
        if let Some(phase) = std::env::var_os(RESTART_SOAK_CHILD) {
            run_restart_soak_child(&phase.to_string_lossy());
            return;
        }

        let home = TempDir::new("switch-archive-restart-home");
        spawn_restart_soak_phase(home.path(), "seed");
        assert_restart_soak_catalog(home.path(), &["Alfa", "Beta", "Gama"]);

        for turn in 0..10 {
            spawn_restart_soak_phase(home.path(), &format!("switch:{}", (turn + 1) % 3));
            assert_restart_soak_catalog(home.path(), &["Alfa", "Beta", "Gama"]);
        }

        // Beta ficou ativa no último ciclo; uma troca real a tira de foco antes do gesto.
        spawn_restart_soak_phase(home.path(), "switch:2");
        spawn_restart_soak_phase(home.path(), "undo");
        assert_restart_soak_catalog(home.path(), &["Alfa", "Beta", "Gama"]);
        spawn_restart_soak_phase(home.path(), "archive");
        assert_restart_soak_catalog(home.path(), &["Alfa", "Gama"]);

        for turn in 0..10 {
            let target = if turn % 2 == 0 { 0 } else { 2 };
            spawn_restart_soak_phase(home.path(), &format!("switch:{target}"));
            assert_restart_soak_catalog(home.path(), &["Alfa", "Gama"]);
        }
        spawn_restart_soak_phase(home.path(), "reject-archived");
        assert_restart_soak_catalog(home.path(), &["Alfa", "Gama"]);
    }

    #[test]
    fn workspace_reliability_settings_switch_dispatches_once_without_touching_old_store() {
        let tmp = TempDir::new("settings-switch-callback");
        let old_events = seed_workspace(&tmp.path().join("old"), "Antigo", "T1");
        let target_events = seed_workspace(&tmp.path().join("target"), "Alvo", "T2");
        let old_count = EventStore::open(&old_events)
            .expect("old store")
            .event_count()
            .expect("old count");
        let target = WorkspaceEntry {
            name: "Alvo".to_owned(),
            workspace_id: "target-id".to_owned(),
            events_dir: target_events,
            focused: false,
            archived: false,
            nodes: 1,
        };
        let mut requested = Vec::new();

        assert!(request_workspace_switch(&target, |root| requested.push(root)));
        assert_eq!(requested, vec![tmp.path().join("target")]);
        let old = EventStore::open(&old_events).expect("old reopen");
        assert_eq!(old.event_count().expect("old count after"), old_count);
        assert!(old
            .events()
            .expect("old events")
            .iter()
            .all(|record| record.kind != "WorkspaceFocusSet"));
    }

    /// T7: alterar um ajuste persiste em settings.json (persiste após reabrir).
    #[test]
    fn settings_persist_across_reopen() {
        let _guard = crate::theme::THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new("settings");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let shared = shared_with(0, false);
        {
            let mut model = PersistenceModel::with_registry_path(
                shared,
                events.clone(),
                tmp.path().to_path_buf(),
                Some(tmp.path().join("workspaces.json")),
            );
            assert_eq!(
                model.settings().theme,
                "sistema",
                "default do produto: segue o sistema (F2-0-D nº2)"
            );
            model.toggle_theme(); // sistema → escuro
            model.toggle_reduce_motion(); // false → true (persistido E fiado no tema vivo)
            assert!(
                theme::active().motion.reduce_motion,
                "toggle fiou o flag vivo (F2-1-3)"
            );
            model.set_default_cwd("/tmp/projetos");
        }
        // "reabrir": relê do disco.
        let reloaded = load_settings(&events);
        assert_eq!(reloaded.theme, "escuro");
        assert!(reloaded.reduce_motion);
        assert_eq!(reloaded.default_cwd, "/tmp/projetos");
        // não vaza estado global.
        theme::set_reduce_motion(false);
        theme::apply(theme::Mode::Dark, theme::DEFAULT_ACCENT);
    }

    /// **F2-1-4 — migração única `escuro`→`sistema`:** settings pré-F2 com o default de fábrica
    /// "escuro" (sem o marcador) migram UMA vez para "sistema" (decisão F2-0-D nº2) e o marcador
    /// é gravado; quem re-escolher "escuro" DEPOIS fica em "escuro". "claro" (escolha explícita
    /// de quem fugiu do default) nunca é tocado.
    #[test]
    fn legacy_escuro_migrates_to_sistema_once() {
        let tmp = TempDir::new("migra");
        let dir = tmp.path().join("events");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("settings.json"),
            r#"{"theme":"escuro","reduce_motion":false,"default_cwd":""}"#,
        )
        .expect("settings pré-F2");

        let s = load_settings(&dir);
        assert_eq!(s.theme, "sistema", "default de fábrica migrou");
        assert!(s.theme_migrated_to_system, "marcador gravado");
        // o arquivo no disco já carrega a migração (re-load não depende da 1ª leitura).
        assert_eq!(load_settings(&dir).theme, "sistema");

        // escolha explícita PÓS-migração é respeitada para sempre.
        let mut explicit = s;
        explicit.theme = "escuro".into();
        save_settings(&dir, &explicit);
        assert_eq!(
            load_settings(&dir).theme,
            "escuro",
            "escuro explícito não re-migra"
        );

        // "claro" pré-F2 era escolha explícita → intocado pela migração.
        std::fs::write(
            dir.join("settings.json"),
            r#"{"theme":"claro","reduce_motion":false,"default_cwd":""}"#,
        )
        .expect("settings claro pré-F2");
        let s = load_settings(&dir);
        assert_eq!(s.theme, "claro", "claro explícito preservado");
        assert!(s.theme_migrated_to_system);
    }

    /// **F2-1-4 — import completo:** `tema.json` com modo + acento + reduzir + ajustes aplica
    /// TUDO ao vivo e persiste (incl. `theme_overrides` no settings.json — fonte única entre
    /// sessões). Guard serializa (toca o tema global).
    #[test]
    fn import_applies_reduzir_and_ajustes_and_persists() {
        let _guard = crate::theme::THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new("import-f214");
        let events = seed_workspace(tmp.path(), "App", "T1");
        let shared = shared_with(0, false);
        let mut model = PersistenceModel::with_registry_path(
            shared,
            events.clone(),
            tmp.path().to_path_buf(),
            Some(tmp.path().join("workspaces.json")),
        );

        std::fs::write(
            events.join("tema.json"),
            r##"{
              "modo": "claro",
              "acento": "rosa",
              "movimento": { "reduzir": true },
              "ajustes": { "superficie.canvas": "#101010" }
            }"##,
        )
        .expect("tema.json completo");
        model.import_theme();
        assert_eq!(
            model.theme_io_message(),
            Some("Tema importado e aplicado ✓")
        );
        assert_eq!(model.settings().theme, "claro");
        assert_eq!(model.settings().accent, "rosa");
        assert!(model.settings().reduce_motion, "reduzir importado");
        assert!(
            theme::active().motion.reduce_motion,
            "reduzir aplicado ao vivo"
        );
        assert_eq!(
            theme::active().surface.canvas,
            0x101010,
            "ajuste por token aplicado ao vivo"
        );
        // persistiu entre sessões (settings.json é a fonte única dos ajustes).
        let reloaded = load_settings(&events);
        assert_eq!(
            reloaded.theme_overrides.get("superficie.canvas"),
            Some(&"#101010".to_string())
        );

        // não vaza estado global.
        theme::set_overrides(BTreeMap::new());
        theme::set_reduce_motion(false);
        theme::apply_setting(theme::ModeSetting::Escuro, theme::DEFAULT_ACCENT);
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
        let shared = shared_with(0, false);
        {
            let mut model = PersistenceModel::with_registry_path(
                shared,
                events.clone(),
                tmp.path().to_path_buf(),
                Some(tmp.path().join("workspaces.json")),
            );
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
        let shared = shared_with(0, false);
        {
            let mut model = PersistenceModel::with_registry_path(
                shared,
                events.clone(),
                tmp.path().to_path_buf(),
                Some(tmp.path().join("workspaces.json")),
            );
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
        theme::apply(reloaded.mode_setting().resolve(true), &reloaded.accent);
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
        let shared = shared_with(0, false);
        let mut model = PersistenceModel::with_registry_path(
            shared,
            events.clone(),
            tmp.path().to_path_buf(),
            Some(tmp.path().join("workspaces.json")),
        );

        model.toggle_theme(); // sistema → escuro
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
        model.toggle_theme(); // claro → sistema
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

    #[test]
    fn workspace_reliability_present_recovery_checks_current_store() {
        let tmp = TempDir::new("present-current-corruption");
        let events = tmp.path().join("events");
        std::fs::create_dir_all(&events).expect("events dir");
        std::fs::write(events.join("lina.db"), b"sqlite corrompido").expect("bad db");

        assert!(matches!(
            present_recovery(&events, false),
            RecoveryStatus::Failed { ref detail } if !detail.is_empty()
        ));
    }

    #[test]
    fn workspace_reliability_failed_recovery_is_never_reported_as_intact() {
        let tmp = TempDir::new("recovery-unavailable");
        let events = tmp.path().join("events");
        std::fs::create_dir_all(&events).expect("events dir");
        std::fs::write(events.join("lina.db"), b"sqlite irrecuperavel").expect("bad db");

        let status = run_recovery(&events);

        assert!(matches!(
            status,
            RecoveryStatus::Failed { ref detail } if !detail.is_empty()
        ));
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

        let shared = shared_with(0, false);
        let mut model = PersistenceModel::with_registry_path(
            shared,
            events.clone(),
            tmp.path().to_path_buf(),
            Some(tmp.path().join("workspaces.json")),
        );
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
