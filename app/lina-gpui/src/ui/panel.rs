//! **F2-2-1 · Panel** — superfície contida (container) e card selecionável da fusão T1+T3.
//! Flat honesto: hierarquia por **degrau de tom + borda 1px**, zero sombra/gradiente (a sombra de
//! ancoragem do light é F2-2-3, não aqui). builder + `RenderOnce`, token-puro.
//!
//! O valor central é encapsular o **idioma do card selecionável de geometria uniforme** — hoje
//! TRIPLICADO e bug-prone (galeria de papéis, opções de autonomia, fila de atenção): borda `2px`
//! SEMPRE presente (cor = `focus.ring` quando ativo, `transparent` quando não → a seleção nunca faz
//! reflow) + troca de fundo `raised_alt`↔base. Isso vive em [`card_skin`], função PURA testável.
//!
//! Toast/badge/banner que COMUNICAM estado ficam de FORA (F2-2-2, sobre o Element de live-region —
//! ADR 0028): aqui só superfície e card.

use gpui::{
    div, prelude::*, px, rgb, transparent_black, AnyElement, App, ClickEvent, ElementId,
    IntoElement, Role, SharedString, Window,
};

use super::RadiusExt;
use crate::theme::{self, Theme};

/// Qual fundo de superfície o painel usa (token, nunca cor crua).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Canvas,
    Panel,
    Card,
    Chrome,
    Raised,
    RaisedAlt,
    SelectedRow,
}

impl Surface {
    #[must_use]
    pub fn token(self, t: &Theme) -> u32 {
        match self {
            Surface::Canvas => t.surface.canvas,
            Surface::Panel => t.surface.panel,
            Surface::Card => t.surface.card,
            Surface::Chrome => t.surface.chrome,
            Surface::Raised => t.surface.raised,
            Surface::RaisedAlt => t.surface.raised_alt,
            Surface::SelectedRow => t.surface.selected_row,
        }
    }
}

/// Passo de espaçamento (escala 4/8/12/16/24/32 + `Zero`) — mapeia para `SpacingTokens`, nunca px cru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// Sem espaço (rótulo+descrição colados, ex.: card de autonomia).
    Zero,
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
    Xxl,
}

impl Space {
    #[must_use]
    pub fn px(self, t: &Theme) -> f32 {
        f32::from(match self {
            Space::Zero => 0,
            Space::Xs => t.spacing.xs,
            Space::Sm => t.spacing.sm,
            Space::Md => t.spacing.md,
            Space::Lg => t.spacing.lg,
            Space::Xl => t.spacing.xl,
            Space::Xxl => t.spacing.xxl,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelVariant {
    /// Container raiz (caixa de modal, painel docado, dropdown, coluna): borda 1px.
    Surface,
    /// Item de lista selecionável: geometria uniforme (borda 2px sempre), anel quando ativo.
    Card,
}

/// **A pele do card selecionável** — `(bg, cor_da_borda)` por estado. Borda SEMPRE 2px no render
/// (geometria uniforme: selecionar não faz reflow); aqui só a COR e o FUNDO mudam. Função pura: é o
/// idioma hoje triplicado e bug-prone, agora com um único teste de verdade.
#[must_use]
pub fn card_skin(selected: bool, base_bg: u32, t: &Theme) -> (u32, Option<u32>) {
    if selected {
        (t.surface.raised_alt, Some(t.focus.ring))
    } else {
        // `None` = borda transparente: a geometria (2px) existe SEMPRE no render, só a COR muda —
        // selecionar não faz reflow.
        (base_bg, None)
    }
}

#[derive(IntoElement)]
pub struct Panel {
    id: Option<ElementId>,
    variant: PanelVariant,
    surface: Surface,
    selected: bool,
    /// Borda explícita do Surface (ex.: caixa de confirmação com borda de acento). `None` = padrão
    /// do variant (`surface.border` 1px para Surface; o anel para Card).
    border: Option<u32>,
    bordered: bool,
    pad: (Space, Space),
    gap: Space,
    row: bool,
    role: Option<Role>,
    aria_label: Option<SharedString>,
    on_click: Option<super::ClickHandler>,
    children: Vec<AnyElement>,
}

impl Panel {
    fn base(variant: PanelVariant, surface: Surface) -> Self {
        Self {
            id: None,
            variant,
            surface,
            selected: false,
            border: None,
            bordered: true,
            pad: (Space::Md, Space::Md),
            gap: Space::Sm,
            row: false,
            role: None,
            aria_label: None,
            on_click: None,
            children: Vec::new(),
        }
    }

    /// Container raiz (fundo `card`, borda 1px).
    pub fn surface() -> Self {
        Self::base(PanelVariant::Surface, Surface::Card)
    }
    /// Item de lista selecionável (fundo `panel`, geometria uniforme).
    pub fn card() -> Self {
        let mut p = Self::base(PanelVariant::Card, Surface::Panel);
        p.pad = (Space::Md, Space::Sm);
        p
    }

    #[must_use]
    pub fn bg(mut self, s: Surface) -> Self {
        self.surface = s;
        self
    }
    #[must_use]
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    #[must_use]
    pub fn border(mut self, color: u32) -> Self {
        self.border = Some(color);
        self
    }
    #[must_use]
    pub fn borderless(mut self) -> Self {
        self.bordered = false;
        self
    }
    #[must_use]
    pub fn pad(mut self, x: Space, y: Space) -> Self {
        self.pad = (x, y);
        self
    }
    #[must_use]
    pub fn gap(mut self, g: Space) -> Self {
        self.gap = g;
        self
    }
    #[must_use]
    pub fn row(mut self) -> Self {
        self.row = true;
        self
    }
    #[must_use]
    pub fn col(mut self) -> Self {
        self.row = false;
        self
    }
    #[must_use]
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
    #[must_use]
    pub fn role(mut self, r: Role) -> Self {
        self.role = Some(r);
        self
    }
    #[must_use]
    pub fn aria(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }
    /// Clique. Passe `cx.listener(...)`. Requer `.id(...)` (a11y/foco).
    #[must_use]
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
    #[must_use]
    pub fn child(mut self, c: impl IntoElement) -> Self {
        self.children.push(c.into_any_element());
        self
    }
    #[must_use]
    pub fn children(mut self, cs: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children
            .extend(cs.into_iter().map(IntoElement::into_any_element));
        self
    }
}

impl RenderOnce for Panel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let base_bg = self.surface.token(&t);
        let (px_x, px_y, gap) = (self.pad.0.px(&t), self.pad.1.px(&t), self.gap.px(&t));

        let styled = div()
            .flex()
            .map(|d| if self.row { d.flex_row() } else { d.flex_col() })
            .px(px(px_x))
            .py(px(px_y))
            .gap(px(gap))
            .rounded_chrome()
            .map(|d| match self.variant {
                PanelVariant::Card => {
                    // Borda 2px SEMPRE (geometria uniforme); só a cor muda — anel quando
                    // selecionado, transparente quando não (anti-reflow).
                    let (bg, ring) = card_skin(self.selected, base_bg, &t);
                    d.bg(rgb(bg))
                        .border_2()
                        .border_color(ring.map_or_else(transparent_black, |c| rgb(c).into()))
                }
                PanelVariant::Surface => {
                    let d = d.bg(rgb(base_bg));
                    if self.bordered {
                        d.border_1()
                            .border_color(rgb(self.border.unwrap_or(t.surface.border)))
                    } else {
                        d
                    }
                }
            })
            .children(self.children);

        // role/aria/on_click exigem id (StatefulInteractiveElement). Container sem id → div puro.
        match self.id {
            Some(id) => {
                let mut el = styled.id(id);
                if let Some(role) = self.role {
                    el = el.role(role);
                }
                if let Some(aria) = self.aria_label {
                    el = el.aria_label(aria);
                }
                if let Some(handler) = self.on_click {
                    el = el.cursor_pointer().on_click(handler);
                }
                el.into_any_element()
            }
            None => styled.into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;

    fn theme() -> Theme {
        Theme::build(Mode::Dark, "azul")
    }

    /// O idioma do card selecionável: ATIVO troca fundo para `raised_alt` e pinta o anel `focus.ring`;
    /// INATIVO mantém o fundo base e a borda fica transparente — a geometria (2px) nunca reflow.
    #[test]
    fn card_skin_swaps_only_color_not_geometry() {
        let t = theme();
        let base = t.surface.panel;
        let (bg_on, ring_on) = card_skin(true, base, &t);
        assert_eq!(bg_on, t.surface.raised_alt);
        assert_eq!(
            ring_on,
            Some(t.focus.ring),
            "selecionado pinta o anel de foco"
        );
        let (bg_off, ring_off) = card_skin(false, base, &t);
        assert_eq!(
            bg_off, base,
            "não-selecionado mantém o fundo base (sem reflow)"
        );
        assert_eq!(
            ring_off, None,
            "não-selecionado = sem anel (borda transparente)"
        );
    }

    /// Cada superfície mapeia para o token correto (nenhuma cor crua no call-site).
    #[test]
    fn surface_maps_to_token() {
        let t = theme();
        assert_eq!(Surface::Card.token(&t), t.surface.card);
        assert_eq!(Surface::Panel.token(&t), t.surface.panel);
        assert_eq!(Surface::Chrome.token(&t), t.surface.chrome);
        assert_eq!(Surface::RaisedAlt.token(&t), t.surface.raised_alt);
    }

    /// A escala de espaçamento sai dos tokens (4/8/12/16/24/32).
    #[test]
    fn space_maps_to_spacing_scale() {
        let t = theme();
        assert_eq!(Space::Md.px(&t), f32::from(t.spacing.md));
        assert_eq!(Space::Xl.px(&t), f32::from(t.spacing.xl));
    }
}
