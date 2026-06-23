//! **Seed de DEMO do painel "Como o [papel] pensa" (F3-3 gate h visual).** Popula um workspace de
//! TESTE ISOLADO com 2 crenças do papel DEVELOPER (1 estabelecida, 1 provisória) para o fundador
//! validar o painel a olho. NÃO é produção — aponta para um dir de teste (arg, ou `/tmp/lina-mentality-demo`).
//!
//! Este bin semeia SÓ os APRENDIZADOS (as crenças). O ROSTER VIVO coerente — um terminal de papel
//! `DEVELOPER` — é semeado pelo PRÓPRIO app no boot via `LINA_DEMO_ROLE=DEVELOPER` (ver
//! `demo-mentality.sh`): o painel enumera papéis pelo roster VIVO (`cards`→`node_role`, projeção do
//! log), então o papel das crenças e o papel de um terminal VIVO TÊM que ser o mesmo. Semear aqui um
//! `NodeAdded`/`NodeRoleAssigned` seria um nó-FANTASMA (escrito por outro processo, nunca admitido ao
//! `NodeManager` vivo) — o painel não o veria e o boot ainda o aposentaria póstumo. Por isso: só crenças.
//!
//! Após popular, PROJETA a `Mentality` e imprime as crenças: o app renderiza essa MESMA projeção, então
//! a saída aqui confirma o que o painel deve mostrar (sem depender de screenshot — TCC).

use std::path::PathBuf;

use lina_core::mentality::{BeliefStatus, Mentality, PromotionPolicy};
use lina_core::{DomainEvent, EventStore};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/lina-mentality-demo/.lina"));
    let events = root.join("events");
    std::fs::create_dir_all(&events)?;
    let mut store = EventStore::open(&events)?;

    let role = "DEVELOPER";

    let seed = [
        // Crença ESTABELECIDA — 2 situações distintas (s1/s2) → vira REGRA do papel ("já vale").
        DomainEvent::BeliefProposed {
            belief_id: "b-pnpm".to_string(),
            role: role.to_string(),
            statement: "Use pnpm, não npm — é o gerenciador de pacotes padrão deste projeto."
                .to_string(),
            provenance: "aprendido quando você corrigiu o comando de instalação".to_string(),
            untrusted_origin: false,
        },
        DomainEvent::BeliefReinforced {
            belief_id: "b-pnpm".to_string(),
            situation_hash: "s1".to_string(),
            evidence: String::new(),
        },
        DomainEvent::BeliefReinforced {
            belief_id: "b-pnpm".to_string(),
            situation_hash: "s2".to_string(),
            evidence: String::new(),
        },
        // Crença PROVISÓRIA — só proposta, sem reforço distinto suficiente ("ainda testando").
        DomainEvent::BeliefProposed {
            belief_id: "b-pt".to_string(),
            role: role.to_string(),
            statement: "Explique as decisões técnicas em português simples, sem jargão."
                .to_string(),
            provenance: "aprendido quando você pediu clareza na explicação".to_string(),
            untrusted_origin: false,
        },
    ];
    for ev in &seed {
        store.append(ev)?;
    }

    // CONFIRMAÇÃO (sem screenshot): projeta a MESMA Mentality que o painel renderiza.
    let m = Mentality::replay(&store, PromotionPolicy::default())?;
    println!("seed OK: {} eventos em {}\n", seed.len(), events.display());
    println!("o painel \"Como o {role} pensa\" deve mostrar:");
    for b in m.beliefs_for_role(role) {
        let selo = match b.status {
            BeliefStatus::Established => "JÁ VALE",
            BeliefStatus::Provisional => "ainda testando",
            BeliefStatus::Retired { .. } => "aposentada",
        };
        println!("  [{selo}] {}", b.statement);
        println!("          de onde veio: {}", b.provenance);
    }
    Ok(())
}
