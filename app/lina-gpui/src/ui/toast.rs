//! **F2-2-2 · Toast** — notificação TRANSITÓRIA do shell (Espaço arquivado, chave salva, erro de
//! rede), sobre o `Element` de live-region (ADR 0028: **nasce compondo** o nó auto-anunciável —
//! retrofit proibido). builder + `RenderOnce`, **view-agnóstico** (o `on_action` é fechado pelo
//! call-site via `cx.listener`), **token-puro** (zero medida literal). O MODELO — empilhamento,
//! expiry, cor por tom, duração de fade — é PURO e testável sem gpui (gpui não roda headless).
//!
//! **Duração vs. movimento (decisão da story, documentada):** a duração VISÍVEL do auto-dismiss é
//! POLÍTICA própria ([`TOAST_DISMISS_MS`]) — `reduce-motion` **NÃO** a corta (cortar a permanência
//! sob reduce-motion roubaria o tempo de LER de quem mais precisa). O que `reduce-motion` corta é a
//! ANIMAÇÃO de entrada/saída ([`fade_duration`] = `motion.effective(base)` → 0ms). São eixos
//! distintos: permanência é acessibilidade; fade é estética.

use std::time::Duration;

use gpui::{div, prelude::*, px, rgb, ClickEvent, IntoElement, SharedString};

use crate::a11y_live::{live_region, Politeness};
use crate::theme::{self, Theme};

/// Tom semântico do toast. **Cor = significado**, mas NUNCA cor sozinha: o glifo ([`Self::glyph`]) e
/// o texto viajam sempre (WCAG 1.4.1 — daltônicos e leitores de tela recebem a mesma informação).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    /// Concluído (Espaço arquivado, chave salva) — verde de estado.
    Success,
    /// Informativo neutro (sincronizado, copiado) — acento do shell.
    Info,
    /// Atenção sem bloqueio (rede instável, limite perto) — âmbar de estado.
    Warning,
}

impl ToastTone {
    /// A cor de acento do tom — ÚNICO ponto que toca tokens de cor. `Info` usa o acento do shell
    /// (não há token de "info" dedicado; o acento é neutro-informativo por construção).
    #[must_use]
    pub fn accent(self, t: &Theme) -> u32 {
        match self {
            ToastTone::Success => t.state.success,
            ToastTone::Info => t.accent.primary,
            ToastTone::Warning => t.state.warning,
        }
    }

    /// O glifo redundante à cor (WCAG 1.4.1). Não é decorativo: é a SEGUNDA via da informação.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            ToastTone::Success => "✓",
            ToastTone::Info => "•",
            ToastTone::Warning => "⚠",
        }
    }
}

/// Empilhamento máximo simultâneo — além disto o mais ANTIGO cai (uma pilha alta deixa de ser
/// transitória e vira ruído que tapa o canvas).
pub const TOAST_STACK_MAX: usize = 3;

/// Duração-POLÍTICA do auto-dismiss de um toast simples (informativo). Veja a doutrina no doc do
/// módulo: é independente de `reduce-motion`.
pub const TOAST_DISMISS_MS: u64 = 5_000;

/// Toast COM ação ("Desfazer") permanece mais (a janela do undo do arquivar, F1) — o usuário precisa
/// de tempo para decidir, não só para ler.
pub const TOAST_ACTION_DISMISS_MS: u64 = 8_000;

/// **Modelo PURO de um toast vivo** (sem gpui): identidade + texto + tom + prazo de auto-dismiss +
/// rótulo de ação opcional. O tempo entra por parâmetro (epoch ms) — testável determinístico, igual
/// ao `ArchiveToast` da F1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub tone: ToastTone,
    /// Epoch ms em que o toast some sozinho.
    pub deadline_ms: u64,
    /// Rótulo da ação de 1 clique (ex.: "Desfazer"). `None` = toast sem ação.
    pub action_label: Option<String>,
}

impl Toast {
    /// Toast simples (sem ação) — dismiss em [`TOAST_DISMISS_MS`].
    #[must_use]
    pub fn new(id: u64, message: impl Into<String>, tone: ToastTone, now_ms: u64) -> Self {
        Self {
            id,
            message: message.into(),
            tone,
            deadline_ms: now_ms.saturating_add(TOAST_DISMISS_MS),
            action_label: None,
        }
    }

    /// Toast com ação de 1 clique — janela mais longa ([`TOAST_ACTION_DISMISS_MS`]).
    #[must_use]
    pub fn with_action(
        id: u64,
        message: impl Into<String>,
        tone: ToastTone,
        action_label: impl Into<String>,
        now_ms: u64,
    ) -> Self {
        Self {
            id,
            message: message.into(),
            tone,
            deadline_ms: now_ms.saturating_add(TOAST_ACTION_DISMISS_MS),
            action_label: Some(action_label.into()),
        }
    }

    /// Prazo venceu → a fiação derruba o toast.
    #[must_use]
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms >= self.deadline_ms
    }

    /// Segundos restantes (teto) — para um countdown opcional no render.
    #[must_use]
    pub fn remaining_secs(&self, now_ms: u64) -> u64 {
        self.deadline_ms.saturating_sub(now_ms).div_ceil(1000)
    }
}

/// **A pilha de toasts** (modelo puro). Mantém no máximo [`TOAST_STACK_MAX`] vivos, dropando o mais
/// antigo; o shell chama [`Self::tick`] no seu loop para expirar os vencidos. Fonte da verdade do
/// que está na tela — o render é casca sobre `visible()`.
#[derive(Debug, Default)]
pub struct ToastStack {
    items: Vec<Toast>,
}

impl ToastStack {
    /// Empurra um toast novo. Estoura o teto → o mais antigo sai (FIFO no limite).
    pub fn push(&mut self, toast: Toast) {
        self.items.push(toast);
        while self.items.len() > TOAST_STACK_MAX {
            self.items.remove(0);
        }
    }

    /// Expira os vencidos (chamar no tick do shell com o relógio vivo).
    pub fn tick(&mut self, now_ms: u64) {
        self.items.retain(|t| !t.expired(now_ms));
    }

    /// Remove por id (clique no ✕ ou ação consumida).
    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    /// Os toasts na tela, do mais antigo ao mais novo (ordem de empilhamento).
    #[must_use]
    pub fn visible(&self) -> &[Toast] {
        &self.items
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Duração da ANIMAÇÃO de fade (entrada/saída), subordinada a `reduce-motion` via
/// [`crate::theme::MotionTokens::effective`]. É o eixo ESTÉTICO — distinto da permanência-política
/// ([`TOAST_DISMISS_MS`]), que `reduce-motion` não toca. O shell usa esta duração ao montar/desmontar.
#[must_use]
pub fn fade_duration(t: &Theme) -> Duration {
    t.motion.effective(t.motion.base)
}

/// **O componente de render de UM toast** (catálogo `ui/`). builder + `RenderOnce`, nasce compondo o
/// live-region (ADR 0028). `on_action` vem de `cx.listener(...)` do call-site.
#[derive(IntoElement)]
pub struct ToastView {
    /// Chave estável (do [`Toast::id`]) — deriva os `ElementId` do nó vivo e do botão de ação.
    key: u64,
    message: SharedString,
    tone: ToastTone,
    action_label: Option<SharedString>,
    on_action: Option<super::ClickHandler>,
}

impl ToastView {
    /// Toast visual sem ação. `key` = [`Toast::id`] (identidade estável na árvore a11y).
    pub fn new(key: u64, message: impl Into<SharedString>, tone: ToastTone) -> Self {
        Self {
            key,
            message: message.into(),
            tone,
            action_label: None,
            on_action: None,
        }
    }

    /// Anexa a ação de 1 clique (rótulo + handler do call-site). É um [`super::Button`] `Ghost`/`Sm`.
    #[must_use]
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.action_label = Some(label.into());
        self.on_action = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ToastView {
    fn render(self, _window: &mut gpui::Window, _cx: &mut gpui::App) -> impl IntoElement {
        let t = theme::active();
        let accent = self.tone.accent(&t);
        let gap = f32::from(t.spacing.sm);
        let pad_x = f32::from(t.spacing.md);
        let pad_y = f32::from(t.spacing.sm);
        let body = f32::from(t.typography.size.body);

        // Visual: superfície QUENTE (raised), FLAT (sem sombra), barra de acento à esquerda = a cor do
        // tom. Glifo colorido + texto = informação em DUAS vias (WCAG 1.4.1), nunca só cor.
        let mut visual = div()
            .flex()
            .items_center()
            .gap(px(gap))
            .px(px(pad_x))
            .py(px(pad_y))
            .bg(rgb(t.surface.raised))
            .rounded_md()
            .border_l_2()
            .border_color(rgb(accent))
            .text_color(rgb(t.text.primary))
            .text_size(px(body))
            .font_family(t.typography.family.ui)
            .child(
                div()
                    .text_color(rgb(accent))
                    .child(gpui::text!(self.tone.glyph().to_string())),
            )
            .child(gpui::text!(self.message.clone()));

        // Ação de 1 clique (reusa o Button do catálogo — Ghost/Sm). Só aparece com rótulo E handler.
        if let (Some(label), Some(handler)) = (self.action_label, self.on_action) {
            visual = visual.child(
                super::Button::new(("toast-action", self.key as usize), label)
                    .ghost()
                    .size(super::ButtonSize::Sm)
                    .on_click(move |ev, w, cx| handler(ev, w, cx)),
            );
        }

        // ADR 0028: o toast NASCE acessível — o texto é anunciado (polite) ao entrar na cena. Id único
        // por toast (`key`) evita colisão com o banner e os demais toasts na árvore a11y.
        live_region(
            ("toast", self.key as usize),
            self.message.to_string(),
            Politeness::Polite,
            visual,
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

    /// Cor = token do tom (state.success/warning, acento p/ info); e o glifo é a SEGUNDA via (WCAG
    /// 1.4.1) — nunca há tom sem glifo distinto.
    #[test]
    fn tone_carries_color_and_a_distinct_glyph() {
        let t = theme();
        assert_eq!(ToastTone::Success.accent(&t), t.state.success);
        assert_eq!(ToastTone::Warning.accent(&t), t.state.warning);
        assert_eq!(ToastTone::Info.accent(&t), t.accent.primary);
        let glyphs = [
            ToastTone::Success.glyph(),
            ToastTone::Info.glyph(),
            ToastTone::Warning.glyph(),
        ];
        // Três glifos DISTINTOS — a cor nunca é o único sinal.
        assert_eq!(
            glyphs
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
    }

    /// Auto-dismiss por prazo (igual ao ArchiveToast): nasce dentro da janela, expira no prazo; o
    /// toast COM ação dura mais (tempo de decidir, não só de ler).
    #[test]
    fn dismiss_deadline_and_action_window() {
        let plain = Toast::new(1, "Copiado", ToastTone::Info, 1_000);
        assert!(!plain.expired(1_000), "nasce vivo");
        assert!(!plain.expired(1_000 + TOAST_DISMISS_MS - 1));
        assert!(plain.expired(1_000 + TOAST_DISMISS_MS), "some no prazo");

        let undo = Toast::with_action(2, "Espaço arquivado", ToastTone::Success, "Desfazer", 1_000);
        assert_eq!(undo.action_label.as_deref(), Some("Desfazer"));
        assert!(
            undo.deadline_ms > plain.deadline_ms,
            "toast com ação permanece mais que o simples"
        );
    }

    /// Empilhamento: teto de 3, o mais antigo cai; tick expira os vencidos; dismiss remove por id.
    #[test]
    fn stack_caps_at_three_and_expires() {
        let mut s = ToastStack::default();
        for i in 0..5 {
            s.push(Toast::new(i, format!("t{i}"), ToastTone::Info, 0));
        }
        assert_eq!(s.visible().len(), TOAST_STACK_MAX, "teto de 3");
        assert_eq!(s.visible()[0].id, 2, "os 2 mais antigos (0,1) caíram");

        s.dismiss(2);
        assert_eq!(s.visible().len(), 2);

        // Todos vencem após o prazo.
        s.tick(TOAST_DISMISS_MS);
        assert!(s.is_empty(), "tick expira os vencidos");
    }

    /// Doutrina permanência×movimento: `reduce-motion` ZERA o fade mas NÃO a permanência-política.
    #[test]
    fn reduce_motion_kills_fade_not_dwell() {
        let mut t = theme();
        // Movimento normal: fade = motion.base (>0).
        assert!(fade_duration(&t) > Duration::ZERO);
        // Sob reduce-motion: fade = 0…
        t.motion.reduce_motion = true;
        assert_eq!(
            fade_duration(&t),
            Duration::ZERO,
            "reduce-motion zera o fade"
        );
        // …mas a permanência (política) é a MESMA constante, intocada por reduce-motion.
        let toast = Toast::new(1, "x", ToastTone::Info, 0);
        assert_eq!(
            toast.deadline_ms, TOAST_DISMISS_MS,
            "permanência não depende de motion"
        );
    }
}
