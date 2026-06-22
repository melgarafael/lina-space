//! F3-5-1 · frente SESSÕES (dono: Terminal B / Ultra Code) — STUB da largada.
//!
//! Projeção `ResumeSessionStore` event-sourced: `SessionPersisted` − `SessionRetired`
//! por `session_id` (admissão SÓ após 1º prompt; poda LRU por `persisted_at_ms`, cap
//! default 50). Molde: `CostLedger::from_records` (router.rs). NÃO confundir com
//! `SessionStore` (câmera, session.rs:68) — é outro domínio (ADR 0029).
//! `session_id` é projeção, JAMAIS identidade (a identidade é `LINA_NODE_ID`).
//! Preencha aqui; o `pub mod` já está declarado em lib.rs (não toque lib.rs).
