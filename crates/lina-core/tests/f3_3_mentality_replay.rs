//! **F3-3 · Mentality — replay idêntico & append-only (gate f), no nível do EVENT LOG.**
//!
//! Gate (f) da onda-f3-3: *"a projeção `Mentality(papel)` reconstrói byte-a-byte por replay; `retired`
//! não injeta; crença NUNCA é deletada (rebaixada/aposentada)"* (inv #4). A PROJEÇÃO em si é o módulo
//! `mentality.rs` (M-PROMO/Terminal I — ainda stub), então o veredito byte-a-byte da projeção é o ALVO
//! diferencial a fechar quando I entregar (ver nota no fim). O que esta suíte fixa AGORA é o ALICERCE
//! sobre o qual aquela garantia repousa e que JÁ existe: o event log é **append-only e determinístico**
//! para os 6 eventos de crença — reabri-lo do disco devolve a MESMA sequência, e `BeliefChallenged`/
//! `BeliefRetired` são ADIÇÕES (rebaixam), nunca deleções. Se este alicerce falhasse, nenhuma projeção
//! por replay poderia ser byte-a-byte.
//!
//! Eval-first (spec 35 §7): controle POSITIVO (a sequência completa sobrevive ao reopen) E NEGATIVO
//! (aposentar/contestar NÃO some com os eventos anteriores — a contagem por tipo prova).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lina_core::{DomainEvent, EventRecord, EventStore};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lina-f33replay-{tag}-{}-{:?}-{n}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// O ciclo de vida COMPLETO de uma crença (spec 35 §3): nasce, é reforçada em 2 situações distintas,
/// estabelece, é contestada (rebaixa) e por fim aposentada. Toda transição é um evento ADITIVO.
fn full_lifecycle(belief_id: &str, role: &str) -> Vec<DomainEvent> {
    vec![
        DomainEvent::CorrectionObserved {
            role: role.to_string(),
            summary: "use pnpm, não npm".to_string(),
            refs: vec!["ev-1".to_string()],
        },
        DomainEvent::BeliefProposed {
            belief_id: belief_id.to_string(),
            role: role.to_string(),
            statement: "Use pnpm. Why: o monorepo deste papel usa workspaces pnpm.".to_string(),
            provenance: "correção da sessão de 21/06".to_string(),
            untrusted_origin: false,
        },
        DomainEvent::BeliefReinforced {
            belief_id: belief_id.to_string(),
            situation_hash: "situacao-A".to_string(),
            evidence: "rodou pnpm install".to_string(),
        },
        DomainEvent::BeliefReinforced {
            belief_id: belief_id.to_string(),
            situation_hash: "situacao-B".to_string(),
            evidence: "rodou pnpm add".to_string(),
        },
        DomainEvent::BeliefEstablished {
            belief_id: belief_id.to_string(),
        },
        DomainEvent::BeliefChallenged {
            belief_id: belief_id.to_string(),
            evidence: "usuário corrigiu de novo".to_string(),
        },
        DomainEvent::BeliefRetired {
            belief_id: belief_id.to_string(),
            reason: "retired_by_human".to_string(),
        },
    ]
}

fn count(events: &[EventRecord], kind: &str) -> usize {
    events.iter().filter(|r| r.kind == kind).count()
}

/// A forma serializada de cada registro — o "byte-a-byte" literal (`EventRecord` não é `PartialEq`,
/// então comparamos o que vai/volta do disco: o JSON canônico de cada evento).
fn as_json(events: &[EventRecord]) -> Vec<String> {
    events
        .iter()
        .map(|r| serde_json::to_string(r).expect("serializar registro"))
        .collect()
}

/// **(f1) Replay determinístico: o log de crença reabre do disco BYTE-A-BYTE.** Persistimos o ciclo de
/// vida completo, lemos a sequência, reabrimos uma NOVA `EventStore` no mesmo diretório e relemos: as
/// duas sequências têm de ser idênticas em `seq`/`kind`/`version`/`payload`. É o alicerce de toda
/// projeção por replay (uma projeção só pode ser byte-a-byte se sua ENTRADA é byte-a-byte).
///
/// QUEBRA SE… a serialização de um evento de crença não fosse estável (ex.: campo não-determinístico,
/// ordem de chave instável, perda no round-trip de disco) → as sequências divergiriam.
#[test]
fn f1_log_de_crenca_reabre_byte_a_byte() {
    let tmp = unique("byte-a-byte");
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");

    let lifecycle = full_lifecycle("b-pnpm", "backend");
    let first = {
        let mut store = EventStore::open(&dir).expect("abrir");
        for ev in &lifecycle {
            store.append(ev).expect("append");
        }
        store.events().expect("events")
    };
    // reabre do DISCO (nova instância) — o replay tem de reproduzir a mesma sequência.
    let second = EventStore::open(&dir)
        .expect("reabrir")
        .events()
        .expect("events");

    assert_eq!(
        first.len(),
        lifecycle.len(),
        "todos os eventos do ciclo de vida persistiram (não-vacuoso)"
    );
    assert_eq!(
        as_json(&first),
        as_json(&second),
        "o log de crença reabre BYTE-A-BYTE do disco — replay determinístico (alicerce do gate f)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// **(f2) Crença NUNCA é deletada: contestar/aposentar são ADIÇÕES.** Após o ciclo completo (que inclui
/// `BeliefChallenged` e `BeliefRetired`), os eventos ANTERIORES (`BeliefProposed`, os 2
/// `BeliefReinforced`, `BeliefEstablished`) continuam TODOS no log — rebaixar/aposentar adiciona um
/// evento, nunca remove o histórico. É o que garante que o painel possa narrar "esta crença foi
/// aposentada" reconstruindo a vida inteira por replay (inv #4).
///
/// QUEBRA SE… `BeliefRetired`/`BeliefChallenged` apagasse/compactasse eventos anteriores (um store que
/// "esquece") → a contagem dos tipos anteriores cairia para 0 e os asserts falhariam.
#[test]
fn f2_aposentar_e_adicao_nunca_delecao() {
    let tmp = unique("nunca-deleta");
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");

    let mut store = EventStore::open(&dir).expect("abrir");
    for ev in &full_lifecycle("b-pnpm", "backend") {
        store.append(ev).expect("append");
    }
    let events = store.events().expect("events");

    // o histórico ANTERIOR à aposentadoria continua intacto (não-vacuoso: cada tipo existe).
    assert_eq!(
        count(&events, "BeliefProposed"),
        1,
        "a proposta original permanece"
    );
    assert_eq!(
        count(&events, "BeliefReinforced"),
        2,
        "os 2 reforços permanecem (nada foi compactado)"
    );
    assert_eq!(
        count(&events, "BeliefEstablished"),
        1,
        "o estabelecimento permanece — o histórico é reconstruível por replay"
    );
    // controle POSITIVO da semântica de aposentadoria: ela é uma ADIÇÃO, presente no log.
    assert_eq!(
        count(&events, "BeliefChallenged"),
        1,
        "contestar é um evento ADICIONADO (rebaixa, não deleta)"
    );
    assert_eq!(
        count(&events, "BeliefRetired"),
        1,
        "aposentar é um evento ADICIONADO — crença sai da injeção, não do log"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────────
// ALVO DIFERENCIAL (fecha com M-PROMO/Terminal I — `mentality.rs`):
//   gate (f) completo = `mentality::project(role)` reconstrói a MESMA Mentality byte-a-byte por replay,
//   uma crença `retired` NÃO entra na seleção top-K injetada, e a crença permanece reconstruível.
//   Quando a API de projeção existir, este arquivo ganha:
//     assert_eq!(project_mentality(&events, "backend"), project_mentality(&reopened, "backend"));
//     assert!(!injected_beliefs(&events, "backend").iter().any(|b| b.id == "b-pnpm")); // retired fora
//   Hoje BLOQUEADO na API (não referencio símbolo inexistente — quebraria a compilação do crate).
// ─────────────────────────────────────────────────────────────────────────────────────────────────────
