//! F3-5-8 · frente DISCO (dono: Terminal M) — STUB da largada.
//!
//! `DiskBudget`: probe periódico (`disk_probe_secs` 300) → `used_pct≥0,85` =
//! `DiskPressureSignaled{warn}`; `≥0,95` = `{critical}` + varre candidatos +
//! `DiskReclaimProposed`. Item entra como `AttentionKind::Custody` (gate humano).
//! REGRA-MÃE (ADR 0043): poda é IRREVERSÍVEL → **zero `DiskReclaimExecuted` sem
//! `DiskReclaimApproved`**; `approved_by` é EXIBIÇÃO, a autoridade é o gesto custodiado
//! (broker::run_custody). Gate em TODOS os níveis de autonomia.
//! CAMADA PURA aqui (decisão de candidatos + projeção + gate, testável com disco
//! simulado) SEPARADA da execução shell (achado dogfooding #36: `rm`/`cargo clean`
//! dispara permission_prompt — o worker NÃO roda a poda real).
//! Preencha aqui; o `pub mod` já está em lib.rs (não toque lib.rs).
