//! **W4-2 — modelo de cena do CANVAS T4 (gpui-free).** O TRONCO da Onda 4: classificação de ZONAS
//! (foco / periferia / suspenso) e BADGE agregado por nó. É o **lar do P0** (degradação de ociosos,
//! red-team doc 12 §6.3 / §0 do design): **poucos em foco, muitos em fila** — 1-4 terminais GRANDES no
//! foco (120fps), o resto em PERIFERIA como cartão COMPACTO com badge honesto, e tudo fora da viewport
//! vira SUSPENSO (0 draw-calls — **reusa o culling W2-3**, não reimplementa).
//!
//! Puro e testável: o render gpui (em `main`) CONSOME este modelo. Manter aqui (e não no `main`) é a
//! modularização P0 do shell (design §5) que libera o leque paralelo W4-3/W4-4/W4-5.

use lina_host::NodeStatus;

/// F1-2-6: focus manager do canvas (widget composto + roving tabindex). O registro do módulo
/// vive AQUI via `#[path]` — promover para `mod canvas_focus;` no crate root é costura futura
/// (mesmo caminho que o `a11y_live` já trilhou na F2-2-2: de `#[path]` a `mod` no crate root).
#[path = "canvas_focus.rs"]
pub mod focus;

/// F2-3-1: lógica pura de arrastar-e-mover um card (delta sob zoom, ordem-z por fractional index,
/// mover-vs-pan, máquina de gesto). Registrado via `#[path]` pelo MESMO caminho do `focus` —
/// promover a `mod canvas_drag;` no crate root é costura futura. Consumido pela fiação de mouse
/// em `main` (dono: Maestro).
#[path = "canvas_drag.rs"]
pub mod drag;

/// F2-3-3: encaixe passivo (snap) ao arrastar — o card imanta num alinhamento limpo (bordas +
/// respiro canônico) ao chegar perto de outro, sem pular sozinho. Registrado via `#[path]` pelo
/// mesmo precedente do `drag`/`focus`. Consumido pela costura de mouse em `main` (dono: Maestro).
#[path = "canvas_snap.rs"]
pub mod snap;

/// F2-3-6: tecla "o" cicla pela fila de aprovação — pula ao próximo nó que pede aprovação, com
/// wrap-around. Registrado via `#[path]` pelo mesmo precedente do `drag`/`snap`/`focus`. Consumido
/// pela tecla em `handle_key` (main, dono: Maestro). A câmera só anda com gesto — a tecla É o gesto.
#[path = "canvas_cycle.rs"]
pub mod cycle;

/// F2-3-5: zoom inteligente (enquadrar tudo / focar seleção / LOD com histerese). Registrado
/// via `#[path]` aqui pelo MESMO caminho que `focus` — promover a `mod canvas_zoom;` no crate
/// root é costura futura. Funções puras consumidas pela fiação de tecla em `main` (dono: Maestro).
#[path = "canvas_zoom.rs"]
pub mod zoom;

/// F2-3-4: "Arrumar" — grade limpa de 4 colunas (espelha `grid_slot`) e preset "lado a lado".
/// Funções puras indexadas. Registrado via `#[path]` pelo mesmo precedente do `zoom`/`drag`/`focus`.
/// Consumido pela costura de paleta/atalho em `main` (dono: Maestro), que emite `NodeMoved` por card.
#[path = "canvas_layout.rs"]
pub mod layout;

/// Teto de nós GRANDES no foco. Red-team doc 12: atenção humana sustenta ~2-6; produto mira **1-4
/// legíveis**. Acima disto a leitura degrada — o excedente vai para a periferia (fila). HOJE o canvas
/// usa FOCO ÚNICO (o nó focado); reservado para o multi-foco (pin de até MAX_FOCUS) das próximas stories.
#[allow(dead_code)]
pub const MAX_FOCUS: usize = 4;

/// A zona de um nó no canvas T4 — decide COMO ele é desenhado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Em foco: terminal GRANDE, alta frequência. 1-4 nós (o que o humano realmente opera).
    Focus,
    /// Periferia: cartão COMPACTO com badge agregado — visível e honesto, mas barato.
    Periphery,
    /// Fora da viewport: SUSPENSO (0 draw-calls). Reusa o culling W2-3.
    Suspended,
}

/// Classe do badge agregado de um nó na periferia — **estado honesto e sem jargão** (invariante #6).
/// Ordem de prioridade do enum = ordem de urgência (NeedsYou primeiro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    /// "precisa de você" — bloqueado aguardando uma decisão humana (ex.: gate de custódia pendente).
    NeedsYou,
    /// "pausado por segurança" (#21) — o CORE pausou o nó (disjuntor de entrega). VIVO mas
    /// PARADO: distinto de `Running` (não está trabalhando) e de `Stopped` (não morreu). A saída
    /// é um gesto humano (liberar); a copy de superfície carrega esse caminho de recuperação.
    Paused,
    /// "rodando" — vivo e produzindo saída.
    Running,
    /// "aguardando" — VIVO mas sem saída recente (o agente terminou e espera você). **NÃO é
    /// "dormindo"**: o CLI continua rodando; só não está produzindo output AGORA. Nunca por falta de FOCO.
    Waiting,
    /// "encerrado" — o processo do CLI morreu (Dead) ou o painel quebrou (Crashed).
    Stopped,
}

/// Badge agregado de um nó: a classe + quantas linhas novas o humano ainda não viu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub kind: BadgeKind,
    /// Linhas de saída novas desde a última vez que o humano olhou o nó (0 = nada novo).
    pub unseen: u32,
}

impl Badge {
    /// Texto curto, honesto e sem jargão do badge (o que o leigo lê no cartão da periferia).
    #[must_use]
    pub fn label(&self) -> String {
        match self.kind {
            BadgeKind::NeedsYou => "precisa de você".to_string(),
            // #21: a copy canônica (estado + saída de 1 clique) vive em produção no crate root e
            // é ASSERTADA pelo gate (`honest_state_tests`); a11y e header leem a MESMA frase.
            BadgeKind::Paused => crate::CIRCUIT_BREAKER_LABEL.to_string(),
            BadgeKind::Running => {
                if self.unseen > 0 {
                    format!("rodando · {} novas", self.unseen)
                } else {
                    "rodando".to_string()
                }
            }
            BadgeKind::Waiting => {
                if self.unseen > 0 {
                    format!("aguardando · {} novas", self.unseen)
                } else {
                    "aguardando".to_string()
                }
            }
            BadgeKind::Stopped => "encerrado".to_string(),
        }
    }

    /// Cor de fundo do badge — TOKENS do design system (F1-2-1): warning=precisa de você,
    /// confirm=rodando, neutro fosco=aguardando, danger fosco=encerrado.
    /// BUG B: a periferia voltou a desenhar o GRID (legível), então o render NÃO usa mais este chip de
    /// badge; reservado p/ um chip de status futuro (`allow(dead_code)` no padrão do módulo).
    #[allow(dead_code)]
    #[must_use]
    pub fn bg(&self) -> u32 {
        let th = crate::theme::active();
        match self.kind {
            BadgeKind::NeedsYou => th.state.warning, // o mesmo do gate de custódia
            // #21: cor de ATENÇÃO própria — o acento do produto (≠ âmbar de "rodando", ≠ vermelho
            // de "encerrado"). O glifo + o texto carregam o sentido sem depender só da cor (1.4.1).
            BadgeKind::Paused => th.accent.primary,
            BadgeKind::Running => th.accent.confirm,
            BadgeKind::Waiting => th.surface.raised_alt, // neutro, NÃO alarmante
            BadgeKind::Stopped => th.surface.danger_muted,
        }
    }
}

/// **Classifica a zona de um nó.** O culling (W2-3) é a fonte de `in_viewport` (mesmo `card_visible`
/// do render) → fora da viewport SEMPRE suspende (0 draw-calls), independente de foco. Dentro da
/// viewport, `focused` decide foco vs periferia. Determinístico, sem efeito colateral.
#[must_use]
pub fn classify_zone(focused: bool, in_viewport: bool) -> Zone {
    if !in_viewport {
        Zone::Suspended
    } else if focused {
        Zone::Focus
    } else {
        Zone::Periphery
    }
}

/// **Agrega o badge de um nó** a partir dos sinais disponíveis: `status` (projeção do nó), `needs_human`
/// (ex.: há um gate de custódia pendente para este nó — "precisa de você" vence tudo) e `unseen`
/// (linhas novas não-vistas). Determinístico. ZERO interpretação semântica de conteúdo (invariante #1).
#[must_use]
pub fn aggregate_badge(status: NodeStatus, needs_human: bool, unseen: u32) -> Badge {
    let kind = if needs_human {
        BadgeKind::NeedsYou
    } else {
        match status {
            NodeStatus::Busy | NodeStatus::Running | NodeStatus::Starting => BadgeKind::Running,
            // BUG 1: VIVO mas sem saída recente = "aguardando" (NÃO "dormindo"). Um chat aberto que
            // terminou a resposta está vivo/pronto — nunca "dormindo" por falta de foco ou de output.
            NodeStatus::Idle => BadgeKind::Waiting,
            // #21: o CORE pausou este nó (disjuntor) — VIVO mas parado. Sem este braço o `_`
            // abaixo o leria como "rodando" (a MESMA mentira que o #21 corrige).
            NodeStatus::Blocked => BadgeKind::Paused,
            NodeStatus::Crashed | NodeStatus::Dead => BadgeKind::Stopped,
            // `NodeStatus` é `#[non_exhaustive]`: um estado FUTURO do core cai aqui → assume "rodando"
            // (vivo, neutro) em vez de fingir encerrado.
            _ => BadgeKind::Running,
        }
    };
    Badge { kind, unseen }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O teto de foco é pequeno (atenção humana ~2-6, red-team doc 12 §6.3) — "poucos em foco".
    #[test]
    fn max_focus_matches_human_attention_budget() {
        assert!(
            (1..=6).contains(&MAX_FOCUS),
            "1-4 grandes no foco; nunca dezenas vivas"
        );
    }

    /// Fora da viewport SEMPRE suspende (culling), mesmo "focado"; dentro, foco vs periferia.
    #[test]
    fn classify_zone_prioritizes_viewport_then_focus() {
        assert_eq!(
            classify_zone(true, false),
            Zone::Suspended,
            "off-viewport = suspenso"
        );
        assert_eq!(classify_zone(false, false), Zone::Suspended);
        assert_eq!(
            classify_zone(true, true),
            Zone::Focus,
            "focado + visível = foco"
        );
        assert_eq!(
            classify_zone(false, true),
            Zone::Periphery,
            "sem foco + visível = periferia"
        );
    }

    /// O badge é honesto: needs_human vence; Busy/Running/Starting=rodando; Idle=💤; Dead/Crashed=encerrado.
    #[test]
    fn aggregate_badge_maps_status_with_needs_human_priority() {
        assert_eq!(
            aggregate_badge(NodeStatus::Idle, true, 0).kind,
            BadgeKind::NeedsYou,
            "needs_human vence o status"
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Busy, false, 0).kind,
            BadgeKind::Running
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Running, false, 0).kind,
            BadgeKind::Running
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Starting, false, 0).kind,
            BadgeKind::Running
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Idle, false, 0).kind,
            BadgeKind::Waiting,
            "BUG 1: VIVO mas idle = aguardando, NUNCA dormindo"
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Blocked, false, 0).kind,
            BadgeKind::Paused,
            "#21: pausado pelo disjuntor NÃO pode cair em 'rodando' (o `_` mentiria)"
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Dead, false, 0).kind,
            BadgeKind::Stopped
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Crashed, false, 0).kind,
            BadgeKind::Stopped
        );
    }

    /// O label inclui "N novas" quando há não-vistas e é sem jargão.
    #[test]
    fn badge_label_is_honest_and_counts_unseen() {
        assert_eq!(
            aggregate_badge(NodeStatus::Idle, false, 3).label(),
            "aguardando · 3 novas"
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Idle, false, 0).label(),
            "aguardando"
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Busy, false, 0).label(),
            "rodando"
        );
        assert_eq!(
            aggregate_badge(NodeStatus::Busy, true, 9).label(),
            "precisa de você"
        );
    }
}
