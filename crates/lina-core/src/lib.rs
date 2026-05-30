//! `lina-core` — o cérebro framework-agnóstico do Lina Space.
//!
//! Reúne o substrato da Onda 0 (Trilho A): pty-host isolado (W0-3),
//! Workspace Bus / Supervisor (W0-4), Event Store (W0-5), recuperação
//! pós-crash (W0-6), Secret Vault (W0-7), CLI Profiles (W0-8), entrega A2A
//! faseada (W0-9) e contrato de fim-de-resposta (W0-10).
//!
//! **Invariante:** este crate NÃO importa tipos de framework de UI. Fala com a
//! superfície apenas via `lina_host::UiHost`. (Âncora de continuidade — `CLAUDE.md`.)

#![allow(dead_code)]

/// Re-exports do contrato de UI para os consumidores do core.
pub use lina_host::{HostEvent, NodeId, UiHost};

/// Envelope canônico de mensagem A2A (versionado — âncora de continuidade).
///
/// Campos definidos desde já (opcionais até o supervisor preenchê-los), para
/// evitar contrato fragmentado por feature (W0-4 + W3-3 + W3-7).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct A2aEnvelope {
    pub id: String,
    pub root_cause_id: Option<String>,
    pub from: NodeId,
    pub to: Recipient,
    pub intent: Option<String>,
    pub hops: u8,
    pub await_reply: bool,
}

/// Endereçamento de mensagem: nó específico, papel (late-bound) ou broadcast.
#[derive(Debug, Clone)]
pub enum Recipient {
    Node(NodeId),
    Role(String),
    Broadcast,
}

// TODO(W0-3): pty-host (reader threads + event-batching + flow-control + catch_unwind).
// TODO(W0-4): Workspace Bus / Supervisor (NodeRegistry, MailQueues serial, wait-for-graph).
// TODO(W0-5): Event Store (rusqlite WAL + JSONL + snapshots; replay determinístico).
// TODO(W0-6): recuperação pós-crash visível (integrity_check -> preservar -> rebuild).
