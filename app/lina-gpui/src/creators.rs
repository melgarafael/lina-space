//! **W4-2 (M3/M4) — criadores de NOTA e PASTA no Espaço.** Backend puro e testável (sem gpui, no
//! mesmo padrão de [`crate::wiring`]): a render/wiring CONSOME estas funções. Cada criação:
//!   1. cunha um `NodeId` e apenda `NodeAdded{kind:"Note"|"Folder"}` — o NÓ no canvas (o que o
//!      replay reconstrói na projeção);
//!   2. apenda o evento META `NoteCreated{name}` / `FolderCreated{name}` (livro-razão da criação —
//!      mesmo papel de `NoteUpdated`; o core já tem ambos commitados);
//!   3. apenda `NodeRenamed{node, name}` para o nó exibir o título;
//!   4. persiste o artefato em `<workspace>/.lina/` (corpo da nota em `notes/<slug>.md`; a pasta
//!      como diretório `folders/<slug>/`).
//!
//! O event log é a fonte da verdade (invariante #4): o `EventStore::project()` reconstrói o nó por
//! replay. O corpo da nota vive no arquivo (o evento `NoteCreated` só carrega o nome — contrato do
//! core), então **persiste-se o arquivo ANTES** de logar (se o disco falhar, nada é logado e a nota
//! não "existe" pela metade).
//!
//! **Anti-colisão (W4-2 polimento, 3 terminais no shell):** módulo NOVO e isolado; não toca
//! `bridge`/`canvas`/etc. Para o nó APARECER de fato no canvas vivo (não só na projeção), a camada de
//! canvas precisa projetar/semear o nó novo — ver o pedido de HOOK no `.entrega-m3m4.md`.

// HOOK LANDADO: a paleta (Cmd-K → "Criar Nota"/"Criar Pasta") consome `CreatorForm::commit` via
// `NodeManager::create_artifact` (bridge), que semeia o nó no canvas vivo; o render desenha o nó
// Note/Folder. Por isso o `#![allow(dead_code)]` foi REMOVIDO — a API agora é usada pelo binário.

use std::path::{Path, PathBuf};

use lina_core::{DomainEvent, EventStore, NodeId, StoreError};
use uuid::Uuid;

/// Posição default de um nó recém-criado no canvas (a wiring/canvas pode reposicionar). Fora do
/// caminho dos terminais-semente (30/740, y=96) para não sobrepor no boot. **O canvas (M3/M4 hook)
/// posiciona via `next_free_slot` (não-sobreposição), então este default fica como API/teste** —
/// `#[allow(dead_code)]` apenas neste item (a lógica do módulo permanece intacta).
#[allow(dead_code)]
pub const DEFAULT_POS: (f64, f64) = (120.0, 320.0);

/// Erro de criação de nota/pasta. Sem `unwrap` em produção: todo caminho falível vira uma variante.
#[derive(Debug)]
pub enum CreatorError {
    /// Título (nota) ou nome (pasta) vazio após `trim`.
    EmptyName,
    /// Falha de I/O ao persistir o artefato em `.lina/`.
    Io(std::io::Error),
    /// Falha ao apendar ao event log.
    Store(StoreError),
}

impl std::fmt::Display for CreatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreatorError::EmptyName => write!(f, "nome vazio (título/nome é obrigatório)"),
            CreatorError::Io(e) => write!(f, "falha ao persistir o artefato: {e}"),
            CreatorError::Store(e) => write!(f, "falha ao apendar evento: {e}"),
        }
    }
}

impl std::error::Error for CreatorError {}

impl From<std::io::Error> for CreatorError {
    fn from(e: std::io::Error) -> Self {
        CreatorError::Io(e)
    }
}

impl From<StoreError> for CreatorError {
    fn from(e: StoreError) -> Self {
        CreatorError::Store(e)
    }
}

/// Qual artefato criar — o formulário (UI) escolhe e a wiring despacha via [`CreatorForm::commit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatorKind {
    Note,
    Folder,
}

/// **Estado do formulário de criação** (UI o preenche; gpui-free, testável). `title` = título da nota
/// ou nome da pasta; `body` só vale para a nota. A render lê/escreve estes campos; `commit` aplica.
#[derive(Debug, Clone, Default)]
pub struct CreatorForm {
    pub title: String,
    pub body: String,
}

impl CreatorForm {
    /// Aplica o formulário: cria a nota ou a pasta (apenda eventos + persiste) e devolve o `NodeId`
    /// novo (para a wiring focar/selecionar o nó no canvas).
    pub fn commit(
        &self,
        kind: CreatorKind,
        store: &mut EventStore,
        lina_dir: &Path,
        pos: (f64, f64),
    ) -> Result<NodeId, CreatorError> {
        match kind {
            CreatorKind::Note => create_note(store, lina_dir, &self.title, &self.body, pos),
            CreatorKind::Folder => create_folder(store, lina_dir, &self.title, pos),
        }
    }
}

/// **M3 — cria uma NOTA** (`título` + `corpo`). Persiste o corpo em `<lina_dir>/notes/<slug>.md`,
/// apenda `NodeAdded{kind:"Note"}` + `NoteCreated{name}` + `NodeRenamed`. Devolve o `NodeId` novo.
///
/// # Errors
/// [`CreatorError::EmptyName`] se o título é vazio; [`CreatorError::Io`] ao persistir;
/// [`CreatorError::Store`] ao logar.
pub fn create_note(
    store: &mut EventStore,
    lina_dir: &Path,
    title: &str,
    body: &str,
    pos: (f64, f64),
) -> Result<NodeId, CreatorError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(CreatorError::EmptyName);
    }
    // Cunha o `node` ANTES de persistir: o nome do arquivo inclui o id, então duas notas de MESMO
    // título (`reuniao.md`) nunca colidem (cada nó é único) — evita sobrescrita silenciosa de corpo.
    let node = Uuid::now_v7();
    // Persiste o corpo PRIMEIRO (a fonte da verdade do corpo é o arquivo; se o disco falhar, não
    // logamos uma nota "pela metade"). `slugify` garante nome de arquivo seguro (sem `/`/`..`).
    let notes_dir = lina_dir.join("notes");
    std::fs::create_dir_all(&notes_dir)?;
    let path = notes_dir.join(format!("{}-{node}.md", slugify(title)));
    write_atomic(&path, body.as_bytes())?;

    store.append(&DomainEvent::NodeAdded {
        node,
        kind: "Note".to_string(),
        x: pos.0,
        y: pos.1,
    })?;
    store.append(&DomainEvent::NoteCreated {
        name: title.to_string(),
    })?;
    store.append(&DomainEvent::NodeRenamed {
        node,
        name: title.to_string(),
    })?;
    Ok(node)
}

/// **M4 — cria uma PASTA** (`nome`). Cria o diretório `<lina_dir>/folders/<slug>/`, apenda
/// `NodeAdded{kind:"Folder"}` + `FolderCreated{name}` + `NodeRenamed`. Devolve o `NodeId` novo.
///
/// # Errors
/// [`CreatorError::EmptyName`] se o nome é vazio; [`CreatorError::Io`] ao criar o diretório;
/// [`CreatorError::Store`] ao logar.
pub fn create_folder(
    store: &mut EventStore,
    lina_dir: &Path,
    name: &str,
    pos: (f64, f64),
) -> Result<NodeId, CreatorError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CreatorError::EmptyName);
    }
    // `node` no nome do diretório → duas pastas de mesmo nome não compartilham/sobrescrevem o diretório.
    let node = Uuid::now_v7();
    let dir = lina_dir
        .join("folders")
        .join(format!("{}-{node}", slugify(name)));
    std::fs::create_dir_all(&dir)?;

    store.append(&DomainEvent::NodeAdded {
        node,
        kind: "Folder".to_string(),
        x: pos.0,
        y: pos.1,
    })?;
    store.append(&DomainEvent::FolderCreated {
        name: name.to_string(),
    })?;
    store.append(&DomainEvent::NodeRenamed {
        node,
        name: name.to_string(),
    })?;
    Ok(node)
}

/// Nome de arquivo/diretório SEGURO a partir de um título livre: minúsculas (Unicode), **acentos
/// pt-br dobrados para ASCII** (`reunião` → `reuniao`), só `[a-z0-9]`, demais viram `-` (colapsado),
/// sem `-` nas pontas. Garante **zero** `/`/`..`/espaço/sentinela → nenhum path-traversal e nenhum
/// nome vazio (`"sem-nome"` no limite). O título original (com acentos) fica no `NodeRenamed`/`NoteCreated`.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    // `to_lowercase` (Unicode) rebaixa também acentuadas maiúsculas (`Ã` → `ã`); depois dobra o acento.
    for c in s
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .map(fold_accent)
    {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let slug = out.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "sem-nome".to_string()
    } else {
        slug
    }
}

/// Dobra um caractere latino acentuado (minúsculo) para seu equivalente ASCII; demais passam direto.
/// Cobre o conjunto pt-br comum (+ alguns latinos), suficiente para nomes de arquivo legíveis.
fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

/// Escrita atômica (tmp + rename no mesmo diretório) — um leitor nunca vê arquivo pela metade.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Diretório temporário único; removido no `Drop`.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-m3m4-{tag}-{}", Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn lina(&self) -> PathBuf {
            self.0.join(".lina")
        }
        fn store_dir(&self) -> PathBuf {
            self.0.join(".lina").join("events")
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn kinds(store: &EventStore) -> Vec<String> {
        store
            .events()
            .expect("events")
            .into_iter()
            .map(|r| r.kind)
            .collect()
    }

    /// ACEITE M3: criar nota → `NoteCreated` + `NodeAdded{kind:Note}` no log, corpo persistido em
    /// `.lina/notes/<slug>.md`, e o nó aparece na projeção (o que o canvas renderiza / replay reconstrói).
    #[test]
    fn create_note_logs_persists_and_projects() {
        let tmp = TempDir::new("note");
        let mut store = EventStore::open(tmp.store_dir()).expect("open store");

        let node = create_note(
            &mut store,
            &tmp.lina(),
            "Ideias da Reunião",
            "- contratar QA\n- revisar preços",
            DEFAULT_POS,
        )
        .expect("create_note");

        // Eventos no log.
        let ks = kinds(&store);
        assert!(ks.contains(&"NodeAdded".to_string()));
        assert!(ks.contains(&"NoteCreated".to_string()));
        assert!(ks.contains(&"NodeRenamed".to_string()));
        let note_ev = store
            .events()
            .unwrap()
            .into_iter()
            .find(|r| r.kind == "NoteCreated")
            .expect("NoteCreated");
        assert_eq!(note_ev.payload["name"], "Ideias da Reunião");

        // Corpo persistido no .lina/notes/<slug>-<node>.md (id no nome → sem colisão de mesmo título).
        let md = tmp
            .lina()
            .join("notes")
            .join(format!("ideias-da-reuniao-{node}.md"));
        assert!(md.exists(), "corpo da nota persistido em {md:?}");
        assert_eq!(
            std::fs::read_to_string(&md).unwrap(),
            "- contratar QA\n- revisar preços"
        );

        // O nó aparece na projeção (o canvas renderiza a projeção; replay reconstrói).
        let st = store.project().expect("project");
        let n = st.nodes.get(&node).expect("nó da nota na projeção");
        assert_eq!(n.kind, "Note");
        assert_eq!(n.name.as_deref(), Some("Ideias da Reunião"));
    }

    /// ACEITE M4: criar pasta → `FolderCreated` + `NodeAdded{kind:Folder}` no log, diretório criado
    /// em `.lina/folders/<slug>/`, e o nó aparece na projeção.
    #[test]
    fn create_folder_logs_persists_and_projects() {
        let tmp = TempDir::new("folder");
        let mut store = EventStore::open(tmp.store_dir()).expect("open store");

        let node = create_folder(&mut store, &tmp.lina(), "Clientes", DEFAULT_POS).expect("create");

        let ks = kinds(&store);
        assert!(ks.contains(&"FolderCreated".to_string()));
        assert!(ks.contains(&"NodeAdded".to_string()));
        let dir = tmp.lina().join("folders").join(format!("clientes-{node}"));
        assert!(dir.is_dir(), "pasta criada em {dir:?}");

        let st = store.project().expect("project");
        let n = st.nodes.get(&node).expect("nó da pasta na projeção");
        assert_eq!(n.kind, "Folder");
        assert_eq!(n.name.as_deref(), Some("Clientes"));
    }

    /// "Replay reconstrói": reabrir o store (replay do log persistido) reconstrói o nó da nota.
    #[test]
    fn note_survives_store_reopen_via_replay() {
        let tmp = TempDir::new("replay");
        let node = {
            let mut store = EventStore::open(tmp.store_dir()).expect("open");
            create_note(&mut store, &tmp.lina(), "Persistente", "corpo", DEFAULT_POS)
                .expect("create")
        }; // store dropado → fechado

        let store2 = EventStore::open(tmp.store_dir()).expect("reopen");
        let st = store2.project().expect("project pós-reopen");
        let n = st.nodes.get(&node).expect("nó reconstruído por replay");
        assert_eq!(n.kind, "Note");
        assert_eq!(n.name.as_deref(), Some("Persistente"));
    }

    /// Título/nome vazio é recusado SEM apendar evento nem escrever arquivo.
    #[test]
    fn empty_name_is_rejected_cleanly() {
        let tmp = TempDir::new("empty");
        let mut store = EventStore::open(tmp.store_dir()).expect("open");
        assert!(matches!(
            create_note(&mut store, &tmp.lina(), "   ", "corpo", DEFAULT_POS),
            Err(CreatorError::EmptyName)
        ));
        assert!(matches!(
            create_folder(&mut store, &tmp.lina(), "", DEFAULT_POS),
            Err(CreatorError::EmptyName)
        ));
        assert_eq!(store.event_count().expect("count"), 0, "nada foi logado");
        assert!(
            !tmp.lina().join("notes").exists(),
            "nenhum arquivo escrito p/ entrada inválida"
        );
    }

    /// `slugify` é seguro: título com path-traversal/sentinelas vira um nome de arquivo contido em
    /// `notes/` (sem `/`/`..`), e a nota é criada dentro do diretório esperado.
    #[test]
    fn slugify_blocks_path_traversal() {
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("a/b/c"), "a-b-c");
        assert_eq!(slugify("  Olá, Mundo!  "), "ola-mundo"); // acento dobrado (á→a)
        assert_eq!(slugify("Reunião — Café"), "reuniao-cafe"); // pt-br acentos → ASCII
        assert_eq!(slugify("***"), "sem-nome");
        assert_eq!(slugify(""), "sem-nome");

        let tmp = TempDir::new("slug");
        let mut store = EventStore::open(tmp.store_dir()).expect("open");
        let node =
            create_note(&mut store, &tmp.lina(), "../escape", "x", DEFAULT_POS).expect("create");
        // O arquivo ficou DENTRO de notes/ (sem escapar via `..`).
        let inside = tmp.lina().join("notes").join(format!("escape-{node}.md"));
        assert!(inside.exists(), "arquivo contido em notes/: {inside:?}");
        // Garante que nada escapou para fora de notes/ (nenhum `..` resolvido).
        let escaped = tmp.lina().join("escape.md");
        assert!(!escaped.exists(), "não pode ter escapado de notes/");
    }

    /// Red-team do próprio módulo: duas notas de MESMO título NÃO colidem (id no nome) — ambos os
    /// corpos sobrevivem (sem sobrescrita silenciosa) e há 2 nós distintos na projeção.
    #[test]
    fn same_title_notes_do_not_collide() {
        let tmp = TempDir::new("collide");
        let mut store = EventStore::open(tmp.store_dir()).expect("open");
        let a = create_note(&mut store, &tmp.lina(), "Reunião", "corpo A", DEFAULT_POS).expect("a");
        let b = create_note(&mut store, &tmp.lina(), "Reunião", "corpo B", DEFAULT_POS).expect("b");
        assert_ne!(a, b, "dois nós distintos");

        let dir = tmp.lina().join("notes");
        let mds: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .collect();
        assert_eq!(mds.len(), 2, "dois arquivos .md (sem sobrescrita)");
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("reuniao-{a}.md"))).unwrap(),
            "corpo A"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("reuniao-{b}.md"))).unwrap(),
            "corpo B"
        );
        // ambos os nós existem na projeção.
        let st = store.project().expect("project");
        assert!(st.nodes.contains_key(&a) && st.nodes.contains_key(&b));
    }

    /// O formulário despacha para a função certa conforme o `CreatorKind`.
    #[test]
    fn creator_form_commit_dispatches_by_kind() {
        let tmp = TempDir::new("form");
        let mut store = EventStore::open(tmp.store_dir()).expect("open");
        let form = CreatorForm {
            title: "Via Form".to_string(),
            body: "corpo via form".to_string(),
        };
        let node = form
            .commit(CreatorKind::Note, &mut store, &tmp.lina(), DEFAULT_POS)
            .expect("commit note");
        let st = store.project().expect("project");
        assert_eq!(st.nodes.get(&node).map(|n| n.kind.as_str()), Some("Note"));
    }
}
