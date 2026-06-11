//! **F1-4-1 (fiação app) — o boot multi-Espaço.** Duas pontas PURAS e testáveis headless:
//! (1) [`pick_production_root`] decide QUAL Espaço abrir (o focado do `WorkspaceRegistry`
//! global `~/.lina/workspaces.json`, com fallback honesto ao default quando o ponteiro está
//! vazio/quebrado); (2) [`register_boot_workspace`] inscreve o Espaço aberto no registry e
//! carimba o foco — é assim que o registry se POPULA (convergência registry×varredura,
//! spec-m8-m9 §6-C3) sem migração manual. Stores antigos sem `WorkspaceCreated` (o boot de
//! produção pré-F1-4 nunca o apendava) ganham o evento UMA vez (migração aditiva, inv#4:
//! acrescenta o fato que falta, nunca reescreve história).

use std::path::{Path, PathBuf};

use lina_core::{DomainEvent, EventStore, Workspace, WorkspaceEntry, WorkspaceRegistry};

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
    registry.set_focus(&id, now_ms);
    registry.save().map_err(|e| e.to_string())?;
    Ok(id)
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
}
