//! **F2-4-3+4 · A "Área de Poderes" — a vitrine do que o leigo tem instalado e do que precisa de ajuste.**
//!
//! **Fio condutor #3 do fundador (`CLAUDE.md` inv #6, "visível-rotulado-first"):** o leigo tem dezenas
//! de Poderes instalados (skills/plugins/agents/commands/hooks/MCPs, em 4 motores de IA distintos) e
//! **não vê nenhum**. Este painel é a vitrine: ele vê o que tem, entende o que funciona em qual
//! terminal, e **conserta com 1 clique** — em português, sem jargão. `PowerState`/`PowerKind`/
//! `frontmatter`/`manifest` **nunca** aparecem na superfície; é a voz da copy, não `{:?}` de Debug.
//!
//! **Duas leis da onda materializadas aqui:**
//! - **a tela nunca mente** (entrega-d4 §II.A5): todo estado que PRECISA de ação tem uma
//!   ([`action_label`]); `Disabled` só se materializa se o app PUDER religar ([`should_render`] +
//!   [`CAN_REENABLE`]) — no MVP o Lina não religa skills, então some.
//! - **mostrar ≠ autorizar** (ADR 0052 §4): o card EXIBE e OFERECE a ação, mas a EXECUÇÃO passa pelo
//!   gate existente (custódia/WorkspaceTrust). A view só DISPARA o gesto via `on_action` (do call-site,
//!   como `BeliefCard::on_retire`); quem autoriza é o supervisor. Nenhum campo lido do disco
//!   (`name`/`origin`) decide autorização — é DADO de exibição.
//!
//! **Identidade = design system F2** (épico 38 §VIII, "Instrumento de Estúdio com a temperatura do
//! Ateliê"). Não invento visual: ESPELHO `mentality_panel`/`goal_card` — Fraunces ([`family.display`])
//! só no título (o "momento" do fundador vendo o seu arsenal), IBM Plex ([`family.ui`]) no corpo, cor
//! semântica fixa (verde/azul/âmbar/neutro) dos tokens, **zero literal** (a catraca `token_ratchet`
//! nasce verde para este arquivo). WCAG 1.4.1: cada estado é **texto + ícone + cor** via [`Badge`].
//!
//! **Padrão da casa:** a DECISÃO (estado→rótulo/tom, ação, origem, resumo) mora em **funções puras
//! testáveis** — gpui não roda headless, então o [`RenderOnce`] é casca fina sobre elas. O painel
//! consome o contrato do core direto (ADR 0052 §3: `Power`/`PowerInventory` JÁ são o view-model — sem
//! duplicar tipo). A ponte `bridge.rs`→inventário é costura do Maestro; aqui renderizo contra um mock.
//!
//! `allow(dead_code)`: itens `pub` sem chamador no bin disparam dead-code no `clippy -D` — caem quando
//! o Maestro fiar `render_powers_panel` + a ponte `bridge.rs` (Camada 2). Mesmo padrão do catálogo `ui`.
//!
//! [`family.display`]: crate::theme::FontFamilyTokens::display
//! [`family.ui`]: crate::theme::FontFamilyTokens::ui
#![allow(dead_code)]

use std::collections::BTreeMap;

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, FontWeight, IntoElement, SharedString, Window,
};

use lina_core::{Power, PowerInventory, PowerKind, PowerOrigin, PowerScope, PowerState};

use crate::theme::{self, Theme};
use crate::ui::{Badge, BadgeTone, Button, ButtonSize, ClickHandler, Panel, Space, Surface};

// ───────────────────────────── Microcopy (pt-br, zero jargão) ─────────────────────────────

/// Título do painel — o "momento" do fundador (Fraunces autorizado). "Seus" = posse, não um rótulo
/// de menu genérico; é o arsenal DELE, não uma "Área de Poderes" anônima.
pub const PANEL_TITLE: &str = "Seus Poderes";

/// Subtítulo discreto: diz o que a tela É, em consequência (não "gerencie seus recursos" de molde).
pub const PANEL_INTRO: &str =
    "O que cada terminal de IA seu sabe fazer — e o que precisa de um ajuste pra funcionar.";

/// Estado vazio: honesto e acolhedor. Explica o CICLO (instalou → aparece) sem prometer mágica, e
/// nunca mostra tela em branco (inv #6).
pub const EMPTY_STATE: &str =
    "Você ainda não tem nenhum Poder instalado por aqui. Quando instalar um — pelo terminal ou \
     por um plugin — ele aparece nesta lista automaticamente.";

/// Rótulo da seção de origem de cada card (de onde o Poder vem). Discreto, em pt-br.
pub const ORIGIN_LABEL: &str = "De onde vem";

// Os 5 rótulos de estado (fonte: entrega-d4 §III.b — a PALAVRA que o leigo lê).
const READY_LABEL: &str = "Pronto pra usar";
const UPDATE_LABEL: &str = "Atualização disponível";
const REPAIR_LABEL: &str = "Precisa de um conserto";
const INERT_LABEL: &str = "Não funciona neste motor";
const DISABLED_LABEL: &str = "Desligada";

// As ações de 1 clique acopladas (cada estado que precisa de ação tem a sua — a tela nunca mente).
const UPDATE_ACTION: &str = "Atualizar";
const REPAIR_ACTION: &str = "Consertar";
const INERT_ACTION: &str = "Instalar aqui";
const DISABLED_ACTION: &str = "Religar";

// Onde o Poder vale (eixo `scope`) — em linguagem de leigo, não "global"/"project" cru.
const SCOPE_GLOBAL: &str = "da sua máquina";
const SCOPE_PROJECT: &str = "só deste Espaço";

// ───────────────────────────── Tipo de Poder → palavra do leigo ─────────────────────────────

/// O nome leigo de um tipo de Poder, com pluralização por contagem ("1 Gatilho" vs "33 Plugins").
/// Skill = "Poder" (a unidade de habilidade por excelência); Hook = "Gatilho"; MCP = "Conexão".
#[must_use]
fn kind_label(kind: PowerKind, n: u32) -> &'static str {
    let one = n == 1;
    match kind {
        PowerKind::Skill => iff(one, "Poder", "Poderes"),
        PowerKind::Plugin => iff(one, "Plugin", "Plugins"),
        PowerKind::Agent => iff(one, "Assistente", "Assistentes"),
        PowerKind::Command => iff(one, "Comando", "Comandos"),
        PowerKind::Hook => iff(one, "Gatilho", "Gatilhos"),
        PowerKind::Mcp => iff(one, "Conexão", "Conexões"),
    }
}

/// Açúcar para escolher singular/plural sem um `if` por braço.
#[must_use]
const fn iff(cond: bool, a: &'static str, b: &'static str) -> &'static str {
    if cond {
        a
    } else {
        b
    }
}

/// O termo TÉCNICO que o leigo cruza em tutoriais externos — âncora discreta no detalhe ("(skill)").
/// Lição do rename do Obsidian (entrega-d4 §A5): o leigo precisa do termo para seguir um tutorial,
/// mas ele fica no canto, nunca como o rótulo principal.
#[must_use]
fn kind_anchor(kind: PowerKind) -> &'static str {
    match kind {
        PowerKind::Skill => "skill",
        PowerKind::Plugin => "plugin",
        PowerKind::Agent => "agent",
        PowerKind::Command => "command",
        PowerKind::Hook => "hook",
        PowerKind::Mcp => "MCP",
    }
}

/// A linha de resumo do nível 1: `"75 Poderes · 33 Plugins · 1 Gatilho"`. Só os tipos PRESENTES
/// (count > 0), na ordem canônica de [`PowerKind`] (o `BTreeMap` já a garante). Pura; vazio = "".
#[must_use]
fn summary_line(counts: &BTreeMap<PowerKind, u32>) -> String {
    counts
        .iter()
        .filter(|(_, &n)| n > 0)
        .map(|(&kind, &n)| format!("{n} {}", kind_label(kind, n)))
        .collect::<Vec<_>>()
        .join(" · ")
}

// ───────────────────────────── Estado leigo → tom + ação ─────────────────────────────

/// O estado de um Poder traduzido para a tela: a PALAVRA do [`Badge`] e seu tom (cor + glifo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateStatus {
    pub label: &'static str,
    pub tone: BadgeTone,
}

/// Traduz [`PowerState`] para o selo de tela do leigo. Pura (a doutrina do mapa vive no teste).
/// `Ready` = sucesso calmo (verde); `UpdateAvailable` = info (azul, opt-in manual); `NeedsRepair` =
/// atenção (âmbar); `InertHere`/`Disabled` = neutro (sem carga, é estado, não erro).
#[must_use]
pub fn state_status(state: PowerState) -> StateStatus {
    match state {
        PowerState::Ready => StateStatus {
            label: READY_LABEL,
            tone: BadgeTone::Success,
        },
        PowerState::UpdateAvailable => StateStatus {
            label: UPDATE_LABEL,
            tone: BadgeTone::Info,
        },
        PowerState::NeedsRepair => StateStatus {
            label: REPAIR_LABEL,
            tone: BadgeTone::Warning,
        },
        PowerState::InertHere => StateStatus {
            label: INERT_LABEL,
            tone: BadgeTone::Neutral,
        },
        PowerState::Disabled => StateStatus {
            label: DISABLED_LABEL,
            tone: BadgeTone::Neutral,
        },
    }
}

/// O rótulo da ação de 1 clique de cada estado — `None` SÓ para `Ready` (resolvido, nada a fazer).
/// "A tela nunca mente" (§II.A5): todo estado que PRECISA de ação tem uma; quem dispara é o card,
/// quem autoriza é o gate (mostrar ≠ autorizar).
#[must_use]
pub fn action_label(state: PowerState) -> Option<&'static str> {
    match state {
        PowerState::Ready => None,
        PowerState::UpdateAvailable => Some(UPDATE_ACTION),
        PowerState::NeedsRepair => Some(REPAIR_ACTION),
        PowerState::InertHere => Some(INERT_ACTION),
        PowerState::Disabled => Some(DISABLED_ACTION),
    }
}

/// O Lina ainda **não religa** Poderes (ADR 0052 §3): não há toggle com enforcement no MVP. Quando
/// existir, isto vira capability viva e `Disabled` passa a aparecer com a ação "Religar".
pub const CAN_REENABLE: bool = false;

/// Decide se um Poder entra na lista. Regra dura (§II.A5 · ADR 0052 §3): `Disabled` só aparece se o
/// app PUDER religá-lo — senão o card seria estado sem ação real (a tela mentiria). Os outros 4
/// estados sempre entram. O call-site filtra por isto ao montar os cards; o core também não emite
/// `Disabled` sem capacidade de religar (defesa em profundidade).
#[must_use]
pub fn should_render(power: &Power, can_reenable: bool) -> bool {
    power.state != PowerState::Disabled || can_reenable
}

// ───────────────────────────── Origem rotulada (de onde o Poder vem) ─────────────────────────────

/// Nome leigo de um motor de IA a partir do id do CLI ("gemini" → "Gemini"). Conhecidos têm grafia
/// bonita; o desconhecido capitaliza a 1ª letra (fallback honesto, nunca o id cru minúsculo).
#[must_use]
fn cli_label(cli: &str) -> String {
    match cli {
        "claude-code" | "claude" => "Claude Code".to_string(),
        "gemini" => "Gemini".to_string(),
        "codex" => "Codex".to_string(),
        "opencode" => "OpenCode".to_string(),
        "copilot" => "Copilot".to_string(),
        other => {
            let mut chars = other.chars();
            chars.next().map_or_else(
                || other.to_string(),
                |first| first.to_uppercase().collect::<String>() + chars.as_str(),
            )
        }
    }
}

/// A origem que o leigo lê: onde vale ("da sua máquina" / "só deste Espaço") e de qual motor
/// ("· do Gemini"). Sem CLI atrelado = só o escopo. Pura.
#[must_use]
fn origin_label(origin: &PowerOrigin) -> String {
    let scope = match origin.scope {
        PowerScope::Global => SCOPE_GLOBAL,
        PowerScope::Project => SCOPE_PROJECT,
    };
    match &origin.cli {
        Some(cli) => format!("{scope} · do {}", cli_label(cli)),
        None => scope.to_string(),
    }
}

/// A frase que explica por que um Poder está inerte AQUI: pertence a outro motor de IA, e o terminal
/// em foco usa outro. Usa o nome leigo do motor, nunca o id de CLI cru.
#[must_use]
fn inert_reason(origin: &PowerOrigin) -> String {
    match &origin.cli {
        Some(cli) => format!(
            "É um Poder do {}, mas este terminal usa outro motor de IA.",
            cli_label(cli)
        ),
        None => "É um Poder de outro motor de IA, não do que este terminal usa.".to_string(),
    }
}

// ───────────────────────────── Helpers de tipografia (token-puros) ─────────────────────────────

/// O título do painel (Fraunces — o "momento", como o hero da Goal/Mentality).
fn title_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.display)
        .text_size(px(f32::from(t.typography.size.title)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.bright))
        .child(gpui::text!(text))
}

/// A linha de resumo do nível 1 (Plex, destaque) — "75 Poderes · 33 Plugins · 1 Gatilho".
fn summary_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.body)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.primary))
        .child(gpui::text!(text))
}

/// Nome do Poder (Plex, cor primária, semibold) — o que o leigo lê primeiro em cada card.
fn name_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.body)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.primary))
        .child(gpui::text!(text))
}

/// Rótulo de seção discreto (Plex, secundário) — "De onde vem".
fn section_label(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.secondary))
        .child(gpui::text!(text))
}

/// A âncora técnica "(skill)" — discreta (Plex, pequeno, apagado), ao lado do nome.
fn anchor_text(t: &Theme, term: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .text_color(rgb(t.text.muted))
        .child(gpui::text!(format!("({term})")))
}

/// Texto discreto (Plex, apagado) — descrição crua, origem e a frase do inerte.
fn muted_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .text_color(rgb(t.text.muted))
        .child(gpui::text!(text))
}

// ───────────────────────────── O card de UM Poder (RenderOnce) ─────────────────────────────

/// **O card de um Poder.** builder + [`RenderOnce`], view-agnóstico (o handler da ação vem do
/// call-site via `cx.listener(...)`, como o resto do catálogo `ui/`). Construa com [`PowerCard::new`].
/// `InertHere` afunda no fundo do canvas (esmaecido = inativo) e ganha a frase do porquê.
#[derive(IntoElement)]
pub struct PowerCard {
    power: Power,
    /// Gesto de 1 clique (Atualizar/Consertar/Instalar). `None` = botão inerte (pré-visualização).
    on_action: Option<ClickHandler>,
}

impl PowerCard {
    /// Novo card para um [`Power`] do inventário.
    #[must_use]
    pub fn new(power: Power) -> Self {
        Self {
            power,
            on_action: None,
        }
    }

    /// Handler da ação de 1 clique. Passe `cx.listener(|view, _ev, _w, cx| …)`. A view só DISPARA o
    /// gesto; o gate (custódia/WorkspaceTrust) é quem autoriza (mostrar ≠ autorizar).
    #[must_use]
    pub fn on_action(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for PowerCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let status = state_status(self.power.state);
        let inert = self.power.state == PowerState::InertHere;

        // Selo de estado (palavra + glifo + cor — WCAG 1.4.1); id estável anuncia a troca de estado.
        let badge = Badge::new(
            SharedString::from(format!("power-state-{}", self.power.name)),
            status.label,
            status.tone,
        );

        // Nome do Poder + âncora técnica discreta ("Conector do banco  (MCP)").
        let name_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(Space::Xs.px(&t)))
            .child(name_text(&t, SharedString::from(self.power.name.clone())))
            .child(anchor_text(&t, kind_anchor(self.power.kind)));

        // Cabeçalho: nome à esquerda, selo de estado à direita.
        let header = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .gap(px(Space::Sm.px(&t)))
            .child(name_row)
            .child(badge);

        // Descrição CRUA do manifesto (só quando há) — DADO, não traduzido pelo painel (ADR 0052 §3).
        let description = (!self.power.description.is_empty())
            .then(|| muted_text(&t, SharedString::from(self.power.description.clone())));

        // Frase do porquê do inerte ("É um Poder do Gemini, mas este terminal usa outro motor").
        let reason =
            inert.then(|| muted_text(&t, SharedString::from(inert_reason(&self.power.origin))));

        // Origem rotulada ("De onde vem: da sua máquina · do Gemini").
        let origin = div()
            .flex()
            .flex_col()
            .gap(px(Space::Xs.px(&t)))
            .child(section_label(&t, ORIGIN_LABEL))
            .child(muted_text(
                &t,
                SharedString::from(origin_label(&self.power.origin)),
            ));

        // Ação de 1 clique (Sm, neutra — reversível): só nos estados que a têm. Sem handler = inerte
        // (pré-visualização/mock). A EXECUÇÃO passa pelo gate; o botão só dispara o gesto.
        let action = action_label(self.power.state).map(|label| {
            let button = Button::new(
                SharedString::from(format!("power-action-{}", self.power.name)),
                label,
            )
            .size(ButtonSize::Sm);
            match self.on_action {
                Some(handler) => button.on_click(handler),
                None => button,
            }
        });

        // `InertHere` afunda no fundo do canvas (esmaecido = inativo), sem opacity literal — só token.
        let surface = if inert {
            Surface::Canvas
        } else {
            Surface::Panel
        };

        Panel::card()
            .bg(surface)
            .gap(Space::Sm)
            .pad(Space::Md, Space::Md)
            .child(header)
            .children(description)
            .children(reason)
            .child(origin)
            .children(action)
    }
}

// ───────────────────────────── O painel da Área de Poderes (RenderOnce) ─────────────────────────────

/// **A "Área de Poderes".** Nível 1 = título (Fraunces), resumo contado ("75 Poderes · 33 Plugins") e
/// subtítulo; nível 2 = a lista de cards (origem, estado e ação), ou o estado vazio acolhedor.
/// builder + [`RenderOnce`]; os cards já vêm com seus handlers do call-site (a view é dona do gesto).
/// Construa com [`PowersPanel::new`] (resumo do inventário) e alimente [`PowersPanel::powers`].
#[derive(IntoElement)]
pub struct PowersPanel {
    summary: String,
    cards: Vec<PowerCard>,
}

impl PowersPanel {
    /// Novo painel a partir do inventário (alimenta o resumo do nível 1 com `inventory.counts`). Os
    /// cards (filtrados + com handlers) vêm depois via [`PowersPanel::powers`].
    #[must_use]
    pub fn new(inventory: &PowerInventory) -> Self {
        Self {
            summary: summary_line(&inventory.counts),
            cards: Vec::new(),
        }
    }

    /// Os cards a exibir — já construídos com seus handlers e JÁ filtrados por [`should_render`]
    /// (o call-site não passa `Disabled` enquanto o app não religar). Sem cards = estado vazio.
    #[must_use]
    pub fn powers(mut self, cards: Vec<PowerCard>) -> Self {
        self.cards = cards;
        self
    }
}

impl RenderOnce for PowersPanel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();

        let title = title_text(&t, SharedString::from(PANEL_TITLE));
        // Resumo do nível 1 — só quando há algo contado (vazio cai no estado vazio abaixo).
        let summary = (!self.summary.is_empty())
            .then(|| summary_text(&t, SharedString::from(self.summary.clone())));
        let intro = muted_text(&t, SharedString::from(PANEL_INTRO));

        // Nível 2: a lista de cards, ou o estado vazio acolhedor (nunca tela em branco).
        let body: gpui::AnyElement = if self.cards.is_empty() {
            muted_text(&t, SharedString::from(EMPTY_STATE)).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(Space::Md.px(&t)))
                .children(self.cards)
                .into_any_element()
            // ponytail: lista direta (mock ~6 itens). Virtualização de 75+ cards é polimento de
            // layout do Maestro na posição final do painel, não desta fatia.
        };

        Panel::surface()
            .gap(Space::Lg)
            .pad(Space::Lg, Space::Lg)
            .child(title)
            .children(summary)
            .child(intro)
            .child(body)
    }
}

// ───────────────────────────── Fixture de tela (founder session) ─────────────────────────────

/// Um [`PowerInventory`] de exemplo, em pt-br realista, para os testes e a pré-visualização do gate.
/// Exercita os 5 estados (incl. um `Disabled` que [`should_render`] esconde) + as duas origens. Só
/// fixture: a tela de produção renderiza o scan VIVO (via a ponte do Maestro), nunca este mock.
#[cfg(test)]
#[must_use]
pub fn mock_inventory() -> PowerInventory {
    let mk = |kind, name: &str, description: &str, scope, cli: Option<&str>, state| Power {
        kind,
        name: name.to_string(),
        description: description.to_string(),
        origin: PowerOrigin {
            scope,
            cli: cli.map(str::to_string),
        },
        state,
    };
    let powers = vec![
        mk(
            PowerKind::Skill,
            "Revisão de código",
            "Revisa um trecho e aponta riscos antes de você seguir.",
            PowerScope::Global,
            Some("claude-code"),
            PowerState::Ready,
        ),
        mk(
            PowerKind::Skill,
            "Pesquisa profunda",
            "Investiga um tema em várias fontes e resume com referências.",
            PowerScope::Global,
            Some("claude-code"),
            PowerState::UpdateAvailable,
        ),
        mk(
            PowerKind::Plugin,
            "Conector do Stripe",
            "Liga seus terminais à sua conta de pagamentos.",
            PowerScope::Project,
            Some("claude-code"),
            PowerState::NeedsRepair,
        ),
        mk(
            PowerKind::Skill,
            "Gerar imagem",
            "Cria imagens a partir de uma descrição.",
            PowerScope::Global,
            Some("gemini"),
            PowerState::InertHere,
        ),
        mk(
            PowerKind::Mcp,
            "Banco de dados",
            "Consulta o seu banco direto pelo terminal.",
            PowerScope::Project,
            None,
            PowerState::Ready,
        ),
        mk(
            PowerKind::Skill,
            "Tradução automática",
            "Traduz textos entre idiomas.",
            PowerScope::Global,
            Some("claude-code"),
            PowerState::Disabled,
        ),
    ];
    let mut counts: BTreeMap<PowerKind, u32> = BTreeMap::new();
    counts.insert(PowerKind::Skill, 75);
    counts.insert(PowerKind::Plugin, 33);
    counts.insert(PowerKind::Hook, 1);
    PowerInventory { powers, counts }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada estado vira a PALAVRA do leigo (entrega-d4 §III.b), com o tom certo, e nenhum vaza o
    /// vocabulário técnico do enum (`InertHere`/`NeedsRepair`/…).
    #[test]
    fn state_speaks_layman_with_right_tone() {
        assert_eq!(state_status(PowerState::Ready).tone, BadgeTone::Success);
        assert_eq!(
            state_status(PowerState::UpdateAvailable).tone,
            BadgeTone::Info
        );
        assert_eq!(
            state_status(PowerState::NeedsRepair).tone,
            BadgeTone::Warning
        );
        assert_eq!(state_status(PowerState::InertHere).tone, BadgeTone::Neutral);
        assert_eq!(state_status(PowerState::Disabled).tone, BadgeTone::Neutral);

        for state in [
            PowerState::Ready,
            PowerState::UpdateAvailable,
            PowerState::NeedsRepair,
            PowerState::InertHere,
            PowerState::Disabled,
        ] {
            let low = state_status(state).label.to_lowercase();
            for jargon in [
                "inerthere",
                "needsrepair",
                "updateavailable",
                "disabled",
                "ready",
                "powerstate",
            ] {
                assert!(
                    !low.contains(jargon),
                    "vazou jargão: {:?}",
                    state_status(state).label
                );
            }
        }
    }

    /// "A tela nunca mente" (§II.A5): todo estado que PRECISA de ação tem uma; só `Ready` (resolvido)
    /// não tem. A cada estado-com-ação corresponde um rótulo não-vazio.
    #[test]
    fn every_needy_state_has_a_one_click_action() {
        assert_eq!(
            action_label(PowerState::Ready),
            None,
            "Ready está resolvido"
        );
        for state in [
            PowerState::UpdateAvailable,
            PowerState::NeedsRepair,
            PowerState::InertHere,
            PowerState::Disabled,
        ] {
            let label = action_label(state).expect("estado que precisa de ação tem rótulo");
            assert!(!label.is_empty());
        }
    }

    /// `Disabled` só aparece se o app PUDER religar (a tela nunca mente). No MVP `CAN_REENABLE=false`,
    /// então some; os outros 4 estados sempre entram.
    #[test]
    fn disabled_is_hidden_until_reenable_is_real() {
        let disabled = &mock_inventory().powers[5];
        assert_eq!(
            disabled.state,
            PowerState::Disabled,
            "fixture: o 6º é Disabled"
        );
        assert!(
            !should_render(disabled, false),
            "sem religar real → escondido"
        );
        assert!(should_render(disabled, true), "com toggle real → aparece");

        // No MVP a capability é falsa — então o `Disabled` da fixture é filtrado fora.
        assert!(
            !should_render(disabled, CAN_REENABLE),
            "MVP esconde o Disabled"
        );
        for power in &mock_inventory().powers {
            if power.state != PowerState::Disabled {
                assert!(
                    should_render(power, CAN_REENABLE),
                    "os 4 estados ativos sempre entram"
                );
            }
        }
    }

    /// O resumo do nível 1 conta e pluraliza: "75 Poderes · 33 Plugins · 1 Gatilho". Só tipos > 0,
    /// na ordem canônica do `PowerKind`. Inventário vazio = "".
    #[test]
    fn summary_line_counts_and_pluralizes() {
        let mut counts: BTreeMap<PowerKind, u32> = BTreeMap::new();
        counts.insert(PowerKind::Skill, 75);
        counts.insert(PowerKind::Plugin, 33);
        counts.insert(PowerKind::Hook, 1);
        assert_eq!(summary_line(&counts), "75 Poderes · 33 Plugins · 1 Gatilho");

        let mut one: BTreeMap<PowerKind, u32> = BTreeMap::new();
        one.insert(PowerKind::Skill, 1);
        assert_eq!(summary_line(&one), "1 Poder", "singular");

        assert_eq!(summary_line(&BTreeMap::new()), "", "vazio = sem resumo");

        let mut zeroed: BTreeMap<PowerKind, u32> = BTreeMap::new();
        zeroed.insert(PowerKind::Agent, 0);
        assert_eq!(summary_line(&zeroed), "", "tipo com 0 não aparece");
    }

    /// A origem é rotulada para o leigo: onde vale + de qual motor; CLI desconhecido capitaliza; sem
    /// CLI = só o escopo. E a frase do inerte usa o nome do motor, nunca o id cru.
    #[test]
    fn origin_and_inert_reason_are_layman() {
        let gemini = PowerOrigin {
            scope: PowerScope::Global,
            cli: Some("gemini".to_string()),
        };
        assert_eq!(origin_label(&gemini), "da sua máquina · do Gemini");

        let project = PowerOrigin {
            scope: PowerScope::Project,
            cli: None,
        };
        assert_eq!(origin_label(&project), "só deste Espaço");

        let unknown = PowerOrigin {
            scope: PowerScope::Global,
            cli: Some("tabby".to_string()),
        };
        assert_eq!(
            origin_label(&unknown),
            "da sua máquina · do Tabby",
            "fallback capitaliza"
        );

        assert!(
            inert_reason(&gemini).contains("Gemini"),
            "a frase nomeia o motor de origem"
        );
        assert!(
            !inert_reason(&gemini).contains("gemini"),
            "nunca o id de CLI cru minúsculo"
        );
    }

    /// A âncora técnica é exatamente o termo que o leigo cruza em tutoriais externos (§A5).
    #[test]
    fn kind_anchor_is_the_external_tutorial_term() {
        assert_eq!(kind_anchor(PowerKind::Skill), "skill");
        assert_eq!(kind_anchor(PowerKind::Plugin), "plugin");
        assert_eq!(kind_anchor(PowerKind::Mcp), "MCP");
        assert_eq!(kind_anchor(PowerKind::Hook), "hook");
    }

    /// Toda a copy LEIGA do painel é pt-br sem jargão técnico de implementação/orquestração — a âncora
    /// "(skill)" é a ÚNICA exceção deliberada (testada à parte), e não entra aqui.
    #[test]
    fn all_layman_copy_is_free_of_jargon() {
        let inv = mock_inventory();
        let dynamic = [
            summary_line(&inv.counts),
            origin_label(&inv.powers[3].origin),
            inert_reason(&inv.powers[3].origin),
        ];
        let surfaces: Vec<&str> = [
            PANEL_TITLE,
            PANEL_INTRO,
            EMPTY_STATE,
            ORIGIN_LABEL,
            READY_LABEL,
            UPDATE_LABEL,
            REPAIR_LABEL,
            INERT_LABEL,
            DISABLED_LABEL,
            UPDATE_ACTION,
            REPAIR_ACTION,
            INERT_ACTION,
            DISABLED_ACTION,
            SCOPE_GLOBAL,
            SCOPE_PROJECT,
        ]
        .into_iter()
        .chain(dynamic.iter().map(String::as_str))
        .collect();

        for s in surfaces {
            assert!(!s.is_empty(), "toda copy tem conteúdo");
            let low = s.to_lowercase();
            for jargon in [
                "frontmatter",
                "manifest",
                "payload",
                "intent",
                "spawn",
                "{:?}",
                "powerstate",
                "powerkind",
                "inerthere",
                "needsrepair",
                "updateavailable",
                "mcpserver",
                "focused_cli",
                "projeção",
            ] {
                assert!(
                    !low.contains(jargon),
                    "copy vazou jargão ({jargon:?}): {s:?}"
                );
            }
        }
    }

    /// A fixture é coerente para o gate de tela: exercita os 5 estados (4 visíveis + 1 `Disabled`
    /// escondido), duas origens e o resumo contado.
    #[test]
    fn mock_is_screen_ready() {
        let inv = mock_inventory();
        assert!(!inv.powers.is_empty());
        let states: Vec<PowerState> = inv.powers.iter().map(|p| p.state).collect();
        for state in [
            PowerState::Ready,
            PowerState::UpdateAvailable,
            PowerState::NeedsRepair,
            PowerState::InertHere,
            PowerState::Disabled,
        ] {
            assert!(states.contains(&state), "fixture exercita {state:?}");
        }
        assert!(
            inv.powers.iter().any(|p| p.origin.cli.is_some()),
            "tem origem com motor"
        );
        assert!(
            inv.powers.iter().any(|p| p.origin.cli.is_none()),
            "tem origem sem motor"
        );
        assert!(
            !summary_line(&inv.counts).is_empty(),
            "resumo do nível 1 conta algo"
        );
    }
}
