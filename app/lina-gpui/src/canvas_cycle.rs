//! `canvas_cycle` — **F2-3-6: tecla "o" cicla pela fila de aprovação (lógica PURA, gpui-free).**
//!
//! O canvas tem uma fila de atenção (F1): nós com gate de custódia pendente — terminais que
//! "precisam de você". A tecla **"o"** pula a vista para o PRÓXIMO desses nós e cicla por todos
//! (foca + revela um a cada toque), para o fundador nunca varrer terminal por terminal nem se perder.
//!
//! Este módulo é só a CICLAGEM pura: dada a lista de quem pede aprovação (em ordem estável) e o foco
//! atual, devolve o próximo na ordem cíclica (wrap-around). O `focus`+`reveal` (que move a câmera) é
//! costura do Maestro em `main.rs`, disparada SÓ ao apertar "o".
//!
//! **Invariante da onda — "a câmera só anda com gesto":** a tecla "o" É o gesto. Esta função não
//! toca câmera nem tem efeito; só calcula o alvo. Sem o toque, nada se move.
//!
//! **Ordem estável:** a costura passa `pending` JÁ ordenada (ex.: ordem da fila da `desk` ou por z) —
//! a mesma entrada produz sempre a mesma saída. Esta função não reordena.

#![allow(dead_code)] // consumidor é a tecla "o" em handle_key (main.rs, dono: Maestro); exercitado por teste.

use lina_host::NodeId;

/// **Próximo nó que pede aprovação, em ordem cíclica.** `pending` = nós com gate pendente (ordem
/// estável, montada pela costura); `current` = foco atual (se houver).
///
/// - `pending` vazio → `None` (no-op: não há para onde ir).
/// - `current` é um dos `pending` → o SEGUINTE, com wrap-around (`…→ último → primeiro`).
/// - `current` é `None` ou não está em `pending` (foco fora da fila) → o PRIMEIRO da fila.
///
/// Lista de 1: o wrap-around natural devolve o próprio único nó (`(0+1) % 1 == 0`) — apertar "o" com
/// um só pendente RE-revela ele (re-centraliza se você scrollou para longe), em vez de um no-op
/// confuso. "o" sempre garante que você está vendo um nó que pede aprovação. Determinística e total.
#[must_use]
pub fn next_pending(pending: &[NodeId], current: Option<NodeId>) -> Option<NodeId> {
    if pending.is_empty() {
        return None;
    }
    match current.and_then(|c| pending.iter().position(|&n| n == c)) {
        Some(i) => Some(pending[(i + 1) % pending.len()]),
        None => Some(pending[0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> NodeId {
        uuid::Uuid::from_u128(n)
    }

    /// Cicla 1→2→3→1 (wrap-around) — não-vacuoso: remover o `% len` quebra o retorno ao primeiro.
    #[test]
    fn cycles_with_wraparound() {
        let q = [id(1), id(2), id(3)];
        assert_eq!(next_pending(&q, Some(id(1))), Some(id(2)));
        assert_eq!(next_pending(&q, Some(id(2))), Some(id(3)));
        assert_eq!(
            next_pending(&q, Some(id(3))),
            Some(id(1)),
            "último volta ao primeiro"
        );
    }

    /// Sem foco corrente → começa no primeiro da fila.
    #[test]
    fn none_current_starts_at_first() {
        let q = [id(1), id(2), id(3)];
        assert_eq!(next_pending(&q, None), Some(id(1)));
    }

    /// Foco fora da fila (ex.: você estava num nó que já resolveu) → vai ao primeiro pendente.
    #[test]
    fn current_not_in_list_goes_to_first() {
        let q = [id(1), id(2), id(3)];
        assert_eq!(next_pending(&q, Some(id(99))), Some(id(1)));
    }

    /// Fila vazia → None (nada pede aprovação; a tecla não faz nada).
    #[test]
    fn empty_queue_is_none() {
        assert_eq!(next_pending(&[], Some(id(1))), None);
        assert_eq!(next_pending(&[], None), None);
    }

    /// Um único pendente: "o" sempre revela ele — esteja você nele ou não.
    #[test]
    fn single_pending_always_targets_it() {
        let q = [id(7)];
        assert_eq!(next_pending(&q, None), Some(id(7)));
        assert_eq!(
            next_pending(&q, Some(id(7))),
            Some(id(7)),
            "já nele → re-revela (re-centraliza)"
        );
        assert_eq!(
            next_pending(&q, Some(id(1))),
            Some(id(7)),
            "fora → vai a ele"
        );
    }
}
