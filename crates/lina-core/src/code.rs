//! Coordenação de Código (F3-4 — spec 36 §3): a projeção do INTEGRADOR.
//!
//! Quando N agentes codam em git worktrees isoladas, cada commit apenda um `CodeChanged{branch}` e
//! cada merge PROVADO apenda um `BranchIntegrated{branch}`. Esta projeção PURA (molde
//! `CostLedger`/`Mentality`, inv #4/#1 — ZERO LLM) reconstrói por replay do event log:
//!   1. `branches_nao_integradas` = `{ branch | ∃ CodeChanged{branch} ∧ ∄ BranchIntegrated{branch} }`;
//!   2. o **gatilho determinístico** do DevOps integrador: dispara SSE há branch pendente E todos os
//!      nós estão Idle/Dead há ≥ `T` (a equipe terminou → hora de juntar tudo na `develop`).
//!
//! O gatilho **DECIDE** (projeção pura); NÃO cria o nó. O disparo é do caminho de spawn governado
//! (`handle_spawn`, ADR 0022 — gates de cascata/cap/custo intactos); o disparo 100% autônomo
//! amadurece sob os mesmos gates (spec 36 §"Pendências"). `branch`/`paths`/`commit`/`author_node` são
//! DADO transportado, JAMAIS autoridade (regra-mãe família ADR 0007).
//!
//! Timing: `now_ms` e a janela de assentamento são INJETADOS (nunca wall-clock embutido — protocolo
//! time-based mede contra o passado), então `from_records` reconstrói o conjunto pendente e o estado
//! dos nós byte-a-byte idêntico independente do relógio (gate f). O `since_ts` (há quanto um nó está
//! parado) NÃO vem de `ProjectedState.nodes` — que carrega só o status corrente, sem a data da
//! transição — por isso a projeção o deriva aqui, foldando `NodeStatusChanged`.

use std::collections::{BTreeMap, BTreeSet};

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};
use crate::{NodeId, NodeStatus};

/// Papel canônico do integrador (spec 36 §3): o que o gatilho pede spawnar — ele carrega a skill
/// `lina-integration`. Token uppercase como os demais papéis (`default-roles.yaml`).
pub const INTEGRATOR_ROLE: &str = "DEVOPS";

/// Janela default de "equipe assentada" antes de integrar: um ciclo de heartbeat de silêncio
/// (`lifecycle.rs` §1, 2 min). Conservador e TUNÁVEL (futuro `RouterConfig`); INJETADO na consulta
/// para a decisão do gatilho ser reprodutível por replay (não depende do relógio).
pub const INTEGRATOR_SETTLE_MS: u64 = crate::lifecycle::HEARTBEAT_SAMPLE_MS;

/// A DECISÃO de spawnar o integrador — saída PURA do gatilho (não um efeito). O supervisor a roteia
/// pelo caminho de spawn governado; o core decide, não cria o nó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegratorDispatch {
    /// Papel a spawnar (`INTEGRATOR_ROLE`).
    pub role: String,
    /// Branches pendentes que motivaram o disparo (ordem estável) — entram na narração ao leigo.
    pub branches: Vec<String>,
}

/// Projeção da coordenação de código por replay. Igualdade estrutural prova o replay idêntico (gate f).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeIntegration {
    /// Branches com ≥1 `CodeChanged` e nenhum `BranchIntegrated` posterior (`BTreeSet` → ordem
    /// estável → projeção determinística).
    pending: BTreeSet<String>,
    /// Por nó: `(status corrente, ts da última transição)` — base do "Idle/Dead por ≥ T".
    nodes: BTreeMap<NodeId, (String, u64)>,
}

impl CodeIntegration {
    /// Reconstrói do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log e reconstrói o conjunto
    /// pendente + o estado dos nós. Testável com log sintético (`ts` controlado) — não embute relógio.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut pending: BTreeSet<String> = BTreeSet::new();
        let mut nodes: BTreeMap<NodeId, (String, u64)> = BTreeMap::new();
        for record in records {
            // Versão futura/indecodificável: a projeção é DERIVADA, não validadora do log (espelha
            // `Mentality::from_records`) — pula sem panicar.
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            match event {
                DomainEvent::CodeChanged { branch, .. } => {
                    pending.insert(branch);
                }
                DomainEvent::BranchIntegrated { branch, .. } => {
                    // Só `BranchIntegrated` fecha uma branch — a ÚNICA prova (spec 36 §3), nunca por
                    // suposição. Um `CodeChanged` posterior re-pendura (há trabalho novo a juntar).
                    pending.remove(&branch);
                }
                DomainEvent::NodeStatusChanged { node, status, .. } => {
                    nodes.insert(node, (status, record.ts));
                }
                // Nó removido sai do roster — não conta mais para "todos os nós" do gatilho.
                DomainEvent::NodeRemoved { node } => {
                    nodes.remove(&node);
                }
                _ => {}
            }
        }
        Self { pending, nodes }
    }

    /// As branches ainda não integradas (ordem estável). Vazio ⇒ nada a juntar.
    #[must_use]
    pub fn pending_branches(&self) -> Vec<&str> {
        self.pending.iter().map(String::as_str).collect()
    }

    /// `true` se a branch tem trabalho não-integrado (≥1 `CodeChanged`, sem `BranchIntegrated`).
    #[must_use]
    pub fn is_branch_pending(&self, branch: &str) -> bool {
        self.pending.contains(branch)
    }

    /// A equipe está ASSENTADA? Há ≥1 nó E todos estão Idle/Dead há ≥ `settle_window_ms` (contra o
    /// `now_ms` INJETADO). Qualquer nó `Busy`/recém-assentado ⇒ `false` (ainda trabalhando).
    #[must_use]
    pub fn team_settled(&self, now_ms: u64, settle_window_ms: u64) -> bool {
        !self.nodes.is_empty()
            && self.nodes.values().all(|(status, since_ts)| {
                is_settled_status(status) && now_ms.saturating_sub(*since_ts) >= settle_window_ms
            })
    }

    /// **O GATILHO determinístico (spec 36 §3, ADR 0042):** dispara SSE há branch pendente E a equipe
    /// está assentada (Idle/Dead ≥ T). PURO de `now_ms` ⇒ o replay reproduz a decisão.
    #[must_use]
    pub fn should_dispatch_integrator(&self, now_ms: u64, settle_window_ms: u64) -> bool {
        !self.pending.is_empty() && self.team_settled(now_ms, settle_window_ms)
    }

    /// A DECISÃO de spawn do integrador quando o gatilho fecha — `None` se não dispara. O core
    /// DECIDE; o disparo é do caminho de spawn governado (`handle_spawn`), NÃO daqui (spec 36 §3).
    #[must_use]
    pub fn integrator_dispatch(
        &self,
        now_ms: u64,
        settle_window_ms: u64,
    ) -> Option<IntegratorDispatch> {
        self.should_dispatch_integrator(now_ms, settle_window_ms)
            .then(|| IntegratorDispatch {
                role: INTEGRATOR_ROLE.to_string(),
                branches: self.pending.iter().cloned().collect(),
            })
    }
}

/// Um nó conta como "assentado" (pronto para o integrador juntar) se está Idle ou Dead — terminou o
/// turno (Idle) ou encerrou (Dead). Qualquer outro estado (Busy/Starting/Ready/...) é trabalho em curso.
fn is_settled_status(status: &str) -> bool {
    status == NodeStatus::Idle.as_str() || status == NodeStatus::Dead.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registro sintético construído do PRÓPRIO `DomainEvent` (serializado como o `append` faz —
    /// carrega a tag interna `event`), com `ts` controlado. Exercita o decode REAL (`from_record`).
    fn record(event: DomainEvent, ts: u64) -> EventRecord {
        EventRecord {
            seq: 0,
            ts,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa evento de domínio"),
        }
    }

    fn changed(branch: &str, ts: u64) -> EventRecord {
        record(
            DomainEvent::CodeChanged {
                branch: branch.into(),
                paths: vec!["crates/lina-core/src/x.rs".into()],
                author_node: "n1".into(),
                commit: format!("c{ts}"),
            },
            ts,
        )
    }

    fn integrated(branch: &str, ts: u64) -> EventRecord {
        record(
            DomainEvent::BranchIntegrated {
                branch: branch.into(),
                into: "develop".into(),
                commit: format!("m{ts}"),
            },
            ts,
        )
    }

    fn status(node: NodeId, st: NodeStatus, ts: u64) -> EventRecord {
        record(
            DomainEvent::NodeStatusChanged {
                node,
                status: st.as_str().to_string(),
                from: String::new(),
                reason: String::new(),
            },
            ts,
        )
    }

    const SETTLE: u64 = INTEGRATOR_SETTLE_MS;

    // ── branches não-integradas: CodeChanged pendura, BranchIntegrated some (gate b/c) ───────────

    #[test]
    fn code_changed_without_integration_is_pending() {
        let c = CodeIntegration::from_records(&[changed("lina/f3-4-a", 100)]);
        assert!(c.is_branch_pending("lina/f3-4-a"));
        assert_eq!(c.pending_branches(), vec!["lina/f3-4-a"]);
    }

    #[test]
    fn branch_integrated_removes_from_pending() {
        let c = CodeIntegration::from_records(&[
            changed("lina/f3-4-a", 100),
            integrated("lina/f3-4-a", 200),
        ]);
        assert!(!c.is_branch_pending("lina/f3-4-a"));
        assert!(c.pending_branches().is_empty());
    }

    #[test]
    fn later_commit_after_integration_reopens_branch() {
        // Controle: um commit DEPOIS do merge re-abre a branch (há trabalho não-integrado de novo).
        let c = CodeIntegration::from_records(&[
            changed("b", 100),
            integrated("b", 200),
            changed("b", 300),
        ]);
        assert!(c.is_branch_pending("b"), "commit pós-merge re-pendura");
    }

    // ── gatilho: dispara só com pendente E todos Idle/Dead ≥ T (controle + e −) ───────────────────

    #[test]
    fn trigger_fires_when_pending_and_all_settled_past_window() {
        let n1 = NodeId::from_u128(1);
        let n2 = NodeId::from_u128(2);
        let c = CodeIntegration::from_records(&[
            changed("b", 100),
            status(n1, NodeStatus::Idle, 100),
            status(n2, NodeStatus::Dead, 100),
        ]);
        let now = 100 + SETTLE + 1;
        assert!(c.team_settled(now, SETTLE));
        assert!(c.should_dispatch_integrator(now, SETTLE));
        let d = c.integrator_dispatch(now, SETTLE).expect("dispara");
        assert_eq!(d.role, INTEGRATOR_ROLE);
        assert_eq!(d.branches, vec!["b".to_string()]);
    }

    #[test]
    fn trigger_does_not_fire_without_pending_branch() {
        // Controle −: equipe ociosa mas nada a juntar → não dispara.
        let n1 = NodeId::from_u128(1);
        let c = CodeIntegration::from_records(&[status(n1, NodeStatus::Idle, 100)]);
        let now = 100 + SETTLE + 1;
        assert!(!c.should_dispatch_integrator(now, SETTLE));
        assert!(c.integrator_dispatch(now, SETTLE).is_none());
    }

    #[test]
    fn trigger_does_not_fire_while_a_node_is_busy() {
        // Controle −: um nó ainda Busy → a equipe NÃO terminou → não junta.
        let n1 = NodeId::from_u128(1);
        let n2 = NodeId::from_u128(2);
        let c = CodeIntegration::from_records(&[
            changed("b", 100),
            status(n1, NodeStatus::Idle, 100),
            status(n2, NodeStatus::Busy, 100),
        ]);
        let now = 100 + SETTLE + 1;
        assert!(
            !c.team_settled(now, SETTLE),
            "um Busy quebra o assentamento"
        );
        assert!(!c.should_dispatch_integrator(now, SETTLE));
    }

    #[test]
    fn trigger_waits_for_settle_window() {
        // Controle −: todos Idle MAS por menos que T → ainda não dispara (a janela absorve o gap
        // fim-de-turno → próximo turno; só dispara quando o silêncio é real).
        let n1 = NodeId::from_u128(1);
        let c =
            CodeIntegration::from_records(&[changed("b", 100), status(n1, NodeStatus::Idle, 100)]);
        assert!(
            !c.should_dispatch_integrator(100 + SETTLE - 1, SETTLE),
            "dentro da janela não dispara"
        );
        assert!(
            c.should_dispatch_integrator(100 + SETTLE, SETTLE),
            "no limite da janela dispara"
        );
    }

    #[test]
    fn empty_team_never_triggers() {
        // Sem nós no roster, não há "equipe que terminou" — não dispara nem com branch pendente.
        let c = CodeIntegration::from_records(&[changed("b", 100)]);
        assert!(!c.should_dispatch_integrator(1_000_000, SETTLE));
    }

    #[test]
    fn node_latest_status_and_since_ts_win() {
        // Um nó que foi Busy e VOLTOU a Idle conta pelo ÚLTIMO estado, com o `since_ts` do último.
        let n1 = NodeId::from_u128(1);
        let c = CodeIntegration::from_records(&[
            changed("b", 100),
            status(n1, NodeStatus::Busy, 100),
            status(n1, NodeStatus::Idle, 500),
        ]);
        assert!(
            !c.should_dispatch_integrator(500 + SETTLE - 1, SETTLE),
            "a janela conta do retorno a Idle (500), não do Busy (100)"
        );
        assert!(c.should_dispatch_integrator(500 + SETTLE, SETTLE));
    }

    #[test]
    fn removed_node_drops_from_roster() {
        // Nó removido não bloqueia o gatilho (não é mais "um nó da equipe").
        let n1 = NodeId::from_u128(1);
        let n2 = NodeId::from_u128(2);
        let c = CodeIntegration::from_records(&[
            changed("b", 100),
            status(n1, NodeStatus::Idle, 100),
            status(n2, NodeStatus::Busy, 100),
            record(DomainEvent::NodeRemoved { node: n2 }, 150),
        ]);
        let now = 100 + SETTLE + 1;
        assert!(
            c.should_dispatch_integrator(now, SETTLE),
            "removido o Busy, a equipe restante assentou"
        );
    }

    // ── replay idêntico (gate f, §7) ─────────────────────────────────────────────────────────────

    #[test]
    fn replay_reconstructs_identical_projection() {
        let n1 = NodeId::from_u128(1);
        let log = vec![
            changed("a", 100),
            changed("b", 150),
            integrated("a", 200),
            status(n1, NodeStatus::Idle, 250),
        ];
        let x = CodeIntegration::from_records(&log);
        let y = CodeIntegration::from_records(&log);
        assert_eq!(x, y, "replay determinístico — projeção idêntica");
    }

    /// **Roundtrip REAL (zero-mock): append no `EventStore` → `replay`.** Prova o caminho de produção
    /// ponta-a-ponta (serialização + `from_record`), não só o `from_records` puro.
    #[test]
    fn replay_from_event_store() {
        let dir = std::env::temp_dir().join(format!(
            "lina-code-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(&dir).expect("abre store temporário");
        store
            .append(&DomainEvent::CodeChanged {
                branch: "lina/f3-4-a".into(),
                paths: vec!["crates/lina-core/src/guard.rs".into()],
                author_node: "n1".into(),
                commit: "abc123".into(),
            })
            .expect("append CodeChanged");
        let c = CodeIntegration::replay(&store).expect("replay do store");
        assert!(c.is_branch_pending("lina/f3-4-a"));
        assert!(!c.pending_branches().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
