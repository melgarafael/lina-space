//! **F2-2-1 · Input** — campo de texto da fusão T1+T3. **Importante:** as telas do shell NÃO usam
//! um `Editor`/`TextInput` real do gpui; cada "campo" é SIMULADO — o estado vive num `String` no
//! modelo gpui-free, o roteamento de teclas é central (o funil de `main.rs` chama
//! `type_char`/`backspace`/`paste`), e o campo só PINTA: valor + caret `▌` de alto contraste (cor do
//! anel de foco, elemento separado que nunca é cortado) quando focado + placeholder (apagado quando
//! vazio e sem foco) + borda `focus.ring` no foco.
//!
//! Cor reservada a significado (fusão): o campo NÃO tem fill colorido; erro usa `danger_muted` +
//! borda `state.danger`, nunca vermelho gratuito. Doutrina de motion: digitação é input frequente →
//! `instant` (0ms), NUNCA animada. A lógica de exibição é PURA ([`Input::display_text`],
//! [`Input::shows_placeholder`]) — testável sem render.

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, ElementId, IntoElement, SharedString, Window,
};

use crate::theme::{self, Theme};

/// Glifo de caret (LEFT HALF BLOCK). No single-line é pintado como elemento separado de alto
/// contraste (cor do anel de foco) que o `flex_shrink_0` impede de ser cortado; no multiline segue
/// inline no texto (a caixa rola na vertical, não há corte horizontal a temer).
const CARET: &str = "\u{258c}";

/// Fator de leading do campo multilinha: a tipografia (F2-1) ainda NÃO tem token de line-height, então
/// a altura de N linhas = `rows · size.body · este fator`. Trocar por um token quando ele existir.
const MULTILINE_LEADING: f32 = 1.6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputKind {
    SingleLine,
    /// Campo de N linhas com rolagem (ex.: doutrina do template).
    Multiline {
        rows: f32,
    },
    /// Só colar (ex.: chave do Lina PRO): sem cursor, o clique lê o clipboard.
    PasteOnly,
}

#[derive(IntoElement)]
pub struct Input {
    id: ElementId,
    value: SharedString,
    placeholder: SharedString,
    focused: bool,
    kind: InputKind,
    /// Erro de validação → fundo `danger_muted` + borda `state.danger` (cor com significado).
    invalid: bool,
    mono: bool,
    aria_label: Option<SharedString>,
    on_click: Option<super::ClickHandler>,
}

impl Input {
    pub fn new(id: impl Into<ElementId>, value: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            placeholder: SharedString::default(),
            focused: false,
            kind: InputKind::SingleLine,
            invalid: false,
            mono: false,
            aria_label: None,
            on_click: None,
        }
    }

    #[must_use]
    pub fn placeholder(mut self, p: impl Into<SharedString>) -> Self {
        self.placeholder = p.into();
        self
    }
    #[must_use]
    pub fn focused(mut self, on: bool) -> Self {
        self.focused = on;
        self
    }
    #[must_use]
    pub fn multiline(mut self, rows: f32) -> Self {
        self.kind = InputKind::Multiline { rows };
        self
    }
    #[must_use]
    pub fn paste_only(mut self) -> Self {
        self.kind = InputKind::PasteOnly;
        self
    }
    #[must_use]
    pub fn invalid(mut self, on: bool) -> Self {
        self.invalid = on;
        self
    }
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    #[must_use]
    pub fn aria(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }
    /// Clique (foca o campo no modelo, ou cola no `PasteOnly`). Passe `cx.listener(...)`.
    #[must_use]
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mostra o placeholder (apagado) — só quando vazio E sem foco.
    #[must_use]
    pub fn shows_placeholder(&self) -> bool {
        self.value.is_empty() && !self.focused
    }

    /// O caret aparece só com foco e em campo digitável (o `PasteOnly` não tem cursor de digitação).
    #[must_use]
    pub fn caret_visible(&self) -> bool {
        self.focused && !matches!(self.kind, InputKind::PasteOnly)
    }

    /// O corpo do campo SEM o caret: placeholder se vazio-e-sem-foco, senão o valor. O render
    /// single-line pinta o caret como elemento separado de alto contraste a partir deste corpo.
    #[must_use]
    pub fn body_text(&self) -> String {
        if self.shows_placeholder() {
            self.placeholder.to_string()
        } else {
            self.value.to_string()
        }
    }

    /// Corpo + caret inline achatados (lógica PURA testável; é o que o multiline pinta): o corpo mais
    /// o glifo `▌` quando [`Input::caret_visible`]. O single-line NÃO usa isto — pinta o caret colorido
    /// à parte (ver `render`), para o caret ter cor própria e não ser cortado no overflow.
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.caret_visible() {
            format!("{}{CARET}", self.body_text())
        } else {
            self.body_text()
        }
    }

    /// `(bg, borda, texto)` resolvidos — único ponto que toca tokens de cor. Erro veste o vermelho
    /// de estado; foco veste o anel; placeholder fica apagado.
    #[must_use]
    fn paint(&self, t: &Theme) -> (u32, u32, u32) {
        let bg = if self.invalid {
            t.surface.danger_muted
        } else {
            t.surface.raised
        };
        let border = if self.invalid {
            t.state.danger
        } else if self.focused {
            t.focus.ring
        } else {
            t.surface.border
        };
        let text = if self.shows_placeholder() {
            t.text.muted
        } else {
            t.text.primary
        };
        (bg, border, text)
    }
}

impl RenderOnce for Input {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let (bg, border, text) = self.paint(&t);
        // Caret de alto contraste: a cor do ANEL de foco (= acento do usuário, com ≥3:1 garantido pelo
        // gate WCAG nos 2 temas). Usar o mesmo pigmento da borda de foco reforça "isto está focado".
        let caret_color = t.focus.ring;
        let show_caret = self.caret_visible();
        let body = self.body_text();
        let inline = self.display_text();
        let kind = self.kind;
        let mono = self.mono;
        let touch_floor = f32::from(t.spacing.xl);
        let family = if mono {
            t.typography.family.mono
        } else {
            t.typography.family.ui
        };
        let id = self.id;
        let aria = self.aria_label;
        let on_click = self.on_click;

        let mut el = div()
            .id(id)
            // min_w(0) deixa o campo encolher no flex; overflow_hidden corta o texto longo.
            .min_w(px(0.))
            .min_h(px(touch_floor))
            .overflow_hidden()
            .px(px(f32::from(t.spacing.md)))
            .py(px(f32::from(t.spacing.sm)))
            .rounded_md()
            .bg(rgb(bg))
            .border_1()
            .border_color(rgb(border))
            .text_size(px(f32::from(t.typography.size.body)))
            .text_color(rgb(text))
            .font_family(family)
            .map(|d| match kind {
                InputKind::Multiline { rows } => d
                    .h(px(rows
                        * f32::from(t.typography.size.body)
                        * MULTILINE_LEADING))
                    .overflow_y_scroll(),
                // Single-line/paste: linha (row) p/ o texto encolher e o caret ficar fixo ao lado.
                _ => d.flex().items_center(),
            });
        if let Some(aria) = aria {
            el = el.aria_label(aria);
        }
        if let Some(handler) = on_click {
            el = el.cursor_pointer().on_click(handler);
        }

        match kind {
            // Multiline: o texto quebra linha e rola na vertical; o caret segue inline no fim.
            InputKind::Multiline { .. } => el.child(inline),
            // Single-line: corpo clipável (min_w 0 + overflow) + caret de alto contraste que o
            // `flex_shrink_0` mantém SEMPRE visível, mesmo quando o valor enche o campo.
            _ => {
                let mut el = el.child(div().min_w(px(0.)).overflow_hidden().child(body));
                if show_caret {
                    el = el.child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(caret_color))
                            .child(CARET),
                    );
                }
                el
            }
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

    /// Placeholder só aparece vazio-e-sem-foco; focar um campo vazio já mostra o cursor, não o placeholder.
    #[test]
    fn placeholder_only_when_empty_and_blurred() {
        let empty = Input::new("f", "");
        assert!(empty.shows_placeholder());
        assert!(!empty.focused(true).shows_placeholder());
        assert!(!Input::new("f", "oi").shows_placeholder());
    }

    /// O cursor `▌` é anexado ao valor quando focado — exceto no campo só-colar (sem digitação).
    #[test]
    fn caret_appended_when_focused_except_paste_only() {
        assert_eq!(
            Input::new("f", "lina").focused(true).display_text(),
            "lina▌"
        );
        assert_eq!(
            Input::new("f", "lina").focused(false).display_text(),
            "lina"
        );
        assert_eq!(
            Input::new("f", "chave")
                .paste_only()
                .focused(true)
                .display_text(),
            "chave",
            "campo só-colar não tem cursor de digitação"
        );
        // vazio focado mostra só o cursor (não o placeholder).
        assert_eq!(
            Input::new("f", "")
                .placeholder("digite")
                .focused(true)
                .display_text(),
            "▌"
        );
        assert_eq!(
            Input::new("f", "").placeholder("digite").display_text(),
            "digite"
        );
    }

    /// `caret_visible` decide o caret: só com foco E em campo digitável (o `PasteOnly` não tem).
    #[test]
    fn caret_visible_only_when_focused_and_typable() {
        assert!(Input::new("f", "x").focused(true).caret_visible());
        assert!(!Input::new("f", "x").caret_visible(), "sem foco, sem caret");
        assert!(
            !Input::new("f", "x")
                .paste_only()
                .focused(true)
                .caret_visible(),
            "campo só-colar não tem caret de digitação"
        );
    }

    /// `body_text` é o corpo SEM caret (o caret single-line é pintado à parte, colorido). Vazio-focado
    /// dá corpo vazio (o caret separado é que aparece); vazio-sem-foco dá o placeholder.
    #[test]
    fn body_text_never_includes_caret() {
        assert_eq!(Input::new("f", "lina").focused(true).body_text(), "lina");
        assert_eq!(Input::new("f", "lina").body_text(), "lina");
        assert_eq!(
            Input::new("f", "").placeholder("digite").body_text(),
            "digite"
        );
        assert_eq!(
            Input::new("f", "")
                .placeholder("digite")
                .focused(true)
                .body_text(),
            "",
            "vazio-focado: corpo vazio, o caret separado é que indica o foco"
        );
    }

    /// Erro veste o vermelho de estado (fundo+borda); foco veste o anel; normal usa a borda neutra.
    #[test]
    fn paint_couples_color_to_meaning() {
        let t = theme();
        let invalid = Input::new("f", "x").invalid(true).paint(&t);
        assert_eq!(
            invalid,
            (t.surface.danger_muted, t.state.danger, t.text.primary)
        );
        let focused = Input::new("f", "x").focused(true).paint(&t);
        assert_eq!(focused.1, t.focus.ring, "foco veste o anel");
        let blurred = Input::new("f", "").paint(&t);
        assert_eq!(blurred.1, t.surface.border);
        assert_eq!(blurred.2, t.text.muted, "vazio-sem-foco = texto apagado");
    }
}
