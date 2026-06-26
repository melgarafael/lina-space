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
//! **Estrutura (correção r2 — pedido do fundador na tela):** área central dedicada (não mais flanco
//! direito espremido), cabeçalho FIXO (título Fraunces + resumo contado + abas por TIPO com contador)
//! e corpo ROLÁVEL com a lista da aba ativa. Listas volumosas (skills/plugins) usam 2 colunas via
//! split intercalado — alturas equilibradas, sem a monotonia de 60 cards iguais empilhados. A
//! filtragem da aba é responsabilidade do call-site (o painel só conta o que recebe).
//!
//! **Identidade = design system F2** (épico 38 §VIII, "Instrumento de Estúdio com a temperatura do
//! Ateliê"). Fraunces ([`family.display`]) só no título (o "momento" do fundador vendo o seu arsenal),
//! IBM Plex ([`family.ui`]) no corpo, cor semântica fixa (verde/azul/âmbar/neutro) dos tokens, **zero
//! literal** (a catraca `token_ratchet` nasce verde para este arquivo). WCAG 1.4.1: cada estado é
//! **texto + ícone + cor** via [`Badge`].
//!
//! **Padrão da casa:** a DECISÃO (estado→rótulo/tom, ação, origem, resumo, modelo das abas, densidade)
//! mora em **funções puras testáveis** — gpui não roda headless, então o [`RenderOnce`] é casca fina
//! sobre elas. O painel consome o contrato do core direto (ADR 0052 §3: `Power`/`PowerInventory` JÁ
//! são o view-model — sem duplicar tipo).
//!
//! `allow(dead_code)`: itens `pub` sem chamador no bin disparam dead-code no `clippy -D` — caem quando
//! o Maestro fiar `render_powers_panel` final (Camada 2). Mesmo padrão do catálogo `ui`.
//!
//! [`family.display`]: crate::theme::FontFamilyTokens::display
//! [`family.ui`]: crate::theme::FontFamilyTokens::ui
#![allow(dead_code)]

use std::collections::BTreeMap;

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, ElementId, FontWeight, IntoElement, SharedString,
    Window,
};

use lina_core::{Power, PowerInventory, PowerKind, PowerOrigin, PowerScope, PowerState};

use crate::theme::{self, Theme};
use crate::ui::{
    Badge, BadgeTone, Button, ButtonSize, ClickHandler, Panel, RadiusExt, Space, Surface,
};

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

/// Rótulo da aba que junta todos os tipos. Não se pluraliza ("Todos · 109") — é coletivo.
pub const TAB_ALL: &str = "Todos";

/// Rótulo acessível do ✕ — o leitor de tela ouve "Fechar"; o glifo visível fica mudo (icon button).
pub const CLOSE_ARIA: &str = "Fechar";

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

/// Slug estável (kebab) p/ o `ElementId` da aba — `None` = "all". Pura; estável entre frames porque
/// se baseia no enum, não no rótulo (que muda com pluralização).
#[must_use]
fn tab_slug(id: Option<PowerKind>) -> &'static str {
    match id {
        None => "all",
        Some(PowerKind::Skill) => "skill",
        Some(PowerKind::Plugin) => "plugin",
        Some(PowerKind::Agent) => "agent",
        Some(PowerKind::Command) => "command",
        Some(PowerKind::Hook) => "hook",
        Some(PowerKind::Mcp) => "mcp",
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

// ───────────────────────────── Abas por tipo (modelo + densidade) ─────────────────────────────

/// O modelo PURO de uma aba — o que ela é (id), quanto conta, e se está ativa. Sem render, sem
/// handler — é o produto de [`tab_models`] e a entrada de [`PowerTab::new`] no call-site (que pluga
/// o handler de troca de aba via `cx.listener(...)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabModel {
    /// `None` = a aba "Todos" (junção); `Some(kind)` = a aba daquele tipo.
    pub id: Option<PowerKind>,
    pub count: u32,
    pub active: bool,
}

/// Constrói a fila canônica de abas para um inventário e uma aba ativa: **"Todos"** primeiro (count =
/// total), depois UMA aba por tipo PRESENTE (`count > 0`), na ordem canônica do `BTreeMap` (a mesma
/// do resumo do cabeçalho). Inventário vazio = `Vec::new()` (sem abas — o painel cai no estado vazio
/// acolhedor). `active = None` ⇒ a "Todos" fica ativa (default natural ao abrir o painel).
///
/// Pura (testada). É o `tabs_for` da API: o call-site mapeia os modelos em [`PowerTab`] com o handler.
#[must_use]
pub fn tab_models(counts: &BTreeMap<PowerKind, u32>, active: Option<PowerKind>) -> Vec<TabModel> {
    let total: u32 = counts.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let mut tabs = Vec::with_capacity(counts.len() + 1);
    tabs.push(TabModel {
        id: None,
        count: total,
        active: active.is_none(),
    });
    for (&kind, &count) in counts.iter().filter(|(_, &n)| n > 0) {
        tabs.push(TabModel {
            id: Some(kind),
            count,
            active: active == Some(kind),
        });
    }
    tabs
}

/// O rótulo da aba, conforme o leigo lê: `"Skills · 75"`, `"Plugin · 1"`, `"Todos · 109"`. Pluraliza
/// o nome do tipo pela contagem; "Todos" é coletivo (não pluraliza). Separador `·` casa a identidade
/// do resumo do cabeçalho.
#[must_use]
pub fn tab_label(model: TabModel) -> String {
    let word = match model.id {
        None => TAB_ALL,
        Some(kind) => kind_label(kind, model.count),
    };
    format!("{word} · {}", model.count)
}

/// Limite de cards de UMA aba a partir do qual a lista vira 2 colunas (split intercalado) — abaixo
/// disso, fica em coluna única (cards mais "respiráveis"). 12 = ~uma tela cheia sem precisar rolar;
/// acima disso a 2-col deixa o panorama legível sem rolar infinito. Pura.
pub const GRID_THRESHOLD: usize = 12;

/// `true` ⇒ a aba ativa renderiza em 2 colunas. Pura (testada).
#[must_use]
pub fn use_grid(card_count: usize) -> bool {
    card_count > GRID_THRESHOLD
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

// ───────────────────────────── Aba (RenderOnce) ─────────────────────────────

/// **Uma aba.** Pílula clicável (`Button` do catálogo, `Secondary`/`Sm`, com anel de seleção quando
/// ativa — token-puro). O modelo é o produto de [`tab_models`]; o `on_select` vem do call-site via
/// `cx.listener(...)`, como o resto do catálogo `ui/`. Construa com [`PowerTab::new`].
#[derive(IntoElement)]
pub struct PowerTab {
    model: TabModel,
    on_select: Option<ClickHandler>,
}

impl PowerTab {
    /// Nova aba a partir do modelo puro.
    #[must_use]
    pub fn new(model: TabModel) -> Self {
        Self {
            model,
            on_select: None,
        }
    }

    /// Handler de clique. Passe `cx.listener(|view, _ev, _w, cx| view.select_powers_tab(id, cx))`.
    #[must_use]
    pub fn on_select(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for PowerTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id: ElementId =
            SharedString::from(format!("powers-tab-{}", tab_slug(self.model.id))).into();
        let label = tab_label(self.model);
        let mut btn = Button::new(id, label)
            .size(ButtonSize::Sm)
            .selected(self.model.active);
        if let Some(handler) = self.on_select {
            btn = btn.on_click(handler);
        }
        btn
    }
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

/// **A "Área de Poderes".** Cabeçalho FIXO com título Fraunces, resumo contado ("75 Poderes · 33
/// Plugins"), intro e ABAS por tipo (com contador). CORPO ROLÁVEL com a lista da aba ativa, em área
/// central dedicada (não flanco direito), com respiro. Listas volumosas (`use_grid` ⇒ split
/// intercalado em 2 colunas) ganham hierarquia legível sem rolar infinito; modestas, 1 coluna.
///
/// builder + [`RenderOnce`]; os cards já vêm filtrados pela aba ativa (o call-site monta) e com seus
/// handlers (a view é dona do gesto). Construa com [`PowersPanel::new`] e alimente:
///
/// - [`PowersPanel::tabs`] (`Vec<PowerTab>` produzido de [`tab_models`] + `cx.listener`),
/// - [`PowersPanel::powers`] (`Vec<PowerCard>` da aba ativa).
#[derive(IntoElement)]
pub struct PowersPanel {
    summary: String,
    tabs: Vec<PowerTab>,
    cards: Vec<PowerCard>,
    /// Gesto de fechar (✕ no canto sup. direito do header). `None` ⇒ painel sem ✕ (pré-visualização /
    /// gate-de-tela). O call-site (main.rs) liga isto ao mesmo handler do clique-fora + Esc.
    on_close: Option<ClickHandler>,
}

impl PowersPanel {
    /// Novo painel a partir do inventário (alimenta o resumo do nível 1 com `inventory.counts`).
    /// As abas e os cards vêm depois via [`PowersPanel::tabs`]/[`PowersPanel::powers`].
    #[must_use]
    pub fn new(inventory: &PowerInventory) -> Self {
        Self {
            summary: summary_line(&inventory.counts),
            tabs: Vec::new(),
            cards: Vec::new(),
            on_close: None,
        }
    }

    /// As abas a renderizar — vindas de [`tab_models`] e mapeadas com o handler de troca pelo
    /// call-site. Sem abas = sem fila no cabeçalho (inventário vazio cai no estado vazio).
    #[must_use]
    pub fn tabs(mut self, tabs: Vec<PowerTab>) -> Self {
        self.tabs = tabs;
        self
    }

    /// Os cards a exibir — já JÁ filtrados pela ABA ATIVA (call-site) e por [`should_render`]
    /// (`Disabled` escondido enquanto o app não religar). Sem cards = estado vazio acolhedor.
    #[must_use]
    pub fn powers(mut self, cards: Vec<PowerCard>) -> Self {
        self.cards = cards;
        self
    }

    /// Handler do ✕ no canto sup. direito do header. Passe `cx.listener(|view, _ev, _w, cx| {
    /// view.powers_panel_open = false; cx.notify(); })`. Sem handler = painel SEM ✕ (preview/teste).
    #[must_use]
    pub fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for PowersPanel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();

        // ── Cabeçalho fixo ───────────────────────────────────────────────────────────
        // r3: 1ª linha = título (Fraunces, à esquerda, `flex_1`) + ✕ (`ghost icon`, à direita) — só
        // existe quando o call-site passa `on_close`. Sem handler, o painel sobe SEM ✕ (preview/teste).
        let title = title_text(&t, SharedString::from(PANEL_TITLE));
        let close_btn = self.on_close.map(|cb| {
            Button::new("powers-close", "✕")
                .icon()
                .ghost()
                .aria(SharedString::from(CLOSE_ARIA))
                .on_click(cb)
        });
        let title_row = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(Space::Sm.px(&t)))
            .child(div().flex_1().child(title))
            .children(close_btn);

        let summary = (!self.summary.is_empty())
            .then(|| summary_text(&t, SharedString::from(self.summary.clone())));
        let intro = muted_text(&t, SharedString::from(PANEL_INTRO));

        let tab_row = (!self.tabs.is_empty()).then(|| {
            // flex_wrap p/ janelas estreitas: as abas quebram pra próxima linha em vez de cortar.
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(Space::Sm.px(&t)))
                .children(self.tabs)
        });

        let header = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(Space::Md.px(&t)))
            .child(title_row)
            .children(summary)
            .child(intro)
            .children(tab_row);

        // ── Body rolável (idioma do app — ver `onboarding.rs:1152-1157`/`attention_ui.rs:1058-1060`):
        //    `flex_1 + min_h(px(0.)) + overflow_y_scroll` num filho do flex_col raiz. `px(0.)` é
        //    reset estrutural (catraca ignora, ver test em `tests/token_ratchet.rs:262`).
        let body: gpui::AnyElement = if self.cards.is_empty() {
            // Estado vazio: acolhedor, centralizado vertical. NUNCA tela em branco (inv #6).
            div()
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .child(muted_text(&t, SharedString::from(EMPTY_STATE)))
                .into_any_element()
        } else if use_grid(self.cards.len()) {
            // Lista volumosa (skills/plugins): SPLIT intercalado em 2 colunas — alturas equilibradas,
            // sem a monotonia de pilha única de 60+ cards.
            let mut col_a: Vec<PowerCard> = Vec::with_capacity(self.cards.len() / 2 + 1);
            let mut col_b: Vec<PowerCard> = Vec::with_capacity(self.cards.len() / 2 + 1);
            for (i, card) in self.cards.into_iter().enumerate() {
                if i % 2 == 0 {
                    col_a.push(card);
                } else {
                    col_b.push(card);
                }
            }
            let gap = px(Space::Md.px(&t));
            div()
                .id("powers-body")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(gap)
                        .child(div().flex_1().flex().flex_col().gap(gap).children(col_a))
                        .child(div().flex_1().flex().flex_col().gap(gap).children(col_b)),
                )
                .into_any_element()
        } else {
            // Lista modesta: 1 coluna, cards "respiráveis".
            div()
                .id("powers-body")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(Space::Md.px(&t)))
                        .children(self.cards),
                )
                .into_any_element()
        };

        // ── Container raiz: identidade da casa via tokens (Card + borda 1px), `size_full` p/ casar
        //    com o pai de altura constrita do call-site (modal-like, ~0.72×0.82 da janela).
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(Space::Lg.px(&t)))
            .px(px(Space::Lg.px(&t)))
            .py(px(Space::Lg.px(&t)))
            .bg(rgb(t.surface.card))
            .border_1()
            .border_color(rgb(t.surface.border))
            .rounded_chrome()
            .child(header)
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

    /// **Abas: "Todos" primeiro com a soma; depois UMA por tipo PRESENTE na ordem canônica do
    /// `BTreeMap`. Tipo zerado NÃO emite aba (não polui o cabeçalho).** Default ativo = "Todos".
    #[test]
    fn tab_models_emit_all_first_then_present_kinds() {
        let mut counts: BTreeMap<PowerKind, u32> = BTreeMap::new();
        counts.insert(PowerKind::Skill, 75);
        counts.insert(PowerKind::Plugin, 33);
        counts.insert(PowerKind::Hook, 1);
        counts.insert(PowerKind::Agent, 0); // zerado: não aparece

        let tabs = tab_models(&counts, None);
        let ids: Vec<_> = tabs.iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            vec![
                None,
                Some(PowerKind::Skill),
                Some(PowerKind::Plugin),
                Some(PowerKind::Hook),
            ],
            "Todos primeiro; ordem canônica; Agent (count=0) NÃO entra"
        );
        assert_eq!(tabs[0].count, 75 + 33 + 1, "Todos = soma das presentes");
        assert!(tabs[0].active, "active=None ⇒ Todos ativa por padrão");
        assert!(
            tabs.iter().skip(1).all(|t| !t.active),
            "nenhuma outra fica ativa com active=None"
        );
    }

    /// Aba ativa específica: o `active=Some(kind)` desliga a "Todos" e liga só o tipo escolhido.
    #[test]
    fn tab_models_active_marks_only_the_chosen_one() {
        let mut counts: BTreeMap<PowerKind, u32> = BTreeMap::new();
        counts.insert(PowerKind::Skill, 5);
        counts.insert(PowerKind::Plugin, 9);
        let tabs = tab_models(&counts, Some(PowerKind::Plugin));
        let active: Vec<_> = tabs.iter().filter(|t| t.active).map(|t| t.id).collect();
        assert_eq!(
            active,
            vec![Some(PowerKind::Plugin)],
            "só Plugin marcada ativa"
        );
    }

    /// Inventário vazio: zero abas (o painel cai no estado vazio acolhedor, sem fila no cabeçalho).
    #[test]
    fn tab_models_empty_inventory_emits_no_tabs() {
        assert!(tab_models(&BTreeMap::new(), None).is_empty());
        let mut zeroed: BTreeMap<PowerKind, u32> = BTreeMap::new();
        zeroed.insert(PowerKind::Skill, 0);
        zeroed.insert(PowerKind::Plugin, 0);
        assert!(
            tab_models(&zeroed, None).is_empty(),
            "total=0 ⇒ sem abas (não emite 'Todos · 0')"
        );
    }

    /// Rótulo da aba: pluraliza o nome do tipo pela contagem; "Todos" é coletivo. Separador `·`.
    #[test]
    fn tab_label_pluralizes_per_count() {
        assert_eq!(
            tab_label(TabModel {
                id: None,
                count: 109,
                active: true
            }),
            "Todos · 109",
            "Todos é coletivo, não pluraliza"
        );
        assert_eq!(
            tab_label(TabModel {
                id: Some(PowerKind::Plugin),
                count: 44,
                active: false
            }),
            "Plugins · 44"
        );
        assert_eq!(
            tab_label(TabModel {
                id: Some(PowerKind::Hook),
                count: 1,
                active: false
            }),
            "Gatilho · 1",
            "singular pela contagem"
        );
    }

    /// Densidade: ≤ `GRID_THRESHOLD` ⇒ coluna única; acima ⇒ 2 colunas. Fronteira testada.
    #[test]
    fn use_grid_kicks_in_above_threshold() {
        assert!(
            !use_grid(0),
            "vazio fica em col simples (cai no estado vazio)"
        );
        assert!(
            !use_grid(GRID_THRESHOLD),
            "fronteira inclusiva: ≤ ⇒ col simples"
        );
        assert!(use_grid(GRID_THRESHOLD + 1), "acima do limite ⇒ 2 col");
        assert!(use_grid(75), "skills volumosos ⇒ 2 col");
    }

    /// r3: o painel constrói com e sem `on_close` — o setter é opcional, a casca é a mesma. Sem
    /// handler, o painel sobe SEM ✕ (preview/teste); com handler, ele entra no header. Não dá pra
    /// inspecionar a árvore gpui headless, então o que provamos aqui é que o builder mantém o
    /// contrato (não-vazio nos dois caminhos, sem panic).
    #[test]
    fn panel_builds_with_and_without_on_close() {
        let inv = mock_inventory();
        let _without = PowersPanel::new(&inv);
        let _with = PowersPanel::new(&inv).on_close(|_ev, _w, _cx| {});
    }

    /// Slug da aba é estável e único por id (não depende do rótulo, que pluraliza).
    #[test]
    fn tab_slug_is_stable_and_unique() {
        let ids = [
            None,
            Some(PowerKind::Skill),
            Some(PowerKind::Plugin),
            Some(PowerKind::Agent),
            Some(PowerKind::Command),
            Some(PowerKind::Hook),
            Some(PowerKind::Mcp),
        ];
        let slugs: Vec<&'static str> = ids.iter().copied().map(tab_slug).collect();
        let unique: std::collections::HashSet<_> = slugs.iter().collect();
        assert_eq!(unique.len(), slugs.len(), "cada aba tem um slug distinto");
        assert_eq!(tab_slug(None), "all", "aba 'Todos' = slug 'all'");
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
            tab_label(TabModel {
                id: None,
                count: 109,
                active: true,
            }),
            tab_label(TabModel {
                id: Some(PowerKind::Plugin),
                count: 44,
                active: false,
            }),
        ];
        let surfaces: Vec<&str> = [
            PANEL_TITLE,
            PANEL_INTRO,
            EMPTY_STATE,
            ORIGIN_LABEL,
            TAB_ALL,
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
