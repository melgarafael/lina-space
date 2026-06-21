//! **F3-3 (gate h) — o aposentar-1-clique funciona de verdade (costura `belief.retire`→`BeliefRetired`).**
//!
//! A UI ("esquecer isso") dispara o gesto `belief.retire`; o `bridge.rs::drain_human_intents` o roteia
//! para `Router::retire_belief`, que emite `BeliefRetired{reason:"retired_by_human"}`. Esta suíte prova
//! que o método REAL faz a crença sumir da injeção (top-K) — **compilar ≠ funcionar**: o botão renderizar
//! não basta, o handler tem de existir e o efeito tem de ser observável por replay. Controle +/-: crença
//! viva É injetada; após aposentar, NÃO (mas não é deletada — inv #4); gesto vazio é inerte (sem órfão).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lina_core::mentality::{BeliefStatus, Mentality, PromotionPolicy};
use lina_core::{DomainEvent, EventStore, Mailbox, Router, RouterConfig, Supervisor};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// tmpdir por `thread::id` + contador — não pisca falso-vermelho no gate paralelo (lição CONF-HARNESS).
fn unique(tag: &str) -> PathBuf {
    let id = format!("{:?}", std::thread::current().id());
    let id = id.replace(['(', ')', ' ', 'T', 'h', 'r', 'e', 'a', 'd', 'I', 'd'], "");
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-f33-aposentar-{tag}-{id}-{n}"))
}

fn store_and_router(tag: &str) -> (PathBuf, EventStore, Router) {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let lina = tmp.join(".lina");
    let store = EventStore::open(lina.join("events")).expect("abrir event store");
    let sup = Arc::new(Supervisor::new());
    let router = Router::with_config(Arc::clone(&sup), Mailbox::new(&lina), RouterConfig::default());
    (tmp, store, router)
}

/// Estabelece uma crença por CONTAGEM (2 situações distintas) — vira regra do papel.
fn establish(store: &mut EventStore, belief_id: &str, role: &str) {
    for ev in [
        DomainEvent::BeliefProposed {
            belief_id: belief_id.to_string(),
            role: role.to_string(),
            statement: "use pnpm, não npm".to_string(),
            provenance: "teste".to_string(),
            untrusted_origin: false,
        },
        DomainEvent::BeliefReinforced {
            belief_id: belief_id.to_string(),
            situation_hash: "h1".to_string(),
            evidence: String::new(),
        },
        DomainEvent::BeliefReinforced {
            belief_id: belief_id.to_string(),
            situation_hash: "h2".to_string(),
            evidence: String::new(),
        },
    ] {
        store.append(&ev).expect("append");
    }
}

fn count_retired(store: &EventStore) -> usize {
    store
        .events()
        .expect("events")
        .iter()
        .filter(|r| r.kind == "BeliefRetired")
        .count()
}

#[test]
fn aposentar_remove_a_crenca_da_injecao_pelo_caminho_real() {
    let (tmp, mut store, mut router) = store_and_router("efeito");
    establish(&mut store, "b-1", "BACKEND");

    // Controle +: a crença estabelecida É injetável ANTES de aposentar.
    let before = Mentality::replay(&store, PromotionPolicy::default()).expect("replay");
    assert!(
        before
            .top_k_for_role("BACKEND", 5, 0)
            .iter()
            .any(|b| b.belief_id == "b-1" && matches!(b.status, BeliefStatus::Established)),
        "pré: a crença estabelecida deve estar injetável"
    );

    // Gesto humano REAL: aposentar pelo método que o bridge chama (push_human_intent→retire_belief).
    assert!(
        router.retire_belief("b-1", &mut store).expect("retire"),
        "retire_belief devolve true ao aposentar"
    );
    assert_eq!(count_retired(&store), 1, "exatamente 1 BeliefRetired emitido");

    // Controle -: após aposentar, a crença SOME da injeção — mas NÃO foi deletada (inv #4).
    let after = Mentality::replay(&store, PromotionPolicy::default()).expect("replay 2");
    assert!(
        !after
            .top_k_for_role("BACKEND", 5, 0)
            .iter()
            .any(|b| b.belief_id == "b-1"),
        "pós: a crença aposentada NÃO é injetada"
    );
    assert!(
        matches!(
            after.belief("b-1").map(|b| &b.status),
            Some(BeliefStatus::Retired { .. })
        ),
        "a crença existe como Retired (não deletada, reconstruível por replay)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn gesto_vazio_e_inerte_nao_loga_evento_orfao() {
    let (tmp, mut store, mut router) = store_and_router("vazio");
    establish(&mut store, "b-1", "BACKEND");
    let before = count_retired(&store);

    // belief_id vazio/whitespace → Ok(false), nenhum evento órfão no log.
    assert!(
        !router.retire_belief("", &mut store).expect("retire vazio"),
        "gesto vazio devolve false"
    );
    assert!(
        !router.retire_belief("   ", &mut store).expect("retire whitespace"),
        "gesto só-espaços devolve false"
    );
    assert_eq!(
        count_retired(&store),
        before,
        "gesto malformado NÃO emite BeliefRetired órfão"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
