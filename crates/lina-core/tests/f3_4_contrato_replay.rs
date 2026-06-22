//! **F3-4 · Gate (f) — contrato `CodeChanged`/`BranchIntegrated`: replay idêntico & META no-op.**
//!
//! A fundação da onda (Maestro/A, commit `0d7f039`) fixou os 2 eventos aditivos em `events.rs`:
//! `CodeChanged{branch,paths,author_node,commit}` (sinal de mudança, spec 36 §2) e
//! `BranchIntegrated{branch,into,commit}` (prova de integração, §3). Esta suíte é a rede que prova
//! AGORA (o contrato já existe na `develop`) as propriedades que o gate (f) e a doutrina §1
//! exigem — sem depender das Trilhas A/B:
//!
//!   1. **Round-trip byte-a-byte:** o log com os 2 eventos reabre do disco idêntico (alicerce de
//!      toda projeção por replay — uma projeção só é byte-a-byte se sua ENTRADA é).
//!   2. **`kind()` canônico:** a tag persistida casa o nome da variante (chave do `from_record`).
//!   3. **`author_node` OBRIGATÓRIO (sem `serde(default)`):** um `CodeChanged` SEM `author_node`
//!      FALHA na desserialização — a metade testável-sem-fiação da regra-mãe (ADR 0007/0026): a
//!      forja por OMISSÃO é barrada pelo contrato. (A outra metade — o carimbo server-side na
//!      EMISSÃO, ignorando um `author_node` forjado no payload — é alvo diferencial da integração;
//!      ver `f3_4_alvos_diferenciais.md`.)
//!   4. **META no-op:** aplicar os 2 eventos NÃO toca `ProjectedState` (são projeção por replay
//!      externo, padrão `CostLedger`/`Mentality`; `events.rs:1778`). Garante o gate (f) "zero
//!      evento novo fora dos 2 declarados" e que replay antigo nunca muda de comportamento.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lina_core::{apply, DomainEvent, EventRecord, EventStore, ProjectedState};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// tmpdir único por (processo, THREAD, contador) — `thread::id` mata o flaky que envenena o gate
/// paralelo (despacho §3; molde de `f3_3_mentality_replay`/`router.rs`).
fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "lina-f34contrato-{tag}-{}-{tid}-{n}",
        std::process::id()
    ))
}

/// A forma serializada literal de cada registro — o "byte-a-byte" (`EventRecord` não é `PartialEq`;
/// comparamos o JSON canônico que vai/volta do disco).
fn as_json(events: &[EventRecord]) -> Vec<String> {
    events
        .iter()
        .map(|r| serde_json::to_string(r).expect("serializar registro"))
        .collect()
}

fn code_changed() -> DomainEvent {
    DomainEvent::CodeChanged {
        branch: "lina/f3-4-a".to_string(),
        paths: vec![
            "crates/lina-core/src/guard.rs".to_string(),
            "app/lina-gpui/src/bridge.rs".to_string(),
        ],
        author_node: "n-abc123".to_string(),
        commit: "deadbeefcafe".to_string(),
    }
}

fn branch_integrated() -> DomainEvent {
    DomainEvent::BranchIntegrated {
        branch: "lina/f3-4-a".to_string(),
        into: "develop".to_string(),
        commit: "feedface0001".to_string(),
    }
}

/// **(f1) O log com `CodeChanged`/`BranchIntegrated` reabre BYTE-A-BYTE do disco.** Persistimos uma
/// sequência realista (criação do Espaço + os 2 eventos de código), lemos, reabrimos uma NOVA
/// `EventStore` no mesmo dir e relemos: as duas sequências têm de ser idênticas em
/// `seq`/`kind`/`version`/`payload`. É o alicerce do gate (f) — sem entrada determinística, nenhuma
/// projeção por replay (módulo `code`, Trilha B) pode reconstruir byte-a-byte.
///
/// QUEBRA SE… a serialização dos 2 eventos não fosse estável (campo não-determinístico, ordem de
/// chave instável, perda no round-trip de disco) → as sequências divergiriam.
#[test]
fn contrato_reabre_byte_a_byte() {
    let tmp = unique("byte-a-byte");
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");

    let seq = [
        DomainEvent::WorkspaceCreated {
            name: "Projeto X".to_string(),
            focus_preset: String::new(),
        },
        code_changed(),
        branch_integrated(),
    ];

    let first = {
        let mut store = EventStore::open(&dir).expect("abrir");
        for ev in &seq {
            store.append(ev).expect("append");
        }
        store.events().expect("events")
    };
    let second = EventStore::open(&dir)
        .expect("reabrir")
        .events()
        .expect("events");

    assert_eq!(
        first.len(),
        seq.len(),
        "todos os eventos persistiram (não-vacuoso)"
    );
    assert_eq!(
        as_json(&first),
        as_json(&second),
        "o log com CodeChanged/BranchIntegrated reabre BYTE-A-BYTE (replay determinístico — gate f)"
    );
    // os 2 eventos de código estão de fato lá, com a tag canônica.
    let kinds: Vec<&str> = second.iter().map(|r| r.kind.as_str()).collect();
    assert!(
        kinds.contains(&"CodeChanged"),
        "CodeChanged persistido sob a tag canônica"
    );
    assert!(
        kinds.contains(&"BranchIntegrated"),
        "BranchIntegrated persistido sob a tag canônica"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// **(f2) `kind()` é a tag canônica** — a chave que o `from_record` usa para reconstruir a variante.
/// Se divergisse do que é persistido, o replay engoliria o evento (lição do fixture sem a tag
/// `event`). Controle direto do contrato.
#[test]
fn kind_canonico_das_variantes_novas() {
    assert_eq!(code_changed().kind(), "CodeChanged");
    assert_eq!(branch_integrated().kind(), "BranchIntegrated");
}

/// **(e1-parcial) `author_node` é OBRIGATÓRIO — forja por OMISSÃO barrada pelo contrato.** A
/// regra-mãe (ADR 0007/0026): `author_node` é binding server-side, jamais lido do payload de um
/// agente. A metade que o CONTRATO já garante (sem fiação de peer): o campo NÃO tem
/// `serde(default)`, então um `CodeChanged` sem ele FALHA na desserialização — um agente não pode
/// emitir um sinal de mudança "anônimo" ou omitir o autor para escapar da auditoria.
///
/// Construo o JSON a partir do evento REAL (sem hardcodar a tag), removo `author_node` e provo que
/// não desserializa. Controle positivo: o JSON COMPLETO volta a desserializar igual (não-vacuoso —
/// o objeto é válido; só a ausência do campo o quebra).
///
/// QUEBRA SE… `author_node` ganhasse `#[serde(default)]` (regressão da doutrina) → a desserialização
/// sem o campo passaria a aceitar com `author_node: ""`, abrindo o sinal anônimo.
#[test]
fn author_node_obrigatorio_forja_por_omissao_rejeitada() {
    let ev = code_changed();
    let full = serde_json::to_value(&ev).expect("serializar CodeChanged");

    // controle positivo: o objeto íntegro desserializa de volta no MESMO evento.
    let round: DomainEvent = serde_json::from_value(full.clone()).expect("round-trip íntegro");
    assert_eq!(
        round, ev,
        "o CodeChanged completo desserializa byte-a-byte (não-vacuoso)"
    );

    // alvo: remover `author_node` (forja por omissão) deve QUEBRAR a desserialização.
    let mut sem_autor = full;
    sem_autor
        .as_object_mut()
        .expect("payload é objeto")
        .remove("author_node")
        .expect("author_node estava presente");
    let res: Result<DomainEvent, _> = serde_json::from_value(sem_autor);
    assert!(
        res.is_err(),
        "CodeChanged SEM author_node deve falhar — campo obrigatório (forja por omissão barrada, ADR 0007)"
    );
}

/// **(f3) META no-op: `CodeChanged`/`BranchIntegrated` NÃO tocam `ProjectedState`.** São META
/// (projeção por replay externo, módulo `code`) — exatamente como `CostLedger`/`Mentality`. Provo
/// partindo de um estado NÃO-trivial (criação do Espaço seta `workspace_name`) e mostro que aplicar
/// os 2 eventos de código deixa a projeção idêntica.
///
/// Não-vacuoso por construção: o sub-assert de controle prova que `apply` MUDA o estado para um
/// evento mutante conhecido (`WorkspaceCreated`) — então a igualdade pós-código não é "apply nunca
/// faz nada". QUEBRA SE… algum dos 2 eventos saísse da lista no-op de `apply` e passasse a mutar a
/// projeção → o `assert_eq!(snapshot)` falharia.
#[test]
fn apply_no_op_nao_toca_projecao() {
    let mut estado = ProjectedState::default();

    // controle de não-vacuosidade: um evento MUTANTE conhecido muda a projeção.
    apply(
        &mut estado,
        &DomainEvent::WorkspaceCreated {
            name: "Projeto X".to_string(),
            focus_preset: String::new(),
        },
    );
    assert_eq!(
        estado.workspace_name.as_deref(),
        Some("Projeto X"),
        "controle: apply de WorkspaceCreated MUTA a projeção (não-vacuoso)"
    );

    let snapshot = estado.clone();

    // alvo: os 2 eventos de código são META → projeção inalterada.
    apply(&mut estado, &code_changed());
    apply(&mut estado, &branch_integrated());
    assert_eq!(
        estado, snapshot,
        "CodeChanged/BranchIntegrated são META: NÃO tocam ProjectedState (projeção por replay externo)"
    );
}
