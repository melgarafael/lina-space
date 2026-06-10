//! Gate F1-4-1 — Multi-workspace de verdade: Espaço = projeto, tudo event-sourced.
//!
//! Fatia CORE headless (despacho `r1-dados.md`; fonte integral `ondas-2-4.md` 163-181).
//! Layout decidido no mini-ADR da entrega: **um store por Espaço** em
//! `<root>/.lina/events` (ADR 0022 §6); o registry global é PONTEIRO re-derivável,
//! nunca autoridade. Critérios:
//!
//! 1. Espaços A e B com times distintos → projeção escopada lista SÓ os nós do ativo.
//! 2. Broadcast em A não chega a B — o LOG de B não contém o evento.
//! 3. Crash simulado (drop sem flush/reabertura) → replay restaura A e B íntegros,
//!    zero vazamento entre Espaços.
//! 4. Replay determinístico por Espaço: apagar projeções → re-derivar → byte-a-byte.
//! 5. Abertura concorrente dos stores (2 conexões/2 Espaços) sem "database is locked"
//!    (regressão da lição W5).
//! 6. Seam ÚNICO de cwd: `default_cwd` → novo nó nasce nele (compartilhamento
//!    INTENCIONAL — ADR 0026 em rascunho: N nós no mesmo cwd é o padrão do produto);
//!    sem → fallback dir gerenciado virgem `n-<key>` (ADR 0022 §4); trocar afeta só
//!    nós FUTUROS; persiste no log e re-deriva no replay.

use std::collections::BTreeSet;

use lina_core::{
    can_create_workspace, resolve_spawn_cwd, DomainEvent, EventStore, HeadlessUiHost, LicenseTier,
    NodeId, ResolvedCwd, Workspace, WorkspaceError, WorkspaceRegistry,
};

// ───────────────────────────── helpers (idioma dos gates) ─────────────────────────────

/// Diretório temporário único; removido no `Drop` (idioma de `gate_onda0.rs`).
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-gate-f141-{tag}-{}-{}",
            std::process::id(),
            NodeId::now_v7()
        ));
        std::fs::create_dir_all(&p).expect("criar tempdir");
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Sobrescreve o meio do `.db` com lixo (lição Codex #21750 — idioma de `gate_onda0.rs`).
/// Simula a perda das PROJEÇÕES (events.db + tabela snapshots): o que sobra é o
/// espelho JSONL append-only, a autoridade de disaster-recovery.
fn corrupt_middle(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};
    let len = std::fs::metadata(path).expect("metadata").len();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("abrir p/ corromper");
    let start = len / 4;
    let span = (len / 2).max(1);
    f.seek(SeekFrom::Start(start)).expect("seek");
    f.write_all(&vec![0xEE_u8; span as usize])
        .expect("escrever lixo");
    f.flush().expect("flush");
}

/// Admissão canônica MÍNIMA de um nó (sequência do ADR 0022 §2, headless): o cwd do
/// `TerminalSpawned` é SEMPRE o resolvido pelo seam único — é o que o critério 6 observa.
fn admit(ws: &mut Workspace, name: &str, resolved: &ResolvedCwd) -> NodeId {
    let node = NodeId::now_v7();
    let store = ws.store_mut();
    store
        .append(&DomainEvent::NodeAdded {
            node,
            kind: "Terminal".into(),
            x: 0.0,
            y: 0.0,
            requested_by: None,
        })
        .expect("NodeAdded");
    store
        .append(&DomainEvent::NodeRenamed {
            node,
            name: name.into(),
        })
        .expect("NodeRenamed");
    store
        .append(&DomainEvent::TerminalSpawned {
            node,
            cli: "claude".into(),
            cwd: Some(resolved.path().display().to_string()),
        })
        .expect("TerminalSpawned");
    store
        .append(&DomainEvent::NodeRoleAssigned {
            node,
            role: "terminal".into(),
        })
        .expect("NodeRoleAssigned");
    node
}

/// Nomes projetados do Espaço (conjunto ordenado — comparação estável).
fn names(ws: &Workspace) -> BTreeSet<String> {
    ws.project()
        .expect("project")
        .nodes
        .values()
        .filter_map(|n| n.name.clone())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

// ───────────────────────────── critério 1 ─────────────────────────────

/// Espaços A e B com times DISTINTOS: a projeção escopada (o store do Espaço focado
/// pelo registry) lista SÓ os nós daquele Espaço — nunca os do vizinho.
#[test]
fn c1_projecao_escopada_lista_so_os_nos_do_espaco_ativo() {
    let tmp = TempDir::new("c1");
    let root_a = tmp.path().join("espaco-a");
    let root_b = tmp.path().join("espaco-b");

    let mut a = Workspace::create(&root_a, "Espaço A", "dev_app", None).expect("criar A");
    let mut b = Workspace::create(&root_b, "Espaço B", "research_content", None).expect("criar B");

    let cwd_a = resolve_spawn_cwd(None, &root_a, &NodeId::now_v7().to_string());
    admit(&mut a, "Maestro-A", &cwd_a);
    admit(&mut a, "Dev-A", &cwd_a);
    let cwd_b = resolve_spawn_cwd(None, &root_b, &NodeId::now_v7().to_string());
    admit(&mut b, "QA-B", &cwd_b);

    // Registry global: PONTEIRO para os Espaços conhecidos; o foco decide o "ativo".
    let mut reg =
        WorkspaceRegistry::load(tmp.path().join("global/workspaces.json")).expect("registry vazio");
    reg.upsert(a.registry_entry().expect("entry A"));
    reg.upsert(b.registry_entry().expect("entry B"));

    let id_a = a.registry_entry().expect("entry A").id;
    let id_b = b.registry_entry().expect("entry B").id;

    assert!(reg.set_focus(&id_a, 1_000), "focar A");
    let focused = reg.focused().expect("há foco").clone();
    let ativo = Workspace::open(&focused.path).expect("abrir ativo");
    assert_eq!(
        names(&ativo),
        set(&["Maestro-A", "Dev-A"]),
        "projeção do ativo (A) lista SÓ o time de A"
    );

    assert!(reg.set_focus(&id_b, 2_000), "focar B");
    let focused = reg.focused().expect("há foco").clone();
    let ativo = Workspace::open(&focused.path).expect("abrir ativo");
    assert_eq!(
        names(&ativo),
        set(&["QA-B"]),
        "projeção do ativo (B) lista SÓ o time de B"
    );
}

// ───────────────────────────── critério 2 ─────────────────────────────

/// Broadcast em A NÃO chega a B — provado pelo LOG (não por ausência visual): o
/// event log de B não contém o evento nem o id da mensagem; o de A contém (controle
/// não-vácuo: se os stores fossem unificados, a 1ª asserção falharia).
#[test]
fn c2_broadcast_em_a_nao_aparece_no_log_de_b() {
    let tmp = TempDir::new("c2");
    let mut a =
        Workspace::create(tmp.path().join("espaco-a"), "Espaço A", "", None).expect("criar A");
    let mut b =
        Workspace::create(tmp.path().join("espaco-b"), "Espaço B", "", None).expect("criar B");

    // Isolamento físico é pré-condição estrutural: stores em diretórios distintos.
    assert_ne!(
        a.store_mut().db_path(),
        b.store_mut().db_path(),
        "um store POR Espaço (mini-ADR opção b)"
    );

    let from = NodeId::now_v7();
    let msg_id = "msg_broadcast_f141";
    a.store_mut()
        .append(&DomainEvent::BusMessageSent {
            id: msg_id.into(),
            from,
            to: "*".into(),
        })
        .expect("BusMessageSent em A");
    a.store_mut()
        .append(&DomainEvent::MessageRouted {
            id: msg_id.into(),
            from,
            to: "*".into(),
            intent: "broadcast".into(),
            root_cause_id: String::new(),
            hops: 0,
            to_node: None,
        })
        .expect("MessageRouted em A");

    // Controle (não-vácuo): o log de A CONTÉM o broadcast.
    let log_a = a.store_mut().events().expect("events A");
    assert!(
        log_a
            .iter()
            .any(|r| r.kind == "MessageRouted" && r.payload["id"] == msg_id),
        "log de A contém o broadcast roteado"
    );

    // Critério: o log de B NÃO contém o evento — nem o kind, nem o id em payload algum.
    let log_b = b.store_mut().events().expect("events B");
    assert!(
        log_b
            .iter()
            .all(|r| r.kind != "MessageRouted" && r.kind != "BusMessageSent"),
        "log de B sem eventos de bus do broadcast de A"
    );
    assert!(
        log_b
            .iter()
            .all(|r| !r.payload.to_string().contains(msg_id)),
        "log de B sem o id da mensagem em payload algum"
    );
}

// ───────────────────────────── critério 3 ─────────────────────────────

/// `kill -9` simulado (drop dos handles SEM snapshot/flush extra) com A aberto →
/// reabertura via replay restaura A íntegro; B íntegro; zero vazamento de estado.
#[test]
fn c3_crash_sem_flush_replay_restaura_a_e_b_sem_vazamento() {
    let tmp = TempDir::new("c3");
    let root_a = tmp.path().join("espaco-a");
    let root_b = tmp.path().join("espaco-b");

    let (fp_a, fp_b) = {
        let mut a = Workspace::create(&root_a, "Espaço A", "dev_app", None).expect("criar A");
        let mut b = Workspace::create(&root_b, "Espaço B", "", None).expect("criar B");
        let cwd_a = resolve_spawn_cwd(None, &root_a, &NodeId::now_v7().to_string());
        admit(&mut a, "Maestro-A", &cwd_a);
        admit(&mut a, "Dev-A", &cwd_a);
        let cwd_b = resolve_spawn_cwd(None, &root_b, &NodeId::now_v7().to_string());
        admit(&mut b, "QA-B", &cwd_b);
        (
            a.project().expect("project A").fingerprint(),
            b.project().expect("project B").fingerprint(),
        )
        // drop SEM take_snapshot: o que sobrevive é só o que o append durável gravou.
    };

    let mut ui = HeadlessUiHost::new();
    let a = Workspace::open_or_recover(&root_a, &mut ui).expect("reabrir A");
    let b = Workspace::open_or_recover(&root_b, &mut ui).expect("reabrir B");

    assert_eq!(
        a.project().expect("project A").fingerprint(),
        fp_a,
        "A íntegro após crash (replay reproduz o estado pré-crash)"
    );
    assert_eq!(
        b.project().expect("project B").fingerprint(),
        fp_b,
        "B íntegro após crash"
    );

    // Zero vazamento: o time de um Espaço não aparece no outro.
    assert_eq!(names(&a), set(&["Maestro-A", "Dev-A"]));
    assert_eq!(names(&b), set(&["QA-B"]));
    assert!(
        names(&a).intersection(&names(&b)).next().is_none(),
        "interseção vazia entre os times projetados"
    );
}

// ───────────────────────────── critério 4 ─────────────────────────────

/// Replay determinístico POR Espaço: apagar as projeções (events.db corrompido +
/// snapshots perdidos) e re-derivar do espelho JSONL reproduz o estado byte-a-byte —
/// e a falha de A não muda um byte do estado de B (isolamento físico de falha).
#[test]
fn c4_replay_por_espaco_re_deriva_estado_byte_a_byte() {
    let tmp = TempDir::new("c4");
    let root_a = tmp.path().join("espaco-a");
    let root_b = tmp.path().join("espaco-b");

    let mut a = Workspace::create(&root_a, "Espaço A", "dev_app", None).expect("criar A");
    let mut b = Workspace::create(&root_b, "Espaço B", "", None).expect("criar B");
    let cwd_a = resolve_spawn_cwd(None, &root_a, &NodeId::now_v7().to_string());
    admit(&mut a, "Maestro-A", &cwd_a);
    a.store_mut().take_snapshot().expect("snapshot A");
    admit(&mut a, "Dev-A", &cwd_a); // eventos APÓS o snapshot (replay = snapshot + cauda)
    let cwd_b = resolve_spawn_cwd(None, &root_b, &NodeId::now_v7().to_string());
    admit(&mut b, "QA-B", &cwd_b);

    let bytes_a = serde_json::to_vec(&a.project().expect("project A")).expect("canonical A");
    let bytes_b = serde_json::to_vec(&b.project().expect("project B")).expect("canonical B");
    let db_a = a.store_mut().db_path().to_path_buf();
    drop(a);

    // "Apagar projeções": o .db (events + snapshots materializados) vira lixo;
    // open_or_recover preserva o corrompido e RE-DERIVA tudo do log.jsonl.
    corrupt_middle(&db_a);
    let mut ui = HeadlessUiHost::new();
    let a2 = Workspace::open_or_recover(&root_a, &mut ui).expect("re-derivar A");
    let bytes_a2 = serde_json::to_vec(&a2.project().expect("project A2")).expect("canonical A2");
    assert_eq!(bytes_a, bytes_a2, "estado de A re-derivado byte-a-byte");

    // B não participa da falha de A: REABERTO do disco, re-projeta byte-a-byte
    // (reabrir evita a asserção tautológica sobre o mesmo handle em memória).
    drop(b);
    let b2 = Workspace::open(&root_b).expect("reabrir B");
    let bytes_b2 = serde_json::to_vec(&b2.project().expect("project B2")).expect("canonical B2");
    assert_eq!(bytes_b, bytes_b2, "B intocado pela re-derivação de A");
}

// ───────────────────────────── critério 5 ─────────────────────────────

/// Regressão da lição W5 no layout multi-Espaço: aberturas/escritas CONCORRENTES —
/// 2 conexões no MESMO Espaço (o caso que motivou busy_timeout+retry) E 2 Espaços
/// em paralelo — sem "database is locked", sem perda de evento.
#[test]
fn c5_abertura_concorrente_de_stores_sem_database_is_locked() {
    let tmp = TempDir::new("c5");
    let root_a = tmp.path().join("espaco-a");
    let root_b = tmp.path().join("espaco-b");
    Workspace::create(&root_a, "Espaço A", "", None).expect("criar A");
    Workspace::create(&root_b, "Espaço B", "", None).expect("criar B");

    // Thread 1: 2º handle do MESMO Espaço A (2 conexões → disputa de lock real).
    let t_a = std::thread::spawn({
        let root_a = root_a.clone();
        move || {
            let mut a2 = Workspace::open(&root_a).expect("handle A2");
            for i in 0..50u32 {
                a2.store_mut()
                    .append(&DomainEvent::TokenUsageReported {
                        node: format!("@A2-{i}"),
                        tokens: u64::from(i),
                    })
                    .expect("append A2");
            }
        }
    });
    // Thread 2: Espaço B inteiro em paralelo (abertura concorrente de OUTRO store).
    let t_b = std::thread::spawn({
        let root_b = root_b.clone();
        move || {
            let mut b = Workspace::open(&root_b).expect("handle B");
            for i in 0..50u32 {
                b.store_mut()
                    .append(&DomainEvent::TokenUsageReported {
                        node: format!("@B-{i}"),
                        tokens: u64::from(i),
                    })
                    .expect("append B");
            }
        }
    });
    // Main: 1º handle de A, escrevendo em paralelo com o 2º handle.
    let mut a1 = Workspace::open(&root_a).expect("handle A1");
    for i in 0..50u32 {
        a1.store_mut()
            .append(&DomainEvent::TokenUsageReported {
                node: format!("@A1-{i}"),
                tokens: u64::from(i),
            })
            .expect("append A1");
    }
    t_a.join().expect("thread A2");
    t_b.join().expect("thread B");

    let conta = |root: &std::path::Path, esperado: usize, tag: &str| {
        let mut ws = Workspace::open(root).expect("reabrir");
        let n = ws
            .store_mut()
            .events()
            .expect("events")
            .iter()
            .filter(|r| r.kind == "TokenUsageReported")
            .count();
        assert_eq!(n, esperado, "{tag}: zero perda sob concorrência");
    };
    conta(&root_a, 100, "Espaço A (2 conexões)");
    conta(&root_b, 50, "Espaço B");
}

// ───────────────────────────── critério 6 ─────────────────────────────

/// `default_cwd` definido na criação → novo nó NASCE nele (o `TerminalSpawned.cwd`
/// reflete) — inclusive N nós no MESMO cwd (compartilhamento intencional; o seam
/// NUNCA usa cwd como chave de identidade — pré-aviso BUG-1/ADR 0026).
#[test]
fn c6a_default_cwd_definido_novo_no_nasce_nele_mesmo_compartilhado() {
    let tmp = TempDir::new("c6a");
    let root = tmp.path().join("espaco-a");
    let projeto = tmp.path().join("projeto-x");
    let mut ws = Workspace::create(&root, "Espaço A", "dev_app", Some(&projeto)).expect("criar");

    let state = ws.project().expect("project");
    assert_eq!(
        state.default_cwd.as_deref(),
        Some(projeto.display().to_string().as_str()),
        "default_cwd escolhido na criação está projetado"
    );

    // O seam ÚNICO resolve para o default_cwd — para QUALQUER nó novo.
    let r1 = resolve_spawn_cwd(
        state.default_cwd.as_deref().map(std::path::Path::new),
        &root,
        &NodeId::now_v7().to_string(),
    );
    let r2 = resolve_spawn_cwd(
        state.default_cwd.as_deref().map(std::path::Path::new),
        &root,
        &NodeId::now_v7().to_string(),
    );
    assert!(matches!(r1, ResolvedCwd::WorkspaceDefault(ref p) if *p == projeto));
    assert_eq!(
        r1.path(),
        r2.path(),
        "N nós compartilham o MESMO default_cwd — sem deduplicação, sem erro"
    );

    let n1 = admit(&mut ws, "Terminal-1", &r1);
    let n2 = admit(&mut ws, "Agente-2", &r2);
    let state = ws.project().expect("project");
    let cwd_esperado = Some(projeto.display().to_string());
    assert_eq!(
        state.nodes[&n1].cwd, cwd_esperado,
        "nó 1 nasceu no default_cwd"
    );
    assert_eq!(
        state.nodes[&n2].cwd, cwd_esperado,
        "nó 2 nasceu no default_cwd"
    );
}

/// SEM `default_cwd` → fallback do ADR 0022: dir gerenciado VIRGEM `n-<key>` sob a
/// raiz do Espaço (isola contra herança de slot reciclado) — único por nó.
#[test]
fn c6b_sem_default_cwd_fallback_dir_gerenciado_virgem() {
    let tmp = TempDir::new("c6b");
    let root = tmp.path().join("espaco-a");
    Workspace::create(&root, "Espaço A", "", None).expect("criar");

    let k1 = NodeId::now_v7().to_string();
    let k2 = NodeId::now_v7().to_string();
    let r1 = resolve_spawn_cwd(None, &root, &k1);
    let r2 = resolve_spawn_cwd(None, &root, &k2);

    assert!(
        matches!(r1, ResolvedCwd::ManagedVirgin(_)),
        "fallback gerenciado"
    );
    assert_eq!(
        r1.path(),
        root.join(format!("n-{k1}")),
        "layout `<root>/n-<key>`"
    );
    assert_ne!(
        r1.path(),
        r2.path(),
        "dir virgem é ÚNICO por nó (nunca reciclado)"
    );
}

/// Defesa em profundidade no seam (doutrina §7): `node_key` hostil (separadores com
/// `..` embutido) NUNCA escapa da raiz do Espaço — a key vira UM componente literal
/// sob `<root>/`. Hoje só o app gera a key (uuid), mas o seam não confia nisso.
#[test]
fn c6e_node_key_hostil_nao_escapa_da_raiz() {
    let tmp = TempDir::new("c6e");
    let root = tmp.path().join("espaco-a");
    for hostil in ["x/../../fora", "/etc/passwd", "..", "a/b"] {
        let r = resolve_spawn_cwd(None, &root, hostil);
        assert_eq!(
            r.path().parent(),
            Some(root.as_path()),
            "key {hostil:?} vira UM componente direto sob a raiz (sem subcaminho injetado)"
        );
    }
}

/// Trocar o `default_cwd` afeta SÓ nós futuros: quem já nasceu não migra
/// (F1-2-4: editar cwd de processo vivo NÃO entra).
#[test]
fn c6c_trocar_default_cwd_afeta_so_nos_futuros() {
    let tmp = TempDir::new("c6c");
    let root = tmp.path().join("espaco-a");
    let antigo = tmp.path().join("projeto-antigo");
    let novo = tmp.path().join("projeto-novo");
    let mut ws = Workspace::create(&root, "Espaço A", "", Some(&antigo)).expect("criar");

    let r_antigo = resolve_spawn_cwd(Some(&antigo), &root, &NodeId::now_v7().to_string());
    let n1 = admit(&mut ws, "Veterano", &r_antigo);

    ws.set_default_cwd(Some(&novo)).expect("trocar default_cwd");

    let state = ws.project().expect("project");
    let r_novo = resolve_spawn_cwd(
        state.default_cwd.as_deref().map(std::path::Path::new),
        &root,
        &NodeId::now_v7().to_string(),
    );
    let n2 = admit(&mut ws, "Novato", &r_novo);

    let state = ws.project().expect("project");
    assert_eq!(
        state.nodes[&n1].cwd,
        Some(antigo.display().to_string()),
        "nó vivo NÃO migra com a troca"
    );
    assert_eq!(
        state.nodes[&n2].cwd,
        Some(novo.display().to_string()),
        "nó futuro nasce no novo default_cwd"
    );
}

/// O `default_cwd` persiste no LOG (não em config paralela) e re-deriva no replay —
/// inclusive na re-derivação a partir do espelho JSONL puro.
#[test]
fn c6d_default_cwd_persiste_no_log_e_re_deriva_no_replay() {
    let tmp = TempDir::new("c6d");
    let root = tmp.path().join("espaco-a");
    let projeto = tmp.path().join("projeto-x");
    let db = {
        let mut ws = Workspace::create(&root, "Espaço A", "", None).expect("criar");
        ws.set_default_cwd(Some(&projeto)).expect("definir");
        ws.store_mut().db_path().to_path_buf()
    }; // drop sem snapshot

    let ws = Workspace::open(&root).expect("reabrir");
    assert_eq!(
        ws.project().expect("project").default_cwd,
        Some(projeto.display().to_string()),
        "re-derivado do log na reabertura"
    );
    drop(ws);

    corrupt_middle(&db);
    let mut ui = HeadlessUiHost::new();
    let ws = Workspace::open_or_recover(&root, &mut ui).expect("recuperar");
    assert_eq!(
        ws.project().expect("project").default_cwd,
        Some(projeto.display().to_string()),
        "re-derivado até do JSONL puro (projeções apagadas)"
    );
}

// ───────────────────────────── aditividade (invariante de eventos) ─────────────────────────────

/// Log ANTIGO (WorkspaceCreated v2, sem os eventos novos da F1-4-1) replaya intacto:
/// os campos novos da projeção degradam para os defaults — nunca quebram. E o
/// `WorkspaceDefaultCwdSet` com `cwd: ""` LIMPA o default (volta ao fallback).
#[test]
fn eventos_novos_sao_aditivos_log_antigo_replaya_intacto() {
    let tmp = TempDir::new("aditivo");
    let mut store = EventStore::open(tmp.path().join("espaco-legado/.lina/events")).expect("store");

    // Shape v2 REAL de um log antigo (payload tagged por `event`) — nada da F1-4-1.
    store
        .insert_raw(
            "WorkspaceCreated",
            2,
            serde_json::json!({
                "event": "WorkspaceCreated",
                "name": "Legado",
                "focus_preset": ""
            }),
        )
        .expect("WorkspaceCreated v2 legado");
    let state = store.project().expect("replay legado");
    assert_eq!(state.workspace_name.as_deref(), Some("Legado"));
    assert_eq!(
        state.workspace_id, None,
        "sem id atribuído → None (degrada honesto)"
    );
    assert_eq!(state.default_cwd, None, "sem default_cwd → None");
    assert!(!state.archived, "sem arquivamento → false");

    // Eventos novos aplicam... e o "" limpa o default_cwd (volta ao fallback ADR 0022).
    store
        .append(&DomainEvent::WorkspaceDefaultCwdSet {
            cwd: "/projeto/x".into(),
        })
        .expect("set");
    assert_eq!(
        store.project().expect("project").default_cwd.as_deref(),
        Some("/projeto/x")
    );
    store
        .append(&DomainEvent::WorkspaceDefaultCwdSet { cwd: String::new() })
        .expect("clear");
    assert_eq!(
        store.project().expect("project").default_cwd,
        None,
        "\"\" limpa"
    );
}

/// Snapshot ANTIGO (estado materializado SEM os campos novos da F1-4-1) desserializa
/// com os defaults — `project()` que parte de snapshot velho nunca quebra.
#[test]
fn snapshot_antigo_desserializa_com_defaults() {
    let json = r#"{"workspace_name":"Legado","nodes":{},"bus_messages":0}"#;
    let state: lina_core::ProjectedState =
        serde_json::from_str(json).expect("snapshot antigo desserializa");
    assert_eq!(state.workspace_id, None);
    assert_eq!(state.default_cwd, None);
    assert!(!state.archived);
}

// ───────────────────────────── gating free=1 / PRO=N ─────────────────────────────

/// Free = 1 Espaço; PRO = N (decisão do Maestro nesta fatia; a licença ed25519 é
/// outra story — aqui é o SEAM puro de gating). Arquivar libera a vaga.
#[test]
fn gating_free_1_pro_n_e_arquivado_libera_vaga() {
    assert!(
        can_create_workspace(LicenseTier::Free, 0).is_ok(),
        "free cria o 1º"
    );
    assert!(
        matches!(
            can_create_workspace(LicenseTier::Free, 1),
            Err(WorkspaceError::LimitReached { limit: 1 })
        ),
        "free NÃO cria o 2º (limite 1)"
    );
    assert!(
        can_create_workspace(LicenseTier::Pro, 1).is_ok(),
        "PRO cria o 2º"
    );
    assert!(
        can_create_workspace(LicenseTier::Pro, 50).is_ok(),
        "PRO = N"
    );

    // Contagem considera SÓ Espaços ativos: arquivar libera a vaga do free.
    let tmp = TempDir::new("gating");
    let root = tmp.path().join("espaco-a");
    let mut ws = Workspace::create(&root, "Espaço A", "", None).expect("criar");
    let mut reg =
        WorkspaceRegistry::load(tmp.path().join("global/workspaces.json")).expect("registry");
    reg.upsert(ws.registry_entry().expect("entry"));
    assert_eq!(reg.active_count(), 1);
    assert!(reg.can_create(LicenseTier::Free).is_err(), "free sem vaga");

    ws.archive().expect("arquivar é evento no log do Espaço");
    reg.upsert(ws.registry_entry().expect("entry re-derivada"));
    assert_eq!(reg.active_count(), 0, "arquivado sai da contagem ativa");
    assert!(reg.can_create(LicenseTier::Free).is_ok(), "vaga liberada");
}

// ───────────────────────────── registry = ponteiro ─────────────────────────────

/// O registry global é PONTEIRO re-derivável: persiste atômico, sobrevive a reload,
/// e quando corrompido NÃO brica o app — a entrada se reconstrói do store do Espaço
/// (o log é a autoridade; o registry nunca).
#[test]
fn registry_e_ponteiro_re_derivavel_do_store() {
    let tmp = TempDir::new("registry");
    let reg_path = tmp.path().join("global/workspaces.json");
    let root = tmp.path().join("espaco-a");
    let ws = Workspace::create(&root, "Espaço A", "dev_app", None).expect("criar");

    let entry = ws.registry_entry().expect("entry do store");
    assert_eq!(
        entry.name, "Espaço A",
        "nome vem do LOG, não de input do registry"
    );
    assert_eq!(entry.path, root);
    assert!(
        !entry.id.is_empty(),
        "id atribuído na criação (WorkspaceIdAssigned)"
    );

    let mut reg = WorkspaceRegistry::load(&reg_path).expect("registry novo (vazio)");
    reg.upsert(entry.clone());
    assert!(reg.set_focus(&entry.id, 42));
    reg.save().expect("save atômico");

    let reg2 = WorkspaceRegistry::load(&reg_path).expect("reload");
    assert_eq!(reg2.entries(), reg.entries(), "round-trip estável");
    assert_eq!(reg2.focused().map(|e| e.id.clone()), Some(entry.id.clone()));

    // Corrupção do PONTEIRO não pode ser silenciosa nem fatal: load falha ALTO...
    std::fs::write(&reg_path, b"{lixo nao-json").expect("corromper registry");
    assert!(
        WorkspaceRegistry::load(&reg_path).is_err(),
        "corrupção é visível"
    );

    // ...e a entrada se RE-DERIVA da autoridade (o store do Espaço).
    let rederived = WorkspaceRegistry::rederive_entry(&root).expect("re-derivar");
    assert_eq!(rederived.id, entry.id);
    assert_eq!(rederived.name, entry.name);
    assert_eq!(rederived.path, root);

    // Dir sem Espaço não vira entrada fantasma.
    assert!(
        WorkspaceRegistry::rederive_entry(tmp.path().join("nao-existe")).is_err(),
        "sem WorkspaceCreated no log → não é um Espaço"
    );
}

// ───────────────────────────── achados da revisão adversarial ─────────────────────────────

/// (revisão/bugs) `default_registry_path` segue o idioma do repo (HOME → USERPROFILE,
/// como `bridge.rs:3275`/`main.rs:3288`): sem HOME, o Windows típico ainda resolve o
/// `~/.lina/workspaces.json` canônico. Env mutado e RESTAURADO; suíte roda
/// `--test-threads=1` (regras da rodada) e nenhum outro teste deste binário lê HOME.
#[test]
fn registry_path_padrao_cai_para_userprofile_sem_home() {
    let home = std::env::var_os("HOME");
    std::env::remove_var("HOME");
    std::env::set_var("USERPROFILE", "/tmp/perfil-windows");
    let resolvido = lina_core::default_registry_path();
    match &home {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    std::env::remove_var("USERPROFILE");
    assert_eq!(
        resolvido,
        Some(std::path::PathBuf::from(
            "/tmp/perfil-windows/.lina/workspaces.json"
        )),
        "fallback USERPROFILE no idioma do repo"
    );
}

/// (revisão/bugs) Path não-UTF8 no `default_cwd` é RECUSADO visivelmente — nunca
/// mutilado em silêncio para U+FFFD num evento que apontaria para dir inexistente
/// (doutrina: degradação visível > mágica silenciosa). A validação roda ANTES de
/// qualquer append: a recusa não deixa Espaço meio-criado.
#[cfg(unix)]
#[test]
fn default_cwd_nao_utf8_e_recusado_visivelmente() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = TempDir::new("nonutf8");
    let root = tmp.path().join("espaco-a");
    let hostil = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/lina-\xff-x"));

    assert!(
        matches!(
            Workspace::create(&root, "Espaço A", "", Some(&hostil)),
            Err(WorkspaceError::NonUtf8Path { .. })
        ),
        "create recusa não-UTF8"
    );
    // A recusa veio ANTES de apender: o mesmo root ainda aceita criação limpa.
    let mut ws = Workspace::create(&root, "Espaço A", "", None).expect("criar limpo");
    assert!(
        matches!(
            ws.set_default_cwd(Some(&hostil)),
            Err(WorkspaceError::NonUtf8Path { .. })
        ),
        "set_default_cwd recusa não-UTF8"
    );
    assert_eq!(
        ws.project().expect("project").default_cwd,
        None,
        "nada foi apendado pela tentativa recusada"
    );
}

/// (revisão/bugs) `save` do registry FUNDE com o arquivo corrente (união por id):
/// dois escritores intercalados (app + CLI futuro) não perdem o Espaço um do outro —
/// um ponteiro que regride é um Espaço "sumido" para o leigo.
#[test]
fn registry_save_funde_com_o_arquivo_corrente_sem_perder_entradas() {
    let tmp = TempDir::new("merge");
    let reg_path = tmp.path().join("global/workspaces.json");
    let a = Workspace::create(tmp.path().join("espaco-a"), "A", "", None).expect("A");
    let b = Workspace::create(tmp.path().join("espaco-b"), "B", "", None).expect("B");
    let id_a = a.registry_entry().expect("entry A").id;
    let id_b = b.registry_entry().expect("entry B").id;

    let mut reg1 = WorkspaceRegistry::load(&reg_path).expect("reg1");
    let mut reg2 = WorkspaceRegistry::load(&reg_path).expect("reg2");
    reg2.upsert(b.registry_entry().expect("entry B"));
    reg2.save().expect("save 2");
    reg1.upsert(a.registry_entry().expect("entry A"));
    reg1.save().expect("save 1 — sem merge apagaria B");

    let reloaded = WorkspaceRegistry::load(&reg_path).expect("reload");
    let ids: BTreeSet<String> = reloaded.entries().iter().map(|e| e.id.clone()).collect();
    assert!(ids.contains(&id_a), "entrada do escritor 1 presente");
    assert!(
        ids.contains(&id_b),
        "entrada do escritor 2 NÃO pode sumir (last-writer-wins é proibido no ponteiro)"
    );
}

/// (revisão/red-team b) Ponteiro adulterado com JSON VÁLIDO (id mentiroso) não abre
/// o Espaço errado em silêncio: `open_verified` confere a identidade re-derivada do
/// store (a autoridade) contra a do ponteiro e falha ALTO no mismatch.
#[test]
fn ponteiro_adulterado_nao_abre_espaco_errado_em_silencio() {
    let tmp = TempDir::new("verify");
    let ws = Workspace::create(tmp.path().join("espaco-a"), "A", "", None).expect("A");
    let mut entry = ws.registry_entry().expect("entry");

    // Ponteiro honesto abre (controle positivo).
    assert!(
        Workspace::open_verified(&entry).is_ok(),
        "ponteiro honesto abre"
    );

    // Ponteiro mentindo o id → falha visível, nunca Espaço errado em silêncio.
    entry.id = "id-forjado".into();
    assert!(
        matches!(
            Workspace::open_verified(&entry),
            Err(WorkspaceError::PointerMismatch { .. })
        ),
        "mismatch ponteiro×store é erro ALTO"
    );
}

// ───────────────────────────── ciclo de vida ─────────────────────────────

/// Renomear e arquivar são EVENTOS no log do Espaço (nunca mutação de registry) —
/// re-derivam no replay como qualquer outro fato.
#[test]
fn ciclo_de_vida_renomear_arquivar_event_sourced() {
    let tmp = TempDir::new("ciclo");
    let root = tmp.path().join("espaco-a");
    {
        let mut ws = Workspace::create(&root, "Nome Velho", "", None).expect("criar");
        ws.rename("Nome Novo").expect("renomear");
        ws.archive().expect("arquivar");
    }
    let ws = Workspace::open(&root).expect("reabrir");
    let state = ws.project().expect("project");
    assert_eq!(
        state.workspace_name.as_deref(),
        Some("Nome Novo"),
        "rename replayado"
    );
    assert_eq!(
        state.plan.workspace, "Nome Novo",
        "cabeçalho do plano espelha o rename"
    );
    assert!(state.archived, "arquivamento replayado");
}
