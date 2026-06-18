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

use crate::events::{AcceptanceCriterion, DomainEvent, EventRecord};
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

/// F3-1-1 (spec 52 §"Reconstrução por replay"): reconstrói as [`Goal`]s varrendo o log por
/// `goal_id`, dobrando o ciclo `Defined → Interpreted → Confirmed → Decomposed → (ReviewVerdict)* →
/// Achieved|Escalated`. Forma PURA (sem I/O), igual a [`crate::ParamsLedger::from_records`] — é o
/// padrão CostLedger/ParamsLedger: a projeção NÃO é fonte da verdade, o log é (invariante #4).
/// Best-effort: payload de versão futura/malformado é ignorado (não derruba o replay); evento de
/// `goal_id` sem `GoalDefined` precedente é órfão e ignorado (a Goal só existe a partir da definição).
#[must_use]
pub fn project_goals(events: &[EventRecord]) -> Vec<Goal> {
    let mut goals: Vec<Goal> = Vec::new();
    for rec in events {
        if !is_goal_event_kind(&rec.kind) {
            continue;
        }
        // Desserializa o payload tipado (best-effort): o store grava o JSON com o tag `event`, então
        // `from_value::<DomainEvent>` reconstrói a variante. Falha (versão futura) → ignora.
        let Ok(ev) = serde_json::from_value::<DomainEvent>(rec.payload.clone()) else {
            continue;
        };
        match ev {
            DomainEvent::GoalDefined {
                goal_id, statement, ..
            } => {
                // Idempotente: a 1ª definição vence (redefinição do mesmo goal_id é ignorada).
                if find_goal_mut(&mut goals, &goal_id).is_some() {
                    continue;
                }
                goals.push(Goal {
                    goal_id,
                    statement,
                    phase: GoalPhase::Defined,
                    interpretation: None,
                    acceptance: Vec::new(),
                    items: Vec::new(),
                    iterations: 0,
                });
            }
            DomainEvent::GoalInterpreted {
                goal_id,
                interpretation,
                acceptance_criteria,
                ..
            } => {
                if let Some(g) = find_goal_mut(&mut goals, &goal_id) {
                    g.interpretation = Some(interpretation);
                    g.acceptance = acceptance_criteria;
                    g.phase = GoalPhase::Interpreted;
                }
            }
            DomainEvent::GoalConfirmed {
                goal_id,
                amended_statement,
                ..
            } => {
                if let Some(g) = find_goal_mut(&mut goals, &goal_id) {
                    if let Some(amended) = amended_statement {
                        g.statement = amended;
                    }
                    g.phase = GoalPhase::Confirmed;
                }
            }
            DomainEvent::GoalDecomposed {
                goal_id,
                plan_items,
            } => {
                if let Some(g) = find_goal_mut(&mut goals, &goal_id) {
                    g.items = plan_items;
                    g.phase = GoalPhase::Decomposed;
                }
            }
            DomainEvent::ReviewVerdict {
                goal_id, iteration, ..
            } => {
                if let Some(g) = find_goal_mut(&mut goals, &goal_id) {
                    // O laço começou — só promove se a Goal ainda não fechou/escalou (terminais).
                    if !matches!(g.phase, GoalPhase::Achieved | GoalPhase::Escalated) {
                        g.phase = GoalPhase::InLoop;
                    }
                    g.iterations = g.iterations.max(iteration);
                }
            }
            DomainEvent::GoalAchieved {
                goal_id,
                iterations,
                ..
            } => {
                if let Some(g) = find_goal_mut(&mut goals, &goal_id) {
                    g.phase = GoalPhase::Achieved;
                    g.iterations = iterations; // valor autoritativo do fechamento.
                }
            }
            DomainEvent::GoalEscalated { goal_id, .. } => {
                if let Some(g) = find_goal_mut(&mut goals, &goal_id) {
                    g.phase = GoalPhase::Escalated;
                }
            }
            _ => {}
        }
    }
    goals
}

/// `true` se o `kind` persistido é um evento do ciclo da Goal — filtro barato (sem alocar) antes de
/// desserializar, no molde de [`crate::ParamsLedger::from_records`] (`kind != "..." → continue`).
fn is_goal_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "GoalDefined"
            | "GoalInterpreted"
            | "GoalConfirmed"
            | "GoalDecomposed"
            | "GoalAchieved"
            | "GoalEscalated"
            | "ReviewVerdict"
    )
}

fn find_goal_mut<'a>(goals: &'a mut [Goal], goal_id: &str) -> Option<&'a mut Goal> {
    goals.iter_mut().find(|g| g.goal_id == goal_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckKind, DomainEvent, NodeId, Verdict};

    /// Embrulha um [`DomainEvent`] num [`EventRecord`] como o store faz (kind = tag serde; payload =
    /// o JSON com o discriminante `event`). Forma pura — sem I/O — para os testes de projeção.
    fn rec(seq: u64, ev: &DomainEvent) -> EventRecord {
        EventRecord {
            seq,
            ts: 0,
            kind: ev.kind().to_string(),
            version: 1,
            payload: serde_json::to_value(ev).expect("serializa DomainEvent"),
        }
    }

    fn crit(desc: &str) -> AcceptanceCriterion {
        AcceptanceCriterion {
            desc: desc.to_string(),
            check_kind: CheckKind::Command,
            check_arg: Some("true".to_string()),
        }
    }

    /// Gate (a): o ciclo completo Defined→Interpreted→Confirmed→Decomposed→(ReviewVerdict)→Achieved
    /// projeta UMA Goal no estado final correto, varrendo por `goal_id`.
    #[test]
    fn projeta_ciclo_completo_ate_achieved() {
        let dev = NodeId::nil();
        let qa = NodeId::max();
        let evs = [
            DomainEvent::GoalDefined {
                goal_id: "g1".into(),
                statement: "criar landing page".into(),
                root_cause_id: "r1".into(),
                origin: "@Maestro".into(),
                budget_tokens: 0,
            },
            DomainEvent::GoalInterpreted {
                goal_id: "g1".into(),
                interpretation: "landing simples de captura".into(),
                strategy: "1 dev frontend".into(),
                proposed_team: vec!["Frontend".into()],
                acceptance_criteria: vec![crit("a página abre sem erro")],
            },
            DomainEvent::GoalConfirmed {
                goal_id: "g1".into(),
                by: dev,
                amended_statement: None,
            },
            DomainEvent::GoalDecomposed {
                goal_id: "g1".into(),
                plan_items: vec!["T1".into(), "T2".into()],
            },
            DomainEvent::ReviewVerdict {
                goal_id: "g1".into(),
                plan_item: "T1".into(),
                target: dev,
                reviewer: qa,
                verdict: Verdict::Pass,
                evidence: String::new(),
                defect_class: String::new(),
                iteration: 0,
            },
            DomainEvent::GoalAchieved {
                goal_id: "g1".into(),
                iterations: 1,
                verdict_event_id: "e6".into(),
            },
        ];
        let recs: Vec<EventRecord> = evs
            .iter()
            .enumerate()
            .map(|(i, e)| rec(i as u64, e))
            .collect();

        let goals = project_goals(&recs);

        assert_eq!(goals.len(), 1, "uma Goal projetada");
        let g = &goals[0];
        assert_eq!(g.goal_id, "g1");
        assert_eq!(g.statement, "criar landing page");
        assert_eq!(g.phase, GoalPhase::Achieved);
        assert_eq!(
            g.interpretation.as_deref(),
            Some("landing simples de captura")
        );
        assert_eq!(g.acceptance, vec![crit("a página abre sem erro")]);
        assert_eq!(g.items, vec!["T1".to_string(), "T2".to_string()]);
        assert_eq!(g.iterations, 1, "iterations do GoalAchieved é autoritativo");
    }

    /// `amended_statement` no `GoalConfirmed` substitui o statement (o humano corrigiu).
    #[test]
    fn confirmacao_com_emenda_substitui_o_statement() {
        let evs = [
            DomainEvent::GoalDefined {
                goal_id: "g".into(),
                statement: "faz um app".into(),
                root_cause_id: "r".into(),
                origin: "@Maestro".into(),
                budget_tokens: 0,
            },
            DomainEvent::GoalConfirmed {
                goal_id: "g".into(),
                by: NodeId::nil(),
                amended_statement: Some("faz um app de notas offline".into()),
            },
        ];
        let recs: Vec<EventRecord> = evs
            .iter()
            .enumerate()
            .map(|(i, e)| rec(i as u64, e))
            .collect();
        let goals = project_goals(&recs);
        assert_eq!(goals[0].statement, "faz um app de notas offline");
        assert_eq!(goals[0].phase, GoalPhase::Confirmed);
    }

    /// Escalada é fase terminal observável; eventos órfãos (sem `GoalDefined`) são ignorados
    /// (best-effort, espelha o replay de params).
    #[test]
    fn escalada_projeta_e_orfaos_sao_ignorados() {
        let evs = [
            // órfão: ReviewVerdict de uma goal nunca definida → ignorado.
            DomainEvent::ReviewVerdict {
                goal_id: "fantasma".into(),
                plan_item: "X".into(),
                target: NodeId::nil(),
                reviewer: NodeId::max(),
                verdict: Verdict::Fail,
                evidence: "boom".into(),
                defect_class: "qualidade".into(),
                iteration: 2,
            },
            DomainEvent::GoalDefined {
                goal_id: "g".into(),
                statement: "meta dura".into(),
                root_cause_id: "r".into(),
                origin: "@Maestro".into(),
                budget_tokens: 0,
            },
            DomainEvent::GoalEscalated {
                goal_id: "g".into(),
                reason: "turn_budget_exhausted".into(),
            },
        ];
        let recs: Vec<EventRecord> = evs
            .iter()
            .enumerate()
            .map(|(i, e)| rec(i as u64, e))
            .collect();
        let goals = project_goals(&recs);
        assert_eq!(goals.len(), 1, "só a goal definida; órfão ignorado");
        assert_eq!(goals[0].phase, GoalPhase::Escalated);
    }
}
