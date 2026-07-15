//! **F1-4-1 (fiação app) — o boot multi-Espaço.** Duas pontas PURAS e testáveis headless:
//! (1) [`pick_production_root`] decide QUAL Espaço abrir (o focado do `WorkspaceRegistry`
//! global `~/.lina/workspaces.json`, validando também qualquer fallback); (2)
//! [`register_boot_workspace`] inscreve o Espaço aberto no registry e
//! carimba o foco — é assim que o registry se POPULA (convergência registry×varredura,
//! spec-m8-m9 §6-C3) sem migração manual. Stores antigos sem `WorkspaceCreated` (o boot de
//! produção pré-F1-4 nunca o apendava) ganham o evento UMA vez (migração aditiva, inv#4:
//! acrescenta o fato que falta, nunca reescreve história).

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use lina_core::{
    can_create_workspace, DomainEvent, EventStore, LicenseTier, ProjectedState, Workspace,
    WorkspaceEntry, WorkspaceRegistry,
};
use lina_host::HeadlessUiHost;
use lina_role_discovery::RoleRegistry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::gallery;

const CREATION_NAMESPACE_PREFIX: &str = ".lina-create-";

#[must_use]
pub(crate) fn is_reserved_creation_path(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .starts_with(CREATION_NAMESPACE_PREFIX)
    })
}

/// Decide a raiz do Espaço a abrir em PRODUÇÃO. Tanto o focado quanto o fallback
/// precisam provar completude, estado ativo e identidade coincidente com qualquer
/// ponteiro existente; nunca devolve um path apenas porque ele era o default.
pub fn pick_production_root(
    registry: &WorkspaceRegistry,
    default_root: PathBuf,
) -> Result<PathBuf, String> {
    let mut candidates = registry
        .entries()
        .iter()
        .filter(|entry| !entry.archived)
        .collect::<Vec<_>>();
    let mut id_counts = std::collections::BTreeMap::<String, usize>::new();
    let mut path_counts = std::collections::BTreeMap::<PathBuf, usize>::new();
    for entry in registry.entries() {
        *id_counts.entry(entry.id.clone()).or_default() += 1;
        *path_counts.entry(entry.path.clone()).or_default() += 1;
    }
    candidates.sort_by(|left, right| {
        right
            .last_focus
            .cmp(&left.last_focus)
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut attempted_paths = std::collections::BTreeSet::new();
    let mut failures = Vec::new();
    for candidate in candidates {
        attempted_paths.insert(candidate.path.clone());
        if id_counts.get(&candidate.id).copied().unwrap_or_default() > 1
            || path_counts
                .get(&candidate.path)
                .copied()
                .unwrap_or_default()
                > 1
        {
            failures.push(format!(
                "{}: identidade ou caminho ambíguo no registry",
                candidate.path.display()
            ));
            continue;
        }
        match validate_pointer_target(candidate) {
            Ok(root) => return Ok(root),
            Err(error) => failures.push(format!("{}: {error}", candidate.path.display())),
        }
    }

    if !attempted_paths.contains(&default_root) {
        match validate_production_root(registry, &default_root) {
            Ok(root) => return Ok(root),
            Err(error) => failures.push(format!("{}: {error}", default_root.display())),
        }
    } else {
        failures.push(format!(
            "o fallback aponta para a mesma pasta já rejeitada ({})",
            default_root.display()
        ));
    }

    Err(format!(
        "nenhum Espaço íntegro pôde ser escolhido: {}",
        failures.join(" | ")
    ))
}

/// Seleciona a raiz a partir da foto que o catálogo acabou de projetar e validar.
/// Não reabre stores: em snapshot completo, repetir a derivação criaria uma
/// segunda foto e faria o custo crescer com o histórico.
pub fn pick_verified_production_root(
    verified_entries: &[WorkspaceEntry],
) -> Result<PathBuf, String> {
    if let Some(entry) = verified_entries
        .iter()
        .find(|entry| is_reserved_creation_path(&entry.path))
    {
        return Err(format!(
            "{} é uma preparação de criação, não um Espaço publicável",
            entry.path.display()
        ));
    }
    let mut id_counts = std::collections::BTreeMap::<&str, usize>::new();
    let mut path_counts = std::collections::BTreeMap::<&Path, usize>::new();
    for entry in verified_entries {
        *id_counts.entry(entry.id.as_str()).or_default() += 1;
        *path_counts.entry(entry.path.as_path()).or_default() += 1;
    }
    if let Some(entry) = verified_entries.iter().find(|entry| {
        id_counts
            .get(entry.id.as_str())
            .copied()
            .unwrap_or_default()
            > 1
            || path_counts
                .get(entry.path.as_path())
                .copied()
                .unwrap_or_default()
                > 1
    }) {
        return Err(format!(
            "{} tem identidade ou caminho ambíguo na foto verificada",
            entry.path.display()
        ));
    }
    verified_entries
        .iter()
        .filter(|entry| !entry.archived)
        .max_by(|left, right| {
            left.last_focus
                .cmp(&right.last_focus)
                .then_with(|| right.id.cmp(&left.id))
                .then_with(|| right.path.cmp(&left.path))
        })
        .map(|entry| entry.path.clone())
        .ok_or_else(|| "a foto verificada não contém Espaço ativo".to_owned())
}

/// Valida um override de produção sem promover store incompleto nem arquivado.
pub fn validate_unregistered_production_root(root: &Path) -> Result<PathBuf, String> {
    let derived = recoverable_workspace_entry(root)?;
    if derived.archived {
        return Err(format!(
            "{} está arquivado; restaure-o antes de abrir",
            root.display()
        ));
    }
    Ok(root.to_path_buf())
}

pub fn validate_production_root(
    registry: &WorkspaceRegistry,
    default_root: &Path,
) -> Result<PathBuf, String> {
    let pointers: Vec<_> = registry
        .entries()
        .iter()
        .filter(|entry| entry.path == default_root)
        .collect();
    match pointers.as_slice() {
        [] => validate_unregistered_production_root(default_root),
        [pointer] => validate_pointer_target(pointer),
        _ => Err(format!(
            "{} tem mais de um ponteiro no registry",
            default_root.display()
        )),
    }
}

fn validate_pointer_target(pointer: &WorkspaceEntry) -> Result<PathBuf, String> {
    let derived = recoverable_workspace_entry(&pointer.path)?;
    if derived.id != pointer.id {
        return Err(format!(
            "a identidade de {} diverge do ponteiro (store={}, registry={})",
            pointer.path.display(),
            derived.id,
            pointer.id
        ));
    }
    if derived.archived {
        return Err(format!(
            "{} está arquivado; restaure-o antes de abrir",
            pointer.path.display()
        ));
    }
    Ok(pointer.path.clone())
}

/// Re-deriva um alvo completo e, somente quando existe espelho JSONL, permite ao
/// core reconstruir um DB ausente/corrompido antes do boot. A projeção recuperada
/// ainda precisa provar nome e identidade; recuperar bytes nunca autoriza um
/// ponteiro com ID divergente nem ressuscita um Espaço arquivado.
fn recoverable_workspace_entry(root: &Path) -> Result<WorkspaceEntry, String> {
    if is_reserved_creation_path(root) {
        return Err(format!(
            "{} é uma preparação de criação, não um Espaço publicável",
            root.display()
        ));
    }
    WorkspaceRegistry::rederive_entry(root).map_err(|error| error.to_string())
}

fn open_projected_store(events_dir: &Path) -> Result<(EventStore, ProjectedState), String> {
    let mut host = HeadlessUiHost::new();
    EventStore::open_projected_or_recover(events_dir, &mut host).map_err(|error| {
        format!(
            "não consegui abrir/projetar {}: {error}",
            events_dir.display()
        )
    })
}

/// Primeiro boot explícito: só o chamador que já provou catálogo vazio usa este
/// funil. Publica um Workspace completo e seu ponteiro antes de permitir o runtime.
pub fn create_initial_workspace(
    default_root: &Path,
    registry: &mut WorkspaceRegistry,
) -> Result<PathBuf, String> {
    create_initial_workspace_with_checkpoint(default_root, registry, |_| Ok(()))
}

fn create_initial_workspace_with_checkpoint(
    default_root: &Path,
    registry: &mut WorkspaceRegistry,
    mut checkpoint: impl FnMut(CreationCheckpoint) -> Result<(), String>,
) -> Result<PathBuf, String> {
    if !registry.entries().is_empty() {
        return Err("o registry não está vazio; criação inicial recusada".to_owned());
    }
    let workspace_entry = match default_root.try_exists() {
        Ok(false) => {
            let parent = default_root.parent().ok_or_else(|| {
                format!(
                    "{} não tem uma pasta-pai válida para a criação inicial",
                    default_root.display()
                )
            })?;
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("não consegui preparar {}: {error}", parent.display()))?;
            let _lock = acquire_creation_parent_lock(parent)?;
            registry
                .merge_from_disk()
                .map_err(|error| format!("não consegui atualizar o catálogo inicial: {error}"))?;
            if !registry.entries().is_empty() {
                return Err(
                    "outro processo publicou um Espaço enquanto o primeiro era preparado"
                        .to_owned(),
                );
            }

            let requested_name = "Meu Espaço".to_owned();
            let seed = CreationSeedIntent::Preset {
                id: gallery::FocusPreset::Blank.id().to_owned(),
            };
            let matching: Vec<PendingCreationIntent> = pending_creation_intents(parent)?
                .into_iter()
                .filter(|intent| intent.final_root == default_root)
                .collect();
            let intent = match matching.as_slice() {
                [] => PendingCreationIntent {
                    version: CREATION_INTENT_VERSION,
                    creation_id: Uuid::now_v7().to_string(),
                    requested_name: requested_name.clone(),
                    final_name: requested_name.clone(),
                    seed: seed.clone(),
                    canonical_cwd: None,
                    final_root: default_root.to_path_buf(),
                },
                [intent]
                    if intent.matches_request(&requested_name, &seed, None)
                        && intent.final_name == requested_name =>
                {
                    intent.clone()
                }
                [intent] => {
                    return Err(format!(
                        "a criação inicial pendente {} pertence a outros dados",
                        intent.creation_id
                    ));
                }
                _ => {
                    return Err(format!(
                        "há {} criações iniciais pendentes para {}",
                        matching.len(),
                        default_root.display()
                    ));
                }
            };
            persist_creation_intent(parent, &intent)?;
            checkpoint(CreationCheckpoint::IntentPersisted)?;
            let creation_id = Uuid::parse_str(&intent.creation_id)
                .map_err(|error| format!("creation_id inicial inválido: {error}"))?;
            create_workspace_with_checkpoint_locked(
                parent,
                &requested_name,
                &gallery::FocusPreset::Blank,
                None,
                registry,
                LicenseTier::Pro,
                None,
                creation_id,
                checkpoint,
            )?;
            WorkspaceRegistry::rederive_entry(default_root).map_err(|error| {
                format!("o primeiro Espaço não projetou sua identidade: {error}")
            })?
        }
        Ok(true) => {
            let entry = WorkspaceRegistry::rederive_entry(default_root).map_err(|error| {
                format!(
                    "{} já existe, mas não é um Espaço completo que eu possa retomar ({error})",
                    default_root.display()
                )
            })?;
            if entry.archived {
                return Err(format!(
                    "{} está arquivado; restaure-o antes de abrir",
                    default_root.display()
                ));
            }
            if let Some(parent) = default_root.parent() {
                let intent_path = creation_intent_path(parent, &entry.id);
                if intent_path.exists() {
                    let intent = read_creation_intent(&intent_path, parent)?;
                    let expected_seed = CreationSeedIntent::Preset {
                        id: gallery::FocusPreset::Blank.id().to_owned(),
                    };
                    if intent.final_root != default_root
                        || intent.final_name != "Meu Espaço"
                        || !intent.matches_request("Meu Espaço", &expected_seed, None)
                    {
                        return Err(format!(
                            "{} não corresponde ao primeiro Espaço publicado",
                            intent_path.display()
                        ));
                    }
                    finish_creation_intent(parent, &entry.id);
                }
            }
            entry
        }
        Err(error) => {
            return Err(format!(
                "não consegui verificar {}: {error}",
                default_root.display()
            ));
        }
    };
    let mut entry = workspace_entry;
    entry.last_focus = 1;
    registry.upsert(entry);
    registry
        .save()
        .map_err(|error| format!("o primeiro Espaço não entrou no registry: {error}"))?;
    Ok(default_root.to_path_buf())
}

/// Inscreve o Espaço recém-aberto no registry global e carimba o foco em `now_ms`.
/// Store sem `WorkspaceCreated` (geração pré-F1-4) ganha o evento com o nome do
/// diretório — migração aditiva, idempotente (na próxima abertura o nome já projeta).
/// Devolve o `id` canônico do Espaço no registry.
///
/// # Errors
/// Propaga falha de projeção/append/save como string acionável (o chamador loga e
/// segue — registrar-se no ponteiro é best-effort, nunca derruba o boot).
#[cfg(test)]
pub fn register_boot_workspace(
    store: &mut EventStore,
    ws_root: &Path,
    registry: &mut WorkspaceRegistry,
    now_ms: u64,
) -> Result<String, String> {
    let id = upsert_registry_entry(store, ws_root, registry)?;
    registry.set_focus(&id, now_ms);
    registry.save().map_err(|e| e.to_string())?;
    Ok(id)
}

/// A inscrição em si, SEM carimbo de foco nem save — o miolo compartilhado entre o boot
/// ([`register_boot_workspace`], que foca) e a criação de fundo ([`create_workspace`],
/// que NÃO foca: o carimbo é do switch chamador). Única casa da lógica de id.
/// Stores realmente legados (sem nome, ou com o marcador de migração `focus_preset=""`)
/// recebem uma identidade; criações modernas interrompidas não são promovidas.
#[cfg(test)]
fn upsert_registry_entry(
    store: &mut EventStore,
    ws_root: &Path,
    registry: &mut WorkspaceRegistry,
) -> Result<String, String> {
    let state = store.project().map_err(|e| e.to_string())?;
    upsert_registry_entry_from_state(store, state, ws_root, registry)
}

fn upsert_registry_entry_from_state(
    store: &mut EventStore,
    mut state: ProjectedState,
    ws_root: &Path,
    registry: &mut WorkspaceRegistry,
) -> Result<String, String> {
    let migrated_legacy_name = state.workspace_name.is_none();
    if migrated_legacy_name {
        let name = ws_root
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Meu Espaço".to_string());
        store
            .append(&DomainEvent::WorkspaceCreated {
                name,
                focus_preset: String::new(),
            })
            .map_err(|e| e.to_string())?;
        state = store.project().map_err(|e| e.to_string())?;
    }
    if state.workspace_id.as_deref().is_none_or(str::is_empty) {
        let legacy_marker = store
            .events()
            .map_err(|error| error.to_string())?
            .iter()
            .any(|record| {
                record.kind == "WorkspaceCreated"
                    && record
                        .payload
                        .get("focus_preset")
                        .and_then(serde_json::Value::as_str)
                        == Some("")
            });
        if !migrated_legacy_name && !legacy_marker {
            return Err(format!(
                "{} contém uma criação incompleta; faltou WorkspaceIdAssigned",
                ws_root.display()
            ));
        }
        store
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: Uuid::now_v7().to_string(),
            })
            .map_err(|error| error.to_string())?;
        state = store.project().map_err(|error| error.to_string())?;
    }
    let name = state
        .workspace_name
        .ok_or_else(|| "WorkspaceCreated apendado mas o nome não projetou".to_string())?;
    let id = state
        .workspace_id
        .ok_or_else(|| "WorkspaceIdAssigned apendado mas a identidade não projetou".to_string())?;
    registry.upsert(WorkspaceEntry {
        id: id.clone(),
        name,
        path: ws_root.to_path_buf(),
        last_focus: 0,
        archived: state.archived,
    });
    Ok(id)
}

// ───────────────────────── M9 — criação de Espaço (M8-T3) ─────────────────────────

/// Erro leigo do Diretório de Trabalho (spec-m8-m9 §2, strings congeladas). Cada
/// variante É a mensagem que a UI mostra — sem tradução intermediária.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkdirError {
    /// É arquivo, drive ausente ou caminho morto.
    NotAccessible,
    /// Existe, é diretório, mas o teste REAL de escrita falhou.
    NoWritePermission,
    /// `create_dir_all` falhou na hora de criar (disco cheio, permissão tardia).
    CreateFailed,
}

impl std::fmt::Display for WorkdirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WorkdirError::NotAccessible => {
                "Essa pasta não está acessível — escolha outra ou crie uma nova."
            }
            WorkdirError::NoWritePermission => "Não consigo trabalhar nessa pasta.",
            WorkdirError::CreateFailed => "Não consegui criar a pasta do Espaço.",
        })
    }
}

impl std::error::Error for WorkdirError {}

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct WorkdirOperationCounts {
    exists: usize,
    canonicalize: usize,
    open: usize,
    write: usize,
    sync: usize,
    remove: usize,
    create: usize,
}

#[cfg(test)]
thread_local! {
    static WORKDIR_OPERATION_COUNTS: std::cell::Cell<WorkdirOperationCounts> =
        const { std::cell::Cell::new(WorkdirOperationCounts {
            exists: 0,
            canonicalize: 0,
            open: 0,
            write: 0,
            sync: 0,
            remove: 0,
            create: 0,
        }) };
}

#[cfg(test)]
fn record_workdir_operation(update: impl FnOnce(&mut WorkdirOperationCounts)) {
    WORKDIR_OPERATION_COUNTS.with(|counts| {
        let mut next = counts.get();
        update(&mut next);
        counts.set(next);
    });
}

#[cfg(test)]
fn reset_workdir_operation_counts() {
    WORKDIR_OPERATION_COUNTS.with(|counts| counts.set(WorkdirOperationCounts::default()));
}

#[cfg(test)]
fn workdir_operation_counts() -> WorkdirOperationCounts {
    WORKDIR_OPERATION_COUNTS.with(std::cell::Cell::get)
}

fn workdir_exists(path: &Path) -> bool {
    #[cfg(test)]
    record_workdir_operation(|counts| counts.exists += 1);
    path.exists()
}

fn workdir_canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    #[cfg(test)]
    record_workdir_operation(|counts| counts.canonicalize += 1);
    path.canonicalize()
}

fn create_workdir(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    record_workdir_operation(|counts| counts.create += 1);
    std::fs::create_dir_all(path)
}

/// Valida o Diretório de Trabalho do M9 **cedo** (na seleção e na criação — nunca só no
/// fim): se o caminho existe, canonicaliza, exige diretório e prova escrita DE VERDADE
/// (cria + remove um arquivo temporário; `access(2)` mente em rede/ACL). Caminho que NÃO
/// existe não é erro — o caminho feliz o cria ([`create_workspace`]).
///
/// # Errors
/// [`WorkdirError`] já com a string leiga congelada da spec §2.
// Aguardando o WIRING do modal M9 (fatia de UI) — exercida pelos testes até lá,
// mesmo padrão de `gallery.rs`. Remover o allow ao wirar.
#[allow(dead_code)]
pub fn validate_workdir(path: &Path) -> Result<PathBuf, WorkdirError> {
    if !workdir_exists(path) {
        return Ok(path.to_path_buf());
    }
    let canonical = workdir_canonicalize(path).map_err(|_| WorkdirError::NotAccessible)?;
    if !canonical.is_dir() {
        return Err(WorkdirError::NotAccessible);
    }
    validate_existing_workdir_with(
        canonical,
        Uuid::now_v7(),
        |probe| probe.write_all(b"ok"),
        |probe_path| std::fs::remove_file(probe_path),
    )
}

fn validate_existing_workdir_with(
    canonical: PathBuf,
    probe_id: Uuid,
    write_probe: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
    remove_probe: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, WorkdirError> {
    let probe_path = canonical.join(format!(".lina-prova-escrita-{probe_id}"));
    #[cfg(test)]
    record_workdir_operation(|counts| counts.open += 1);
    let mut probe = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .map_err(|_| WorkdirError::NoWritePermission)?;
    #[cfg(test)]
    record_workdir_operation(|counts| counts.write += 1);
    let write_result = write_probe(&mut probe).and_then(|()| {
        #[cfg(test)]
        record_workdir_operation(|counts| counts.sync += 1);
        probe.sync_all()
    });
    drop(probe);
    #[cfg(test)]
    record_workdir_operation(|counts| counts.remove += 1);
    if write_result.is_err() {
        remove_probe(&probe_path).map_err(|_| WorkdirError::NoWritePermission)?;
        return Err(WorkdirError::NoWritePermission);
    }
    remove_probe(&probe_path).map_err(|_| WorkdirError::NoWritePermission)?;
    Ok(canonical)
}

/// Materializa um Diretório de Trabalho ausente e aplica a mesma prova de raiz
/// usada pelo modal. `create_dir_all` percorre somente os componentes do caminho;
/// nenhuma entrada dentro do projeto é enumerada.
fn ensure_workdir(path: &Path) -> Result<PathBuf, WorkdirError> {
    if !workdir_exists(path) {
        create_workdir(path).map_err(|_| WorkdirError::CreateFailed)?;
    }
    validate_workdir(path)
}

/// Nome de pasta derivado do nome do Espaço: preserva o nome humano (o leigo reconhece
/// a pasta no Finder) e só neutraliza separadores/reservados de caminho.
fn folder_slug(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '-',
            other => other,
        })
        .collect();
    if cleaned.is_empty() {
        "Meu Espaço".to_string()
    } else {
        cleaned
    }
}

/// Candidatos do padrão M6 (« (2)», « (3)»…), em ordem. A mesma sequência
/// serve tanto para escolher um nome novo quanto para localizar uma criação publicada
/// antes de o registry ser salvo.
fn workspace_name_candidates(name: &str) -> Vec<String> {
    let base = name.trim();
    let base = if base.is_empty() { "Meu Espaço" } else { base };
    let mut candidates = Vec::with_capacity(100);
    candidates.push(base.to_string());
    candidates.extend((2..100).map(|i| format!("{base} ({i})")));
    candidates.push(format!("{base} (∞)"));
    candidates
}

/// Dedup do padrão M6 (« (2)», « (3)»…): o nome final não pode colidir nem com uma
/// PASTA já existente em `parent` nem com um NOME já inscrito no registry.
fn unique_workspace_name(parent: &Path, name: &str, registry: &WorkspaceRegistry) -> String {
    let taken = |candidate: &str| {
        parent.join(folder_slug(candidate)).exists()
            || registry
                .entries()
                .iter()
                .any(|e| e.name.trim() == candidate)
    };
    workspace_name_candidates(name)
        .into_iter()
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| "Meu Espaço (∞)".to_string())
}

const CREATION_INTENT_VERSION: u32 = 1;
const CREATION_INTENT_SUFFIX: &str = ".intent.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CreationSeedIntent {
    Preset { id: String },
    Template { slug: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingCreationIntent {
    version: u32,
    creation_id: String,
    requested_name: String,
    final_name: String,
    seed: CreationSeedIntent,
    canonical_cwd: Option<String>,
    final_root: PathBuf,
}

impl PendingCreationIntent {
    fn matches_request(
        &self,
        requested_name: &str,
        seed: &CreationSeedIntent,
        canonical_cwd: Option<&str>,
    ) -> bool {
        self.requested_name == requested_name
            && &self.seed == seed
            && self.canonical_cwd.as_deref() == canonical_cwd
    }
}

fn normalized_requested_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "Meu Espaço".to_owned()
    } else {
        name.to_owned()
    }
}

fn creation_seed_intent(
    preset: gallery::FocusPreset,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
) -> CreationSeedIntent {
    template.map_or_else(
        || CreationSeedIntent::Preset {
            id: preset.id().to_owned(),
        },
        |template| CreationSeedIntent::Template {
            slug: template.slug.clone(),
        },
    )
}

fn resolve_creation_seed(
    seed: &CreationSeedIntent,
) -> Result<
    (
        gallery::FocusPreset,
        Option<lina_core::workspace_template::WorkspaceTemplate>,
    ),
    String,
> {
    match seed {
        CreationSeedIntent::Preset { id } => gallery::FocusPreset::all()
            .into_iter()
            .find(|preset| preset.id() == id)
            .map(|preset| (preset, None))
            .ok_or_else(|| format!("a intenção usa um Foco desconhecido: {id}")),
        CreationSeedIntent::Template { slug } => gallery::CreateSpaceModal::templates()
            .into_iter()
            .find(|template| template.slug == *slug)
            .map(|template| (gallery::FocusPreset::Blank, Some(template)))
            .ok_or_else(|| format!("a intenção usa um gabarito desconhecido: {slug}")),
    }
}

fn canonical_future_path(path: &Path) -> Result<PathBuf, String> {
    if workdir_exists(path) {
        return workdir_canonicalize(path).map_err(|_| WorkdirError::NotAccessible.to_string());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("Não consegui resolver o Diretório de Trabalho: {error}"))?
            .join(path)
    };
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !workdir_exists(existing) {
        let name = existing
            .file_name()
            .ok_or_else(|| WorkdirError::NotAccessible.to_string())?;
        missing.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| WorkdirError::NotAccessible.to_string())?;
    }
    let mut canonical =
        workdir_canonicalize(existing).map_err(|_| WorkdirError::NotAccessible.to_string())?;
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

fn canonical_creation_cwd(default_cwd: Option<&Path>) -> Result<Option<String>, String> {
    default_cwd
        .map(canonical_future_path)
        .transpose()?
        .map(|path| {
            path.to_str().map(str::to_owned).ok_or_else(|| {
                format!(
                    "Diretório de Trabalho com caminho não-UTF8: {}",
                    path.display()
                )
            })
        })
        .transpose()
}

fn creation_intent_path(parent: &Path, creation_id: &str) -> PathBuf {
    parent.join(format!(
        "{CREATION_NAMESPACE_PREFIX}{creation_id}{CREATION_INTENT_SUFFIX}"
    ))
}

fn creation_staging_path(parent: &Path, creation_id: &str) -> PathBuf {
    parent.join(format!("{CREATION_NAMESPACE_PREFIX}{creation_id}"))
}

fn validate_creation_intent(
    intent_path: &Path,
    parent: &Path,
    intent: PendingCreationIntent,
) -> Result<PendingCreationIntent, String> {
    if intent.version != CREATION_INTENT_VERSION {
        return Err(format!(
            "{} usa versão de intenção desconhecida ({})",
            intent_path.display(),
            intent.version
        ));
    }
    Uuid::parse_str(&intent.creation_id).map_err(|error| {
        format!(
            "{} contém creation_id inválido: {error}",
            intent_path.display()
        )
    })?;
    if intent_path != creation_intent_path(parent, &intent.creation_id) {
        return Err(format!(
            "{} não corresponde ao creation_id declarado",
            intent_path.display()
        ));
    }
    if intent.final_name.trim().is_empty()
        || intent.final_root.parent() != Some(parent)
        || is_reserved_creation_path(&intent.final_root)
    {
        return Err(format!(
            "{} aponta para um alvo de publicação inválido",
            intent_path.display()
        ));
    }
    if intent.canonical_cwd.as_deref().is_some_and(str::is_empty) {
        return Err(format!(
            "{} contém Diretório de Trabalho vazio",
            intent_path.display()
        ));
    }
    resolve_creation_seed(&intent.seed)?;
    Ok(intent)
}

fn read_creation_intent(path: &Path, parent: &Path) -> Result<PendingCreationIntent, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Não consegui ler {}: {error}", path.display()))?;
    let intent: PendingCreationIntent = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Intenção de criação inválida em {}: {error}",
            path.display()
        )
    })?;
    validate_creation_intent(path, parent, intent)
}

fn pending_creation_intents(parent: &Path) -> Result<Vec<PendingCreationIntent>, String> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Não consegui procurar criações pendentes em {}: {error}",
                parent.display()
            ));
        }
    };
    let mut intents = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Não consegui ler uma criação pendente em {}: {error}",
                parent.display()
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(CREATION_NAMESPACE_PREFIX) && name.ends_with(CREATION_INTENT_SUFFIX) {
            intents.push(read_creation_intent(&entry.path(), parent)?);
        }
    }
    intents.sort_by(|left, right| left.creation_id.cmp(&right.creation_id));
    Ok(intents)
}

#[cfg(unix)]
fn sync_creation_parent(parent: &Path) -> std::io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_creation_parent(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

fn persist_creation_intent(parent: &Path, intent: &PendingCreationIntent) -> Result<(), String> {
    persist_creation_intent_with_link(parent, intent, |temporary, path| {
        std::fs::hard_link(temporary, path)
    })
}

fn persist_creation_intent_with_link(
    parent: &Path,
    intent: &PendingCreationIntent,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let path = creation_intent_path(parent, &intent.creation_id);
    if path.exists() {
        let existing = read_creation_intent(&path, parent)?;
        return (existing == *intent)
            .then_some(())
            .ok_or_else(|| format!("{} pertence a outra intenção", path.display()));
    }
    let json = serde_json::to_vec_pretty(intent)
        .map_err(|error| format!("Não consegui serializar a intenção: {error}"))?;
    let temporary = parent.join(format!(
        "{CREATION_NAMESPACE_PREFIX}intent-tmp-{}",
        Uuid::now_v7()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Não consegui preparar {}: {error}", temporary.display()))?;
    if let Err(error) = file.write_all(&json).and_then(|()| file.sync_all()) {
        drop(file);
        let cleanup = std::fs::remove_file(&temporary);
        return Err(match cleanup {
            Ok(()) => format!("Não consegui persistir a intenção: {error}"),
            Err(cleanup_error) => format!(
                "Não consegui persistir a intenção: {error}; cleanup de {} falhou: {cleanup_error}",
                temporary.display()
            ),
        });
    }
    drop(file);
    let publication = match link(&temporary, &path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            read_creation_intent(&path, parent).and_then(|existing| {
                (existing == *intent)
                    .then_some(())
                    .ok_or_else(|| format!("{} pertence a outra intenção", path.display()))
            })
        }
        Err(error) => Err(format!("Não consegui publicar a intenção: {error}")),
    };
    let cleanup = std::fs::remove_file(&temporary);
    match (publication, cleanup) {
        (Err(error), Ok(())) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(format!(
                "{error}; cleanup de {} falhou: {cleanup_error}",
                temporary.display()
            ));
        }
        (Ok(()), Err(error)) => {
            eprintln!(
                "lina-gpui: intenção persistida, mas o temporário {} não foi removido: {error}",
                temporary.display()
            );
        }
        (Ok(()), Ok(())) => {}
    }
    sync_creation_parent(parent)
        .map_err(|error| format!("Não consegui sincronizar a intenção: {error}"))
}

fn finish_creation_intent(parent: &Path, creation_id: &str) {
    let path = creation_intent_path(parent, creation_id);
    match std::fs::remove_file(&path) {
        Ok(()) => {
            if let Err(error) = sync_creation_parent(parent) {
                eprintln!(
                    "lina-gpui: criação concluída, mas fsync após remover {} falhou: {error}",
                    path.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "lina-gpui: criação concluída; journal {} será reconciliado no próximo boot: {error}",
            path.display()
        ),
    }
}

struct CreationParentLock {
    _connection: rusqlite::Connection,
}

#[allow(clippy::too_many_arguments)]
fn preflight_creation_before_parent_lock(
    parent: &Path,
    name: &str,
    preset: gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &WorkspaceRegistry,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: Uuid,
    resume_matching_request: bool,
) -> Result<(), String> {
    if creation_id.is_nil() {
        return Err("A intenção de criação precisa de uma identidade válida".to_string());
    }
    let gate_error = match can_create_workspace(tier, registry.active_count()) {
        Ok(()) => return Ok(()),
        Err(error) => error.to_string(),
    };
    match parent.try_exists() {
        Ok(false) => return Err(gate_error),
        Ok(true) => {}
        Err(error) => {
            return Err(format!(
                "Não consegui verificar a pasta de criação {}: {error}",
                parent.display()
            ));
        }
    }

    let creation_id = creation_id.to_string();
    if registry
        .entries()
        .iter()
        .any(|entry| entry.id == creation_id)
    {
        return Ok(());
    }
    let intents = pending_creation_intents(parent)?;
    if intents
        .iter()
        .any(|intent| intent.creation_id == creation_id)
    {
        return Ok(());
    }
    if resume_matching_request {
        let requested_name = normalized_requested_name(name);
        let seed = creation_seed_intent(preset, template);
        let canonical_cwd = canonical_creation_cwd(default_cwd)?;
        if intents
            .iter()
            .any(|intent| intent.matches_request(&requested_name, &seed, canonical_cwd.as_deref()))
        {
            return Ok(());
        }
    }
    Err(gate_error)
}

fn acquire_creation_parent_lock(parent: &Path) -> Result<CreationParentLock, String> {
    std::fs::create_dir_all(parent).map_err(|_| WorkdirError::CreateFailed.to_string())?;
    let path = parent.join(".lina-create-lock.sqlite3");
    let connection = rusqlite::Connection::open(&path)
        .map_err(|error| format!("Não consegui preparar a coordenação da criação: {error}"))?;
    connection
        .busy_timeout(std::time::Duration::ZERO)
        .map_err(|error| format!("Não consegui configurar a coordenação da criação: {error}"))?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS creation_lock (id INTEGER PRIMARY KEY); BEGIN IMMEDIATE;",
        )
        .map_err(|error| match &error {
            rusqlite::Error::SqliteFailure(sqlite, _)
                if matches!(
                    sqlite.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                "Já existe uma criação sendo finalizada. Tente novamente em instantes.".to_owned()
            }
            _ => format!("Não consegui coordenar a criação agora: {error}"),
        })?;
    Ok(CreationParentLock {
        _connection: connection,
    })
}

/// Fronteiras observáveis do funil de criação. A callback de produção é vazia;
/// testes injetam uma falha exata sem depender de corrida, permissão ou disco cheio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreationCheckpoint {
    IntentPersisted,
    StagingCreated,
    StoreOpened,
    WorkspaceCreated,
    ContentSeeded,
    IdentityAssigned,
    DefaultCwdAssigned,
    ProjectionValidated,
    Published,
    RegistrySaved,
}

/// Prova que o store preparado tem a sequência canônica COMPLETA antes de ele poder
/// aparecer no caminho final. Esta mesma prova protege a retomada pós-crash: existir
/// no disco não basta para ser adotado.
fn validate_complete_workspace(
    store: &EventStore,
    state: &ProjectedState,
    final_name: &str,
    preset: gallery::FocusPreset,
    default_cwd: Option<&str>,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    expected_workspace_id: Option<&str>,
) -> Result<(), String> {
    let expected_focus = template.map_or_else(|| preset.id(), |t| t.focus_preset.as_str());
    if state.workspace_name.as_deref() != Some(final_name) {
        return Err(format!(
            "Espaço preparado incompleto: nome esperado '{final_name}', veio {:?}",
            state.workspace_name
        ));
    }
    if state.focus_preset.as_deref() != Some(expected_focus) {
        return Err(format!(
            "Espaço preparado incompleto: foco esperado '{expected_focus}', veio {:?}",
            state.focus_preset
        ));
    }
    if state.default_cwd.as_deref() != default_cwd {
        return Err(format!(
            "Espaço preparado incompleto: Diretório de Trabalho esperado {default_cwd:?}, veio {:?}",
            state.default_cwd
        ));
    }
    if state.archived {
        return Err("Espaço preparado nasceu arquivado".to_string());
    }
    let workspace_id = state
        .workspace_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Espaço preparado incompleto: identidade ausente".to_string())?;
    if expected_workspace_id.is_some_and(|expected| expected != workspace_id) {
        return Err(format!(
            "A intenção de criação já aponta para outra identidade ({workspace_id})"
        ));
    }

    let records = store
        .events()
        .map_err(|e| format!("Não consegui conferir os eventos do Espaço preparado: {e}"))?;
    let first = records
        .first()
        .ok_or_else(|| "Espaço preparado incompleto: log vazio".to_string())?;
    if first.kind != "WorkspaceCreated" {
        return Err(format!(
            "Espaço preparado incompleto: primeiro evento deveria ser WorkspaceCreated, veio {}",
            first.kind
        ));
    }

    let mut cursor = 1;
    match template {
        Some(template) => {
            let expected_seed =
                lina_core::workspace_template::seed_events(template, lina_core::HUMAN_GESTURE);
            for expected_event in expected_seed {
                let record = records.get(cursor).ok_or_else(|| {
                    format!(
                        "Espaço preparado incompleto: faltou {} do gabarito",
                        expected_event.kind()
                    )
                })?;
                let expected_payload = serde_json::to_value(&expected_event)
                    .map_err(|e| format!("Não consegui conferir o gabarito: {e}"))?;
                if record.kind != expected_event.kind() || record.payload != expected_payload {
                    return Err(format!(
                        "Espaço preparado incompleto: evento {cursor} divergiu do gabarito"
                    ));
                }
                cursor += 1;
            }
            if !state.nodes.is_empty() {
                return Err(
                    "Espaço preparado inválido: gabarito criou terminal antes do gate".to_string(),
                );
            }
        }
        None => {
            let roles = RoleRegistry::with_defaults().map_err(|e| e.to_string())?;
            let expected_team = gallery::preset_team(preset, &roles);
            for agent in &expected_team {
                let added = records.get(cursor).ok_or_else(|| {
                    format!(
                        "Espaço preparado incompleto: faltou NodeAdded de {}",
                        agent.role
                    )
                })?;
                let assigned = records.get(cursor + 1).ok_or_else(|| {
                    format!(
                        "Espaço preparado incompleto: faltou NodeRoleAssigned de {}",
                        agent.role
                    )
                })?;
                if added.kind != "NodeAdded"
                    || assigned.kind != "NodeRoleAssigned"
                    || added.payload.get("node") != assigned.payload.get("node")
                    || added
                        .payload
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        != Some("Terminal")
                    || assigned
                        .payload
                        .get("role")
                        .and_then(serde_json::Value::as_str)
                        != Some(agent.role.as_str())
                {
                    return Err(format!(
                        "Espaço preparado incompleto: par de eventos do papel {} divergiu",
                        agent.role
                    ));
                }
                cursor += 2;
            }
            if state.nodes.len() != expected_team.len()
                || state
                    .nodes
                    .values()
                    .any(|node| node.kind != "Terminal" || node.role.is_none())
            {
                return Err(format!(
                    "Espaço preparado incompleto: roster projetado tem {} de {} agentes",
                    state.nodes.len(),
                    expected_team.len()
                ));
            }
        }
    }

    let identity = records
        .get(cursor)
        .ok_or_else(|| "Espaço preparado incompleto: faltou WorkspaceIdAssigned".to_string())?;
    if identity.kind != "WorkspaceIdAssigned"
        || identity
            .payload
            .get("workspace_id")
            .and_then(serde_json::Value::as_str)
            != Some(workspace_id)
    {
        return Err("Espaço preparado incompleto: identidade não fecha a sequência".to_string());
    }
    cursor += 1;

    if let Some(cwd) = default_cwd {
        let cwd_record = records.get(cursor).ok_or_else(|| {
            "Espaço preparado incompleto: faltou WorkspaceDefaultCwdSet".to_string()
        })?;
        if cwd_record.kind != "WorkspaceDefaultCwdSet"
            || cwd_record
                .payload
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                != Some(cwd)
        {
            return Err(
                "Espaço preparado incompleto: Diretório de Trabalho não fecha a sequência"
                    .to_string(),
            );
        }
        cursor += 1;
    }
    if cursor != records.len() {
        return Err(format!(
            "Espaço preparado inválido: esperava {cursor} eventos, vieram {}",
            records.len()
        ));
    }
    Ok(())
}

/// Localiza uma publicação completa da MESMA intenção. A identidade durável é
/// a única chave de retomada: coincidência de nome/foco/cwd nunca autoriza adotar
/// um Espaço criado por outra intenção humana.
#[allow(clippy::too_many_arguments)]
fn find_registered_workspace_for_creation(
    name: &str,
    preset: gallery::FocusPreset,
    default_cwd: Option<&str>,
    registry: &WorkspaceRegistry,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: &str,
) -> Result<Option<PathBuf>, String> {
    let candidates = workspace_name_candidates(name);
    let matching_entries: Vec<&WorkspaceEntry> = registry
        .entries()
        .iter()
        .filter(|entry| entry.id == creation_id)
        .collect();
    if matching_entries.len() > 1 {
        return Err(format!(
            "A intenção de criação {creation_id} aparece mais de uma vez no registry"
        ));
    }
    if let Some(entry) = matching_entries.first() {
        if !candidates.iter().any(|candidate| candidate == &entry.name) {
            return Err(format!(
                "A intenção de criação {creation_id} já pertence ao Espaço '{}'",
                entry.name
            ));
        }
        if !Workspace::events_dir(&entry.path).exists() {
            return Err(format!(
                "A intenção de criação {creation_id} aponta para uma raiz sem store: {}",
                entry.path.display()
            ));
        }
        let (store, state) = open_projected_store(&Workspace::events_dir(&entry.path))
            .map_err(|error| format!("Não consegui retomar o Espaço '{}': {error}", entry.name))?;
        validate_complete_workspace(
            &store,
            &state,
            &entry.name,
            preset,
            default_cwd,
            template,
            Some(creation_id),
        )?;
        return Ok(Some(entry.path.clone()));
    }

    Ok(None)
}

fn register_completed_workspace(
    ws_root: &Path,
    registry: &mut WorkspaceRegistry,
) -> Result<PathBuf, String> {
    let (mut store, state) = open_projected_store(&Workspace::events_dir(ws_root))?;
    upsert_registry_entry_from_state(&mut store, state, ws_root, registry)?;
    registry.save().map_err(|e| e.to_string())?;
    Ok(ws_root.to_path_buf())
}

fn remove_owned_staging(staging_root: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(staging_root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Não consegui limpar a preparação {}: {error}",
            staging_root.display()
        )),
    }
}

fn cleanup_owned_staging(staging_root: &Path, original_error: String) -> String {
    match remove_owned_staging(staging_root) {
        Ok(()) => original_error,
        Err(cleanup_error) => format!("{original_error}; {cleanup_error}"),
    }
}

/// Fecha a janela de durabilidade do `rename` no volume local. A publicação já
/// aconteceu quando isto roda; portanto falha de `fsync` é diagnóstico, nunca um
/// `Err` semântico que induziria o usuário a repetir uma criação concluída.
#[cfg(unix)]
fn sync_published_parent(parent: &Path) {
    if let Err(error) = std::fs::File::open(parent).and_then(|directory| directory.sync_all()) {
        eprintln!(
            "lina-gpui: publicação concluída, mas fsync de {} falhou: {error}",
            parent.display()
        );
    }
}

#[cfg(not(unix))]
fn sync_published_parent(_parent: &Path) {}

/// Um staging usa SOMENTE o `creation_id` como chave. Depois de um crash, a
/// mesma intenção encontra exatamente a sua preparação: se completa, publica;
/// se parcial, limpa e refaz. Nunca varre nem apaga staging de outra intenção.
fn recover_owned_staging(
    staging_root: &Path,
    final_name: &str,
    preset: gallery::FocusPreset,
    default_cwd: Option<&str>,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: &str,
) -> Result<bool, String> {
    if !staging_root.exists() {
        return Ok(false);
    }
    if !Workspace::events_dir(staging_root).exists() {
        remove_owned_staging(staging_root)?;
        return Ok(false);
    }

    let (store, state) =
        open_projected_store(&Workspace::events_dir(staging_root)).map_err(|error| {
            format!(
                "Não consegui retomar a preparação {}: {error}",
                staging_root.display()
            )
        })?;
    if state
        .workspace_id
        .as_deref()
        .is_some_and(|stored| stored != creation_id)
        || state
            .workspace_name
            .as_deref()
            .is_some_and(|stored| stored != final_name)
        || state.focus_preset.as_deref().is_some_and(|stored| {
            stored != template.map_or_else(|| preset.id(), |t| t.focus_preset.as_str())
        })
        || state
            .default_cwd
            .as_deref()
            .is_some_and(|stored| Some(stored) != default_cwd)
    {
        return Err(format!(
            "A preparação {} pertence a outra criação; não vou apagá-la",
            staging_root.display()
        ));
    }

    let complete = validate_complete_workspace(
        &store,
        &state,
        final_name,
        preset,
        default_cwd,
        template,
        Some(creation_id),
    )
    .is_ok();
    drop(store);
    if complete {
        Ok(true)
    } else {
        remove_owned_staging(staging_root)?;
        Ok(false)
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_prepared_workspace(
    staging_root: &Path,
    ws_root: &Path,
    final_name: &str,
    preset: gallery::FocusPreset,
    default_cwd: Option<&str>,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: &str,
    registry: &mut WorkspaceRegistry,
    checkpoint: &mut impl FnMut(CreationCheckpoint) -> Result<(), String>,
) -> Result<PathBuf, String> {
    if let Err(error) = std::fs::rename(staging_root, ws_root) {
        return Err(cleanup_owned_staging(
            staging_root,
            format!("Não consegui publicar o Espaço: {error}"),
        ));
    }
    if let Some(parent) = ws_root.parent() {
        sync_published_parent(parent);
    }
    checkpoint(CreationCheckpoint::Published)?;

    let (mut store, state) = open_projected_store(&Workspace::events_dir(ws_root))?;
    validate_complete_workspace(
        &store,
        &state,
        final_name,
        preset,
        default_cwd,
        template,
        Some(creation_id),
    )?;
    upsert_registry_entry_from_state(&mut store, state, ws_root, registry)?;
    registry.save().map_err(|error| error.to_string())?;
    checkpoint(CreationCheckpoint::RegistrySaved)?;
    if let Some(parent) = ws_root.parent() {
        finish_creation_intent(parent, creation_id);
    }
    Ok(ws_root.to_path_buf())
}

/// **O seam compatível de criação de Espaço (M9).** Mantém a API histórica;
/// callers que precisam repetir a mesma intenção devem usar
/// [`create_workspace_for_intent`] e conservar o `creation_id` entre tentativas.
///
/// Os `NodeId` do time são cunhados aqui (`Uuid::now_v7`, o mesmo formato do
/// `Supervisor`): não há shell vivo num Espaço de FUNDO — quem spawna os PTYs é o
/// restore na primeira abertura, a partir destes ids projetados (`plan_restore`).
///
/// # Errors
/// String acionável — gating (`LimitReached`), cwd não-UTF8, ou a string leiga congelada
/// de falha de pasta ([`WorkdirError::CreateFailed`]).
// Aguardando o WIRING do switch/M9 (fatia ii) — exercida pelos testes até lá,
// mesmo padrão de `gallery.rs`. Remover o allow ao wirar.
// F3-5-10: `template` opcional — quando `Some`, o MESMO funil gated semeia o gabarito (roster +
// params + pistas + backlog) via o core, em vez do time do Foco; quando `None`, o caminho é
// EXATAMENTE o de hoje (gate f: "sem template = criação atual inalterada").
#[allow(dead_code)]
// F3-5-10: +1 arg (template) levou de 7→8 — mesma natureza dos campos da criação (cada um é um
// dado distinto do plano de criação), agrupá-los num struct seria cerimônia sem ganho.
#[allow(clippy::too_many_arguments)]
pub fn create_workspace(
    parent: &Path,
    name: &str,
    preset: &gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &mut WorkspaceRegistry,
    _now_ms: u64,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
) -> Result<PathBuf, String> {
    create_workspace_with_checkpoint(
        parent,
        name,
        preset,
        default_cwd,
        registry,
        tier,
        template,
        Uuid::now_v7(),
        |_| Ok(()),
    )
}

/// Cria uma intenção exatamente uma vez. `creation_id` também vira a identidade
/// durável do Espaço: repetir este UUID, inclusive após reiniciar o processo, retoma
/// a raiz completa; um UUID novo com o mesmo nome continua recebendo o sufixo M6.
///
/// # Errors
/// Propaga falha de validação, preparação, publicação ou registry sem publicar
/// store parcial. UUID nulo é recusado para não criar uma chave compartilhada acidental.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn create_workspace_for_intent(
    parent: &Path,
    name: &str,
    preset: &gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &mut WorkspaceRegistry,
    _now_ms: u64,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: Uuid,
) -> Result<PathBuf, String> {
    create_workspace_with_checkpoint(
        parent,
        name,
        preset,
        default_cwd,
        registry,
        tier,
        template,
        creation_id,
        |_| Ok(()),
    )
}

/// Variante da superfície humana: conserva o `creation_id` do modal. Depois de um
/// restart, o catálogo reabsorve publicações completas; uma nova intenção de mesmo
/// nome é distinta e recebe o sufixo normal.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn create_workspace_for_user_intent(
    parent: &Path,
    name: &str,
    preset: &gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &mut WorkspaceRegistry,
    _now_ms: u64,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: Uuid,
) -> Result<PathBuf, String> {
    create_workspace_for_user_intent_with_checkpoint(
        parent,
        name,
        preset,
        default_cwd,
        registry,
        tier,
        template,
        creation_id,
        |_| Ok(()),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_workspace_for_user_intent_with_checkpoint(
    parent: &Path,
    name: &str,
    preset: &gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &mut WorkspaceRegistry,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: Uuid,
    checkpoint: impl FnMut(CreationCheckpoint) -> Result<(), String>,
) -> Result<PathBuf, String> {
    preflight_creation_before_parent_lock(
        parent,
        name,
        *preset,
        default_cwd,
        registry,
        tier,
        template,
        creation_id,
        true,
    )?;
    let _lock = acquire_creation_parent_lock(parent)?;
    let requested_name = normalized_requested_name(name);
    let canonical_cwd = canonical_creation_cwd(default_cwd)?;
    let seed = creation_seed_intent(*preset, template);
    let matching: Vec<PendingCreationIntent> = pending_creation_intents(parent)?
        .into_iter()
        .filter(|intent| intent.matches_request(&requested_name, &seed, canonical_cwd.as_deref()))
        .collect();
    let resolved_creation_id = match matching.as_slice() {
        [] => creation_id,
        [intent] => Uuid::parse_str(&intent.creation_id)
            .map_err(|error| format!("A intenção pendente tem identidade inválida: {error}"))?,
        _ => {
            return Err(format!(
                "Há {} criações pendentes com os mesmos dados. Resolva a ambiguidade antes de tentar novamente.",
                matching.len()
            ));
        }
    };
    create_workspace_with_checkpoint_locked(
        parent,
        name,
        preset,
        default_cwd,
        registry,
        tier,
        template,
        resolved_creation_id,
        checkpoint,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_workspace_with_checkpoint(
    parent: &Path,
    name: &str,
    preset: &gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &mut WorkspaceRegistry,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: Uuid,
    checkpoint: impl FnMut(CreationCheckpoint) -> Result<(), String>,
) -> Result<PathBuf, String> {
    preflight_creation_before_parent_lock(
        parent,
        name,
        *preset,
        default_cwd,
        registry,
        tier,
        template,
        creation_id,
        false,
    )?;
    let _lock = acquire_creation_parent_lock(parent)?;
    create_workspace_with_checkpoint_locked(
        parent,
        name,
        preset,
        default_cwd,
        registry,
        tier,
        template,
        creation_id,
        checkpoint,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_workspace_with_checkpoint_locked(
    parent: &Path,
    name: &str,
    preset: &gallery::FocusPreset,
    default_cwd: Option<&Path>,
    registry: &mut WorkspaceRegistry,
    tier: LicenseTier,
    template: Option<&lina_core::workspace_template::WorkspaceTemplate>,
    creation_id: Uuid,
    mut checkpoint: impl FnMut(CreationCheckpoint) -> Result<(), String>,
) -> Result<PathBuf, String> {
    if creation_id.is_nil() {
        return Err("A intenção de criação precisa de uma identidade válida".to_string());
    }
    registry.merge_from_disk().map_err(|error| {
        format!("Não consegui atualizar o catálogo sob o bloqueio de criação: {error}")
    })?;
    let requested_name = normalized_requested_name(name);
    let seed = creation_seed_intent(*preset, template);
    let creation_id = creation_id.to_string();
    let existing_intent = pending_creation_intents(parent)?
        .into_iter()
        .find(|intent| intent.creation_id == creation_id);
    let registered_retry = registry
        .entries()
        .iter()
        .any(|entry| entry.id == creation_id);
    if existing_intent.is_none() && !registered_retry {
        can_create_workspace(tier, registry.active_count()).map_err(|error| error.to_string())?;
    }
    crate::runtime::ensure_fd_headroom(8)?;
    let canonical_cwd = canonical_creation_cwd(default_cwd)?;
    let intent = match existing_intent {
        Some(intent) => {
            if !intent.matches_request(&requested_name, &seed, canonical_cwd.as_deref()) {
                return Err(format!(
                    "A intenção {creation_id} já existe com outros dados; não vou misturar criações"
                ));
            }
            intent
        }
        None => {
            if registered_retry {
                return find_registered_workspace_for_creation(
                    name,
                    *preset,
                    canonical_cwd.as_deref(),
                    registry,
                    template,
                    &creation_id,
                )?
                .ok_or_else(|| {
                    format!(
                        "A intenção de criação {creation_id} desapareceu do registry durante a retomada"
                    )
                });
            }
            let final_name = unique_workspace_name(parent, name, registry);
            let prospective_staging = creation_staging_path(parent, &creation_id);
            if prospective_staging.exists() {
                return Err(format!(
                    "{} é uma preparação sem journal explícito; ficou oculta e não será adotada",
                    prospective_staging.display()
                ));
            }
            let intent = PendingCreationIntent {
                version: CREATION_INTENT_VERSION,
                creation_id: creation_id.clone(),
                requested_name,
                final_root: parent.join(folder_slug(&final_name)),
                final_name,
                seed,
                canonical_cwd,
            };
            persist_creation_intent(parent, &intent)?;
            checkpoint(CreationCheckpoint::IntentPersisted)?;
            intent
        }
    };
    let final_name = intent.final_name.clone();
    let ws_root = intent.final_root.clone();
    let default_cwd = intent.canonical_cwd.clone();
    let default_cwd_path = default_cwd.as_deref().map(PathBuf::from);

    if ws_root.exists() {
        let (store, state) = open_projected_store(&Workspace::events_dir(&ws_root))?;
        validate_complete_workspace(
            &store,
            &state,
            &final_name,
            *preset,
            default_cwd.as_deref(),
            template,
            Some(&creation_id),
        )?;
        drop(store);
        let root = register_completed_workspace(&ws_root, registry)?;
        checkpoint(CreationCheckpoint::RegistrySaved)?;
        finish_creation_intent(parent, &creation_id);
        return Ok(root);
    }

    if let Some(path) = default_cwd_path.as_deref() {
        ensure_workdir(path).map_err(|error| error.to_string())?;
    }

    let staging_root = creation_staging_path(parent, &creation_id);
    if recover_owned_staging(
        &staging_root,
        &final_name,
        *preset,
        default_cwd.as_deref(),
        template,
        &creation_id,
    )? {
        return publish_prepared_workspace(
            &staging_root,
            &ws_root,
            &final_name,
            *preset,
            default_cwd.as_deref(),
            template,
            &creation_id,
            registry,
            &mut checkpoint,
        );
    }
    std::fs::create_dir(&staging_root).map_err(|_| WorkdirError::CreateFailed.to_string())?;
    if let Err(error) = checkpoint(CreationCheckpoint::StagingCreated) {
        return Err(cleanup_owned_staging(&staging_root, error));
    }

    let preparation = (|| -> Result<(), String> {
        let mut store =
            EventStore::open(Workspace::events_dir(&staging_root)).map_err(|e| e.to_string())?;
        checkpoint(CreationCheckpoint::StoreOpened)?;
        match template {
            Some(t) => {
                store
                    .append(&DomainEvent::WorkspaceCreated {
                        name: final_name.clone(),
                        focus_preset: t.focus_preset.clone(),
                    })
                    .map_err(|e| e.to_string())?;
                checkpoint(CreationCheckpoint::WorkspaceCreated)?;
                lina_core::workspace_template::instanciar(t, &mut store, lina_core::HUMAN_GESTURE)
                    .map_err(|e| e.to_string())?;
            }
            None => {
                let roles = RoleRegistry::with_defaults().map_err(|e| e.to_string())?;
                let placed: Vec<gallery::PlacedAgent> = gallery::preset_team(*preset, &roles)
                    .into_iter()
                    .map(|agent| gallery::PlacedAgent {
                        node: Uuid::now_v7(),
                        name: agent.name,
                        role: agent.role,
                    })
                    .collect();
                store
                    .append(&DomainEvent::WorkspaceCreated {
                        name: final_name.clone(),
                        focus_preset: preset.id().to_owned(),
                    })
                    .map_err(|e| e.to_string())?;
                checkpoint(CreationCheckpoint::WorkspaceCreated)?;
                gallery::apply_preset_roster(&placed, &mut store).map_err(|e| e.to_string())?;
            }
        }
        checkpoint(CreationCheckpoint::ContentSeeded)?;
        store
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: creation_id.clone(),
            })
            .map_err(|e| e.to_string())?;
        checkpoint(CreationCheckpoint::IdentityAssigned)?;
        if let Some(cwd) = default_cwd.as_ref() {
            store
                .append(&DomainEvent::WorkspaceDefaultCwdSet { cwd: cwd.clone() })
                .map_err(|e| e.to_string())?;
        }
        checkpoint(CreationCheckpoint::DefaultCwdAssigned)?;
        let projected = store
            .project()
            .map_err(|error| format!("Não consegui conferir o Espaço preparado: {error}"))?;
        validate_complete_workspace(
            &store,
            &projected,
            &final_name,
            *preset,
            default_cwd.as_deref(),
            template,
            Some(&creation_id),
        )?;
        checkpoint(CreationCheckpoint::ProjectionValidated)?;
        Ok(())
    })();
    if let Err(error) = preparation {
        return Err(cleanup_owned_staging(&staging_root, error));
    }

    // Publica de uma vez. Daqui em diante uma falha pode deixar uma raiz FINAL,
    // mas ela já passou pela prova completa e será retomada pelo mesmo creation_id.
    publish_prepared_workspace(
        &staging_root,
        &ws_root,
        &final_name,
        *preset,
        default_cwd.as_deref(),
        template,
        &creation_id,
        registry,
        &mut checkpoint,
    )
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CreationRecoveryReport {
    pub recovered_roots: Vec<PathBuf>,
    pub issues: Vec<String>,
}

pub(crate) fn recover_pending_workspace_creations(
    parent: &Path,
    registry_path: &Path,
) -> Result<CreationRecoveryReport, String> {
    match parent.try_exists() {
        Ok(false) => return Ok(CreationRecoveryReport::default()),
        Ok(true) => {}
        Err(error) => {
            return Err(format!(
                "Não consegui verificar preparações em {}: {error}",
                parent.display()
            ));
        }
    }
    let _lock = acquire_creation_parent_lock(parent)?;
    let intents = pending_creation_intents(parent)?;
    let mut request_counts = std::collections::BTreeMap::<String, usize>::new();
    for intent in &intents {
        let key =
            serde_json::to_string(&(&intent.requested_name, &intent.seed, &intent.canonical_cwd))
                .map_err(|error| format!("Não consegui comparar intenções pendentes: {error}"))?;
        *request_counts.entry(key).or_default() += 1;
    }
    if let Some((_, count)) = request_counts.into_iter().find(|(_, count)| *count > 1) {
        return Err(format!(
            "Há {count} criações pendentes com os mesmos dados; nenhuma foi escolhida no escuro"
        ));
    }

    let owned_staging: std::collections::BTreeSet<PathBuf> = intents
        .iter()
        .map(|intent| creation_staging_path(parent, &intent.creation_id))
        .collect();
    let mut report = CreationRecoveryReport::default();
    let entries = std::fs::read_dir(parent).map_err(|error| {
        format!(
            "Não consegui reconciliar preparações em {}: {error}",
            parent.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Não consegui ler uma preparação em {}: {error}",
                parent.display()
            )
        })?;
        let path = entry.path();
        let is_directory = entry
            .file_type()
            .map_err(|error| format!("Não consegui identificar {}: {error}", path.display()))?
            .is_dir();
        if is_directory && is_reserved_creation_path(&path) && !owned_staging.contains(&path) {
            report.issues.push(format!(
                "{} é uma preparação sem journal explícito; ficou oculta e não foi apagada",
                path.display()
            ));
        }
    }

    if intents.is_empty() {
        return Ok(report);
    }

    let mut registry = WorkspaceRegistry::load(registry_path)
        .map_err(|error| format!("Não consegui abrir o registry para recovery: {error}"))?;
    for intent in intents {
        let (preset, template) = resolve_creation_seed(&intent.seed)?;
        let default_cwd = intent.canonical_cwd.as_deref().map(PathBuf::from);
        match create_workspace_with_checkpoint_locked(
            parent,
            &intent.requested_name,
            &preset,
            default_cwd.as_deref(),
            &mut registry,
            LicenseTier::Pro,
            template.as_ref(),
            Uuid::parse_str(&intent.creation_id)
                .map_err(|error| format!("creation_id inválido no recovery: {error}"))?,
            |_| Ok(()),
        ) {
            Ok(root) => report.recovered_roots.push(root),
            Err(error) => report.issues.push(format!(
                "Não consegui retomar a intenção {}: {error}",
                intent.creation_id
            )),
        }
    }
    Ok(report)
}

/// A RAIZ do Espaço a partir do `events_dir` listado pelo T6 (`persistence_ui` aceita
/// `<root>/.lina/events`, `<root>/events` ou o próprio dir como store). Inverso determinístico
/// dos três candidatos da varredura — necessário porque o registry aponta para a RAIZ.
#[must_use]
pub fn ws_root_of_events_dir(events_dir: &Path) -> PathBuf {
    let ends_with = |p: &Path, name: &str| p.file_name().is_some_and(|f| f == name);
    if ends_with(events_dir, "events") {
        match events_dir.parent() {
            Some(lina) if ends_with(lina, ".lina") => lina
                .parent()
                .map_or_else(|| events_dir.to_path_buf(), Path::to_path_buf),
            Some(root) => root.to_path_buf(),
            None => events_dir.to_path_buf(),
        }
    } else {
        events_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lina-wsboot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("mkdir base");
        base
    }

    fn registry_at(base: &Path) -> WorkspaceRegistry {
        WorkspaceRegistry::load(base.join("workspaces.json")).expect("registry")
    }

    fn creation_checkpoint(name: &str) -> CreationCheckpoint {
        match name {
            "IntentPersisted" => CreationCheckpoint::IntentPersisted,
            "StagingCreated" => CreationCheckpoint::StagingCreated,
            "StoreOpened" => CreationCheckpoint::StoreOpened,
            "WorkspaceCreated" => CreationCheckpoint::WorkspaceCreated,
            "ContentSeeded" => CreationCheckpoint::ContentSeeded,
            "IdentityAssigned" => CreationCheckpoint::IdentityAssigned,
            "DefaultCwdAssigned" => CreationCheckpoint::DefaultCwdAssigned,
            "ProjectionValidated" => CreationCheckpoint::ProjectionValidated,
            "Published" => CreationCheckpoint::Published,
            "RegistrySaved" => CreationCheckpoint::RegistrySaved,
            other => panic!("checkpoint child desconhecido: {other}"),
        }
    }

    fn run_creation_subprocess_mode() {
        let Some(mode) = std::env::var_os("LINA_CREATION_CHILD_MODE") else {
            return;
        };
        let mode = mode.to_string_lossy().into_owned();
        let parent =
            PathBuf::from(std::env::var_os("LINA_CREATION_CHILD_PARENT").expect("child parent"));
        let registry_path = PathBuf::from(
            std::env::var_os("LINA_CREATION_CHILD_REGISTRY").expect("child registry"),
        );
        let cwd = PathBuf::from(std::env::var_os("LINA_CREATION_CHILD_CWD").expect("child cwd"));
        let creation_id = std::env::var("LINA_CREATION_CHILD_ID")
            .ok()
            .map(|raw| Uuid::parse_str(&raw).expect("child creation id"))
            .unwrap_or_else(Uuid::now_v7);
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("child registry load");
        let result = if let Some(checkpoint_name) = mode.strip_prefix("crash:") {
            let target = creation_checkpoint(checkpoint_name);
            create_workspace_for_user_intent_with_checkpoint(
                &parent,
                "App Resiliente",
                &gallery::FocusPreset::DevApp,
                Some(&cwd),
                &mut registry,
                LicenseTier::Pro,
                None,
                creation_id,
                move |checkpoint| {
                    if checkpoint == target {
                        std::process::exit(86);
                    }
                    Ok(())
                },
            )
        } else if mode == "retry-batch" {
            let retries = std::env::var("LINA_CREATION_CHILD_RETRIES")
                .expect("child retries")
                .parse::<usize>()
                .expect("child retries usize");
            (0..retries).try_fold(parent.join("App Resiliente"), |_, attempt| {
                create_workspace_for_intent(
                    &parent,
                    "App Resiliente",
                    &gallery::FocusPreset::DevApp,
                    Some(&cwd),
                    &mut registry,
                    attempt as u64,
                    LicenseTier::Pro,
                    None,
                    creation_id,
                )
            })
        } else if mode == "holder" {
            let signal = PathBuf::from(
                std::env::var_os("LINA_CREATION_CHILD_SIGNAL").expect("holder signal"),
            );
            let release = PathBuf::from(
                std::env::var_os("LINA_CREATION_CHILD_RELEASE").expect("holder release"),
            );
            create_workspace_for_user_intent_with_checkpoint(
                &parent,
                "App Resiliente",
                &gallery::FocusPreset::DevApp,
                Some(&cwd),
                &mut registry,
                LicenseTier::Pro,
                None,
                creation_id,
                move |checkpoint| {
                    if checkpoint == CreationCheckpoint::IntentPersisted {
                        std::fs::write(&signal, b"locked")
                            .map_err(|error| format!("holder signal: {error}"))?;
                        for _ in 0..1_000 {
                            if release.exists() {
                                return Ok(());
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        return Err("holder não recebeu release".to_owned());
                    }
                    Ok(())
                },
            )
        } else {
            create_workspace_for_user_intent(
                &parent,
                "App Resiliente",
                &gallery::FocusPreset::DevApp,
                Some(&cwd),
                &mut registry,
                1,
                LicenseTier::Pro,
                None,
                creation_id,
            )
        };
        match result {
            Ok(_) => std::process::exit(0),
            Err(error) if error.contains("Já existe uma criação sendo finalizada") => {
                std::process::exit(75);
            }
            Err(error) => {
                eprintln!("child creation failed: {error}");
                std::process::exit(76);
            }
        }
    }

    /// **Boot honra o foco do ponteiro:** com um Espaço B focado e com store no disco,
    /// o boot abre B — não mais o default fixo. Ponteiro vazio → default. Se o foco
    /// sumiu, a próxima entrada íntegra vence antes do default.
    #[test]
    fn pick_production_root_honors_focused_entry_with_existing_store() {
        let base = temp_base("pick");
        let default_root = base.join("walking-skeleton");
        let b_root = base.join("cliente-x");
        drop(
            Workspace::create(&default_root, "Padrão", "blank", None)
                .expect("store completo default"),
        );
        drop(Workspace::create(&b_root, "Cliente X", "blank", None).expect("store completo de B"));

        // Ponteiro vazio → default.
        let mut reg = registry_at(&base);
        assert_eq!(
            pick_production_root(&reg, default_root.clone()).expect("default válido"),
            default_root,
            "registry vazio → Espaço default"
        );

        // B focado, completo e com identidade coincidente → B.
        let mut b_entry = WorkspaceRegistry::rederive_entry(&b_root).expect("entrada de B");
        let b_id = b_entry.id.clone();
        b_entry.last_focus = 0;
        reg.upsert(b_entry);
        reg.set_focus(&b_id, 1_000);
        assert_eq!(
            pick_production_root(&reg, default_root.clone()).expect("focado válido"),
            b_root,
            "o focado com store vivo é o que abre"
        );

        // Focado apontando para pasta sem store → próximo ponteiro íntegro.
        reg.upsert(WorkspaceEntry {
            id: "c".into(),
            name: "Sumido".into(),
            path: base.join("nao-existe"),
            last_focus: 0,
            archived: false,
        });
        reg.set_focus("c", 2_000);
        assert_eq!(
            pick_production_root(&reg, default_root.clone()).expect("fallback válido"),
            b_root,
            "focado sem store no disco → próximo Espaço íntegro"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_reliability_picker_rejects_reserved_creation_staging() {
        let base = temp_base("picker-staging");
        let staging_root = base.join(format!(".lina-create-{}", Uuid::now_v7()));
        drop(Workspace::create(&staging_root, "Preparação", "blank", None).expect("staging"));
        let entry = WorkspaceRegistry::rederive_entry(&staging_root).expect("entry");
        let mut registry = registry_at(&base);
        registry.upsert(entry);

        assert!(validate_production_root(&registry, &staging_root).is_err());
        assert!(pick_verified_production_root(registry.entries()).is_err());

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_boot_tries_next_healthy_registry_entry() {
        let base = temp_base("pick-next-healthy");
        let healthy_root = base.join("healthy");
        drop(Workspace::create(&healthy_root, "Saudável", "blank", None).expect("healthy"));
        let mut registry = registry_at(&base);
        let mut healthy = WorkspaceRegistry::rederive_entry(&healthy_root).expect("entry");
        healthy.last_focus = 10;
        registry.upsert(healthy);
        registry.upsert(WorkspaceEntry {
            id: "broken-focused".into(),
            name: "Quebrado".into(),
            path: base.join("missing-focused"),
            last_focus: 20,
            archived: false,
        });

        let selected = pick_production_root(&registry, base.join("missing-default"))
            .expect("próxima entrada íntegra continua carregável");

        assert_eq!(selected, healthy_root);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_initial_boot_rejects_focused_partial_store() {
        let base = temp_base("pick-partial");
        let default_root = base.join("default");
        drop(Workspace::create(&default_root, "Padrão", "blank", None).expect("default completo"));
        let partial_root = base.join("tomikos-parcial");
        let mut store =
            EventStore::open(Workspace::events_dir(&partial_root)).expect("store parcial");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "TomikOS".into(),
                focus_preset: "dev_app".into(),
            })
            .expect("evento interrompido");
        drop(store);

        let mut registry = registry_at(&base);
        registry.upsert(WorkspaceEntry {
            id: partial_root.display().to_string(),
            name: "TomikOS".into(),
            path: partial_root,
            last_focus: 1,
            archived: false,
        });

        assert_eq!(
            pick_production_root(&registry, default_root.clone()).expect("fallback completo"),
            default_root,
            "criação sem WorkspaceIdAssigned nunca vira o boot inicial"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_initial_boot_rejects_mismatch_and_archived_store() {
        let base = temp_base("pick-authority");
        let default_root = base.join("default");
        drop(Workspace::create(&default_root, "Padrão", "blank", None).expect("default completo"));
        let mismatch_root = base.join("mismatch");
        drop(
            Workspace::create(&mismatch_root, "Identidade real", "blank", None)
                .expect("store completo"),
        );
        let mut registry = registry_at(&base);
        registry.upsert(WorkspaceEntry {
            id: "id-forjado".into(),
            name: "Ponteiro".into(),
            path: mismatch_root,
            last_focus: 10,
            archived: false,
        });
        assert_eq!(
            pick_production_root(&registry, default_root.clone()).expect("fallback completo"),
            default_root,
            "store completo não compensa identidade divergente"
        );

        let archived_root = base.join("archived");
        let mut archived =
            Workspace::create(&archived_root, "Arquivado", "blank", None).expect("workspace");
        archived.archive().expect("arquivar");
        drop(archived);
        let mut stale_pointer = WorkspaceRegistry::rederive_entry(&archived_root)
            .expect("entrada rederivada do arquivado");
        stale_pointer.archived = false;
        stale_pointer.last_focus = 20;
        registry.upsert(stale_pointer);
        assert_eq!(
            pick_production_root(&registry, default_root.clone()).expect("fallback completo"),
            default_root,
            "ponteiro velho não ressuscita store arquivado"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_rejected_focused_path_cannot_return_as_default() {
        let base = temp_base("pick-same-rejected-default");
        let root = base.join("same-root");
        drop(Workspace::create(&root, "Identidade real", "blank", None).expect("workspace"));
        let mut registry = registry_at(&base);
        registry.upsert(WorkspaceEntry {
            id: "id-forjado".into(),
            name: "Ponteiro adulterado".into(),
            path: root.clone(),
            last_focus: 10,
            archived: false,
        });

        let error = pick_production_root(&registry, root)
            .expect_err("o mesmo path rejeitado não pode atravessar o fallback");

        assert!(error.contains("mesma pasta"));
        assert!(error.contains("diverge"));
        assert!(
            validate_production_root(&registry, &base.join("same-root")).is_err(),
            "override explícito também respeita a identidade do ponteiro"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_invalid_default_without_focus_is_rejected() {
        let base = temp_base("pick-invalid-default");
        let partial_root = base.join("partial-default");
        EventStore::open(Workspace::events_dir(&partial_root))
            .expect("partial store")
            .append(&DomainEvent::WorkspaceCreated {
                name: "Parcial".into(),
                focus_preset: "blank".into(),
            })
            .expect("created only");
        let registry = registry_at(&base);

        assert!(pick_production_root(&registry, partial_root).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_clean_first_boot_creates_complete_registered_workspace() {
        let base = temp_base("clean-first-boot");
        let default_root = base.join("walking-skeleton");
        let mut registry = registry_at(&base);

        let selected = create_initial_workspace(&default_root, &mut registry)
            .expect("catálogo vazio pode criar primeiro Espaço");

        assert_eq!(selected, default_root);
        let derived = WorkspaceRegistry::rederive_entry(&default_root)
            .expect("primeiro Espaço já nasce completo");
        assert!(!derived.id.is_empty());
        assert_eq!(derived.name, "Meu Espaço");
        assert_eq!(
            registry.focused().map(|entry| entry.id.as_str()),
            Some(derived.id.as_str())
        );
        let reloaded = registry_at(&base);
        assert_eq!(
            reloaded.focused().map(|entry| &entry.path),
            Some(&default_root)
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_initial_creation_restart_converges_after_staging_crash() {
        const CHILD: &str = "LINA_INITIAL_CREATION_CRASH_CHILD";
        const ROOT: &str = "LINA_INITIAL_CREATION_ROOT";
        const TEST_NAME: &str = "workspace_boot::tests::workspace_reliability_initial_creation_restart_converges_after_staging_crash";
        if std::env::var_os(CHILD).is_some() {
            let default_root =
                PathBuf::from(std::env::var_os(ROOT).expect("raiz inicial do subprocesso"));
            let base = default_root.parent().expect("base inicial");
            let mut registry = registry_at(base);
            let _ = create_initial_workspace_with_checkpoint(
                &default_root,
                &mut registry,
                |checkpoint| {
                    if checkpoint == CreationCheckpoint::StagingCreated {
                        std::process::exit(86);
                    }
                    Ok(())
                },
            );
            std::process::exit(87);
        }

        let base = temp_base("initial-staging-crash");
        let default_root = base.join("walking-skeleton");
        let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .args(["--exact", TEST_NAME, "--test-threads=1", "--nocapture"])
            .env(CHILD, "1")
            .env(ROOT, &default_root)
            .status()
            .expect("subprocesso de crash inicial");
        assert_eq!(status.code(), Some(86), "crash ocorreu no corte esperado");
        assert!(
            !default_root.exists(),
            "o caminho final nunca aparece antes da projeção completa"
        );
        assert!(
            !unfinished_creation_artifacts(&base).is_empty(),
            "journal explícito conserva a intenção do crash"
        );

        let mut restarted_registry = registry_at(&base);
        let selected = create_initial_workspace(&default_root, &mut restarted_registry)
            .expect("restart retoma a mesma intenção");
        assert_eq!(selected, default_root);
        let entry = WorkspaceRegistry::rederive_entry(&default_root)
            .expect("restart publica store completo");
        assert_eq!(restarted_registry.entries().len(), 1);
        assert_eq!(restarted_registry.entries()[0].id, entry.id);
        assert!(unfinished_creation_artifacts(&base).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_initial_creation_adopts_published_cut_and_finishes_journal() {
        let base = temp_base("initial-published-cut");
        let default_root = base.join("walking-skeleton");
        let mut registry = registry_at(&base);
        let error =
            create_initial_workspace_with_checkpoint(&default_root, &mut registry, |checkpoint| {
                (checkpoint != CreationCheckpoint::Published)
                    .then_some(())
                    .ok_or_else(|| "crash injetado depois do rename".to_owned())
            })
            .expect_err("corte pós-publicação");
        assert!(error.contains("crash injetado"));
        let published = WorkspaceRegistry::rederive_entry(&default_root)
            .expect("o corte deixa somente um store completo");
        assert!(registry.entries().is_empty(), "catálogo ainda não publicou");
        assert_eq!(
            unfinished_creation_artifacts(&base).len(),
            1,
            "journal ficou"
        );

        let mut restarted = registry_at(&base);
        create_initial_workspace(&default_root, &mut restarted)
            .expect("restart adota identidade publicada");
        assert_eq!(restarted.entries().len(), 1);
        assert_eq!(restarted.entries()[0].id, published.id);
        assert!(unfinished_creation_artifacts(&base).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_first_boot_retry_adopts_complete_unregistered_root() {
        let base = temp_base("first-boot-published-cut");
        let default_root = base.join("walking-skeleton");
        let workspace =
            Workspace::create(&default_root, "Meu Espaço", "blank", None).expect("published");
        let expected_id = workspace
            .project()
            .expect("project")
            .workspace_id
            .expect("id");
        drop(workspace); // corte pós-publicação, antes do registry
        let mut registry = registry_at(&base);

        create_initial_workspace(&default_root, &mut registry)
            .expect("retry adota apenas o store completo");

        assert_eq!(
            registry.focused().map(|entry| entry.id.as_str()),
            Some(expected_id.as_str())
        );
        assert_eq!(registry.active_count(), 1);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_first_boot_retry_rejects_partial_existing_root() {
        let base = temp_base("first-boot-partial-cut");
        let default_root = base.join("walking-skeleton");
        EventStore::open(Workspace::events_dir(&default_root))
            .expect("partial store")
            .append(&DomainEvent::WorkspaceCreated {
                name: "Meu Espaço".into(),
                focus_preset: "blank".into(),
            })
            .expect("created without id");
        let mut registry = registry_at(&base);

        assert!(create_initial_workspace(&default_root, &mut registry).is_err());
        assert!(registry.entries().is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_initial_boot_recovers_missing_db_from_valid_jsonl() {
        let base = temp_base("pick-jsonl-recovery");
        let default_root = base.join("default");
        let ws_root = base.join("recuperavel");
        let workspace =
            Workspace::create(&ws_root, "Recuperável", "blank", None).expect("workspace");
        let mut entry = workspace
            .registry_entry()
            .expect("entrada antes da perda do DB");
        let id = entry.id.clone();
        entry.last_focus = 100;
        drop(workspace);
        let events_dir = Workspace::events_dir(&ws_root);
        assert!(events_dir.join("log.jsonl").is_file());
        std::fs::remove_file(events_dir.join("lina.db")).expect("simular DB ausente");

        let mut registry = registry_at(&base);
        registry.upsert(entry);
        assert_eq!(
            pick_production_root(&registry, default_root).expect("alvo recuperado"),
            ws_root,
            "espelho válido recupera o alvo antes de selecioná-lo"
        );
        let recovered = WorkspaceRegistry::rederive_entry(&ws_root).expect("DB reconstruído");
        assert_eq!(recovered.id, id);
        assert!(!recovered.archived);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_initial_boot_recovers_corrupt_db_from_valid_jsonl() {
        let base = temp_base("pick-corrupt-db-recovery");
        let default_root = base.join("default");
        let ws_root = base.join("recuperavel");
        let workspace =
            Workspace::create(&ws_root, "Recuperável", "blank", None).expect("workspace");
        let mut entry = workspace
            .registry_entry()
            .expect("entrada antes da corrupção");
        let id = entry.id.clone();
        entry.last_focus = 100;
        drop(workspace);
        let events_dir = Workspace::events_dir(&ws_root);
        std::fs::write(events_dir.join("lina.db"), b"sqlite corrompido")
            .expect("simular DB corrompido");

        let mut registry = registry_at(&base);
        registry.upsert(entry);
        assert_eq!(
            pick_production_root(&registry, default_root).expect("alvo recuperado"),
            ws_root,
            "espelho válido recupera DB corrompido antes da seleção"
        );
        let recovered = WorkspaceRegistry::rederive_entry(&ws_root).expect("DB reconstruído");
        assert_eq!(recovered.id, id);
        assert!(
            std::fs::read_dir(&events_dir)
                .expect("events dir")
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("lina.db.corrupt-")),
            "o DB corrompido foi preservado"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    /// **O registry se popula sozinho no boot:** um store ANTIGO (sem `WorkspaceCreated`,
    /// como o de produção pré-F1-4) ganha o evento com o nome do diretório (1×, idempotente)
    /// e vira entrada focada no ponteiro. Rodar de novo NÃO duplica nada.
    #[test]
    fn register_boot_workspace_migrates_old_store_and_is_idempotent() {
        let base = temp_base("register");
        let ws_root = base.join("meu-projeto");
        let mut store = EventStore::open(Workspace::events_dir(&ws_root)).expect("store do Espaço");
        let mut reg = registry_at(&base);

        let id =
            register_boot_workspace(&mut store, &ws_root, &mut reg, 1_000).expect("auto-inscrição");
        let proj = store.project().expect("project");
        assert_eq!(
            proj.workspace_name.as_deref(),
            Some("meu-projeto"),
            "store antigo ganhou WorkspaceCreated com o nome do diretório"
        );
        assert_eq!(
            reg.focused().map(|e| e.id.clone()),
            Some(id.clone()),
            "o Espaço aberto vira o focado do ponteiro"
        );

        // Idempotência: segundo boot não duplica evento nem entrada.
        let events_before = store.event_count().expect("count");
        let id2 =
            register_boot_workspace(&mut store, &ws_root, &mut reg, 2_000).expect("re-inscrição");
        assert_eq!(id, id2, "mesmo id entre boots");
        assert_eq!(
            store.event_count().expect("count"),
            events_before,
            "segundo boot não apenda WorkspaceCreated de novo"
        );
        assert_eq!(reg.active_count(), 1, "uma entrada só no ponteiro");
        assert_eq!(
            reg.focused().map(|e| e.last_focus),
            Some(2_000),
            "o foco re-carimba a cada boot"
        );

        // Persistiu no disco: recarregar o registry devolve a mesma entrada focada.
        let reg2 = registry_at(&base);
        assert_eq!(
            reg2.focused().map(|e| e.path.clone()),
            Some(ws_root.clone()),
            "o ponteiro sobrevive em ~/.lina/workspaces.json"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Inverso da varredura do T6:** os três shapes de `events_dir` aceitos pela lista
    /// (`<root>/.lina/events`, `<root>/events`, o próprio dir) voltam à RAIZ do Espaço.
    #[test]
    fn ws_root_of_events_dir_inverts_the_three_scan_shapes() {
        let root = PathBuf::from("/tmp/espacos/cliente-x");
        assert_eq!(
            ws_root_of_events_dir(&root.join(".lina").join("events")),
            root,
            "shape canônico <root>/.lina/events"
        );
        assert_eq!(
            ws_root_of_events_dir(&root.join("events")),
            root,
            "shape <root>/events"
        );
        assert_eq!(
            ws_root_of_events_dir(&root),
            root,
            "o próprio dir já sendo o store"
        );
    }

    /// **Nome existente é respeitado:** store que JÁ tem `WorkspaceCreated{name}` entra no
    /// ponteiro com aquele nome — a migração só age na ausência.
    #[test]
    fn register_boot_workspace_preserves_existing_name() {
        let base = temp_base("nome");
        let ws_root = base.join("dir-feio");
        let mut store = EventStore::open(Workspace::events_dir(&ws_root)).expect("store do Espaço");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Cliente X".into(),
                focus_preset: String::new(),
            })
            .expect("seed");
        let mut reg = registry_at(&base);

        register_boot_workspace(&mut store, &ws_root, &mut reg, 1_000).expect("inscrição");
        assert_eq!(
            reg.focused().map(|e| e.name.clone()),
            Some("Cliente X".to_string()),
            "o nome do log vence o nome do diretório"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Criação feliz (M9):** eventos na ordem canônica, registry ganhou a entrada SEM
    /// foco (o carimbo é do switch caller), e o replay de um store reaberto re-deriva
    /// nome, preset e Diretório de Trabalho — o log é a fonte da verdade.
    #[test]
    fn create_workspace_happy_path_events_registry_and_replay() {
        let base = temp_base("create");
        let parent = base.join("espacos");
        let cwd_dir = base.join("projetos");
        std::fs::create_dir_all(&cwd_dir).expect("cwd dir");
        let mut reg = registry_at(&base);

        let ws_root = create_workspace(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::DevApp,
            Some(&cwd_dir),
            &mut reg,
            1_000,
            LicenseTier::Pro,
            None,
        )
        .expect("criação feliz");
        assert_eq!(ws_root, parent.join("Cliente X"), "pasta = slug do nome");

        // Ordem canônica: WorkspaceCreated → 4×(NodeAdded+NodeRoleAssigned) →
        // WorkspaceIdAssigned → WorkspaceDefaultCwdSet.
        let store = EventStore::open(Workspace::events_dir(&ws_root)).expect("reabrir store");
        let kinds: Vec<String> = store
            .events()
            .expect("events")
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert_eq!(kinds.first().map(String::as_str), Some("WorkspaceCreated"));
        assert_eq!(
            kinds.iter().filter(|k| *k == "NodeAdded").count(),
            4,
            "time do preset App nasce (4 agentes); veio {kinds:?}"
        );
        assert_eq!(kinds.iter().filter(|k| *k == "NodeRoleAssigned").count(), 4);
        assert_eq!(
            kinds.last().map(String::as_str),
            Some("WorkspaceDefaultCwdSet"),
            "o cwd fecha a sequência"
        );
        assert!(
            kinds.iter().any(|k| k == "WorkspaceIdAssigned"),
            "identidade durável apendada"
        );

        // Replay re-deriva tudo (inv#4: evento é verdade).
        let proj = store.project().expect("replay");
        assert_eq!(proj.workspace_name.as_deref(), Some("Cliente X"));
        assert_eq!(proj.focus_preset.as_deref(), Some("dev_app"));
        assert_eq!(
            proj.default_cwd.as_deref(),
            cwd_dir.canonicalize().expect("canonical cwd").to_str(),
            "Diretório de Trabalho re-derivado do log"
        );
        assert_eq!(proj.nodes.len(), 4, "4 nós projetados");

        // Registry: entrada existe, mas NÃO focada (foco é do caller).
        assert_eq!(reg.active_count(), 1, "entrada inscrita no ponteiro");
        assert!(
            reg.focused().is_none(),
            "criar NÃO carimba foco — isso é do switch"
        );
        let reg2 = registry_at(&base);
        assert_eq!(
            reg2.entries().iter().map(|e| e.path.clone()).next(),
            Some(ws_root.clone()),
            "inscrição persistiu no disco"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **Gating ANTES do esforço:** Free com 1 Espaço ativo → `Blocked` e NENHUMA pasta
    /// criada (zero side effects — o limite aparece antes, nunca nega no fim).
    #[test]
    fn create_workspace_blocked_on_free_creates_nothing() {
        let base = temp_base("blocked");
        let parent = base.join("espacos");
        let mut reg = registry_at(&base);
        reg.upsert(WorkspaceEntry {
            id: "primeiro".into(),
            name: "Meu Espaço".into(),
            path: base.join("primeiro"),
            last_focus: 0,
            archived: false,
        });

        let err = create_workspace(
            &parent,
            "Segundo",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            1_000,
            LicenseTier::Free,
            None,
        )
        .expect_err("free=1 com 1 ativo bloqueia");
        assert!(
            err.contains("limite"),
            "erro acionável de limite; veio: {err}"
        );
        assert!(
            !parent.exists(),
            "Blocked não cria pasta nenhuma (nem o parent)"
        );
        assert_eq!(reg.active_count(), 1, "registry intocado");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_reliability_free_gate_with_existing_parent_does_not_create_lock() {
        let base = temp_base("blocked-existing-parent");
        let parent = base.join("espacos");
        std::fs::create_dir_all(&parent).expect("parent existente");
        std::fs::write(parent.join("arquivo-do-usuario"), b"preservar").expect("sentinela");
        let mut registry = registry_at(&base);
        registry.upsert(WorkspaceEntry {
            id: "primeiro".into(),
            name: "Meu Espaço".into(),
            path: base.join("primeiro"),
            last_focus: 0,
            archived: false,
        });
        let top_level = || -> std::collections::BTreeSet<std::ffi::OsString> {
            std::fs::read_dir(&parent)
                .expect("listar parent")
                .map(|entry| entry.expect("entrada").file_name())
                .collect()
        };
        let before = top_level();

        let error = create_workspace(
            &parent,
            "Segundo",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            1,
            LicenseTier::Free,
            None,
        )
        .expect_err("nova intenção continua bloqueada");

        assert!(error.contains("limite"));
        assert_eq!(top_level(), before, "preflight bloqueado é somente leitura");
        assert!(
            !parent.join(".lina-create-lock.sqlite3").exists(),
            "gate ocorre antes de abrir o banco de coordenação"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_free_gate_still_allows_pending_retry() {
        let base = temp_base("free-pending-retry");
        let parent = base.join("espacos");
        std::fs::create_dir_all(&parent).expect("parent");
        let creation_id = Uuid::now_v7();
        persist_creation_intent(
            &parent,
            &PendingCreationIntent {
                version: CREATION_INTENT_VERSION,
                creation_id: creation_id.to_string(),
                requested_name: "Pendente".to_owned(),
                final_name: "Pendente".to_owned(),
                seed: CreationSeedIntent::Preset {
                    id: gallery::FocusPreset::Blank.id().to_owned(),
                },
                canonical_cwd: None,
                final_root: parent.join("Pendente"),
            },
        )
        .expect("journal iniciado antes do limite");
        let mut registry = registry_at(&base);
        registry.upsert(WorkspaceEntry {
            id: "workspace-concorrente".into(),
            name: "Já existente".into(),
            path: base.join("ja-existente"),
            last_focus: 1,
            archived: false,
        });

        let root = create_workspace_for_intent(
            &parent,
            "Pendente",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            2,
            LicenseTier::Free,
            None,
            creation_id,
        )
        .expect("retry identificado por journal pode concluir");

        assert_eq!(root, parent.join("Pendente"));
        assert_eq!(
            WorkspaceRegistry::rederive_entry(&root)
                .expect("workspace completo")
                .id,
            creation_id.to_string()
        );
        assert!(!creation_intent_path(&parent, &creation_id.to_string()).exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// **Colisão de nome/pasta → sufixo « (2)» (padrão M6):** o segundo "Cliente X" nasce
    /// como "Cliente X (2)" — em pasta própria E com o nome sufixado no log.
    #[test]
    fn create_workspace_name_collision_gets_m6_suffix() {
        let base = temp_base("colisao");
        let parent = base.join("espacos");
        let mut reg = registry_at(&base);

        let first = create_workspace(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            1_000,
            LicenseTier::Pro,
            None,
        )
        .expect("primeiro");
        let second = create_workspace(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            2_000,
            LicenseTier::Pro,
            None,
        )
        .expect("segundo");

        assert_eq!(first, parent.join("Cliente X"));
        assert_eq!(second, parent.join("Cliente X (2)"), "pasta sufixada");
        let proj = EventStore::open(Workspace::events_dir(&second))
            .expect("store do segundo")
            .project()
            .expect("replay");
        assert_eq!(
            proj.workspace_name.as_deref(),
            Some("Cliente X (2)"),
            "o NOME no log também leva o sufixo (pasta e nome coerentes)"
        );
        assert_eq!(reg.active_count(), 2, "duas entradas distintas");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Coincidência de conteúdo não prova retry: sem o mesmo `creation_id`, um
    /// Espaço completo de mesmo nome continua sendo outra intenção.
    #[test]
    fn workspace_reliability_complete_unregistered_workspace_is_not_adopted_by_content() {
        let base = temp_base("resume-unregistered");
        let parent = base.join("espacos");
        let ws_root = parent.join("Cliente X");
        let mut store = EventStore::open(Workspace::events_dir(&ws_root)).expect("store preparado");
        gallery::apply_preset(gallery::FocusPreset::Blank, "Cliente X", &[], &mut store)
            .expect("WorkspaceCreated completo");
        store
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: Uuid::now_v7().to_string(),
            })
            .expect("identidade durável");
        drop(store);

        let mut reg = registry_at(&base);
        let created = create_workspace_for_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            1_000,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect("nova intenção recebe outra raiz");

        assert_eq!(created, parent.join("Cliente X (2)"));
        assert!(ws_root.exists(), "a intenção anterior permanece intacta");
        assert_eq!(
            reg.active_count(),
            1,
            "só a nova intenção foi catalogada aqui"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    fn unfinished_creation_artifacts(parent: &Path) -> Vec<PathBuf> {
        let mut artifacts = std::fs::read_dir(parent)
            .expect("creation parent")
            .map(|entry| entry.expect("creation entry").path())
            .filter(|path| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default();
                name.starts_with(CREATION_NAMESPACE_PREFIX)
                    && (path.is_dir()
                        || name.ends_with(CREATION_INTENT_SUFFIX)
                        || name.contains("intent-tmp"))
            })
            .collect::<Vec<_>>();
        artifacts.sort();
        artifacts
    }

    #[test]
    fn workspace_reliability_failure_matrix_never_publishes_partial() {
        run_creation_subprocess_mode();
        assert_in_process_pre_publish_failure_matrix();
        const TEST_NAME: &str =
            "workspace_boot::tests::workspace_reliability_failure_matrix_never_publishes_partial";
        let checkpoints = [
            "IntentPersisted",
            "StagingCreated",
            "StoreOpened",
            "WorkspaceCreated",
            "ContentSeeded",
            "IdentityAssigned",
            "DefaultCwdAssigned",
            "ProjectionValidated",
            "Published",
            "RegistrySaved",
        ];
        for checkpoint in checkpoints {
            let base = temp_base(&format!("child-crash-{checkpoint}"));
            let parent = base.join("workspaces");
            let registry_path = base.join("catalog/workspaces.json");
            let cwd = base.join("project");
            let creation_id = Uuid::now_v7();
            std::fs::create_dir_all(&cwd).expect("cwd");
            std::fs::create_dir_all(registry_path.parent().expect("catalog parent"))
                .expect("catalog parent");
            let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
                .arg("--exact")
                .arg(TEST_NAME)
                .arg("--test-threads=1")
                .env("LINA_CREATION_CHILD_MODE", format!("crash:{checkpoint}"))
                .env("LINA_CREATION_CHILD_PARENT", &parent)
                .env("LINA_CREATION_CHILD_REGISTRY", &registry_path)
                .env("LINA_CREATION_CHILD_CWD", &cwd)
                .env("LINA_CREATION_CHILD_ID", creation_id.to_string())
                .status()
                .expect("spawn crash child");
            assert_eq!(
                status.code(),
                Some(86),
                "child precisa cair exatamente em {checkpoint}"
            );

            let report = recover_pending_workspace_creations(&parent, &registry_path)
                .expect("novo processo retoma só pelo journal");
            assert!(
                report.issues.is_empty(),
                "{checkpoint}: {:?}",
                report.issues
            );
            let registry = WorkspaceRegistry::load(&registry_path).expect("registry restart");
            assert_eq!(registry.entries().len(), 1, "{checkpoint}: uma linha");
            let entry = registry.entries().first().expect("created entry");
            assert_eq!(entry.id, creation_id.to_string(), "{checkpoint}: mesmo id");
            let (store, state) =
                open_projected_store(&Workspace::events_dir(&entry.path)).expect("complete store");
            let canonical_cwd = cwd.canonicalize().expect("canonical cwd");
            validate_complete_workspace(
                &store,
                &state,
                "App Resiliente",
                gallery::FocusPreset::DevApp,
                canonical_cwd.to_str(),
                None,
                Some(&entry.id),
            )
            .expect("sequência canônica após restart");
            let roles = RoleRegistry::with_defaults().expect("roles");
            assert_eq!(
                state.nodes.len(),
                gallery::preset_team(gallery::FocusPreset::DevApp, &roles).len(),
                "{checkpoint}: roster real do preset"
            );
            assert!(
                unfinished_creation_artifacts(&parent).is_empty(),
                "{checkpoint}: zero staging/journal após recovery"
            );

            let mut boot_catalog = crate::persistence_ui::load_or_rebuild_workspace_registry(
                &registry_path,
                &parent,
                &Workspace::events_dir(&entry.path),
            )
            .expect("boot usa registry restaurado");
            assert!(
                boot_catalog.scan.is_complete(),
                "{checkpoint}: foto completa"
            );
            assert_eq!(
                pick_verified_production_root(&boot_catalog.verified_entries)
                    .expect("boot seleciona store restaurado"),
                entry.path,
                "{checkpoint}: fronteira de boot"
            );
            assert!(
                boot_catalog.registry.set_focus(&entry.id, 10),
                "{checkpoint}: foco conhece a identidade restaurada"
            );
            boot_catalog.registry.save().expect("persistir foco");
            let focused_registry =
                WorkspaceRegistry::load(&registry_path).expect("reload depois do foco");
            assert_eq!(
                focused_registry.focused().map(|focused| &focused.path),
                Some(&entry.path),
                "{checkpoint}: foco sobrevive ao restart"
            );

            drop(store);
            let _ = std::fs::remove_dir_all(base);
        }

        // O gate nomeado é a matriz completa, não apenas a metade de criação:
        // primeiro boot retomável + boot/restore de cada agente + foco pré/pós-commit.
        // Cada caso roda em processo novo para que Drop/RAM do caso anterior não o ajudem.
        for test_name in [
            "workspace_boot::tests::workspace_reliability_initial_creation_restart_converges_after_staging_crash",
            "runtime::tests::workspace_reliability_boot_restore_and_focus_failure_matrix",
        ] {
            let status = std::process::Command::new(std::env::current_exe().expect("test exe"))
                .args([
                    "--exact",
                    test_name,
                    "--test-threads=1",
                    "--nocapture",
                ])
                .env_remove("LINA_CREATION_CHILD_MODE")
                .env_remove("LINA_CREATION_CHILD_PARENT")
                .env_remove("LINA_CREATION_CHILD_REGISTRY")
                .env_remove("LINA_CREATION_CHILD_CWD")
                .env_remove("LINA_CREATION_CHILD_ID")
                .status()
                .expect("subprocesso da fronteira pós-criação");
            assert!(
                status.success(),
                "fronteira obrigatória do Gate C falhou ({test_name}): {status}"
            );
        }
    }

    #[test]
    fn workspace_reliability_two_processes_share_creation_lock() {
        run_creation_subprocess_mode();
        const TEST_NAME: &str =
            "workspace_boot::tests::workspace_reliability_two_processes_share_creation_lock";
        let base = temp_base("two-process-lock");
        let parent = base.join("workspaces");
        let registry_path = base.join("catalog/workspaces.json");
        let cwd = base.join("project");
        let signal = base.join("holder-locked");
        let release = base.join("holder-release");
        std::fs::create_dir_all(&cwd).expect("cwd");
        std::fs::create_dir_all(registry_path.parent().expect("catalog parent"))
            .expect("catalog parent");
        let mut holder = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .arg("--exact")
            .arg(TEST_NAME)
            .arg("--test-threads=1")
            .env("LINA_CREATION_CHILD_MODE", "holder")
            .env("LINA_CREATION_CHILD_PARENT", &parent)
            .env("LINA_CREATION_CHILD_REGISTRY", &registry_path)
            .env("LINA_CREATION_CHILD_CWD", &cwd)
            .env("LINA_CREATION_CHILD_SIGNAL", &signal)
            .env("LINA_CREATION_CHILD_RELEASE", &release)
            .spawn()
            .expect("spawn holder");
        for _ in 0..1_000 {
            if signal.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(signal.exists(), "holder adquiriu o lock");
        let contender = std::process::Command::new(std::env::current_exe().expect("test exe"))
            .arg("--exact")
            .arg(TEST_NAME)
            .arg("--test-threads=1")
            .env("LINA_CREATION_CHILD_MODE", "contender")
            .env("LINA_CREATION_CHILD_PARENT", &parent)
            .env("LINA_CREATION_CHILD_REGISTRY", &registry_path)
            .env("LINA_CREATION_CHILD_CWD", &cwd)
            .status()
            .expect("spawn contender");
        assert_eq!(contender.code(), Some(75), "concorrente recebe retry");
        std::fs::write(&release, b"continue").expect("release holder");
        assert!(holder.wait().expect("holder exit").success());

        let registry = WorkspaceRegistry::load(&registry_path).expect("registry final");
        assert_eq!(registry.entries().len(), 1);
        assert!(unfinished_creation_artifacts(&parent).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_completed_user_intent_does_not_capture_new_intent() {
        let base = temp_base("new-intent-after-complete");
        let parent = base.join("workspaces");
        let registry_path = base.join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let first = create_workspace_for_user_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            1,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect("first intent");
        let first_id = registry.entries().first().expect("first entry").id.clone();
        let second = create_workspace_for_user_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            2,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect("new intent");

        assert_eq!(first, parent.join("Cliente X"));
        assert_eq!(second, parent.join("Cliente X (2)"));
        assert_eq!(registry.entries().len(), 2);
        assert!(registry.entries().iter().any(|entry| entry.id != first_id));
        assert!(unfinished_creation_artifacts(&parent).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_new_modal_resumes_one_explicit_pending_intent() {
        let base = temp_base("new-modal-resumes-pending");
        let parent = base.join("workspaces");
        let registry_path = base.join("workspaces.json");
        let original_id = Uuid::now_v7();
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        create_workspace_for_user_intent_with_checkpoint(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            LicenseTier::Pro,
            None,
            original_id,
            |checkpoint| {
                (checkpoint != CreationCheckpoint::StagingCreated)
                    .then_some(())
                    .ok_or_else(|| "processo interrompido".to_owned())
            },
        )
        .expect_err("primeiro processo interrompe");

        let mut restarted_registry = WorkspaceRegistry::load(&registry_path).expect("restart");
        let resumed = create_workspace_for_user_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut restarted_registry,
            2,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect("novo modal encontra journal sem receber UUID antigo");

        assert_eq!(resumed, parent.join("Cliente X"));
        assert_eq!(restarted_registry.entries().len(), 1);
        assert_eq!(restarted_registry.entries()[0].id, original_id.to_string());
        assert!(unfinished_creation_artifacts(&parent).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_ambiguous_or_invalid_creation_intent_blocks_visibly() {
        let base = temp_base("ambiguous-intents");
        let parent = base.join("workspaces");
        std::fs::create_dir_all(&parent).expect("parent");
        let seed = CreationSeedIntent::Preset {
            id: gallery::FocusPreset::Blank.id().to_owned(),
        };
        for (index, creation_id) in [Uuid::now_v7(), Uuid::now_v7()].into_iter().enumerate() {
            let final_name = if index == 0 {
                "Cliente X".to_owned()
            } else {
                "Cliente X (2)".to_owned()
            };
            persist_creation_intent(
                &parent,
                &PendingCreationIntent {
                    version: CREATION_INTENT_VERSION,
                    creation_id: creation_id.to_string(),
                    requested_name: "Cliente X".to_owned(),
                    final_root: parent.join(&final_name),
                    final_name,
                    seed: seed.clone(),
                    canonical_cwd: None,
                },
            )
            .expect("intent fixture");
        }
        let mut registry = registry_at(&base);
        let ambiguous = create_workspace_for_user_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            1,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect_err("duas pendências iguais bloqueiam");
        assert!(ambiguous.contains("2 criações pendentes"));

        let invalid_base = temp_base("invalid-intent");
        let invalid_parent = invalid_base.join("workspaces");
        std::fs::create_dir_all(&invalid_parent).expect("invalid parent");
        let invalid_id = Uuid::now_v7();
        std::fs::write(
            creation_intent_path(&invalid_parent, &invalid_id.to_string()),
            b"{ intent truncado",
        )
        .expect("invalid intent");
        let mut invalid_registry = registry_at(&invalid_base);
        let invalid = create_workspace_for_user_intent(
            &invalid_parent,
            "Outro",
            &gallery::FocusPreset::Blank,
            None,
            &mut invalid_registry,
            1,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect_err("journal inválido bloqueia");
        assert!(invalid.contains("Intenção de criação inválida"));

        let _ = std::fs::remove_dir_all(base);
        let _ = std::fs::remove_dir_all(invalid_base);
    }

    #[test]
    fn workspace_reliability_conflicting_intent_publish_cleans_temporary() {
        let base = temp_base("intent-publish-conflict");
        let parent = base.join("workspaces");
        std::fs::create_dir_all(&parent).expect("parent");
        let creation_id = Uuid::now_v7();
        let expected = PendingCreationIntent {
            version: CREATION_INTENT_VERSION,
            creation_id: creation_id.to_string(),
            requested_name: "Esperado".to_owned(),
            final_name: "Esperado".to_owned(),
            final_root: parent.join("Esperado"),
            seed: CreationSeedIntent::Preset {
                id: gallery::FocusPreset::Blank.id().to_owned(),
            },
            canonical_cwd: None,
        };
        let conflicting = PendingCreationIntent {
            requested_name: "Concorrente".to_owned(),
            final_name: "Concorrente".to_owned(),
            final_root: parent.join("Concorrente"),
            ..expected.clone()
        };

        let error =
            persist_creation_intent_with_link(&parent, &expected, |temporary, published| {
                let bytes = serde_json::to_vec_pretty(&conflicting).expect("serializar conflito");
                std::fs::write(published, bytes).expect("publicação concorrente");
                std::fs::hard_link(temporary, published)
            })
            .expect_err("journal concorrente divergente é recusado");
        assert!(error.contains("pertence a outra intenção"));
        assert_eq!(
            read_creation_intent(
                &creation_intent_path(&parent, &creation_id.to_string()),
                &parent,
            )
            .expect("journal concorrente preservado"),
            conflicting
        );
        let temporary_leftovers: Vec<PathBuf> = std::fs::read_dir(&parent)
            .expect("parent")
            .map(|entry| entry.expect("entrada").path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with(".lina-create-intent-tmp-")
                })
            })
            .collect();
        assert!(
            temporary_leftovers.is_empty(),
            "a corrida não deixa intent-tmp órfão: {temporary_leftovers:?}"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    /// Cada falha ANTES do rename remove somente o staging desta operação e
    /// conserva o journal explícito retomável; nada parcial vira linha normal.
    fn assert_in_process_pre_publish_failure_matrix() {
        let failures = [
            CreationCheckpoint::IntentPersisted,
            CreationCheckpoint::StagingCreated,
            CreationCheckpoint::StoreOpened,
            CreationCheckpoint::WorkspaceCreated,
            CreationCheckpoint::ContentSeeded,
            CreationCheckpoint::IdentityAssigned,
            CreationCheckpoint::DefaultCwdAssigned,
            CreationCheckpoint::ProjectionValidated,
        ];

        for failure in failures {
            let base = temp_base(&format!("atomic-{failure:?}"));
            let parent = base.join("espacos");
            let cwd = base.join("projeto");
            std::fs::create_dir_all(&cwd).expect("cwd");
            let mut reg = registry_at(&base);
            let err = create_workspace_with_checkpoint(
                &parent,
                "Cliente X",
                &gallery::FocusPreset::DevApp,
                Some(&cwd),
                &mut reg,
                LicenseTier::Pro,
                None,
                Uuid::now_v7(),
                |checkpoint| {
                    if checkpoint == failure {
                        Err(format!("falha injetada em {failure:?}"))
                    } else {
                        Ok(())
                    }
                },
            )
            .expect_err("a fronteira escolhida falha");

            assert!(
                err.contains("falha injetada"),
                "a falha observada é a injetada; veio: {err}"
            );
            assert!(
                !parent.join("Cliente X").exists(),
                "{failure:?}: raiz final parcial nunca aparece"
            );
            let leftovers = unfinished_creation_artifacts(&parent);
            assert!(
                leftovers.len() == 1
                    && leftovers[0].file_name().is_some_and(|name| name
                        .to_string_lossy()
                        .ends_with(CREATION_INTENT_SUFFIX)),
                "{failure:?}: só o journal retomável pode restar; veio {leftovers:?}"
            );
            assert_eq!(reg.active_count(), 0, "{failure:?}: registry intocado");

            let _ = std::fs::remove_dir_all(&base);
        }
    }

    /// Um encerramento abrupto não executa cleanup. No restart, o staging
    /// determinístico da MESMA intenção é encontrado: parcial é refeito; completo
    /// é publicado diretamente; nenhum `.lina-create-*` se acumula.
    #[test]
    fn workspace_reliability_crashed_staging_is_resumed_or_cleaned() {
        let base = temp_base("crashed-staging");
        let parent = base.join("espacos");
        std::fs::create_dir_all(&parent).expect("parent");
        let mut reg = registry_at(&base);

        let partial_id = Uuid::now_v7();
        persist_creation_intent(
            &parent,
            &PendingCreationIntent {
                version: CREATION_INTENT_VERSION,
                creation_id: partial_id.to_string(),
                requested_name: "Parcial".to_owned(),
                final_name: "Parcial".to_owned(),
                seed: CreationSeedIntent::Preset {
                    id: gallery::FocusPreset::Blank.id().to_owned(),
                },
                canonical_cwd: None,
                final_root: parent.join("Parcial"),
            },
        )
        .expect("journal parcial");
        let partial_staging = parent.join(format!(".lina-create-{partial_id}"));
        let mut partial_store =
            EventStore::open(Workspace::events_dir(&partial_staging)).expect("staging parcial");
        partial_store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Parcial".to_string(),
                focus_preset: "blank".to_string(),
            })
            .expect("caiu antes da identidade");
        drop(partial_store);

        let partial_root = create_workspace_for_intent(
            &parent,
            "Parcial",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            1_000,
            LicenseTier::Pro,
            None,
            partial_id,
        )
        .expect("staging parcial é limpo e refeito");
        assert_eq!(partial_root, parent.join("Parcial"));
        assert!(!partial_staging.exists(), "staging parcial não sobra");

        let complete_id = Uuid::now_v7();
        persist_creation_intent(
            &parent,
            &PendingCreationIntent {
                version: CREATION_INTENT_VERSION,
                creation_id: complete_id.to_string(),
                requested_name: "Completo".to_owned(),
                final_name: "Completo".to_owned(),
                seed: CreationSeedIntent::Preset {
                    id: gallery::FocusPreset::Blank.id().to_owned(),
                },
                canonical_cwd: None,
                final_root: parent.join("Completo"),
            },
        )
        .expect("journal completo");
        let complete_staging = parent.join(format!(".lina-create-{complete_id}"));
        let mut complete_store =
            EventStore::open(Workspace::events_dir(&complete_staging)).expect("staging completo");
        complete_store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Completo".to_string(),
                focus_preset: "blank".to_string(),
            })
            .expect("WorkspaceCreated");
        complete_store
            .append(&DomainEvent::WorkspaceIdAssigned {
                workspace_id: complete_id.to_string(),
            })
            .expect("identidade fecha preparação");
        drop(complete_store);

        let complete_root = create_workspace_for_intent(
            &parent,
            "Completo",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            2_000,
            LicenseTier::Pro,
            None,
            complete_id,
        )
        .expect("staging completo é publicado");
        assert_eq!(complete_root, parent.join("Completo"));
        assert!(
            !complete_staging.exists(),
            "staging completo virou raiz final"
        );

        let staging_leftovers = unfinished_creation_artifacts(&parent);
        assert!(
            staging_leftovers.is_empty(),
            "nenhum staging acumulado; restou {staging_leftovers:?}"
        );
        assert_eq!(
            reg.active_count(),
            2,
            "as duas intenções completas entraram"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Crash/falha DEPOIS do rename deixa apenas um store completo. O mesmo
    /// `creation_id` o encontra no restart, termina o registry e não cria sufixo.
    #[test]
    fn workspace_reliability_published_creation_is_resumed_by_creation_id() {
        let base = temp_base("resume-by-id");
        let parent = base.join("espacos");
        let creation_id = Uuid::now_v7();
        let mut reg = registry_at(&base);

        let err = create_workspace_with_checkpoint(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut reg,
            LicenseTier::Pro,
            None,
            creation_id,
            |checkpoint| {
                if checkpoint == CreationCheckpoint::Published {
                    Err("processo caiu depois da publicação".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("simula crash pós-rename");
        assert!(err.contains("processo caiu"));
        assert!(
            parent.join("Cliente X").exists(),
            "a raiz publicada já está completa"
        );
        assert_eq!(reg.active_count(), 0, "registry ainda não foi tocado");

        // Simula restart: registry novo, mesma intenção durável.
        let mut restarted_registry = registry_at(&base);
        let resumed = create_workspace_for_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut restarted_registry,
            2_000,
            LicenseTier::Pro,
            None,
            creation_id,
        )
        .expect("restart retoma a mesma intenção");

        assert_eq!(resumed, parent.join("Cliente X"));
        assert!(!parent.join("Cliente X (2)").exists());
        assert_eq!(restarted_registry.active_count(), 1);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_reliability_new_process_recovers_catalog_but_new_intent_stays_distinct() {
        let base = temp_base("resume-new-process");
        let parent = base.join("espacos");
        let original_id = Uuid::now_v7();
        let mut registry = registry_at(&base);
        create_workspace_with_checkpoint(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            LicenseTier::Pro,
            None,
            original_id,
            |checkpoint| {
                (checkpoint != CreationCheckpoint::Published)
                    .then_some(())
                    .ok_or_else(|| "crash depois do rename".to_string())
            },
        )
        .expect_err("crash simulado");

        // O startup fecha primeiro o journal explícito e só então publica o catálogo.
        recover_pending_workspace_creations(&parent, &base.join("workspaces.json"))
            .expect("startup reconcilia a intenção");
        let catalog = crate::persistence_ui::load_or_rebuild_workspace_registry(
            &base.join("workspaces.json"),
            &parent,
            &Workspace::events_dir(&parent.join("Cliente X")),
        )
        .expect("startup reconstrói o catálogo");
        assert_eq!(catalog.registry.entries().len(), 1);
        assert_eq!(catalog.registry.entries()[0].id, original_id.to_string());

        // Um novo modal produz outro UUID. Mesmo nome/foco/cwd não é prova de
        // retry: a nova intenção cria outro Espaço com o sufixo normal.
        let mut restarted_registry = catalog.registry;
        let new_id = Uuid::now_v7();
        let created = create_workspace_for_user_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut restarted_registry,
            2_000,
            LicenseTier::Pro,
            None,
            new_id,
        )
        .expect("nova intenção cria raiz distinta");
        assert_eq!(created, parent.join("Cliente X (2)"));
        assert!(parent.join("Cliente X").exists());
        assert_eq!(restarted_registry.active_count(), 2);
        assert!(restarted_registry
            .entries()
            .iter()
            .any(|entry| entry.id == new_id.to_string() && entry.path == created));

        let _ = std::fs::remove_dir_all(base);
    }

    /// A publicação já é completa quando o catálogo falha. O retry com a
    /// mesma intenção usa a entrada mantida em memória e jamais cria `(2)`.
    #[test]
    fn workspace_reliability_registry_save_failure_retries_same_published_root() {
        let base = temp_base("registry-save-failure");
        let parent = base.join("espacos");
        let creation_id = Uuid::now_v7();
        let mut registry = registry_at(&base);
        let blocked_tmp = base.join(format!("workspaces.tmp-{}", std::process::id()));
        std::fs::create_dir(&blocked_tmp).expect("bloquear File::create do tmp do registry");

        let error = create_workspace_for_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            1_000,
            LicenseTier::Pro,
            None,
            creation_id,
        )
        .expect_err("save do registry foi injetado para falhar");
        assert!(
            error.contains("directory") || error.contains("diretório"),
            "a falha observada vem do tmp bloqueado: {error}"
        );
        assert!(parent.join("Cliente X").exists(), "raiz já publicada");
        assert_eq!(registry.active_count(), 1, "upsert fica na memória");

        std::fs::remove_dir(&blocked_tmp).expect("desbloquear registry");
        let resumed = create_workspace_for_intent(
            &parent,
            "Cliente X",
            &gallery::FocusPreset::Blank,
            None,
            &mut registry,
            2_000,
            LicenseTier::Pro,
            None,
            creation_id,
        )
        .expect("retry termina o catálogo");
        assert_eq!(resumed, parent.join("Cliente X"));
        assert!(!parent.join("Cliente X (2)").exists());
        assert_eq!(registry_at(&base).active_count(), 1, "catálogo durável");

        let _ = std::fs::remove_dir_all(base);
    }

    /// Gate B do loop: cem reenvios da MESMA intenção, distribuídos por
    /// processos/restarts reais, convergem para uma raiz e uma sequência.
    #[test]
    fn workspace_reliability_same_creation_id_is_exactly_once() {
        run_creation_subprocess_mode();
        const TEST_NAME: &str =
            "workspace_boot::tests::workspace_reliability_same_creation_id_is_exactly_once";
        let base = temp_base("exactly-once");
        let parent = base.join("espacos");
        let registry_path = base.join("catalog/workspaces.json");
        let cwd = base.join("projeto");
        std::fs::create_dir_all(&cwd).expect("cwd");
        std::fs::create_dir_all(registry_path.parent().expect("catalog parent"))
            .expect("catalog parent");
        let creation_id = Uuid::now_v7();
        let run_retry_process = |id: Uuid, retries: usize| {
            std::process::Command::new(std::env::current_exe().expect("test exe"))
                .arg("--exact")
                .arg(TEST_NAME)
                .arg("--test-threads=1")
                .env("LINA_CREATION_CHILD_MODE", "retry-batch")
                .env("LINA_CREATION_CHILD_PARENT", &parent)
                .env("LINA_CREATION_CHILD_REGISTRY", &registry_path)
                .env("LINA_CREATION_CHILD_CWD", &cwd)
                .env("LINA_CREATION_CHILD_ID", id.to_string())
                .env("LINA_CREATION_CHILD_RETRIES", retries.to_string())
                .status()
                .expect("spawn retry child")
        };
        for restart in 0..5 {
            assert!(
                run_retry_process(creation_id, 20).success(),
                "restart {restart} conclui seus 20 retries"
            );
        }

        let roots: Vec<PathBuf> = std::fs::read_dir(&parent)
            .expect("parent")
            .map(|entry| entry.expect("entrada"))
            .filter(|entry| entry.file_type().expect("tipo da entrada").is_dir())
            .map(|entry| entry.path())
            .collect();
        assert_eq!(roots, vec![parent.join("App Resiliente")], "uma única raiz");
        let store = EventStore::open(Workspace::events_dir(&roots[0])).expect("store final");
        let records = store.events().expect("eventos");
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == "WorkspaceCreated")
                .count(),
            1,
            "uma criação canônica"
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == "WorkspaceIdAssigned")
                .count(),
            1,
            "uma identidade durável"
        );
        let expected_id = creation_id.to_string();
        assert_eq!(
            store.project().expect("projeção").workspace_id.as_deref(),
            Some(expected_id.as_str())
        );
        let state = store.project().expect("projeção final");
        let roles = RoleRegistry::with_defaults().expect("roles");
        assert_eq!(
            state.nodes.len(),
            gallery::preset_team(gallery::FocusPreset::DevApp, &roles).len(),
            "roster DevApp íntegro"
        );
        assert_eq!(
            state.default_cwd.as_deref(),
            cwd.canonicalize().expect("canonical cwd").to_str()
        );
        let registry = WorkspaceRegistry::load(&registry_path).expect("registry após 100 retries");
        assert_eq!(registry.entries().len(), 1, "uma entrada no registry");
        assert_eq!(registry.entries()[0].id, expected_id);

        let new_creation_id = Uuid::now_v7();
        assert!(
            run_retry_process(new_creation_id, 1).success(),
            "nova intenção conclui em outro processo"
        );
        let registry = WorkspaceRegistry::load(&registry_path).expect("registry com nova intenção");
        assert_eq!(registry.entries().len(), 2);
        let distinct = registry
            .entries()
            .iter()
            .find(|entry| entry.id == new_creation_id.to_string())
            .expect("nova identidade no catálogo");
        assert_eq!(distinct.path, parent.join("App Resiliente (2)"));
        assert!(roots[0].exists(), "a primeira intenção permanece intacta");
        assert!(unfinished_creation_artifacts(&parent).is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_reliability_stale_catalog_retry_reuses_published_creation() {
        let base = temp_base("stale-catalog-retry");
        let parent = base.join("espacos");
        let registry_path = base.join("workspaces.json");
        let cwd = base.join("projeto");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let creation_id = Uuid::now_v7();

        // Simula duas janelas abertas antes da primeira publicação. A segunda mantém
        // deliberadamente o snapshot vazio enquanto a primeira conclui no disco.
        let mut stale_window = WorkspaceRegistry::load(&registry_path).expect("snapshot antigo");
        let mut publishing_window =
            WorkspaceRegistry::load(&registry_path).expect("snapshot publicador");
        let first = create_workspace_for_intent(
            &parent,
            "App Resiliente",
            &gallery::FocusPreset::Blank,
            Some(&cwd),
            &mut publishing_window,
            1,
            LicenseTier::Pro,
            None,
            creation_id,
        )
        .expect("primeira janela publica");
        assert!(stale_window.entries().is_empty(), "fixture realmente stale");

        let retry = create_workspace_for_intent(
            &parent,
            "App Resiliente",
            &gallery::FocusPreset::Blank,
            Some(&cwd),
            &mut stale_window,
            2,
            LicenseTier::Pro,
            None,
            creation_id,
        )
        .expect("janela stale retoma a mesma intenção");

        assert_eq!(retry, first, "retry stale devolve a raiz já publicada");
        assert!(
            !parent.join("App Resiliente (2)").exists(),
            "a mesma intenção jamais ganha sufixo por snapshot antigo"
        );
        let registry = WorkspaceRegistry::load(&registry_path).expect("catálogo final");
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].id, creation_id.to_string());
        assert_eq!(registry.entries()[0].path, first);
        assert!(unfinished_creation_artifacts(&parent).is_empty());

        let _ = std::fs::remove_dir_all(base);
    }

    /// **`validate_workdir` nos 4 casos da spec §2:** existe-e-escreve → Ok canonicalizado;
    /// não-existe → Ok (o caminho feliz cria); é-arquivo → «não está acessível»; sem
    /// permissão de escrita → «não consigo trabalhar» (teste REAL, via chmod 000).
    #[test]
    fn validate_workdir_four_cases() {
        let base = temp_base("workdir");

        // (1) Existe, é diretório, escreve → Ok canonicalizado.
        let ok_dir = base.join("escrevivel");
        std::fs::create_dir_all(&ok_dir).expect("mkdir");
        let canonical = validate_workdir(&ok_dir).expect("dir gravável é válido");
        assert_eq!(canonical, ok_dir.canonicalize().expect("canonicalize"));

        // (2) Não existe → Ok (não é erro: a criação fará a pasta).
        let missing = base.join("ainda-nao-existe");
        assert_eq!(
            validate_workdir(&missing).expect("inexistente não é erro"),
            missing
        );

        // (3) É arquivo → NotAccessible (string leiga congelada).
        let file = base.join("um-arquivo.txt");
        std::fs::write(&file, b"x").expect("write");
        let err = validate_workdir(&file).expect_err("arquivo não é pasta");
        assert_eq!(err, WorkdirError::NotAccessible);
        assert_eq!(
            err.to_string(),
            "Essa pasta não está acessível — escolha outra ou crie uma nova."
        );

        // (4) Sem permissão de escrita → NoWritePermission (prova REAL de escrita).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let sealed = base.join("selada");
            std::fs::create_dir_all(&sealed).expect("mkdir");
            std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000))
                .expect("chmod 000");
            let err = validate_workdir(&sealed).expect_err("sem escrita");
            assert_eq!(err, WorkdirError::NoWritePermission);
            assert_eq!(err.to_string(), "Não consigo trabalhar nessa pasta.");
            // Devolve a permissão para o cleanup não falhar.
            std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755))
                .expect("chmod de volta");
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn workspace_reliability_workdir_probe_never_touches_preexisting_collision() {
        let base = temp_base("workdir-probe-collision");
        let workdir = base.join("project");
        std::fs::create_dir_all(&workdir).expect("workdir");
        let legacy_collision = workdir.join(format!(".lina-prova-escrita-{}", std::process::id()));
        let original = b"arquivo do usuario";
        std::fs::write(&legacy_collision, original).expect("legacy collision fixture");
        let probe_id = Uuid::now_v7();
        let exclusive_collision = workdir.join(format!(".lina-prova-escrita-{probe_id}"));
        std::fs::write(&exclusive_collision, original).expect("exclusive collision fixture");

        let error = validate_existing_workdir_with(
            workdir.canonicalize().expect("canonical"),
            probe_id,
            |probe| probe.write_all(b"ok"),
            |probe_path| std::fs::remove_file(probe_path),
        )
        .expect_err("create_new recusa colisão");
        assert_eq!(error, WorkdirError::NoWritePermission);
        validate_workdir(&workdir).expect("UUID novo não colide com fixtures");

        assert_eq!(
            std::fs::read(&legacy_collision).expect("legacy collision preserved"),
            original
        );
        assert_eq!(
            std::fs::read(&exclusive_collision).expect("exclusive collision preserved"),
            original
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_workdir_probe_has_no_residue_and_cleanup_errors_surface() {
        let base = temp_base("workdir-probe-cleanup");
        let workdir = base.join("project");
        std::fs::create_dir_all(&workdir).expect("workdir");
        let canonical = workdir.canonicalize().expect("canonical");

        let success_id = Uuid::now_v7();
        let success_probe = canonical.join(format!(".lina-prova-escrita-{success_id}"));
        validate_existing_workdir_with(
            canonical.clone(),
            success_id,
            |probe| probe.write_all(b"ok"),
            |probe_path| std::fs::remove_file(probe_path),
        )
        .expect("success");
        assert!(!success_probe.exists(), "sucesso remove o probe");

        let failed_write_id = Uuid::now_v7();
        let failed_write_probe = canonical.join(format!(".lina-prova-escrita-{failed_write_id}"));
        let write_error = validate_existing_workdir_with(
            canonical.clone(),
            failed_write_id,
            |_| Err(std::io::Error::other("falha de escrita injetada")),
            |probe_path| std::fs::remove_file(probe_path),
        )
        .expect_err("falha de escrita é visível");
        assert_eq!(write_error, WorkdirError::NoWritePermission);
        assert!(
            !failed_write_probe.exists(),
            "falha de escrita também remove o probe"
        );

        let cleanup_error_id = Uuid::now_v7();
        let cleanup_error_probe = canonical.join(format!(".lina-prova-escrita-{cleanup_error_id}"));
        let cleanup_error = validate_existing_workdir_with(
            canonical,
            cleanup_error_id,
            |probe| probe.write_all(b"ok"),
            |probe_path| {
                std::fs::remove_file(probe_path)?;
                Err(std::io::Error::other("falha de cleanup injetada"))
            },
        )
        .expect_err("erro retornado pelo cleanup não é engolido");
        assert_eq!(cleanup_error, WorkdirError::NoWritePermission);
        assert!(
            !cleanup_error_probe.exists(),
            "o seam remove antes de simular a falha reportada"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    #[ignore = "gate manual: requer LINA_REAL_WORKDIR e toca a pasta só com o probe efêmero"]
    fn workspace_reliability_real_workdir_gate_has_no_residue_or_fd_leak() {
        let requested_workdir = std::env::var_os("LINA_REAL_WORKDIR").unwrap_or_else(|| {
            panic!(
                "defina LINA_REAL_WORKDIR e rode este teste com --ignored --exact --test-threads=1"
            )
        });
        let real_workdir = PathBuf::from(requested_workdir)
            .canonicalize()
            .expect("LINA_REAL_WORKDIR precisa apontar para uma pasta acessível");
        let expected_workdir = PathBuf::from("/Users/rafaelmelgaco/tomikos");
        assert_eq!(
            real_workdir, expected_workdir,
            "este gate deliberadamente valida apenas /Users/rafaelmelgaco/tomikos"
        );
        assert!(real_workdir.is_dir(), "LINA_REAL_WORKDIR precisa ser pasta");
        let top_level_entries =
            |path: &Path| -> std::io::Result<std::collections::BTreeSet<std::ffi::OsString>> {
                std::fs::read_dir(path)?
                    .map(|entry| entry.map(|entry| entry.file_name()))
                    .collect()
            };
        let open_fd_count = || {
            std::fs::read_dir("/dev/fd")
                .expect("/dev/fd é obrigatório neste gate macOS")
                .count()
        };
        let entries_before = top_level_entries(&real_workdir).expect("snapshot top-level inicial");
        let fds_before = open_fd_count();
        eprintln!(
            "gate real: path={} fds_before={fds_before}",
            real_workdir.display()
        );

        let temp_root = temp_base("real-workdir-gate");
        let workspace_parent = temp_root.join("workspaces");
        let mut registry = registry_at(&temp_root);
        let creation = create_workspace_for_intent(
            &workspace_parent,
            "Gate real de Diretório de Trabalho",
            &gallery::FocusPreset::Blank,
            Some(&real_workdir),
            &mut registry,
            1,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        );

        let entries_after = top_level_entries(&real_workdir).expect("snapshot top-level final");
        let fds_after = open_fd_count();
        eprintln!(
            "gate real: path={} fds_after={fds_after}",
            real_workdir.display()
        );
        assert_eq!(
            entries_after, entries_before,
            "o gate real não pode deixar probe ou qualquer outro resíduo"
        );
        assert_eq!(
            fds_after, fds_before,
            "a criação não pode vazar descritores"
        );
        creation.expect("criação temporária usando o Diretório de Trabalho real");

        let _ = std::fs::remove_dir_all(temp_root);
    }

    #[test]
    fn workspace_reliability_creation_materializes_missing_workdir_without_scanning_siblings() {
        let base = temp_base("create-missing-workdir");
        let inaccessible_sibling = base.join("nao-tocar");
        std::fs::create_dir(&inaccessible_sibling).expect("sibling");
        let workdir = base.join("projetos").join("TomikOS");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                &inaccessible_sibling,
                std::fs::Permissions::from_mode(0o000),
            )
            .expect("selar sibling");
        }

        let parent = base.join("espacos");
        let mut registry = registry_at(&base);
        let root = create_workspace_for_intent(
            &parent,
            "TomikOS",
            &gallery::FocusPreset::Blank,
            Some(&workdir),
            &mut registry,
            1,
            LicenseTier::Pro,
            None,
            Uuid::now_v7(),
        )
        .expect("cria o diretório escolhido pela raiz");

        assert!(
            workdir.is_dir(),
            "o Diretório de Trabalho foi materializado"
        );
        let projected = EventStore::open(Workspace::events_dir(&root))
            .expect("store")
            .project()
            .expect("projeção");
        assert_eq!(
            projected.default_cwd.as_deref(),
            workdir.canonicalize().expect("canonical").to_str()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                &inaccessible_sibling,
                std::fs::Permissions::from_mode(0o700),
            )
            .expect("restaurar sibling");
        }
        let _ = std::fs::remove_dir_all(base);
    }

    /// Gate D: o Diretório de Trabalho é validado somente na raiz. Uma árvore
    /// profunda, Unicode, symlink circular e subárvore sem leitura não aumentam o
    /// conjunto de recursos mantidos pela criação nem são percorridos.
    #[test]
    fn workspace_reliability_large_workdir_does_not_expand_creation_work() {
        let base = temp_base("large-workdir");
        let empty_workdir = base.join("Vazio");
        std::fs::create_dir_all(&empty_workdir).expect("empty workdir");
        let workdir = base.join("TomikOS 🖥️");
        std::fs::create_dir_all(&workdir).expect("workdir");
        for directory in 0..40 {
            let branch = workdir.join(format!("módulo-{directory:02}"));
            std::fs::create_dir(&branch).expect("branch");
            for file in 0..25 {
                std::fs::write(branch.join(format!("arquivo-{file:02}.txt")), b"fixture")
                    .expect("arquivo");
            }
        }
        let unreadable = workdir.join("nao-percorrer");
        std::fs::create_dir(&unreadable).expect("subárvore");
        std::fs::write(unreadable.join("segredo"), b"nao ler").expect("folha");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt as _};
            symlink(&workdir, workdir.join("volta-para-raiz")).expect("symlink circular");
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000))
                .expect("tirar leitura");
        }

        let run_creation = |tag: &str, selected_workdir: &Path| {
            let scenario = base.join(tag);
            let parent = scenario.join("espacos");
            let mut registry = registry_at(&scenario);
            reset_workdir_operation_counts();
            let before_fds = std::fs::read_dir("/dev/fd")
                .expect("/dev/fd é obrigatório no gate de recursos")
                .count();
            let root = create_workspace_for_intent(
                &parent,
                "TomikOS",
                &gallery::FocusPreset::Blank,
                Some(selected_workdir),
                &mut registry,
                1,
                LicenseTier::Pro,
                None,
                Uuid::now_v7(),
            )
            .expect("criar sem percorrer o projeto");
            let after_fds = std::fs::read_dir("/dev/fd")
                .expect("/dev/fd continua disponível")
                .count();
            (
                root,
                workdir_operation_counts(),
                after_fds as isize - before_fds as isize,
            )
        };
        let (_, empty_operations, empty_fd_delta) = run_creation("empty", &empty_workdir);
        let (root, large_operations, large_fd_delta) = run_creation("large", &workdir);

        assert_eq!(
            large_operations, empty_operations,
            "árvore grande deve executar exatamente as mesmas operações sobre o workdir"
        );
        assert_eq!(
            large_fd_delta, empty_fd_delta,
            "delta de FDs precisa ser comparável: vazio={empty_fd_delta}, grande={large_fd_delta}"
        );
        assert!(
            large_fd_delta <= 0,
            "a criação não pode reter FD do workdir: delta={large_fd_delta}"
        );

        let state = EventStore::open(Workspace::events_dir(&root))
            .expect("store")
            .project()
            .expect("projeção");
        assert_eq!(
            state.default_cwd.as_deref(),
            workdir.canonicalize().expect("canonical workdir").to_str()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700))
                .expect("restaurar permissão");
        }
        let _ = std::fs::remove_dir_all(base);
    }

    /// **F3-5-10 (gate f + regra-mãe):** criar COM gabarito pelo MESMO funil gated → o foco é o do
    /// gabarito, o roster é SEMEADO como `SpawnRequested` (PEDIDOS, sem nó vivo), o instanciador é
    /// `HUMAN_GESTURE` (origem honesta, não forja), e o Espaço ENTRA no registry (não some da lista).
    #[test]
    fn create_workspace_from_template_seeds_roster_with_human_gesture_and_registers() {
        let base = temp_base("template");
        let parent = base.join("espacos");
        let mut reg = registry_at(&base);
        let saas = lina_core::workspace_template::template_saas();

        let ws_root = create_workspace(
            &parent,
            "Meu SaaS",
            &gallery::FocusPreset::Blank, // ignorado quando há gabarito (o foco vem do template)
            None,
            &mut reg,
            1_000,
            LicenseTier::Pro,
            Some(&saas),
        )
        .expect("criação com gabarito");

        let store = EventStore::open(Workspace::events_dir(&ws_root)).expect("reabrir");
        let records = store.events().expect("events");
        assert_eq!(
            records.first().map(|r| r.kind.as_str()),
            Some("WorkspaceCreated"),
            "WorkspaceCreated abre a sequência (liga as doutrinas pelo foco)"
        );
        let proj = store.project().expect("replay");
        assert_eq!(
            proj.workspace_name.as_deref(),
            Some("Meu SaaS"),
            "o nome do usuário vence o nome do gabarito"
        );
        assert_eq!(
            proj.focus_preset.as_deref(),
            Some("dev_app"),
            "o foco vem do gabarito (doutrinas-como-hooks)"
        );

        // Roster semeado como PEDIDOS, não nós vivos — só o gate `admit_node` materializa terminal.
        let spawns: Vec<_> = records
            .iter()
            .filter(|r| r.kind == "SpawnRequested")
            .collect();
        assert_eq!(
            spawns.len(),
            saas.roster.len(),
            "um pedido por papel do gabarito"
        );
        assert_eq!(
            records.iter().filter(|r| r.kind == "NodeAdded").count(),
            0,
            "gabarito não cria nó vivo (sem escalar privilégio)"
        );

        // Regra-mãe: o instanciador é HUMAN_GESTURE — origem auditável, JAMAIS um id forjado.
        // Comparo pelo payload serializado (from_record é privado fora do core): o `requested_by`
        // gravado tem de bater com a serialização do sentinela.
        let expected = serde_json::to_value(lina_core::HUMAN_GESTURE).expect("serializa NodeId");
        assert_eq!(
            spawns[0].payload.get("requested_by"),
            Some(&expected),
            "instanciador = gesto humano (origem honesta), não forja"
        );

        // Gating + registry preservados pelo funil único (não furou o gate, não sumiu da lista).
        assert_eq!(
            reg.active_count(),
            1,
            "o Espaço do gabarito entra no registry"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
