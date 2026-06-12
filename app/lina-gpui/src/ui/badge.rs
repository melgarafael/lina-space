//! **F2-2-2 · Badge** — selo de ESTADO persistente (status do Agente/Espaço/fila: "Trabalhando",
//! "Pronto", "Precisa de você"). Sobre o `Element` de live-region (ADR 0028: **nasce compondo** — ao
//! mudar o `value`, o adapter anuncia a TROCA de estado). builder + `RenderOnce`, **token-puro**.
//!
//! **WCAG 1.4.1 (cor nunca sozinha):** texto + glifo + cor SEMPRE — o badge nunca comunica só pela
//! cor (daltônicos e leitores de tela recebem a mesma informação pela palavra e pelo ícone).
//!
//! **Cortesia (decisão da story):** `polite` por padrão (um estado que mudou não interrompe a fala);
//! `assertive` SÓ quando o badge significa "precisa de você AGORA" ([`Badge::needs_you`]) — assertar à
//! toa treina o usuário a ignorar o leitor de tela. A cortesia é escolha do CALL-SITE, não do tom (um
//! `Danger` pode ser histórico passivo; um `Warning` pode exigir ação) — por isso é um opt-in, não
//! derivado da cor.

use gpui::{div, prelude::*, px, rgb, ElementId, IntoElement, SharedString};

use crate::a11y_live::{live_region, Politeness};
use crate::theme::{self, Theme};

/// Tom semântico do estado. Cor = significado; [`Self::glyph`] é a SEGUNDA via (WCAG 1.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeTone {
    /// Concluído/saudável (Pronto, Conectado) — verde de estado.
    Success,
    /// Em andamento/informativo (Trabalhando, Sincronizando) — acento do shell.
    Info,
    /// Atenção (Limite perto, Rede instável) — âmbar de estado.
    Warning,
    /// Falha/bloqueio (Erro, Recusado) — vermelho de estado.
    Danger,
    /// Sem carga semântica (Ocioso, Pausado) — superfície quente, sem acento.
    Neutral,
}

impl BadgeTone {
    /// `(cor_texto_e_ícone, fundo_discreto)` — ÚNICO ponto de cor. Fundos "muted" onde o tema os tem
    /// (sucesso/perigo têm superfície discreta dedicada); os demais pousam na superfície quente.
    #[must_use]
    pub fn paint(self, t: &Theme) -> (u32, u32) {
        match self {
            BadgeTone::Success => (t.state.success, t.surface.success_muted),
            BadgeTone::Info => (t.accent.primary, t.surface.raised),
            BadgeTone::Warning => (t.state.warning, t.surface.raised),
            BadgeTone::Danger => (t.state.danger, t.surface.danger_muted),
            BadgeTone::Neutral => (t.text.secondary, t.surface.raised),
        }
    }

    /// O glifo redundante à cor — informação, não decoração (WCAG 1.4.1).
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            BadgeTone::Success => "●",
            BadgeTone::Info => "◐",
            BadgeTone::Warning => "▲",
            BadgeTone::Danger => "■",
            BadgeTone::Neutral => "○",
        }
    }
}

/// A cortesia do anúncio de troca de estado: `assertive` só quando o badge "precisa de você"
/// ([`Badge::needs_you`]); senão `polite`. Pura — para travar a doutrina no teste.
#[must_use]
pub fn announce_politeness(needs_you: bool) -> Politeness {
    if needs_you {
        Politeness::Assertive
    } else {
        Politeness::Polite
    }
}

/// **O badge** (catálogo `ui/`). builder + `RenderOnce`, nasce compondo o live-region. O `id` ESTÁVEL
/// por badge é o que permite ao adapter detectar a MUDANÇA de `value` (mesmo nó, value novo = anúncio).
#[derive(IntoElement)]
pub struct Badge {
    id: ElementId,
    /// A PALAVRA do estado (o que o humano lê e o leitor de tela fala). Nunca vazia (1.4.1).
    label: SharedString,
    tone: BadgeTone,
    /// Glifo opcional sobrepondo o default do tom (ex.: ✦ para "Novo").
    icon: Option<SharedString>,
    /// `true` = "precisa de você" → anúncio assertive (interrompe). Default `false` (polite).
    needs_you: bool,
}

impl Badge {
    /// Novo badge. `id` ESTÁVEL (mesmo badge ao longo do tempo) — é o que faz a troca de estado
    /// anunciar em vez de criar um nó novo. `label` é a palavra do estado (obrigatória, 1.4.1).
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tone: BadgeTone) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tone,
            icon: None,
            needs_you: false,
        }
    }

    /// Sobrepõe o glifo do tom.
    #[must_use]
    pub fn icon(mut self, glyph: impl Into<SharedString>) -> Self {
        self.icon = Some(glyph.into());
        self
    }

    /// Marca como "precisa de você" → o anúncio da troca de estado vira assertive (interrompe). Use com
    /// parcimônia (gate humano, erro que trava); o default polite é o certo para quase tudo.
    #[must_use]
    pub fn needs_you(mut self) -> Self {
        self.needs_you = true;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut gpui::Window, _cx: &mut gpui::App) -> impl IntoElement {
        let t = theme::active();
        let (fg, bg) = self.tone.paint(&t);
        let gap = f32::from(t.spacing.xs);
        let pad_x = f32::from(t.spacing.sm);
        let pad_y = f32::from(t.spacing.xs);
        let size = f32::from(t.typography.size.small);
        let glyph = self
            .icon
            .clone()
            .unwrap_or_else(|| self.tone.glyph().into());

        // Pílula (rounded full), FLAT, fundo discreto + texto/ícone na cor do estado. Glifo E palavra
        // SEMPRE presentes (1.4.1): a cor é a terceira via, nunca a única.
        let pill = div()
            .flex()
            .items_center()
            .gap(px(gap))
            .px(px(pad_x))
            .py(px(pad_y))
            .rounded_full()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .text_size(px(size))
            .font_family(t.typography.family.ui)
            .child(gpui::text!(glyph))
            .child(gpui::text!(self.label.clone()));

        // ADR 0028: a troca de estado é anunciada (cortesia conforme `needs_you`). Id estável ⇒ o
        // adapter detecta a mudança de value no MESMO nó (event.rs:298-306), em vez de um nó novo.
        live_region(
            self.id,
            self.label.to_string(),
            announce_politeness(self.needs_you),
            pill,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;

    fn theme() -> Theme {
        Theme::build(Mode::Dark, "azul")
    }

    /// Cor = token do estado; e cada tom tem um glifo DISTINTO — a cor nunca é o único sinal (1.4.1).
    #[test]
    fn tone_paints_state_token_with_distinct_glyph() {
        let t = theme();
        assert_eq!(BadgeTone::Success.paint(&t).0, t.state.success);
        assert_eq!(BadgeTone::Warning.paint(&t).0, t.state.warning);
        assert_eq!(BadgeTone::Danger.paint(&t).0, t.state.danger);
        assert_eq!(BadgeTone::Info.paint(&t).0, t.accent.primary);
        assert_eq!(BadgeTone::Neutral.paint(&t).0, t.text.secondary);
        // Neutral não veste acento de estado.
        assert_ne!(BadgeTone::Neutral.paint(&t).0, t.state.success);

        let glyphs = [
            BadgeTone::Success.glyph(),
            BadgeTone::Info.glyph(),
            BadgeTone::Warning.glyph(),
            BadgeTone::Danger.glyph(),
            BadgeTone::Neutral.glyph(),
        ];
        assert_eq!(
            glyphs
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            5,
            "5 tons, 5 glifos distintos — cor nunca sozinha"
        );
    }

    /// Cortesia: `needs_you` vira assertive (interrompe); o resto é polite (não interrompe).
    #[test]
    fn needs_you_announces_assertively() {
        assert_eq!(announce_politeness(true), Politeness::Assertive);
        assert_eq!(announce_politeness(false), Politeness::Polite);
    }
}
