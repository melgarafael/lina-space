//! **F2-2-1 · Modal** — sobreposição centralizada da fusão T1+T3. builder + `RenderOnce`, compõe
//! o [`Button`] do catálogo no ✕ e nas ações (prova viva da composição).
//!
//! **Honestidade de arquitetura (ADR 0028 + costura central):** o componente NÃO sequestra o
//! teclado — o shell tem UM `handle_key` central que roteia Esc por estado. O Modal EXPÕE os hooks
//! de dismiss (✕, clique-no-véu, ações) e deixa o Esc onde está. O **véu visual** (`dim`) é
//! **opt-in, default DESLIGADO** — o véu discreto da fusão liga quando a validação na tela
//! confirmar. O **bloqueio de mouse** (`occlude`) é **default LIGADO** (M2/bugM2): um modal nunca
//! deixa o clique vazar para o canvas/terminal atrás. Como o véu occludente é transparente quando
//! `dim` está OFF, ligar a captura NÃO muda nada visualmente — só impede o vazamento. Migrar um
//! modal para cá ganha `role=Dialog`+aria (o maior buraco de a11y do shell) e a oclusão de graça,
//! SEM mudar o comportamento visual.
//!
//! O clamp do frame à janela é PURO ([`clamp_frame`]) e testado — espelha o `modal_frame` do M6.

use gpui::{
    div, prelude::*, px, rgb, AnyElement, App, ClickEvent, ElementId, IntoElement, Role,
    SharedString, Window,
};

use super::{Button, ButtonVariant};
use crate::theme;

/// Onde a caixa ancora no eixo vertical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// Centro da janela (modal de Agente).
    Centered,
    /// Ancorado a `top` px do topo (modal Criar Espaço usa ~96px).
    TopAnchored(f32),
}

/// O frame da caixa após clampar à janela: largura cabe na janela (≥ piso), altura limitada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub w: f32,
    pub max_h: f32,
}

/// Clampa a caixa à janela nos dois eixos (espelha `agent_modal::modal_frame`): a largura encolhe
/// para caber (`window_w - 2·margin`) mas nunca abaixo de `width_min`; a altura é
/// `window_h - 2·margin`, com um piso. PURA — testável sem render.
#[must_use]
pub fn clamp_frame(
    window_w: f32,
    window_h: f32,
    width_pref: f32,
    width_min: f32,
    margin: f32,
    height_floor: f32,
) -> Frame {
    Frame {
        w: width_pref.min(window_w - 2.0 * margin).max(width_min),
        max_h: (window_h - 2.0 * margin).max(height_floor),
    }
}

/// Uma ação do rodapé do modal — vira um [`Button`] da variante semântica.
pub struct ModalAction {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    on_click: super::ClickHandler,
}

impl ModalAction {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        variant: ButtonVariant,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant,
            on_click: Box::new(on_click),
        }
    }
}

#[derive(IntoElement)]
pub struct Modal {
    id: ElementId,
    frame: Frame,
    placement: Placement,
    /// Véu discreto sobre o canvas (token escuro + opacidade; nunca blur/glass). Default OFF.
    dim: bool,
    /// Bloqueia o mouse para o canvas atrás (véu transparente que captura o clique). Default ON
    /// (M2): nenhum modal deixa o clique vazar para o terminal de trás. `.occlude(false)` opta-fora.
    occlude: bool,
    /// `role(Dialog)` + nome acessível — o buraco de a11y que a consolidação fecha.
    aria_label: Option<SharedString>,
    /// Borda da caixa (default `accent.create` — a moldura "criação" do M6).
    border: Option<u32>,
    title: Option<SharedString>,
    show_close: bool,
    on_close: Option<super::ClickHandler>,
    on_backdrop: Option<super::ClickHandler>,
    actions: Vec<ModalAction>,
    body: Option<AnyElement>,
}

impl Modal {
    pub fn new(id: impl Into<ElementId>, frame: Frame) -> Self {
        Self {
            id: id.into(),
            frame,
            placement: Placement::Centered,
            dim: false,
            occlude: true,
            aria_label: None,
            border: None,
            title: None,
            show_close: false,
            on_close: None,
            on_backdrop: None,
            actions: Vec::new(),
            body: None,
        }
    }

    #[must_use]
    pub fn placement(mut self, p: Placement) -> Self {
        self.placement = p;
        self
    }
    #[must_use]
    pub fn dim(mut self, on: bool) -> Self {
        self.dim = on;
        self
    }
    /// Liga/desliga a captura de mouse pelo véu. **Default ON** (ver [`Modal::new`]) — chame
    /// `.occlude(false)` só num caso raro em que o clique DEVE atravessar para o canvas.
    #[must_use]
    pub fn occlude(mut self, on: bool) -> Self {
        self.occlude = on;
        self
    }
    #[must_use]
    pub fn aria(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }
    #[must_use]
    pub fn border(mut self, color: u32) -> Self {
        self.border = Some(color);
        self
    }
    #[must_use]
    pub fn title(mut self, t: impl Into<SharedString>) -> Self {
        self.title = Some(t.into());
        self
    }
    /// Mostra o ✕ e fecha pelo handler dado.
    #[must_use]
    pub fn close(
        mut self,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.show_close = true;
        self.on_close = Some(Box::new(on_close));
        self
    }
    /// Liga o véu + clique-fora-fecha pelo handler dado.
    #[must_use]
    pub fn dismiss_on_backdrop(
        mut self,
        on_backdrop: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_backdrop = Some(Box::new(on_backdrop));
        self
    }
    #[must_use]
    pub fn action(mut self, a: ModalAction) -> Self {
        self.actions.push(a);
        self
    }
    #[must_use]
    pub fn body(mut self, e: impl IntoElement) -> Self {
        self.body = Some(e.into_any_element());
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let body_id: ElementId = SharedString::from(format!("{}-body", &self.id)).into();
        let veil_id: ElementId = SharedString::from(format!("{}-veil", &self.id)).into();
        let panel_id = self.id.clone();

        // Cabeçalho opcional: título (cede espaço) + ✕ (não encolhe).
        let header = self.title.clone().map(|title| {
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(f32::from(t.spacing.sm)))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(f32::from(t.typography.size.title)))
                        .font_family(t.typography.family.ui)
                        .font_weight(gpui::FontWeight(f32::from(t.typography.weight.semibold)))
                        .text_color(rgb(t.text.bright))
                        .child(title),
                );
            if self.show_close {
                let close = self.on_close;
                row = row.child(close.map_or_else(
                    || {
                        Button::new("modal-close", "✕")
                            .icon()
                            .ghost()
                            .aria("Fechar")
                    },
                    |cb| {
                        Button::new("modal-close", "✕")
                            .icon()
                            .ghost()
                            .aria("Fechar")
                            .on_click(move |ev, w, cx| cb(ev, w, cx))
                    },
                ));
            }
            row
        });

        // Rodapé opcional: ações como Buttons da variante semântica, alinhadas à direita.
        let footer = if self.actions.is_empty() {
            None
        } else {
            let mut bar = div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(px(f32::from(t.spacing.sm)));
            for a in self.actions {
                let cb = a.on_click;
                bar = bar.child(
                    Button::new(a.id, a.label)
                        .variant(a.variant)
                        .no_shrink()
                        .on_click(move |ev, w, cx| cb(ev, w, cx)),
                );
            }
            Some(bar)
        };

        let mut panel = div()
            .id(panel_id)
            // O painel BLOQUEIA o mouse. Sem isto, o clique no painel atravessa para o véu
            // occludente atrás (que carrega o on_click de dismiss) e QUALQUER clique fecha o
            // modal — hitbox Normal não oclui o irmão de trás (gpui). Conserta todo modal com
            // .dismiss_on_backdrop (webhook, credential, …): o clique fora fecha, o clique dentro não.
            .occlude()
            .w(px(self.frame.w))
            .max_h(px(self.frame.max_h))
            .flex()
            .flex_col()
            .px(px(f32::from(t.spacing.xl)))
            .py(px(f32::from(t.spacing.lg)))
            .gap(px(f32::from(t.spacing.sm)))
            .rounded_lg()
            .bg(rgb(t.surface.card))
            .border_2()
            .border_color(rgb(self.border.unwrap_or(t.accent.create)));
        if let Some(aria) = self.aria_label {
            panel = panel.role(Role::Dialog).aria_label(aria);
        }
        let mut panel = panel.when_some(header, |d, h| d.child(h)).child(
            // corpo rola em bloco: min_h(0) destrava o encolhimento do filho flex-col.
            div()
                .id(body_id)
                .min_h(px(0.))
                .overflow_y_scroll()
                .w_full()
                .when_some(self.body, |d, b| d.child(b)),
        );
        if let Some(f) = footer {
            panel = panel.child(f);
        }

        // Backdrop full-bleed; o véu/occlude é um irmão ATRÁS da caixa (opacity no véu não dimina a caixa).
        let mut backdrop = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .justify_center()
            .map(|d| match self.placement {
                Placement::Centered => d.items_center(),
                Placement::TopAnchored(top) => d.items_start().pt(px(top)),
            });
        if self.dim || self.on_backdrop.is_some() || self.occlude {
            let mut veil = div()
                .id(veil_id)
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0();
            if self.dim {
                // token escuro fixo (terminal.bg) + opacidade = véu discreto nos 2 modos (sem glass).
                veil = veil.bg(rgb(t.terminal.bg)).opacity(0.5);
            }
            if self.occlude {
                veil = veil.occlude();
            }
            if let Some(cb) = self.on_backdrop {
                veil = veil.cursor_pointer().on_click(cb);
            }
            backdrop = backdrop.child(veil);
        }
        backdrop.child(panel).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Largura de projeto quando cabe; encolhe (mas nunca abaixo do mínimo) em janela estreita;
    /// altura limitada à janela com piso. Espelha `agent_modal::modal_frame`.
    #[test]
    fn clamp_frame_fits_window_both_axes() {
        // Janela larga: largura de projeto preservada.
        let f = clamp_frame(1400.0, 900.0, 640.0, 320.0, 16.0, 160.0);
        assert!((f.w - 640.0).abs() < 0.01, "largura de projeto: {}", f.w);
        assert!(
            (f.max_h - (900.0 - 32.0)).abs() < 0.01,
            "altura clampa à janela"
        );

        // Janela estreita: encolhe para caber, com margem dos dois lados.
        let f = clamp_frame(500.0, 800.0, 640.0, 320.0, 16.0, 160.0);
        assert!(f.w <= 500.0 - 32.0, "cabe com margem: {}", f.w);
        assert!(f.w >= 320.0, "nunca degenera abaixo do mínimo");

        // Janela baixíssima: altura nunca abaixo do piso.
        let f = clamp_frame(1400.0, 100.0, 640.0, 320.0, 16.0, 160.0);
        assert!((f.max_h - 160.0).abs() < 0.01, "piso de altura respeitado");
    }

    /// M2/bugM2 (NÃO-VACUOSO): o wrapper liga a captura de mouse por DEFAULT — o clique num modal
    /// nunca vaza para o terminal/canvas atrás. Se alguém reverter o default em `new()`, a 1ª
    /// asserção FALHA. O véu VISUAL (`dim`) permanece opt-in (gate-de-tela), e `.occlude(false)`
    /// segue podendo optar-fora num caso raro (prova que o toggle está conectado, não hardcoded).
    #[test]
    fn occlude_on_by_default_dim_stays_opt_in() {
        let frame = Frame {
            w: 480.0,
            max_h: 600.0,
        };
        let m = Modal::new("m", frame);
        assert!(
            m.occlude,
            "occlude default-ON: clique no modal não vaza pro canvas atrás"
        );
        assert!(!m.dim, "véu VISUAL segue opt-in (não escurece sem pedir)");
        assert!(
            !Modal::new("m", frame).occlude(false).occlude,
            ".occlude(false) opta-fora (toggle conectado)"
        );
    }
}
