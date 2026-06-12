//! **F2-2-1 · Button** — o botão do catálogo (fusão T1+T3: cor = significado acoplado ao
//! acionável, OP-1). builder + `RenderOnce`, **view-agnóstico**: o `on_click` é fechado pelo
//! call-site via `cx.listener(...)`, que converte um `Fn(&mut V, &E, &mut Window, &mut Context<V>)`
//! no `Fn(&ClickEvent, &mut Window, &mut App)` que guardamos — então o MESMO componente serve a
//! `WorkspaceView`, `OnboardingView` e `PersistenceView` sem genéricos.
//!
//! **Token-puro:** toda cor/medida vem de [`theme::active`] — zero medida literal (`px` cru) e zero
//! peso de fonte por constante no corpo: o peso vem de `t.typography.weight.*` via `FontWeight(..)`
//! (a catraca F2-1-5 nasce verde para este arquivo e CAI nos call-sites que ele substitui).
//! A decisão de cor/peso/medida por variante mora em funções PURAS ([`ButtonVariant::paint`],
//! [`ButtonSize::metrics`]) — testáveis sem render (gpui não roda headless; o render é casca fina).

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, ElementId, FontWeight, IntoElement, Role,
    SharedString, Window,
};

use crate::theme::{self, Theme};

/// Variante **por SIGNIFICADO**, não por estilo (fusão §VIII.1 / OP-1): a cor do botão É a cor do
/// que ele aciona. Acento (`action`/`create`/`confirm`) para CTAs; `state.danger` para destrutivo;
/// superfície neutra-quente (`raised`) para ação reversível — onde cor seria ruído, não há cor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// CTA de ação do time (⚡ A2A, instalar, ativar chave) — `accent.action`.
    Action,
    /// Criação (✦ Novo Agente) — `accent.create`.
    Create,
    /// Confirmação (➕ Terminal, salvar, retomar) — `accent.confirm`.
    Confirm,
    /// Ação destrutiva/irreversível (recusar, remover) — `state.danger` (OP-1: o "sim, remover"
    /// veste o vermelho do pedido).
    Destructive,
    /// Ação reversível neutra (cancelar, voltar, trocar, escolher pasta) — superfície quente, SEM
    /// cor de acento (cor é reservada a significado).
    Secondary,
    /// Disclosure/terciário (links "Avançado ▸", "por quê?", "agora não") — superfície quente discreta.
    Ghost,
    /// Par `(bg, fg)` escolhido pelo call-site — para toggles pintados POR ESTADO (som on/off,
    /// reduce-motion) e pares WCAG-cobertos legados. A cor ainda vem de TOKENS (o chamador passa
    /// `t.surface.*`/`t.state.*`), só a escolha é no call-site; não é porta para hex cru.
    Custom(u32, u32),
}

impl ButtonVariant {
    /// `(fundo, texto, ênfase)` resolvidos do tema vivo — o ÚNICO ponto que toca tokens de cor.
    /// O fundo é `None` no `Ghost` (sem fill — ✕, links de disclosure). `ênfase=true` → peso
    /// `semibold` (CTAs e destrutivo pesam mais que o corpo).
    #[must_use]
    pub fn paint(self, t: &Theme) -> (Option<u32>, u32, bool) {
        match self {
            ButtonVariant::Action => (Some(t.accent.action), t.text.on_accent, false),
            ButtonVariant::Create => (Some(t.accent.create), t.text.on_accent, true),
            ButtonVariant::Confirm => (Some(t.accent.confirm), t.text.on_accent, true),
            ButtonVariant::Destructive => (Some(t.state.danger), t.text.on_emphasis, true),
            ButtonVariant::Secondary => (Some(t.surface.raised), t.text.primary, false),
            ButtonVariant::Ghost => (None, t.text.secondary, false),
            ButtonVariant::Custom(bg, fg) => (Some(bg), fg, false),
        }
    }
}

/// Densidade do botão. Mapeia para a escala de espaçamento/tipografia (nunca px cru). Todo tamanho
/// respeita o piso de alvo de toque ≥24px (WCAG 2.5.8) imposto no render via `spacing.xl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    /// Compacto (fila de atenção, chips) — `spacing.sm`/`xs`, texto `small`.
    Sm,
    /// Padrão (rodapés de modal, listas) — `spacing.lg`/`sm`, texto `body`.
    Md,
    /// Hero (CTA de avançar no onboarding) — `spacing.xl`/`sm`, texto `body`.
    Lg,
}

impl ButtonSize {
    /// `(padding_x, padding_y, text_size)` em valores de TOKEN — função pura, único ponto de medida.
    #[must_use]
    pub fn metrics(self, t: &Theme) -> (f32, f32, f32) {
        let sz = &t.spacing;
        let body = f32::from(t.typography.size.body);
        match self {
            ButtonSize::Sm => (
                f32::from(sz.sm),
                f32::from(sz.xs),
                f32::from(t.typography.size.small),
            ),
            ButtonSize::Md => (f32::from(sz.lg), f32::from(sz.sm), body),
            ButtonSize::Lg => (f32::from(sz.xl), f32::from(sz.sm), body),
        }
    }
}

/// O botão do catálogo. Construa com [`Button::new`] + atalhos de variante; o `on_click` recebe o
/// resultado de `cx.listener(...)` do call-site.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    size: ButtonSize,
    /// Botão de glifo único (✕, swatch): padding simétrico + alvo quadrado ≥24px.
    icon: bool,
    /// Default `Role::Button`; toggles passam `CheckBox`/`RadioButton` (a11y honesta).
    role: Role,
    /// Nome acessível quando o rótulo visível não basta (glifo, swatch de cor).
    aria_label: Option<SharedString>,
    /// Estado selecionado (chip de motor, opção de autonomia) → anel de foco, sem reflow.
    selected: bool,
    disabled: bool,
    /// `flex_none` para rodapés de modal (não encolhe sob pressão de layout).
    no_shrink: bool,
    on_click: Option<super::ClickHandler>,
}

impl Button {
    /// Novo botão `Secondary`/`Md`. `id` é obrigatório (sem ele o nó some da árvore a11y e o
    /// `on_click` não engata — disciplina do shell).
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant: ButtonVariant::Secondary,
            size: ButtonSize::Md,
            icon: false,
            role: Role::Button,
            aria_label: None,
            selected: false,
            disabled: false,
            no_shrink: false,
            on_click: None,
        }
    }

    #[must_use]
    pub fn variant(mut self, v: ButtonVariant) -> Self {
        self.variant = v;
        self
    }
    #[must_use]
    pub fn action(self) -> Self {
        self.variant(ButtonVariant::Action)
    }
    #[must_use]
    pub fn create(self) -> Self {
        self.variant(ButtonVariant::Create)
    }
    #[must_use]
    pub fn confirm(self) -> Self {
        self.variant(ButtonVariant::Confirm)
    }
    #[must_use]
    pub fn destructive(self) -> Self {
        self.variant(ButtonVariant::Destructive)
    }
    #[must_use]
    pub fn secondary(self) -> Self {
        self.variant(ButtonVariant::Secondary)
    }
    #[must_use]
    pub fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }
    #[must_use]
    pub fn size(mut self, s: ButtonSize) -> Self {
        self.size = s;
        self
    }
    /// Glifo único, alvo quadrado.
    #[must_use]
    pub fn icon(mut self) -> Self {
        self.icon = true;
        self
    }
    #[must_use]
    pub fn role(mut self, r: Role) -> Self {
        self.role = r;
        self
    }
    #[must_use]
    pub fn aria(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }
    #[must_use]
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    #[must_use]
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// Rodapé de modal: não encolhe.
    #[must_use]
    pub fn no_shrink(mut self) -> Self {
        self.no_shrink = true;
        self
    }
    /// O clique. Passe `cx.listener(|view, _ev, window, cx| …)` do call-site.
    #[must_use]
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let (bg, fg, emphasized) = self.variant.paint(&t);
        let (pad_x, pad_y, text) = self.size.metrics(&t);
        let icon_pad = f32::from(t.spacing.sm);
        let touch_floor = f32::from(t.spacing.xl); // 24px — alvo de toque (WCAG 2.5.8)
        let weight = if emphasized {
            t.typography.weight.semibold
        } else {
            t.typography.weight.medium
        };
        let aria = self.aria_label.unwrap_or_else(|| self.label.clone());

        let mut el = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .min_h(px(touch_floor))
            .map(|d| {
                if self.icon {
                    d.min_w(px(touch_floor)).px(px(icon_pad)).py(px(icon_pad))
                } else {
                    d.px(px(pad_x)).py(px(pad_y))
                }
            })
            .rounded_md()
            .when_some(bg, |d, c| d.bg(rgb(c)))
            .text_color(rgb(fg))
            .text_size(px(text))
            .font_family(t.typography.family.ui)
            .font_weight(FontWeight(f32::from(weight)))
            .role(self.role)
            .aria_label(aria)
            .when(self.no_shrink, |d| d.flex_none())
            .when(self.selected, |d| {
                d.border_2().border_color(rgb(t.focus.ring))
            })
            .when(self.disabled, |d| d.opacity(0.5))
            .when(!self.disabled, |d| d.cursor_pointer())
            .child(self.label);

        if let (Some(handler), false) = (self.on_click, self.disabled) {
            el = el.on_click(handler);
        }
        el
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;

    fn theme() -> Theme {
        Theme::build(Mode::Dark, "azul")
    }

    /// A cor do botão É o token do seu SIGNIFICADO (OP-1): destrutivo veste o vermelho de estado,
    /// confirmar/criar/ação vestem o acento certo, neutro/ghost ficam na superfície quente SEM acento.
    #[test]
    fn variant_paints_the_meaning_token() {
        let t = theme();
        assert_eq!(
            ButtonVariant::Action.paint(&t),
            (Some(t.accent.action), t.text.on_accent, false)
        );
        assert_eq!(
            ButtonVariant::Create.paint(&t),
            (Some(t.accent.create), t.text.on_accent, true)
        );
        assert_eq!(
            ButtonVariant::Confirm.paint(&t),
            (Some(t.accent.confirm), t.text.on_accent, true)
        );
        assert_eq!(
            ButtonVariant::Destructive.paint(&t),
            (Some(t.state.danger), t.text.on_emphasis, true),
            "destrutivo = vermelho de estado (OP-1), nunca um acento decorativo"
        );
        // Neutro = superfície quente; ghost = SEM fill (None) — nenhum dos dois carrega cor de acento.
        let (sec_bg, sec_fg, _) = ButtonVariant::Secondary.paint(&t);
        let (ghost_bg, _, _) = ButtonVariant::Ghost.paint(&t);
        assert_eq!(sec_bg, Some(t.surface.raised));
        assert_eq!(sec_fg, t.text.primary);
        assert_eq!(
            ghost_bg, None,
            "ghost não tem fundo (✕, links de disclosure)"
        );
        for v in [ButtonVariant::Secondary, ButtonVariant::Ghost] {
            let (bg, _, _) = v.paint(&t);
            assert_ne!(
                bg,
                Some(t.accent.action),
                "neutro não carrega cor de acento"
            );
            assert_ne!(bg, Some(t.accent.create));
        }
    }

    /// Tamanho mapeia para a escala de espaçamento/tipografia — nenhum px avulso.
    #[test]
    fn size_metrics_come_from_tokens() {
        let t = theme();
        assert_eq!(
            ButtonSize::Md.metrics(&t),
            (
                f32::from(t.spacing.lg),
                f32::from(t.spacing.sm),
                f32::from(t.typography.size.body)
            )
        );
        assert_eq!(
            ButtonSize::Sm.metrics(&t),
            (
                f32::from(t.spacing.sm),
                f32::from(t.spacing.xs),
                f32::from(t.typography.size.small)
            )
        );
        assert_eq!(ButtonSize::Lg.metrics(&t).0, f32::from(t.spacing.xl));
    }
}
