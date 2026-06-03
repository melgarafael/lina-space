//! **TETO DE CUSTO REAL (backlog #22) — metragem de uso por nó (gpui-free).** O core
//! (`CostLedger`/`CostCeilingHit`/`lina resume`) já SOMA `TokenUsageReported` e PAUSA no teto, provado
//! headless; faltava o app ALIMENTAR. Este módulo é a fonte de uso.
//!
//! ## Fonte da contagem de tokens (decisão de design — ver `.entrega-teto-real.md`)
//! O CLI roda como **shell interativo (TUI renderizada)** — não há NDJSON/`result` nem telemetria de
//! tokens exposta pelo PTY, e o `CliProfile` não declara parse de uso. A única fonte VIÁVEL é uma
//! **ESTIMATIVA por BYTES de output do turno** (`estimate_tokens`). O fim-de-turno é por **IDLE**
//! (a ideia do W0-10: silêncio por `idle_ms`).
//!
//! **Margem/segurança:** a saída TUI inclui **redraws ANSI** (a tela inteira a cada token), então a
//! estimativa por bytes **SUPER-conta** → o teto PAUSA MAIS CEDO que o uso real. Para um **teto de
//! segurança (anti-runaway), errar pausando cedo é o lado seguro** — não é um medidor de cobrança. O
//! fundador calibra `token_budget_day` nesta unidade (≈ bytes/4). O caminho-ouro (parse de `usage` do
//! `result` NDJSON) entra quando o app capturar a CLI em modo stream-json (futuro).

use std::collections::HashMap;

use lina_host::NodeId;

/// Heurística canônica ~**4 bytes/token** (texto). Conservadora de propósito para um TETO DE SEGURANÇA.
#[must_use]
pub fn estimate_tokens(output_bytes: u64) -> u64 {
    output_bytes / 4
}

/// Acumulador de UM turno por nó: bytes de output + quando saiu pela última vez (relógio lógico ms).
#[derive(Debug, Clone, Copy)]
struct TurnAcc {
    bytes: u64,
    last_output_ms: u64,
}

/// **Medidor de custo por nó.** Acumula bytes de output (alimentado pelo stream de `GridDelta`) e
/// fecha um TURNO por IDLE (silêncio ≥ `idle_ms` desde a última saída), rendendo a estimativa de tokens
/// do turno. Determinístico (relógio lógico injetado) → testável headless.
#[derive(Debug, Default)]
pub struct CostMeter {
    turns: HashMap<NodeId, TurnAcc>,
}

impl CostMeter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra `bytes` de output do `node` em `now_ms` — abre/estende o turno corrente.
    pub fn record_output(&mut self, node: NodeId, bytes: usize, now_ms: u64) {
        let acc = self.turns.entry(node).or_insert(TurnAcc {
            bytes: 0,
            last_output_ms: now_ms,
        });
        acc.bytes = acc.bytes.saturating_add(bytes as u64);
        acc.last_output_ms = now_ms;
    }

    /// Fecha os turnos que ficaram IDLE (sem output há ≥ `idle_ms`) e devolve `(node, tokens_estimados)`
    /// para cada — REMOVENDO-os (o próximo output abre um turno novo). Só rende turnos com bytes > 0.
    pub fn poll_finished_turns(&mut self, now_ms: u64, idle_ms: u64) -> Vec<(NodeId, u64)> {
        let finished: Vec<NodeId> = self
            .turns
            .iter()
            .filter(|(_, acc)| {
                acc.bytes > 0 && now_ms.saturating_sub(acc.last_output_ms) >= idle_ms
            })
            .map(|(n, _)| *n)
            .collect();
        finished
            .into_iter()
            .filter_map(|n| {
                self.turns
                    .remove(&n)
                    .map(|acc| (n, estimate_tokens(acc.bytes)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn node() -> NodeId {
        Uuid::now_v7()
    }

    /// ~4 bytes/token; conservadora (saturada em 0 p/ pouco output).
    #[test]
    fn estimate_tokens_is_bytes_over_four() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(3), 0);
        assert_eq!(estimate_tokens(4), 1);
        assert_eq!(estimate_tokens(4000), 1000);
    }

    /// Um turno fecha SÓ após idle (silêncio ≥ idle_ms); enquanto há output, não fecha.
    #[test]
    fn turn_closes_only_after_idle_and_yields_estimate() {
        let mut m = CostMeter::new();
        let n = node();
        m.record_output(n, 4000, 1000); // turno produz 4000 bytes @ t=1000
                                        // Ainda dentro da janela de atividade (idle_ms=200) → nada fecha.
        assert!(
            m.poll_finished_turns(1100, 200).is_empty(),
            "100ms < idle_ms: turno aberto"
        );
        // Silêncio ≥ idle_ms → fecha com a estimativa (4000/4 = 1000 tokens).
        let finished = m.poll_finished_turns(1300, 200);
        assert_eq!(
            finished,
            vec![(n, 1000)],
            "fecha o turno e rende a estimativa"
        );
        // Já fechado → não rende de novo (até novo output).
        assert!(m.poll_finished_turns(2000, 200).is_empty());
    }

    /// Output contínuo ESTENDE o turno (cada saída reabre a janela de idle).
    #[test]
    fn continued_output_extends_the_turn() {
        let mut m = CostMeter::new();
        let n = node();
        m.record_output(n, 100, 1000);
        m.record_output(n, 100, 1150); // 150ms depois: estende
        assert!(
            m.poll_finished_turns(1300, 200).is_empty(),
            "150ms desde a última saída < 200"
        );
        let finished = m.poll_finished_turns(1360, 200); // 210ms desde 1150
        assert_eq!(
            finished,
            vec![(n, estimate_tokens(200))],
            "soma os 200 bytes do turno"
        );
    }

    /// Turno sem bytes não rende (idle puro não cobra).
    #[test]
    fn idle_without_output_yields_nothing() {
        let mut m = CostMeter::new();
        assert!(m.poll_finished_turns(9999, 200).is_empty());
    }
}
