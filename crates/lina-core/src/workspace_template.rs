//! F3-5-10 · frente TEMPLATES (dono: Terminal G) — STUB da largada.
//!
//! Templates de Workspace: gabaritos de Espaço (roster + `SystemParams` + pistas +
//! doutrinas pré-definidos). Instanciar um template SEMEIA o Espaço encadeando appends
//! de eventos EXISTENTES (`SpawnRequested` / `SystemParamsChanged` / `ClueSetDefined` /
//! `PlanItemAttributed`) — NÃO toca `Workspace::create` (workspace.rs:241), encadeia
//! DEPOIS. Verificável por REPLAY (reabrir reconstrói o Espaço semeado idêntico).
//! Sem template → criação atual inalterada (compat). Depende de `clue` (ClueSetDefined).
//! Preencha aqui; o `pub mod` já está em lib.rs (não toque lib.rs).
