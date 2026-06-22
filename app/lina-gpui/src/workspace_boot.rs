//! **F1-4-1 (fiação app) — o boot multi-Espaço.** Duas pontas PURAS e testáveis headless:
//! (1) [`pick_production_root`] decide QUAL Espaço abrir (o focado do `WorkspaceRegistry`
//! global `~/.lina/workspaces.json`, com fallback honesto ao default quando o ponteiro está
//! vazio/quebrado); (2) [`register_boot_workspace`] inscreve o Espaço aberto no registry e
//! carimba o foco — é assim que o registry se POPULA (convergência registry×varredura,
//! spec-m8-m9 §6-C3) sem migração manual. Stores antigos sem `WorkspaceCreated` (o boot de
//! produção pré-F1-4 nunca o apendava) ganham o evento UMA vez (migração aditiva, inv#4:
//! acrescenta o fato que falta, nunca reescreve história).

use std::path::{Path, PathBuf};

use lina_core::{
    can_create_workspace, DomainEvent, EventStore, LicenseTier, Workspace, WorkspaceEntry,
    WorkspaceRegistry,
};
use lina_role_discovery::RoleRegistry;
use uuid::Uuid;

use crate::gallery;

/// Decide a raiz do Espaço a abrir em PRODUÇÃO: o focado do registry (último
/// `last_focus`, não-arquivado) **se o store dele ainda existe no disco**; senão o
/// `default_root` (o Espaço histórico). Nunca falha — ponteiro vazio/quebrado degrada
/// para o default (inv#6: o boot nunca trava por causa de um ponteiro).
#[must_use]
pub fn pick_production_root(registry: &WorkspaceRegistry, default_root: PathBuf) -> PathBuf {
    match registry.focused() {
        Some(entry) if Workspace::events_dir(&entry.path).exists() => entry.path.clone(),
        _ => default_root,
    }
}

/// Inscreve o Espaço recém-aberto no registry global e carimba o foco em `now_ms`.
/// Store sem `WorkspaceCreated` (geração pré-F1-4) ganha o evento com o nome do
/// diretório — migração aditiva, idempotente (na próxima abertura o nome já projeta).
/// Devolve o `id` canônico do Espaço no registry.
///
/// # Errors
/// Propaga falha de projeção/append/save como string acionável (o chamador loga e
/// segue — registrar-se no ponteiro é best-effort, nunca derruba o boot).
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
/// que NÃO foca: o carimbo é do switch chamador). Única casa da lógica de id
/// (`workspace_id` do log; fallback o path) — não duplicar.
fn upsert_registry_entry(
    store: &mut EventStore,
    ws_root: &Path,
    registry: &mut WorkspaceRegistry,
) -> Result<String, String> {
    let mut state = store.project().map_err(|e| e.to_string())?;
    if state.workspace_name.is_none() {
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
    let name = state
        .workspace_name
        .ok_or_else(|| "WorkspaceCreated apendado mas o nome não projetou".to_string())?;
    let id = state
        .workspace_id
        .unwrap_or_else(|| ws_root.display().to_string());
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
    if !path.exists() {
        return Ok(path.to_path_buf());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| WorkdirError::NotAccessible)?;
    if !canonical.is_dir() {
        return Err(WorkdirError::NotAccessible);
    }
    let probe = canonical.join(format!(".lina-prova-escrita-{}", std::process::id()));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(canonical)
        }
        Err(_) => Err(WorkdirError::NoWritePermission),
    }
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
    let base = name.trim();
    let base = if base.is_empty() { "Meu Espaço" } else { base };
    if !taken(base) {
        return base.to_string();
    }
    for i in 2..100 {
        let candidate = format!("{base} ({i})");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base} (∞)")
}

/// **O seam ÚNICO de criação de Espaço (M9).** Cria um Espaço NOVO de verdade, na ordem
/// que protege o usuário Free: (a) gating ANTES de qualquer esforço — `Blocked` não cria
/// pasta nenhuma; (b) nome/pasta únicos (padrão M6); (c) pasta + store + a sequência
/// canônica de eventos — `WorkspaceCreated{name, focus_preset}` + time do preset (via
/// [`gallery::apply_preset`], mesmo contrato W4-5) + `WorkspaceIdAssigned` (identidade
/// durável, mesmo contrato de `Workspace::create` — sem ele o id degradaria para o path)
/// e `WorkspaceDefaultCwdSet` quando o Diretório de Trabalho veio; (d) inscrição no
/// registry global SEM carimbo de foco — o `WorkspaceFocusSet`/foco é do switch CALLER.
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
    // (a) Gating PRIMEIRO — o limite aparece antes do esforço; Blocked = zero side effects.
    can_create_workspace(tier, registry.active_count()).map_err(|e| e.to_string())?;
    // Recusa de input ANTES de tocar o disco (mesmo contrato de `Workspace::create`).
    let default_cwd = default_cwd
        .map(|p| {
            p.to_str().map(str::to_string).ok_or_else(|| {
                format!(
                    "Diretório de Trabalho com caminho não-UTF8: {}",
                    p.display()
                )
            })
        })
        .transpose()?;

    // (b) Nome/pasta únicos.
    let final_name = unique_workspace_name(parent, name, registry);
    let ws_root = parent.join(folder_slug(&final_name));

    // (c) Pasta + store + eventos (a fonte da verdade nasce aqui).
    std::fs::create_dir_all(&ws_root).map_err(|_| WorkdirError::CreateFailed.to_string())?;
    let mut store = EventStore::open(Workspace::events_dir(&ws_root)).map_err(|e| e.to_string())?;
    match template {
        // F3-5-10: gabarito → `WorkspaceCreated{foco do template}` + semeia roster/params/pistas/
        // backlog pelo core. O roster vira `SpawnRequested` (PEDIDOS): o terminal físico só nasce
        // pelo funil `admit_node`/gate (ADR 0022), sem escalar privilégio. O instanciador é o
        // sentinela canônico `HUMAN_GESTURE` (origem auditável = gesto humano, JAMAIS autoridade —
        // o usuário criou o Espaço pela tela; nada de NodeId fantasma nem forja, regra-mãe da onda).
        Some(t) => {
            store
                .append(&DomainEvent::WorkspaceCreated {
                    name: final_name.clone(),
                    focus_preset: t.focus_preset.clone(),
                })
                .map_err(|e| e.to_string())?;
            lina_core::workspace_template::instanciar(t, &mut store, lina_core::HUMAN_GESTURE)
                .map_err(|e| e.to_string())?;
        }
        // Sem gabarito: o caminho de hoje, INALTERADO (time do Foco).
        None => {
            let roles = RoleRegistry::with_defaults().map_err(|e| e.to_string())?;
            let placed: Vec<gallery::PlacedAgent> = gallery::preset_team(*preset, &roles)
                .into_iter()
                .map(|a| gallery::PlacedAgent {
                    node: Uuid::now_v7(),
                    name: a.name,
                    role: a.role,
                })
                .collect();
            gallery::apply_preset(*preset, &final_name, &placed, &mut store)
                .map_err(|e| e.to_string())?;
        }
    }
    store
        .append(&DomainEvent::WorkspaceIdAssigned {
            workspace_id: Uuid::now_v7().to_string(),
        })
        .map_err(|e| e.to_string())?;
    if let Some(cwd) = default_cwd {
        store
            .append(&DomainEvent::WorkspaceDefaultCwdSet { cwd })
            .map_err(|e| e.to_string())?;
    }

    // (d) Registro no ponteiro global — sem foco (o carimbo é do caller).
    upsert_registry_entry(&mut store, &ws_root, registry)?;
    registry.save().map_err(|e| e.to_string())?;
    Ok(ws_root)
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

/// **A metade do switcher que dura entre boots:** carimba `target_root` como o Espaço focado
/// do ponteiro global. Alvo já conhecido → `set_focus` direto; alvo descoberto pela varredura
/// mas nunca registrado → inscreve pelo MESMO caminho do boot ([`register_boot_workspace`],
/// que abre o store do alvo). O chamador NUNCA usa isto para o Espaço VIVO (abriria o store
/// dele duas vezes — corrida com a conexão do canvas).
///
/// # Errors
/// Propaga falha de store/save como string acionável (o chamador loga e segue).
pub fn focus_target_workspace(
    target_root: &Path,
    registry: &mut WorkspaceRegistry,
    now_ms: u64,
) -> Result<String, String> {
    let known = registry
        .entries()
        .iter()
        .find(|e| e.path == target_root)
        .map(|e| e.id.clone());
    match known {
        Some(id) => {
            registry.set_focus(&id, now_ms);
            registry.save().map_err(|e| e.to_string())?;
            Ok(id)
        }
        None => {
            let mut store =
                EventStore::open(Workspace::events_dir(target_root)).map_err(|e| e.to_string())?;
            register_boot_workspace(&mut store, target_root, registry, now_ms)
        }
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

    /// **Boot honra o foco do ponteiro:** com um Espaço B focado e com store no disco,
    /// o boot abre B — não mais o default fixo. Ponteiro vazio → default. Focado cuja
    /// pasta sumiu → default (nunca trava, nunca abre pasta morta).
    #[test]
    fn pick_production_root_honors_focused_entry_with_existing_store() {
        let base = temp_base("pick");
        let default_root = base.join("walking-skeleton");
        let b_root = base.join("cliente-x");
        std::fs::create_dir_all(Workspace::events_dir(&b_root)).expect("events dir de B");

        // Ponteiro vazio → default.
        let mut reg = registry_at(&base);
        assert_eq!(
            pick_production_root(&reg, default_root.clone()),
            default_root,
            "registry vazio → Espaço default"
        );

        // B focado e existente → B.
        reg.upsert(WorkspaceEntry {
            id: "b".into(),
            name: "Cliente X".into(),
            path: b_root.clone(),
            last_focus: 0,
            archived: false,
        });
        reg.set_focus("b", 1_000);
        assert_eq!(
            pick_production_root(&reg, default_root.clone()),
            b_root,
            "o focado com store vivo é o que abre"
        );

        // Focado apontando para pasta sem store → default (ponteiro quebrado degrada).
        reg.upsert(WorkspaceEntry {
            id: "c".into(),
            name: "Sumido".into(),
            path: base.join("nao-existe"),
            last_focus: 0,
            archived: false,
        });
        reg.set_focus("c", 2_000);
        assert_eq!(
            pick_production_root(&reg, default_root.clone()),
            default_root,
            "focado sem store no disco → fallback honesto"
        );

        let _ = std::fs::remove_dir_all(&base);
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

    /// **Trocar carimba o foco que dura entre boots:** alvo conhecido ganha `set_focus`;
    /// alvo descoberto pela varredura (nunca registrado) é inscrito pelo caminho do boot.
    /// Nos dois casos, `pick_production_root` do PRÓXIMO boot devolve o alvo.
    #[test]
    fn focus_target_workspace_makes_next_boot_open_the_target() {
        let base = temp_base("switch");
        let a_root = base.join("espaco-a");
        let b_root = base.join("espaco-b");
        let mut store_a = EventStore::open(Workspace::events_dir(&a_root)).expect("store de A");
        // B existe no disco (varredura o acharia) mas NUNCA foi registrado no ponteiro.
        drop(EventStore::open(Workspace::events_dir(&b_root)).expect("store de B"));

        let mut reg = registry_at(&base);
        register_boot_workspace(&mut store_a, &a_root, &mut reg, 1_000).expect("boot em A");
        assert_eq!(
            pick_production_root(&reg, base.join("default")),
            a_root,
            "antes da troca, o próximo boot abriria A"
        );

        // Troca para B (alvo desconhecido do ponteiro → inscrito na hora).
        focus_target_workspace(&b_root, &mut reg, 2_000).expect("troca para B");
        assert_eq!(
            pick_production_root(&reg, base.join("default")),
            b_root,
            "depois da troca, o próximo boot abre B"
        );

        // Troca de volta para A (alvo já conhecido → set_focus direto, sem reabrir store).
        focus_target_workspace(&a_root, &mut reg, 3_000).expect("troca para A");
        assert_eq!(
            pick_production_root(&reg, base.join("default")),
            a_root,
            "trocar de volta re-foca A"
        );
        assert_eq!(reg.active_count(), 2, "duas entradas, zero duplicatas");

        let _ = std::fs::remove_dir_all(&base);
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
            cwd_dir.to_str(),
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
