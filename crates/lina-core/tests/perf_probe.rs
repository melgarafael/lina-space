//! **Sonda de custo do `EventStore::project()` (r5 perf-ws, BUG 3).**
//!
//! Em produção NINGUÉM chama `take_snapshot` (zero callers fora de teste), então cada
//! `project()` re-parseia o log INTEIRO (JSON por evento). O `MailboxPump::tick` chama
//! `project()` a cada ~120ms POR RUNTIME (`bridge.rs project_cli_by_node`) — com um log
//! real de ~21k eventos (workspace walking-skeleton), N Espaços pagam N×8 replays
//! completos por segundo, para sempre. Números no stdout (`--nocapture`); asserções
//! são de invariante, não de tempo (sem flake).

use std::time::Instant;

use lina_core::{DomainEvent, EventStore};

/// Store sintético com a FORMA do log real (~21k eventos: status + uso de tokens).
fn build_store(dir: &std::path::Path, events: u32) -> EventStore {
    let mut store = EventStore::open(dir).expect("abrir store");
    let node = "019eb71d-1258-78d2-842e-14e56aac2f05"
        .parse()
        .expect("node id");
    for i in 0..events {
        let ev = if i % 2 == 0 {
            DomainEvent::TokenUsageReported {
                node: format!("nó-{}", i % 9),
                tokens: u64::from(i % 700),
            }
        } else {
            DomainEvent::NodeStatusChanged {
                node,
                status: if i % 4 == 1 { "Busy" } else { "Idle" }.into(),
                from: "Running".into(),
                reason: "end_of_response".into(),
            }
        };
        store.append(&ev).expect("append");
    }
    store
}

#[test]
fn probe_project_cost_on_realistic_log() {
    let dir = std::env::temp_dir().join(format!("lina-coreprobe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = build_store(&dir.join("events"), 21_000);

    // Custo de UM project() no shape real (pago a cada 120ms por runtime no ANTES).
    let t0 = Instant::now();
    let p1 = store.project().expect("project 1");
    let first_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Repetido SEM nenhum evento novo (o caso do tick: quase sempre nada mudou).
    let t1 = Instant::now();
    for _ in 0..8 {
        let _ = store.project().expect("project N");
    }
    let repeat_ms = t1.elapsed().as_secs_f64() * 1000.0 / 8.0;

    // Conexão NOVA no mesmo log (o caso `project_store_read_only` do sidebar refresh).
    let fresh = EventStore::open(dir.join("events")).expect("reabrir");
    let t2 = Instant::now();
    let p2 = fresh.project().expect("project fresh");
    let fresh_ms = t2.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(p1, p2, "mesmo log → mesma projeção (replay determinístico)");

    println!("[PERF-WS] log sintético de 21k eventos (forma do walking-skeleton):");
    println!("[PERF-WS] project() 1º (instância quente): {first_ms:.1} ms");
    println!(
        "[PERF-WS] project() repetido sem evento novo (tick de 120ms do pump): {repeat_ms:.1} ms"
    );
    println!(
        "[PERF-WS] project() em conexão NOVA (refresh do sidebar por Espaço): {fresh_ms:.1} ms"
    );
    println!(
        "[PERF-WS] projeção: N=4 runtimes × ~8 ticks/s × {repeat_ms:.1} ms = {:.0}% de um core, contínuo",
        4.0 * 8.0 * repeat_ms / 10.0
    );

    let _ = std::fs::remove_dir_all(&dir);
}
