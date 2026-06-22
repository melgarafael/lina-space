//! F3-5-6 · frente CONTEXTO (dono: Terminal I) — **pistas**.
//!
//! Pistas (doc-fonte 65): o usuário seleciona pastas/arquivos que a IA passa a enxergar num
//! projeto → `ClueSetDefined { scope, paths, label }`; a projeção [`ClueSet`] reconstrói o
//! conjunto de pistas por ESCOPO por replay do log (padrão `CostLedger`: o último
//! `ClueSetDefined` de um `scope` vence). Uma camada do briefing (`briefing.rs`, também de I)
//! lê esta projeção e injeta os `paths` no contexto do terminal daquele projeto.
//!
//! ## Invariantes honrados
//! - **`paths` é DADO, jamais autoridade** (regra-mãe da onda): a pista só INFORMA o que a IA
//!   olha; não concede acesso além do que o nó já tem. Por isso `define_clue` não valida
//!   existência em disco nem trata `paths` como permissão — só registra o fato no log.
//! - **Projeção do log (inv #4):** `from_records` é PURA sobre `EventRecord`s; replay reconstrói
//!   byte-a-byte (sem relógio, sem I/O). Eventos indecodificáveis são pulados (a projeção é
//!   DERIVADA, não validadora do log — espelha `Mentality`/`ApprovalExecutor::replay`).
//! - **Remover a pista = re-definir com `paths` vazio** (some no próximo turno). Não há evento de
//!   remoção (inv #4: nada de variante nova fora das 10 da largada) — o `scope` com `paths` vazio
//!   RETRAI a chave, e o fold determinístico faz a pista sumir no replay seguinte.

use std::collections::BTreeMap;

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};

/// Uma pista resolvida de um escopo — o `paths`/`label` do último `ClueSetDefined` daquele `scope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clue {
    /// Pastas/arquivos que o terminal do escopo passa a enxergar. DADO de contexto, nunca autoridade.
    pub paths: Vec<String>,
    /// Rótulo humano opcional da pista (ex.: "código do checkout").
    pub label: Option<String>,
}

/// Projeção das pistas por ESCOPO (padrão `CostLedger`: último `ClueSetDefined` por `scope` vence).
/// `BTreeMap` garante ordem estável → relatório/fingerprint determinístico.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClueSet {
    by_scope: BTreeMap<String, Clue>,
}

impl ClueSet {
    /// Reconstrói a projeção do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log e reconstrói a pista
    /// corrente de cada escopo. Testável com log sintético — não embute relógio nem I/O.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut by_scope: BTreeMap<String, Clue> = BTreeMap::new();
        for record in records {
            // Registro indecodificável (versão futura): a projeção é DERIVADA, não validadora do
            // log (espelha `Mentality`/`ApprovalExecutor::replay`) — pula sem panicar.
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            if let DomainEvent::ClueSetDefined {
                scope,
                paths,
                label,
            } = event
            {
                if paths.is_empty() {
                    // Redefinição com `paths` vazio = REMOÇÃO da pista (sem evento de remoção,
                    // inv #4): o escopo é retraído e some no próximo briefing.
                    by_scope.remove(&scope);
                } else {
                    // Último-vence (padrão `CostLedger`): substitui a pista anterior do escopo.
                    by_scope.insert(scope, Clue { paths, label });
                }
            }
        }
        Self { by_scope }
    }

    /// Os `paths` que o terminal de `scope` enxerga (slice vazia se não há pista ativa). Gating por
    /// escopo: um terminal só recebe as pistas do SEU escopo (o caller passa o escopo do nó).
    #[must_use]
    pub fn paths_for(&self, scope: &str) -> &[String] {
        self.by_scope.get(scope).map_or(&[], |c| c.paths.as_slice())
    }

    /// A pista completa (`paths` + `label`) do escopo, se houver uma ativa.
    #[must_use]
    pub fn clue_for(&self, scope: &str) -> Option<&Clue> {
        self.by_scope.get(scope)
    }

    /// `true` se nenhum escopo tem pista ativa.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_scope.is_empty()
    }

    /// Os escopos com pista ativa, em ordem estável.
    pub fn scopes(&self) -> impl Iterator<Item = &str> {
        self.by_scope.keys().map(String::as_str)
    }
}

// ─────────────────────────── handlers do verbo `lina clue` (Maestro fia o dispatch) ───────────────────────────

/// Define (ou substitui) a pista do `scope`: emite `ClueSetDefined`. Açúcar de remoção: `paths`
/// vazio (ou só entradas em branco) RETRAI a pista do escopo (some no próximo briefing).
/// `paths` é DADO de contexto — não validamos existência em disco nem o tratamos como autoridade.
///
/// # Errors
/// Falha ao persistir o evento.
pub fn define_clue(
    store: &mut EventStore,
    scope: &str,
    paths: &[String],
    label: Option<String>,
) -> Result<u64, StoreError> {
    // Higiene: descarta entradas em branco (um `paths` vazio após a limpeza é a forma de REMOVER).
    let cleaned: Vec<String> = paths
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    store.append(&DomainEvent::ClueSetDefined {
        scope: scope.trim().to_string(),
        paths: cleaned,
        label: label.and_then(|l| {
            let t = l.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }),
    })
}

/// Remove a pista do `scope` (açúcar para [`define_clue`] com `paths` vazio).
///
/// # Errors
/// Falha ao persistir o evento.
pub fn clear_clue(store: &mut EventStore, scope: &str) -> Result<u64, StoreError> {
    define_clue(store, scope, &[], None)
}

/// Relatório legível das pistas ativas (para `lina clue` / `lina clue list`). Honesto: sem pistas
/// → uma linha que diz isso, nunca uma lista falsa.
///
/// # Errors
/// Falha ao ler o event log.
pub fn clue_report(store: &EventStore) -> Result<String, StoreError> {
    Ok(render_clue_report(&ClueSet::replay(store)?))
}

/// Render PURO do relatório de pistas (separado do I/O para teste determinístico).
#[must_use]
fn render_clue_report(clues: &ClueSet) -> String {
    if clues.is_empty() {
        return "pistas: — (nenhuma pista definida ainda)".to_string();
    }
    let mut out = String::from("pistas ativas (o que a IA enxerga por projeto):\n");
    for scope in clues.scopes() {
        if let Some(clue) = clues.clue_for(scope) {
            let label = clue
                .label
                .as_deref()
                .map_or_else(String::new, |l| format!(" — {l}"));
            out.push_str(&format!("  [{scope}]{label}\n"));
            for path in &clue.paths {
                out.push_str(&format!("    - {path}\n"));
            }
        }
    }
    out.trim_end().to_string()
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Constrói um `EventRecord` de `ClueSetDefined` serializado COMO no log: a tag interna `event`
    /// (enum `#[serde(tag = "event")]`) precisa estar no payload, senão `from_record` não decodifica.
    /// Por isso serializamos o `DomainEvent` REAL (via `kind`/`current_version`/`serde_json`), nunca
    /// um JSON montado à mão sem a tag.
    fn clue_record(seq: u64, scope: &str, paths: &[&str], label: Option<&str>) -> EventRecord {
        let event = DomainEvent::ClueSetDefined {
            scope: scope.to_string(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
            label: label.map(str::to_string),
        };
        EventRecord {
            seq,
            ts: seq, // ts irrelevante para a projeção de pistas (último-vence por ordem de log).
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa o evento"),
        }
    }

    /// Critério (c): pista definida → o terminal do escopo enxerga os arquivos.
    #[test]
    fn defined_clue_is_visible_for_its_scope() {
        let log = [clue_record(
            1,
            "checkout",
            &["src/checkout/", "docs/api.md"],
            Some("código do checkout"),
        )];
        let clues = ClueSet::from_records(&log);
        assert_eq!(
            clues.paths_for("checkout"),
            &["src/checkout/".to_string(), "docs/api.md".to_string()],
            "o escopo enxerga exatamente os paths definidos"
        );
        assert_eq!(
            clues.clue_for("checkout").and_then(|c| c.label.as_deref()),
            Some("código do checkout")
        );
    }

    /// Critério (c) — gating por escopo: um escopo SÓ vê as próprias pistas (zero vazamento).
    #[test]
    fn clue_is_scoped_no_leak_across_scopes() {
        let log = [
            clue_record(1, "checkout", &["src/checkout/"], None),
            clue_record(2, "landing", &["src/landing/"], None),
        ];
        let clues = ClueSet::from_records(&log);
        assert_eq!(clues.paths_for("checkout"), &["src/checkout/".to_string()]);
        assert_eq!(clues.paths_for("landing"), &["src/landing/".to_string()]);
        assert!(
            clues.paths_for("inexistente").is_empty(),
            "escopo sem pista → slice vazia (nunca vaza de outro escopo)"
        );
    }

    /// Padrão `CostLedger`: o ÚLTIMO `ClueSetDefined` de um escopo vence (substitui, não acumula).
    #[test]
    fn last_definition_per_scope_wins() {
        let log = [
            clue_record(1, "checkout", &["antigo/"], Some("v1")),
            clue_record(2, "checkout", &["novo/", "outro/"], Some("v2")),
        ];
        let clues = ClueSet::from_records(&log);
        assert_eq!(
            clues.paths_for("checkout"),
            &["novo/".to_string(), "outro/".to_string()],
            "a redefinição substitui a pista anterior"
        );
        assert_eq!(
            clues.clue_for("checkout").and_then(|c| c.label.as_deref()),
            Some("v2")
        );
    }

    /// Critério (c) — remover a pista (redefinir com `paths` vazio) → some no próximo turno.
    #[test]
    fn redefining_with_empty_paths_retracts_the_clue() {
        let log = [
            clue_record(1, "checkout", &["src/checkout/"], None),
            clue_record(2, "checkout", &[], None), // remoção
        ];
        let clues = ClueSet::from_records(&log);
        assert!(
            clues.paths_for("checkout").is_empty(),
            "pista removida → escopo não enxerga mais nada"
        );
        assert!(
            clues.clue_for("checkout").is_none(),
            "o escopo é retraído da projeção, não fica como pista vazia"
        );
    }

    /// Replay determinístico (inv #4): mesma sequência de eventos → projeção idêntica.
    #[test]
    fn replay_is_deterministic() {
        let log = [
            clue_record(1, "a", &["x/"], None),
            clue_record(2, "b", &["y/"], None),
            clue_record(3, "a", &["z/"], None),
        ];
        assert_eq!(ClueSet::from_records(&log), ClueSet::from_records(&log));
    }

    /// Relatório honesto: sem pistas → diz isso (nunca lista falsa); com pistas → lista por escopo.
    #[test]
    fn report_is_honest() {
        let empty = render_clue_report(&ClueSet::default());
        assert!(empty.contains("nenhuma pista definida"));

        let log = [clue_record(1, "checkout", &["src/checkout/"], Some("loja"))];
        let report = render_clue_report(&ClueSet::from_records(&log));
        assert!(report.contains("[checkout]"));
        assert!(report.contains("loja"));
        assert!(report.contains("src/checkout/"));
    }
}
