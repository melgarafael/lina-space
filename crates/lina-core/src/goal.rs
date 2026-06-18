//! **F3-1 · A Goal como entidade event-sourced (`[3]` Objetivos de Meadows).**
//!
//! **Invariante do `CLAUDE.md` (#4):** o event log é a fonte da verdade; a Goal é uma **projeção
//! reconstruível**, NÃO uma estrutura mutável de primeira ordem. Varrer o log por `goal_id`
//! reconstrói o ciclo `Defined → Interpreted → Confirmed → Decomposed → (ReviewVerdict)* →
//! Achieved|Escalated` (spec 52 §"Reconstrução por replay") — mesmo padrão de
//! [`crate::CostLedger`]/[`crate::ParamsLedger`].
//!
//! Esta fatia (Rα-0) materializa só o CONTRATO (os tipos da projeção); a lógica real de
//! [`project_goals`] (varrer os [`DomainEvent`](crate::DomainEvent) da Goal e dobrar o ciclo) é a
//! fatia CORE-Goal-lógica seguinte.

use crate::events::{AcceptanceCriterion, EventRecord};
use serde::{Deserialize, Serialize};

/// Fase do ciclo de vida de uma [`Goal`] (spec 52 §1). Estados sucessivos do termostato: a Goal
/// nasce `Defined`, é interpretada e confirmada (gate humano), decompõe-se em itens, entra no laço
/// (`InLoop`) e FECHA em `Achieved` (todos os itens `Pass`) ou PAUSA em `Escalated` (backstop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalPhase {
    /// Declarada (`GoalDefined`) — âncora dos eventos da meta.
    Defined,
    /// Interpretada (`GoalInterpreted`) — estratégia + critérios propostos (SUGERE).
    Interpreted,
    /// Confirmada (`GoalConfirmed`) — gate humano passado.
    Confirmed,
    /// Decomposta (`GoalDecomposed`) — itens do plano semeados.
    Decomposed,
    /// No laço executa→revisa→itera (entre `Decomposed` e o fechamento).
    InLoop,
    /// Fechou (`GoalAchieved`) — todo item com `ReviewVerdict::Pass`.
    Achieved,
    /// Escalada ao humano sem fechar (`GoalEscalated`) — pausa resumível, nunca kill.
    Escalated,
}

/// A projeção viva de uma Goal — derivada por replay (NÃO é fonte da verdade; ver módulo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    /// UUIDv7 cunhado pelo supervisor no `GoalDefined`.
    pub goal_id: String,
    /// O pedido bruto do humano/origem (dado, jamais autoridade).
    pub statement: String,
    /// Fase corrente do ciclo.
    pub phase: GoalPhase,
    /// O que o Maestro entendeu (`None` até o `GoalInterpreted`).
    pub interpretation: Option<String>,
    /// Critérios de aceite OBSERVÁVEIS — o setpoint do laço de qualidade.
    pub acceptance: Vec<AcceptanceCriterion>,
    /// ids dos `PlanItem` que servem a esta Goal.
    pub items: Vec<String>,
    /// Voltas do OUTER loop já gastas (turn-budget).
    pub iterations: u32,
}

/// F3-1 (spec 52 §"Reconstrução por replay"): reconstrói as [`Goal`]s varrendo os eventos por
/// `goal_id`. **Esqueleto Rα-0** — corpo mínimo; a projeção real (dobrar o ciclo `Defined → … →
/// Achieved|Escalated`) é a fatia CORE-Goal-lógica (F3-1-1).
#[must_use]
pub fn project_goals(_events: &[EventRecord]) -> Vec<Goal> {
    Vec::new()
}
