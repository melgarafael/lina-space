//! **F3-3 · Mentality — o CENÁRIO BINÁRIO (gate a, o coração) + promoção/cap event-sourced (b/c).**
//!
//! Camada de QA INDEPENDENTE da unit-suite do implementador (`mentality.rs`, Terminal I): aquela prova
//! a MECÂNICA da projeção (de `from_records` sobre vecs sintéticos); esta prova o CENÁRIO end-to-end
//! pelo caminho REAL de persistência (`EventStore` no disco → `Mentality::replay`) e o gate-mãe da onda.
//!
//! **Gate (a) — "corrigiu uma vez, nunca repete" (spec 35 §5; onda-f3-3 gate a):** Sessão 1 o usuário
//! corrige um PAPEL; Sessão 2 um terminal NOVO **do mesmo papel** já carrega a crença. O proxy
//! OBSERVÁVEL no CORE é a SELEÇÃO que alimenta o injetor (`top_k_for_role`): a crença estabelecida do
//! papel entra; a de OUTRO papel não (isolamento por papel = o controle negativo). A renderização na
//! doutrina do spawn é do M-INJETOR (bridge.rs, app/lina-gpui) e a prova na tela é do fundador (gate h,
//! gpui não roda headless) — aqui fixamos o que decide o conteúdo daquela injeção.
//!
//! Eval-first (spec 35 §7): cada teste com controle POSITIVO E NEGATIVO. Usa só a API GREEN de I
//! (`from_records`/`replay`/`top_k_for_role`/`belief`/`beliefs_for_role`); `promotable`/`is_capturable`
//! são stubs RED de I — cobertos pela unit-suite dele, fora desta camada.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lina_core::mentality::{BeliefStatus, Mentality, PromotionPolicy};
use lina_core::{DomainEvent, EventStore};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lina-f33cen-{tag}-{}-{:?}-{n}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// Um event store headless num tmp único (removido no fim de cada teste).
fn store(tag: &str) -> (PathBuf, EventStore) {
    let tmp = unique(tag);
    let _ = std::fs::remove_dir_all(&tmp);
    let s = EventStore::open(tmp.join(".lina/events")).expect("abrir event store");
    (tmp, s)
}

/// "O usuário corrigiu o papel e a lição se firmou": grava o ciclo que ESTABELECE uma crença — a
/// correção observada, a proposta, e DOIS reforços em situações DISTINTAS (a política default promove
/// com N=2). É o substrato REAL que o detector/refletor (M-DETECTOR) emitirá; aqui o injetamos cru no
/// log, que é exatamente o que a projeção consome por replay.
fn establish_belief_in_log(s: &mut EventStore, belief_id: &str, role: &str, statement: &str) {
    let events = [
        DomainEvent::CorrectionObserved {
            role: role.to_string(),
            summary: statement.to_string(),
            refs: vec![],
        },
        DomainEvent::BeliefProposed {
            belief_id: belief_id.to_string(),
            role: role.to_string(),
            statement: statement.to_string(),
            provenance: "correção do usuário (sessão 1)".to_string(),
            untrusted_origin: false,
        },
        DomainEvent::BeliefReinforced {
            belief_id: belief_id.to_string(),
            situation_hash: format!("{belief_id}-situacao-A"),
            evidence: String::new(),
        },
        DomainEvent::BeliefReinforced {
            belief_id: belief_id.to_string(),
            situation_hash: format!("{belief_id}-situacao-B"),
            evidence: String::new(),
        },
    ];
    for ev in &events {
        s.append(ev).expect("append");
    }
}

/// `now_ms` folgado dentro do TTL — estabelecidas não expiram, mas a assinatura exige um relógio.
const NOW: u64 = 10_000_000_000;

// ════════════════════════════════ gate (a): o cenário binário ════════════════════════════════

/// **(a) O CORAÇÃO: corrigiu o papel uma vez (sessão 1) → o novo terminal do MESMO papel já carrega a
/// crença (sessão 2); o de OUTRO papel NÃO.** Sessão 1: o usuário corrige o BACKEND ("use pnpm, não
/// npm") e a lição se firma (2 situações distintas → estabelecida). Sessão 2 = um terminal NOVO: a
/// SELEÇÃO que alimenta a doutrina do spawn BACKEND (`top_k_for_role`) traz a crença pnpm — "usa pnpm
/// sem ser lembrado". CONTROLE NEGATIVO (isolamento por papel): o spawn FRONTEND não recebe nada.
///
/// QUEBRA SE… a crença não promovesse (sumiria de `top_k`), OU vazasse entre papéis (apareceria no
/// FRONTEND). Não-vacuosidade: o BACKEND DE FATO recebe (controle +); o FRONTEND DE FATO não (−).
#[test]
fn a_corrigiu_uma_vez_o_papel_novo_terminal_herda_e_outro_papel_nao() {
    let (tmp, mut s) = store("cenario-binario");

    // ── Sessão 1: o usuário corrige o papel BACKEND; a lição se firma. ──
    establish_belief_in_log(&mut s, "b-pnpm", "BACKEND", "use pnpm, não npm");

    // ── Sessão 2: um terminal NOVO. A projeção reconstrói por replay do log (cross-sessão). ──
    let mentality = Mentality::replay(&s, PromotionPolicy::default()).expect("replay");

    // a crença firmou (não-vacuoso).
    let belief = mentality.belief("b-pnpm").expect("crença existe");
    assert_eq!(
        belief.status,
        BeliefStatus::Established,
        "a correção do usuário virou REGRA do papel (2 situações distintas)"
    );

    // CONTROLE POSITIVO — o spawn BACKEND recebe a crença na seleção (= injetada na doutrina dele).
    let backend_top = mentality.top_k_for_role("BACKEND", 5, NOW);
    assert!(
        backend_top.iter().any(|b| b.belief_id == "b-pnpm"),
        "o novo terminal BACKEND herda 'use pnpm' sem ser lembrado (gate-mãe)"
    );

    // CONTROLE NEGATIVO — isolamento por papel: o spawn FRONTEND NÃO recebe a crença do BACKEND.
    let frontend_top = mentality.top_k_for_role("FRONTEND", 5, NOW);
    assert!(
        !frontend_top.iter().any(|b| b.belief_id == "b-pnpm"),
        "isolamento por papel: a crença do BACKEND não vaza para o FRONTEND"
    );
    assert!(
        frontend_top.is_empty(),
        "papel sem correções não inventa crença (controle negativo limpo)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// **(a-contraste) Uma correção AINDA NÃO firmada (1 situação só) NÃO é herdada como regra.** O gate-mãe
/// exige que a lição se FIRME — uma correção isolada fica provisória (hipótese), e o despacho/injetor a
/// trata como "ainda testando", não regra. Aqui provamos que com UMA só situação a crença é
/// `Provisional` (não `Established`) — o limiar é real, não decorativo.
///
/// QUEBRA SE… uma única correção já promovesse → o status seria `Established` (gaming trivial). Controle
/// +: a crença provisória AINDA é injetável (como hipótese, dentro do TTL) — o mecanismo não a engole.
#[test]
fn a_correcao_isolada_fica_provisoria_nao_vira_regra() {
    let (tmp, mut s) = store("cenario-provisorio");
    s.append(&DomainEvent::BeliefProposed {
        belief_id: "b-uma-vez".to_string(),
        role: "BACKEND".to_string(),
        statement: "talvez prefira tabs".to_string(),
        provenance: "uma correção".to_string(),
        untrusted_origin: false,
    })
    .expect("append");
    s.append(&DomainEvent::BeliefReinforced {
        belief_id: "b-uma-vez".to_string(),
        situation_hash: "unica".to_string(),
        evidence: String::new(),
    })
    .expect("append");

    let mentality = Mentality::replay(&s, PromotionPolicy::default()).expect("replay");
    let belief = mentality.belief("b-uma-vez").expect("existe");
    assert_eq!(
        belief.status,
        BeliefStatus::Provisional,
        "uma situação só não firma a regra — fica provisória (hipótese)"
    );
    // controle +: provisória dentro do TTL ainda é injetável (como hipótese marcada).
    let now_fresh = belief.last_activity_ms + 1;
    assert!(
        mentality
            .top_k_for_role("BACKEND", 5, now_fresh)
            .iter()
            .any(|b| b.belief_id == "b-uma-vez"),
        "a provisória é injetável como hipótese (não engolida) dentro do TTL"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ════════════════════════════ gate (b): promoção determinística, event-sourced ════════════════════════════

/// **(b) Pelo caminho REAL de persistência: situações distintas promovem; a MESMA situação 2× não.** A
/// unit-suite de I prova isto sobre vecs; aqui prova-se atravessando o `EventStore` (disco → replay) —
/// o caminho que o produto usa de fato. Contraste lado a lado (anti-gaming §6.4).
///
/// QUEBRA SE… a dedup por `situation_hash` falhasse no caminho event-sourced → a meta gamed (mesma
/// situação) promoveria. Não-vacuosidade: o lado das situações distintas DE FATO promove.
#[test]
fn b_distinct_promove_mesma_situacao_nao_event_sourced() {
    // lado + : situações distintas → estabelece.
    let (tmp_a, mut sa) = store("promo-distinct");
    establish_belief_in_log(&mut sa, "b-ok", "BACKEND", "use pnpm");
    let ma = Mentality::replay(&sa, PromotionPolicy::default()).expect("replay");
    assert_eq!(
        ma.belief("b-ok").expect("existe").status,
        BeliefStatus::Established,
        "situações distintas promovem (caminho event-sourced)"
    );
    let _ = std::fs::remove_dir_all(&tmp_a);

    // lado − : MESMA situação repetida → NÃO estabelece (anti-gaming).
    let (tmp_b, mut sb) = store("promo-gaming");
    sb.append(&DomainEvent::BeliefProposed {
        belief_id: "b-gamed".to_string(),
        role: "BACKEND".to_string(),
        statement: "use pnpm".to_string(),
        provenance: "x".to_string(),
        untrusted_origin: false,
    })
    .expect("append");
    for _ in 0..3 {
        sb.append(&DomainEvent::BeliefReinforced {
            belief_id: "b-gamed".to_string(),
            situation_hash: "MESMA".to_string(),
            evidence: String::new(),
        })
        .expect("append");
    }
    let mb = Mentality::replay(&sb, PromotionPolicy::default()).expect("replay");
    let gamed = mb.belief("b-gamed").expect("existe");
    assert_eq!(
        gamed.status,
        BeliefStatus::Provisional,
        "mesma situação 3× é gaming — NÃO promove"
    );
    assert_eq!(
        gamed.distinct_situations, 1,
        "hash repetido conta 1 situação"
    );
    let _ = std::fs::remove_dir_all(&tmp_b);
}

/// **(b-challenge) Corrigir de novo (`BeliefChallenged`) REBAIXA a estabelecida.** Cenário: a regra
/// firmou, e o usuário corrige outra vez no mesmo tema — a crença volta a `Provisional` (zera o
/// progresso). É o anti-fixação: a Lina não teima numa lição que o usuário desautorizou.
///
/// QUEBRA SE… `BeliefChallenged` não zerasse → a crença seguiria `Established` ignorando a nova
/// correção. Controle +: antes do challenge ela ESTAVA estabelecida (verificado no teste b acima).
#[test]
fn b_challenge_rebaixa_a_estabelecida() {
    let (tmp, mut s) = store("challenge");
    establish_belief_in_log(&mut s, "b-x", "BACKEND", "use pnpm");
    s.append(&DomainEvent::BeliefChallenged {
        belief_id: "b-x".to_string(),
        evidence: "usuário corrigiu de novo".to_string(),
    })
    .expect("append");

    let mentality = Mentality::replay(&s, PromotionPolicy::default()).expect("replay");
    assert_eq!(
        mentality.belief("b-x").expect("existe").status,
        BeliefStatus::Provisional,
        "a contestação rebaixa a regra — a Lina cede à nova correção do usuário"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ════════════════════════════ gate (c): cap top-K (anti-context-rot) ════════════════════════════

/// **(c) K+1 crenças do papel → exatamente K injetadas (cap é critério, não opção).** Estabelecemos 3
/// crenças do BACKEND e pedimos `top_k(role, 2)`: vêm 2, todas estabelecidas. É o anti-context-rot — o
/// spawn não afoga na própria memória.
///
/// QUEBRA SE… o cap não truncasse → viriam 3 (ou todas). Não-vacuosidade: pedir k=10 traz as 3 (o cap
/// é o que limita, não falta de crença).
#[test]
fn c_cap_top_k_limita_em_k() {
    let (tmp, mut s) = store("cap-k");
    for id in ["b1", "b2", "b3"] {
        establish_belief_in_log(&mut s, id, "BACKEND", "regra do backend");
    }
    let mentality = Mentality::replay(&s, PromotionPolicy::default()).expect("replay");

    // não-vacuosidade: as 3 existem e são injetáveis.
    assert_eq!(
        mentality.top_k_for_role("BACKEND", 10, NOW).len(),
        3,
        "k folgado traz todas as 3 (controle: há o que cap-ear)"
    );
    // a propriedade: cap=2 → exatamente 2.
    let capped = mentality.top_k_for_role("BACKEND", 2, NOW);
    assert_eq!(
        capped.len(),
        2,
        "K+1 estabelecidas → exatamente K injetadas"
    );
    assert!(
        capped.iter().all(|b| b.status == BeliefStatus::Established),
        "as injetadas são estabelecidas (regra, não hipótese)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
