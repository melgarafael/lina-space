//! F3-5-7 · frente BUFFERS (dono: Terminal K) — STUB da largada.
//!
//! `BufferRegistry`: projeção unificada de ocupação (`BTreeMap<buffer_id, GaugeSnapshot>`,
//! último `BufferOccupancySampled` por `buffer_id` vence). `capacity==0` = ilimitado,
//! `pressure_ratio=0.0`. Anti-amplificação: amostra só quando cruza bucket de 10% OU
//! muda warn/critical (`gauge_warn_ratio` default 0,80). Amostragem no tick do FlushGuard
//! (scrollback) e no drain do mailbox (`mailbox:<node>`). Entrega `buffer_report()` PURO —
//! o braço `lina check --buffers` em router.rs é fiado pelo Maestro (não edite router.rs).
//! Preencha aqui; o `pub mod` já está em lib.rs (não toque lib.rs).
