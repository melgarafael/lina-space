//! `lina-host` — a fronteira entre o CORE e o SHELL de UI (story **W0-11**).
//!
//! ## Âncora de continuidade #1 (ver `CLAUDE.md`)
//! O CORE Rust fala com a superfície de UI **apenas** por esta trait. Isso mantém
//! a UI **substituível** (gpui ↔ Slint, decidido no spike da Onda 1) sem reescrever
//! o cérebro. **Nenhum** crate do core pode importar tipos de um framework de UI.
//!
//! Este é um esqueleto: a story W0-11 preenche a trait conforme o core (bus,
//! event store) ganha forma. Mantenha o contrato mínimo e estável.

use uuid::Uuid;

/// Identificador estável de um nó no canvas (UUID v7, ordenável por tempo).
pub type NodeId = Uuid;

/// Eventos que o core empurra para a UI (a UI é uma projeção descartável do estado).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HostEvent {
    /// Um nó passou a existir / mudou de estado.
    NodeChanged { node: NodeId },
    /// Linhas novas/sujas no grid de um terminal (a UI re-renderiza só o delta).
    GridDelta { node: NodeId },
    /// Recuperação pós-crash em andamento — a UI mostra o banner (ver W0-6 / W4-4).
    Recovering,
    /// Recuperação concluída — a UI sai do modo de recuperação.
    Recovered,
}

/// O contrato que qualquer shell de UI implementa para o core.
///
/// O core chama estes métodos; a UI nunca chama o core diretamente fora deste
/// contrato. Mantém o split core/shell e a porta de troca de framework aberta.
pub trait UiHost: Send + 'static {
    /// Entrega um evento do core para a UI renderizar.
    fn on_event(&mut self, event: HostEvent);
    // TODO(W0-11): expandir conforme o core amadurece (apresentar nós, câmera,
    // hit-test) — mantendo o mínimo necessário e ZERO tipos de toolkit aqui.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Garante que um shell de teste pode implementar o contrato (a porta de
    /// troca de UI está aberta).
    #[test]
    fn ui_host_is_implementable_by_a_dummy_shell() {
        struct DummyShell {
            events: usize,
        }
        impl UiHost for DummyShell {
            fn on_event(&mut self, _event: HostEvent) {
                self.events += 1;
            }
        }
        let mut shell = DummyShell { events: 0 };
        shell.on_event(HostEvent::Recovering);
        shell.on_event(HostEvent::Recovered);
        assert_eq!(shell.events, 2);
    }
}
