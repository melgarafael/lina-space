//! `lina-host` — a fronteira entre o CORE e o SHELL de UI (story **W0-11**).
//!
//! ## Âncora de continuidade #1 (ver `CLAUDE.md`)
//! O CORE Rust fala com a superfície de UI **apenas** por este contrato. Isso mantém
//! a UI **substituível** (gpui ↔ Slint, decidido no spike da Onda 1) sem reescrever
//! o cérebro. **Nenhum** crate do core pode importar tipos de um framework de UI, e
//! **este crate não importa nenhum** (`wgpu`/`gpui`/`slint`/`winit`/`vello`/…): sua
//! única dependência é `uuid`. Trocar de framework = reimplementar este contrato no
//! novo shell, sem tocar no core.
//!
//! > **Gate de fronteira (CI, fora deste crate):** `cargo tree -p lina-core` não pode
//! > conter nenhum crate de toolkit; um lint de CI falha se `lina-core` referenciar
//! > `wgpu|gpui|slint|winit|vello|cosmic_text|accesskit`. Documentado aqui; aplicado
//! > no CI (não neste crate).
//!
//! ## O contrato em duas direções
//! - **core → UI:** [`UiHost::on_event`] recebe [`HostEvent`]s. A UI é uma **projeção
//!   descartável** que reconstrói seu estado aplicando esses deltas.
//! - **UI → core:** [`InputSink::submit`] envia [`WriteOp`]s (teclas do humano, texto
//!   de A2A, submit). O core implementa `InputSink`; o shell apenas o invoca.
//!
//! ## Garantias do core
//! - Os eventos de um mesmo nó chegam **em ordem** (a ordem em que o core os produziu).
//! - [`HostEvent::GridDelta`] carrega **exatamente** as linhas sujas do viewport desde
//!   o último delta daquele nó (vindas do `damaged_rows()` do `VtBackend`, W0-2) — a UI
//!   re-renderiza só o delta, nunca o grid inteiro.
//! - `Recovering`/`Recovered` vêm **emparelhados** (todo `Recovering` é seguido por um
//!   `Recovered`); fora desse par, o app está em estado normal.
//! - O core **nunca** chama de volta o shell fora de `UiHost`; o shell **nunca** fala
//!   com o core fora de `InputSink`.
//!
//! ## Obrigações da UI
//! - `on_event` **não deve bloquear** (é chamado do laço do core); enfileire/aplique e
//!   retorne. Trabalho pesado de render acontece no próprio laço da UI.
//! - Como [`HostEvent`], [`NodeKind`], [`NodeStatus`], [`BusTarget`] e [`WriteOp`] são
//!   `#[non_exhaustive]`, todo `match` num shell **deve** ter um braço `_ => {}` para
//!   sobreviver à adição de variantes em versões futuras **sem recompilar o contrato**.

use uuid::Uuid;

/// Identificador estável de um nó no canvas (UUID v7, ordenável por tempo).
pub type NodeId = Uuid;

/// Tipo de um nó do canvas — o shell precisa disto para saber **o que desenhar**.
/// Taxonomia canônica (ver doc `20 - Arquitetura Tecnica` §2.1); na Onda 0 só
/// `Terminal` é produzido, mas o contrato já nasce aberto para o resto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeKind {
    Terminal,
    Note,
    WebBrowser,
    Folder,
    Attachment,
    Trigger,
    TaskPanel,
    Feed,
}

/// Estado de presença/atividade de um nó. Cobre o ciclo de vida que o core produz:
/// `running/busy/dead` (Bus, W0-4), `crashed` (pty-host contém panic, W0-3), `idle`
/// (idle-detection do A2A, W0-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeStatus {
    /// Spawn em andamento (PTY abrindo / bootstrap).
    Starting,
    /// Processo vivo e ocioso (prompt pronto).
    Idle,
    /// Processo vivo e produzindo saída.
    Busy,
    /// F3-CONF-2 (#21): vivo, porém PAUSADO pelo core (ex.: disjuntor de entrega). Distinto de
    /// `Busy` — a tela honesta mostra "pausado por segurança", não "trabalhando". O motivo viaja
    /// no `reason` de `HostEvent::NodeStatusChanged`, não nesta variante (mantém `Copy`).
    Blocked,
    /// Vivo, sem distinção idle/busy disponível.
    Running,
    /// O painel quebrou (panic contido pelo pty-host); o processo, não.
    Crashed,
    /// O processo do CLI morreu.
    Dead,
}

/// Destinatário de uma mensagem de barramento (endereçamento do Bus, W0-4).
/// A UI usa isto para animar a metáfora "sem fios" (pulso A→B / aura por papel).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BusTarget {
    Node(NodeId),
    Role(String),
    Broadcast,
}

/// Eventos que o core empurra para a UI (a UI é uma projeção descartável do estado).
///
/// Mínimo e `#[non_exhaustive]`: variantes podem ser adicionadas sem quebrar shells
/// que já tratem o braço `_`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HostEvent {
    /// Um nó passou a existir no canvas.
    NodeAdded { node: NodeId, kind: NodeKind },
    /// Mudança de presença/status de um nó (idle↔busy, crashed, dead…).
    /// F3-CONF-2 (#21): `reason` carrega o motivo surfacável (ex.: `"circuit_breaker"`) quando há um
    /// — DADO para a tela honesta, nunca autoridade. `None` na maioria das transições.
    NodeStatusChanged {
        node: NodeId,
        status: NodeStatus,
        reason: Option<String>,
    },
    /// Um nó deixou de existir (removido / `node.died`).
    NodeRemoved { node: NodeId },
    /// Linhas sujas no grid de um terminal — a UI re-renderiza só o delta.
    /// `dirty_rows` são índices de linha do viewport (de `VtBackend::damaged_rows`).
    GridDelta {
        node: NodeId,
        dirty_rows: Vec<usize>,
    },
    /// Tráfego A2A no barramento (para feedback visual: ghost wire / linha do tempo).
    BusMessage { from: NodeId, to: BusTarget },
    /// Recuperação pós-crash em andamento — a UI mostra o banner (W0-6 / W4-4).
    Recovering,
    /// Recuperação concluída — a UI sai do modo de recuperação.
    Recovered,
}

/// Operação de escrita do shell para o core, roteada pela MailQueue serial do nó
/// (humano e agente passam pela MESMA fila — nunca interleavam bytes no master, W0-4).
/// A entrega A2A é **faseada** (CLAUDE.md): `AgentText` e depois `Submit` separados.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteOp {
    /// Teclas cruas do humano (inclui controles, ex.: Ctrl-C = `0x03`).
    HumanKeys(Vec<u8>),
    /// Texto injetado por um agente (A2A), antes do submit.
    AgentText(String),
    /// Confirmação de submissão (Enter `0x0D`), enviada como passo próprio.
    Submit,
}

/// **core → UI.** O contrato que qualquer shell de UI implementa.
///
/// O core chama `on_event`; o shell renderiza. Mantém o split core/shell e a porta
/// de troca de framework aberta. `Send + 'static` porque o core roda o shell a partir
/// do seu próprio laço/thread.
pub trait UiHost: Send + 'static {
    /// Entrega um evento do core para a UI projetar. **Não deve bloquear.**
    fn on_event(&mut self, event: HostEvent);
}

/// **UI → core.** Canal de input do shell para o core. O **core** implementa este
/// trait (roteando para as MailQueues); o shell apenas chama `submit`. Definido aqui
/// (e não no core) para que `lina-host` permaneça o crate de contrato puro — quem
/// depende de quem: o **core depende de `lina-host`**, nunca o contrário.
pub trait InputSink: Send + Sync {
    /// Enfileira uma escrita para o terminal `node` (ordem serial garantida pelo core).
    fn submit(&self, node: NodeId, op: WriteOp);
}

/// Host de UI **headless** — o consumidor do gate da Onda 0. Não desenha nada: apenas
/// registra (loga) os eventos recebidos, provando que o core roda ponta a ponta sem
/// nenhum framework de UI. É também a base dos testes de contrato.
#[derive(Debug, Default)]
pub struct HeadlessUiHost {
    events: Vec<HostEvent>,
}

impl HeadlessUiHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Todos os eventos recebidos, em ordem.
    pub fn events(&self) -> &[HostEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Linhas sujas acumuladas para um nó (soma dos `GridDelta` recebidos) — útil para
    /// o gate W0 asseverar que o delta de um terminal real chegou.
    pub fn dirty_rows_for(&self, node: NodeId) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .events
            .iter()
            .filter_map(|e| match e {
                HostEvent::GridDelta {
                    node: n,
                    dirty_rows,
                } if *n == node => Some(dirty_rows),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }
}

impl UiHost for HeadlessUiHost {
    fn on_event(&mut self, event: HostEvent) {
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sink de input dummy: registra os `WriteOp` recebidos (prova o caminho UI→core).
    #[derive(Default)]
    struct RecordingSink {
        ops: std::sync::Mutex<Vec<(NodeId, WriteOp)>>,
    }
    impl InputSink for RecordingSink {
        fn submit(&self, node: NodeId, op: WriteOp) {
            // `lock()` num Mutex sem envenenamento aqui; em produção o core usa um
            // canal mpsc. Evitamos `unwrap()`: em caso de poison, descartamos.
            if let Ok(mut ops) = self.ops.lock() {
                ops.push((node, op));
            }
        }
    }

    fn node() -> NodeId {
        Uuid::now_v7()
    }

    /// Critério de aceite: um shell dummy implementa `UiHost` e recebe uma sequência
    /// de `HostEvent` cobrindo os variantes principais — a porta de troca de framework
    /// está aberta.
    #[test]
    fn dummy_shell_receives_full_event_sequence() {
        let mut shell = HeadlessUiHost::new();
        let n = node();

        let seq = [
            HostEvent::Recovering,
            HostEvent::NodeAdded {
                node: n,
                kind: NodeKind::Terminal,
            },
            HostEvent::NodeStatusChanged {
                node: n,
                status: NodeStatus::Starting,
                reason: None,
            },
            HostEvent::GridDelta {
                node: n,
                dirty_rows: vec![0, 1, 2],
            },
            HostEvent::NodeStatusChanged {
                node: n,
                status: NodeStatus::Busy,
                reason: None,
            },
            HostEvent::GridDelta {
                node: n,
                dirty_rows: vec![2, 5],
            },
            HostEvent::BusMessage {
                from: n,
                to: BusTarget::Role("reviewer".into()),
            },
            HostEvent::NodeStatusChanged {
                node: n,
                status: NodeStatus::Idle,
                reason: None,
            },
            HostEvent::NodeRemoved { node: n },
            HostEvent::Recovered,
        ];
        for ev in seq.iter().cloned() {
            shell.on_event(ev);
        }

        assert_eq!(shell.len(), 10);
        // Projeção do delta: linhas sujas acumuladas do terminal = união {0,1,2,5}.
        assert_eq!(shell.dirty_rows_for(n), vec![0, 1, 2, 5]);
        // O par de recuperação chegou emparelhado (primeiro e último).
        assert!(matches!(
            shell.events().first(),
            Some(HostEvent::Recovering)
        ));
        assert!(matches!(shell.events().last(), Some(HostEvent::Recovered)));
    }

    /// Um shell pode classificar todos os variantes (match exaustivo no crate que
    /// define o enum). Shells downstream usam o braço `_` por causa de `#[non_exhaustive]`.
    #[test]
    fn every_host_event_variant_is_dispatchable() {
        fn label(ev: &HostEvent) -> &'static str {
            match ev {
                HostEvent::NodeAdded { .. } => "node_added",
                HostEvent::NodeStatusChanged { .. } => "node_status",
                HostEvent::NodeRemoved { .. } => "node_removed",
                HostEvent::GridDelta { .. } => "grid_delta",
                HostEvent::BusMessage { .. } => "bus_message",
                HostEvent::Recovering => "recovering",
                HostEvent::Recovered => "recovered",
            }
        }
        let n = node();
        assert_eq!(
            label(&HostEvent::NodeAdded {
                node: n,
                kind: NodeKind::Note
            }),
            "node_added"
        );
        assert_eq!(
            label(&HostEvent::GridDelta {
                node: n,
                dirty_rows: vec![]
            }),
            "grid_delta"
        );
        assert_eq!(
            label(&HostEvent::BusMessage {
                from: n,
                to: BusTarget::Broadcast
            }),
            "bus_message"
        );
        assert_eq!(label(&HostEvent::Recovering), "recovering");
    }

    /// Caminho de input UI→core: as três operações de escrita (humano, agente, submit)
    /// chegam ao sink na ordem enviada — entrega A2A faseada (texto depois submit).
    #[test]
    fn input_sink_records_write_ops_in_order() {
        let sink = RecordingSink::default();
        let n = node();
        sink.submit(n, WriteOp::AgentText("revise o diff".into()));
        sink.submit(n, WriteOp::Submit);
        sink.submit(n, WriteOp::HumanKeys(vec![0x03])); // Ctrl-C

        let ops = sink.ops.lock().expect("mutex não envenenado no teste");
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0], (n, WriteOp::AgentText("revise o diff".into())));
        assert_eq!(ops[1], (n, WriteOp::Submit));
        assert_eq!(ops[2], (n, WriteOp::HumanKeys(vec![0x03])));
    }

    /// O contrato é objeto-seguro dos dois lados (dyn) — o core segura `dyn UiHost` e
    /// o shell segura `dyn InputSink`.
    #[test]
    fn contract_traits_are_object_safe() {
        let mut shell: Box<dyn UiHost> = Box::new(HeadlessUiHost::new());
        shell.on_event(HostEvent::Recovering);
        let sink: std::sync::Arc<dyn InputSink> = std::sync::Arc::new(RecordingSink::default());
        sink.submit(node(), WriteOp::Submit);
    }

    /// Garante os bounds do contrato: `UiHost: Send + 'static`, `InputSink: Send + Sync`.
    #[test]
    fn contract_bounds_hold() {
        fn assert_send_static<T: Send + 'static>() {}
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_static::<HeadlessUiHost>();
        assert_send_sync::<RecordingSink>();
    }
}
