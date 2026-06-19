//! **F3-1-7 · O rosto da Goal na tela do leigo (spec 52 §"Superfície para o leigo").**
//!
//! **Invariante #6 do `CLAUDE.md` (não-técnico-first):** zero jargão na superfície. O usuário
//! descreveu o que quer; este card devolve isso em pt-br simples — a meta **como ele a disse**
//! ([`Goal::statement`]), **"o que entendi"** ([`Goal::interpretation`]), os **critérios em
//! linguagem clara** ("a página abre sem erro", "o teste passa") e uma **barra de progresso por
//! iteração**. Confirmar a interpretação é **1 toque** (sim/ajustar); ao escalar, um **aviso
//! narrado** ("Tentei N vezes e não fechou sozinho…"). `ReviewVerdict`/`root_cause_id`/`effort`/
//! `goal_id` **nunca** aparecem aqui.
//!
//! **Identidade = design system F2** (épico 38): toda cor/medida sai de [`crate::theme::active`]
//! (zero literal — a catraca F2-1-5 nasce verde para este arquivo), e o card reusa o catálogo
//! [`crate::ui`] (`Panel` = card, `Button` = CTA, `Badge` = estado). Fraunces ([`family.display`])
//! só no **hero** (a meta do fundador é o "momento" autorizado — mesmo idioma de `empty_canvas_hint`);
//! o corpo é IBM Plex ([`family.ui`]).
//!
//! **Padrão da casa** (mesmo de `ui/panel.rs`/`ui/button.rs`): a DECISÃO (humanizar critério,
//! fase→badge, fração de progresso, rótulo do CTA) mora em **funções puras testáveis** — gpui não
//! roda headless, então o [`RenderOnce`] é casca fina sobre elas.
//!
//! [`family.display`]: crate::theme::FontFamilyTokens::display
//! [`family.ui`]: crate::theme::FontFamilyTokens::ui

// F3-1-7 (wiring): a projeção `Goal` VIVA é o call-site real — `main.rs::render_goals_panel` varre o
// event log via `lina_core::project_goals` e renderiza um card por meta viva ([`surfaced`] escolhe
// quais e em que ordem). `mock_goal` virou fixture só de teste (`cfg(test)`).

use gpui::{
    div, prelude::*, px, relative, rgb, App, ClickEvent, FontWeight, IntoElement, SharedString,
    Window,
};
use lina_core::{AcceptanceCriterion, CheckKind, Goal, GoalPhase};

use crate::theme::{self, Theme};
use crate::ui::{Badge, BadgeTone, Button, ClickHandler, Panel, Space, Surface};

/// Teto de voltas do OUTER loop antes de escalar ao humano (spec 52 §"O turn-budget freia":
/// `goal_max_iterations`, default **3**). É um PARÂMETRO DE SISTEMA (spec 51), não um número de
/// design: o card o recebe via [`GoalCard::iteration_budget`]; este default só serve a [`mock_goal`]
/// e à pré-visualização enquanto o Maestro não passa o valor vivo na integração.
pub const DEFAULT_ITERATION_BUDGET: u32 = 3;

// ───────────────────────────── Fase → estado sem jargão (Badge) ─────────────────────────────

/// O estado da Goal traduzido para o leigo: a PALAVRA que o [`Badge`] mostra, seu tom e se ele
/// "precisa de você" (anúncio assertivo). NUNCA o nome técnico da fase ([`GoalPhase`]) na tela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseStatus {
    pub label: &'static str,
    pub tone: BadgeTone,
    pub needs_you: bool,
}

/// Traduz a [`GoalPhase`] para o estado de tela do leigo. Pura (a doutrina do mapa vive no teste).
///
/// `Interpreted` é o **gate humano** (autonomia `assistido`: PROPÕE → você confirma, spec 52
/// §"Confirmação da meta") — por isso "precisa de você"; é onde os botões sim/ajustar aparecem.
/// `Escalated` é **pausa resumível, nunca falha** (spec 52 §1) — tom de atenção (âmbar), não de
/// erro (vermelho): o sistema parou pra te ouvir.
#[must_use]
pub fn phase_status(phase: GoalPhase) -> PhaseStatus {
    let (label, tone, needs_you) = match phase {
        GoalPhase::Defined => ("Anotando sua meta", BadgeTone::Neutral, false),
        GoalPhase::Interpreted => ("Confirmar com você", BadgeTone::Info, true),
        GoalPhase::Confirmed => ("Combinado", BadgeTone::Info, false),
        GoalPhase::Decomposed => ("Organizando o trabalho", BadgeTone::Info, false),
        GoalPhase::InLoop => ("Trabalhando nisso", BadgeTone::Info, false),
        GoalPhase::Achieved => ("Pronto", BadgeTone::Success, false),
        GoalPhase::Escalated => ("Precisa de você", BadgeTone::Warning, true),
    };
    PhaseStatus {
        label,
        tone,
        needs_you,
    }
}

// ───────────────────────────── Critério → linguagem clara ─────────────────────────────

/// A linha de um critério de aceite em pt-br do leigo. O [`AcceptanceCriterion::desc`] já é
/// redigido **sem jargão** (doc em `events.rs`: "narrado no card"), então é a fonte primária;
/// quando vazio, cai numa frase honesta derivada de COMO o critério é verificado.
///
/// **Segurança/superfície (spec 52 §Superfície + §Segurança 3):** NUNCA exibe
/// [`AcceptanceCriterion::check_arg`] — ele pode conter um comando, glob ou caminho (jargão, e às
/// vezes ação sensível). O leigo vê a INTENÇÃO ("o teste passa"), nunca o `cargo test --glob …`.
#[must_use]
pub fn criterion_line(c: &AcceptanceCriterion) -> String {
    let desc = c.desc.trim();
    if !desc.is_empty() {
        return desc.to_string();
    }
    match c.check_kind {
        CheckKind::HumanReview => "você confere e aprova".to_string(),
        CheckKind::Command => "roda sem erro".to_string(),
        CheckKind::FileExists => "o arquivo é criado".to_string(),
        CheckKind::TestPass => "o teste passa".to_string(),
    }
}

// ───────────────────────────── Decisão humana → rótulos do CTA ─────────────────────────────

/// Os rótulos `(afirmar, ajustar)` da decisão de 1 toque da fase atual — ou `None` quando a fase
/// não pede decisão do humano. O par mapeia para `Button::confirm`/`Button::secondary`.
///
/// - `Interpreted`: confirmar a interpretação (spec 52 §"Confirmação da meta").
/// - `Escalated`: a escolha do aviso narrado — reforçar o time ou olhar junto (spec 52 §Superfície).
#[must_use]
pub fn cta_labels(phase: GoalPhase) -> Option<(&'static str, &'static str)> {
    match phase {
        GoalPhase::Interpreted => Some(("Sim, é isso", "Quero ajustar")),
        GoalPhase::Escalated => Some(("Reforçar o time", "Olhar com você")),
        _ => None,
    }
}

/// O aviso narrado ao escalar (spec 52 §Superfície, palavra-por-palavra adaptada ao budget vivo):
/// o sistema admite que tentou e parou pra te ouvir — opção, não erro cru.
#[must_use]
pub fn escalation_notice(budget: u32) -> String {
    format!(
        "Tentei {budget} vezes e ainda não fechou sozinho. \
         Quer que eu use um time mais reforçado, ou prefere olhar comigo?"
    )
}

// ───────────────────────────── Microcopy do ciclo de correção ─────────────────────────────

/// Título da seção quando o usuário clicou "Quero ajustar": deixa ÓBVIO que corrigir abre uma NOVA
/// rodada de interpretação (o ciclo propor→corrigir→re-propor), não um simples "salvar texto". 1ª
/// pessoa do sistema (mesma voz de "O que entendi" / "Tentei N vezes…"). Sem jargão — testado.
pub const CORRECTION_TITLE: &str = "Me corrija que eu repenso";

/// Dica de ação do editor de correção. O verbo é "envia" — não "salva" — porque o texto vai ao
/// Maestro RE-INTERPRETAR, fechando o ciclo numa proposta nova; guia as duas teclas. Sem jargão.
pub const CORRECTION_HINT: &str = "Escreva do seu jeito · Enter envia · Esc cancela";

// ───────────────────────────── Barra de progresso por iteração ─────────────────────────────

/// O tom da barra de progresso — cada um carrega um SIGNIFICADO (não decoração), espelhando os
/// tons do [`Badge`]. A cor nunca é o único sinal: o [`Progress::label`] visível diz o mesmo
/// (WCAG 1.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressTone {
    /// Em andamento (laço rodando, ou ainda começando) — acento do shell.
    Working,
    /// Fechou — verde de estado.
    Done,
    /// Pausado pedindo você (escalou) — âmbar de estado.
    Attention,
}

impl ProgressTone {
    /// A cor de preenchimento da barra, resolvida do tema vivo (único ponto de cor da barra).
    #[must_use]
    pub fn fill(self, t: &Theme) -> u32 {
        match self {
            ProgressTone::Working => t.accent.primary,
            ProgressTone::Done => t.state.success,
            ProgressTone::Attention => t.state.warning,
        }
    }
}

/// O estado renderizável da barra: a FRAÇÃO preenchida (0..=1), o rótulo visível em pt-br e o tom.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub fraction: f32,
    pub label: String,
    pub tone: ProgressTone,
}

/// **A barra de progresso "por iteração"** (spec 52 §Superfície) derivada do estado da [`Goal`].
/// Pura — a regra de quanto preencher e o que dizer vive no teste.
///
/// A fração reflete o ESFORÇO no laço: a "tentativa atual" ocupa sua fatia do budget. Antes do laço
/// (fases de preparo) não houve tentativa — a barra fica no começo, honestamente. Ao fechar, cheia
/// e verde, independentemente de quantas voltas levou. Ao escalar, congela no teto com tom de atenção.
#[must_use]
pub fn progress(goal: &Goal, budget: u32) -> Progress {
    match goal.phase {
        GoalPhase::Achieved => Progress {
            fraction: 1.0,
            label: "Concluído".to_string(),
            tone: ProgressTone::Done,
        },
        GoalPhase::Escalated => Progress {
            fraction: 1.0,
            label: "Pausado pra falar com você".to_string(),
            tone: ProgressTone::Attention,
        },
        GoalPhase::InLoop => {
            // `iterations` = voltas JÁ gastas; a tentativa em curso é a próxima (limitada ao budget).
            // A FRAÇÃO carrega o avanço pelo orçamento; o RÓTULO traduz a rodada em ESTADO humano — o
            // leigo nunca lê "tentativa N de M" (despacho F3-2-7): a 1ª passada é só trabalho; uma
            // rodada do meio é um reajuste; a última admite o limite e PREPARA a escalada sem assustar.
            let budget = budget.max(1);
            let attempt = (goal.iterations + 1).min(budget);
            let label = if attempt == 1 {
                "Trabalhando nisso"
            } else if attempt >= budget {
                "Última tentativa antes de chamar você"
            } else {
                "Revisando e tentando de novo"
            };
            Progress {
                fraction: attempt_fraction(attempt, budget),
                label: label.to_string(),
                tone: ProgressTone::Working,
            }
        }
        GoalPhase::Decomposed => Progress {
            fraction: 0.0,
            label: "Pronto pra começar".to_string(),
            tone: ProgressTone::Working,
        },
        GoalPhase::Defined | GoalPhase::Interpreted | GoalPhase::Confirmed => Progress {
            fraction: 0.0,
            label: "Começando".to_string(),
            tone: ProgressTone::Working,
        },
    }
}

/// Fração da barra para a tentativa `attempt` (1-based) dentro de `budget`. Clampada a 0..=1;
/// `budget == 0` é tratado como 1 (degrada honesto, nunca divide por zero).
#[must_use]
fn attempt_fraction(attempt: u32, budget: u32) -> f32 {
    let budget = budget.max(1);
    (attempt as f32 / budget as f32).clamp(0.0, 1.0)
}

// ───────────────────────────── Quais Goals merecem rosto na tela ─────────────────────────────

/// Filtra e ORDENA as Goals vivas para o painel do canvas. Pura (a regra de superfície vive no
/// teste). Esconde as que já FECHARAM (`Achieved` — o trabalho acabou, o card não precisa mais
/// chamar atenção) e ordena por URGÊNCIA: primeiro as que pedem você (`Escalated`, depois
/// `Interpreted`), depois as em curso, depois o preparo. Estável no empate (preserva a ordem do log).
#[must_use]
pub fn surfaced(goals: Vec<Goal>) -> Vec<Goal> {
    let mut live: Vec<Goal> = goals
        .into_iter()
        .filter(|g| g.phase != GoalPhase::Achieved)
        .collect();
    live.sort_by_key(|g| urgency_rank(g.phase));
    live
}

/// Ordem de urgência (menor = mais urgente). Determinística e exaustiva sobre [`GoalPhase`].
#[must_use]
fn urgency_rank(phase: GoalPhase) -> u8 {
    match phase {
        GoalPhase::Escalated => 0,
        GoalPhase::Interpreted => 1,
        GoalPhase::InLoop => 2,
        GoalPhase::Decomposed => 3,
        GoalPhase::Confirmed => 4,
        GoalPhase::Defined => 5,
        GoalPhase::Achieved => 6, // filtrada acima; o braço mantém o match exaustivo.
    }
}

// ───────────────────────────── Fixture de tela (founder session) ─────────────────────────────

/// Uma [`Goal`] de exemplo, em pt-br realista, para os testes — na fase `Interpreted` (onde o card
/// mostra interpretação + critérios + o CTA de 1 toque). Só fixture de teste: a tela de produção
/// renderiza a projeção VIVA (`main.rs::render_goals_panel`), nunca este mock.
#[cfg(test)]
#[must_use]
pub fn mock_goal() -> Goal {
    Goal {
        goal_id: "preview".to_string(),
        statement: "Quero uma página de vendas para o meu curso de confeitaria".to_string(),
        phase: GoalPhase::Interpreted,
        interpretation: Some(
            "Montar uma página única que apresenta o curso, mostra as aulas e tem um botão de \
             compra que leva ao pagamento."
                .to_string(),
        ),
        strategy: Some(
            "Um time enxuto: alguém monta a página, alguém revisa o texto, e eu confiro no fim."
                .to_string(),
        ),
        acceptance: vec![
            AcceptanceCriterion {
                desc: "A página abre sem erro no navegador".to_string(),
                check_kind: CheckKind::Command,
                check_arg: None,
            },
            AcceptanceCriterion {
                desc: "O botão de compra leva para o pagamento".to_string(),
                check_kind: CheckKind::HumanReview,
                check_arg: None,
            },
            AcceptanceCriterion {
                desc: "O texto da página está revisado e sem erros".to_string(),
                check_kind: CheckKind::HumanReview,
                check_arg: None,
            },
        ],
        items: Vec::new(),
        iterations: 0,
    }
}

// ───────────────────────────── O componente (RenderOnce) ─────────────────────────────

/// **O card da Goal.** builder + [`RenderOnce`], view-agnóstico (os handlers de 1 toque vêm do
/// call-site via `cx.listener(...)`, como o resto do catálogo `ui/`). Construa com [`GoalCard::new`].
#[derive(IntoElement)]
pub struct GoalCard {
    goal: Goal,
    iteration_budget: u32,
    /// Recolhido = só o cabeçalho + a meta (faixa fina), para o card não cobrir o canvas. O estado
    /// vive na view (por `goal_id`); o card só REFLETE e oferece o toggle.
    collapsed: bool,
    /// Decisão afirmativa da fase (confirmar interpretação / reforçar time). `None` = botão inerte
    /// (pré-visualização). Recebe `cx.listener(...)`.
    on_confirm: Option<ClickHandler>,
    /// Decisão de ajuste (corrigir interpretação / olhar junto). `None` = botão inerte.
    on_correct: Option<ClickHandler>,
    /// Toggle recolher/expandir (botão do cabeçalho). `None` = sem toggle (pré-visualização).
    on_toggle: Option<ClickHandler>,
    /// Fechar (dispensar) o card da tela — gesto DURÁVEL (a view persiste nos settings). `None` = sem
    /// botão (pré-visualização).
    on_dismiss: Option<ClickHandler>,
    /// Editor inline do entendimento ATIVO (`Some(buffer)` = o usuário clicou "Quero ajustar" e digita
    /// a correção). O buffer e o teclado vivem na view; o card só RENDERIZA o campo no lugar de "O que
    /// entendi".
    editing: Option<SharedString>,
}

impl GoalCard {
    /// Novo card para uma [`Goal`]. Budget de iteração começa no default da spec até o call-site
    /// passar o parâmetro de sistema vivo via [`Self::iteration_budget`].
    #[must_use]
    pub fn new(goal: Goal) -> Self {
        Self {
            goal,
            iteration_budget: DEFAULT_ITERATION_BUDGET,
            collapsed: false,
            on_confirm: None,
            on_correct: None,
            on_toggle: None,
            on_dismiss: None,
            editing: None,
        }
    }

    /// Handler do botão fechar/dispensar (cabeçalho). Passe `cx.listener(...)`.
    #[must_use]
    pub fn on_dismiss(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }

    /// Reflete o editor inline do entendimento (`Some(buffer)` = em edição). A view é a dona do buffer
    /// e do teclado; o card só RENDERIZA o campo.
    #[must_use]
    pub fn editing(mut self, buffer: Option<SharedString>) -> Self {
        self.editing = buffer;
        self
    }

    /// Reflete o estado recolhido (a view é a dona). Recolhido esconde tudo menos o cabeçalho + a meta.
    #[must_use]
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Handler do toggle recolher/expandir (botão do cabeçalho). Passe `cx.listener(...)`.
    #[must_use]
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    /// O teto de iterações vivo (parâmetro de sistema, spec 51) — alimenta a barra e o aviso de escalada.
    #[must_use]
    pub fn iteration_budget(mut self, budget: u32) -> Self {
        self.iteration_budget = budget;
        self
    }

    /// Handler da decisão afirmativa (sim/reforçar). Passe `cx.listener(|view, _ev, window, cx| …)`.
    #[must_use]
    pub fn on_confirm(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_confirm = Some(Box::new(handler));
        self
    }

    /// Handler da decisão de ajuste (corrigir/olhar junto). Passe `cx.listener(...)`.
    #[must_use]
    pub fn on_correct(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_correct = Some(Box::new(handler));
        self
    }
}

/// Rótulo de seção ("eyebrow"/título de seção) em Plex, discreto — o idioma de rótulo recorrente
/// (NUNCA Fraunces aqui; o serif é só o hero).
fn section_label(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.secondary))
        .child(gpui::text!(text))
}

/// Um parágrafo de corpo (Plex, cor primária) — a interpretação narrada.
fn body_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.body)))
        .text_color(rgb(t.text.primary))
        .child(gpui::text!(text))
}

impl RenderOnce for GoalCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let status = phase_status(self.goal.phase);
        let prog = progress(&self.goal, self.iteration_budget);
        let collapsed = self.collapsed;

        // Botão recolher/expandir (discreto, ghost) — só quando a view fia o toggle.
        let toggle = self.on_toggle.map(|h| {
            Button::new(
                "goal-collapse",
                if collapsed { "expandir" } else { "recolher" },
            )
            .ghost()
            .on_click(h)
        });
        // Botão fechar/dispensar — tira o card da tela (durável; a meta segue no log).
        let dismiss = self
            .on_dismiss
            .map(|h| Button::new("goal-dismiss", "fechar").ghost().on_click(h));

        // Cabeçalho: rótulo "Sua meta" + selo de estado + toggle (Badge anuncia troca via live-region).
        let header = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .gap(px(Space::Sm.px(&t)))
            .child(section_label(&t, "Sua meta"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(Space::Sm.px(&t)))
                    .child({
                        let badge = Badge::new("goal-phase", status.label, status.tone);
                        if status.needs_you {
                            badge.needs_you()
                        } else {
                            badge
                        }
                    })
                    .children(toggle)
                    .children(dismiss),
            );

        // Hero: a meta COMO O USUÁRIO DISSE — o "momento" do fundador (Fraunces autorizado aqui).
        let hero = div()
            .font_family(t.typography.family.display)
            .text_size(px(f32::from(t.typography.size.title)))
            .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
            .text_color(rgb(t.text.bright))
            .child(gpui::text!(SharedString::from(self.goal.statement.clone())));

        // "O que entendi" — ou o EDITOR inline quando o usuário clicou "Quero ajustar" (F3-1-7). O
        // cursor "|" marca onde o texto cresce; o teclado (Enter salva · Esc cancela) é tratado na view.
        let understanding = if let Some(buf) = self.editing.clone() {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Space::Xs.px(&t)))
                    .child(section_label(&t, CORRECTION_TITLE))
                    .child(
                        Panel::surface()
                            .bg(Surface::RaisedAlt)
                            .border(t.accent.primary)
                            .pad(Space::Md, Space::Sm)
                            .child(body_text(&t, SharedString::from(format!("{buf}|")))),
                    )
                    .child(
                        div()
                            .font_family(t.typography.family.ui)
                            .text_size(px(f32::from(t.typography.size.small)))
                            .text_color(rgb(t.text.muted))
                            .child(gpui::text!(CORRECTION_HINT)),
                    ),
            )
        } else {
            self.goal.interpretation.clone().map(|interp| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Space::Xs.px(&t)))
                    .child(section_label(&t, "O que entendi"))
                    .child(body_text(&t, SharedString::from(interp)))
            })
        };

        // "Como vou fazer" — a estratégia proposta (só fora do editor e quando há estratégia).
        let strategy_section = self
            .editing
            .is_none()
            .then(|| self.goal.strategy.clone())
            .flatten()
            .map(|strat| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Space::Xs.px(&t)))
                    .child(section_label(&t, "Como vou fazer"))
                    .child(body_text(&t, SharedString::from(strat)))
            });

        // "Como vou saber que deu certo" — critérios em linguagem clara, marcados feito quando fechou.
        let achieved = self.goal.phase == GoalPhase::Achieved;
        let criteria_rows: Vec<_> = self
            .goal
            .acceptance
            .iter()
            .map(|c| criterion_row(&t, &criterion_line(c), achieved))
            .collect();
        let criteria = (!criteria_rows.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(Space::Xs.px(&t)))
                .child(section_label(&t, "Como vou saber que deu certo"))
                .children(criteria_rows)
        });

        // Barra de progresso por iteração.
        let bar = progress_bar(&t, &prog);

        // Aviso narrado de escalada (só na pausa) — opção, não erro cru.
        let escalation = (self.goal.phase == GoalPhase::Escalated).then(|| {
            Panel::surface()
                .bg(Surface::RaisedAlt)
                .border(t.state.warning)
                .pad(Space::Md, Space::Sm)
                .child(body_text(
                    &t,
                    SharedString::from(escalation_notice(self.iteration_budget)),
                ))
        });

        // CTA de 1 toque (sim/ajustar) — só nas fases que pedem decisão humana.
        let actions = cta_labels(self.goal.phase).map(|(affirm, adjust)| {
            let mut confirm = Button::new("goal-confirm", affirm).confirm();
            if let Some(h) = self.on_confirm {
                confirm = confirm.on_click(h);
            }
            let mut correct = Button::new("goal-correct", adjust).secondary();
            if let Some(h) = self.on_correct {
                correct = correct.on_click(h);
            }
            div()
                .flex()
                .flex_row()
                .gap(px(Space::Sm.px(&t)))
                .child(confirm)
                .child(correct)
        });

        // `Panel` é um builder próprio (não implementa o `FluentBuilder` do gpui, logo sem
        // `when_some`): compomos os filhos em ordem — os opcionais entram só quando há conteúdo — e
        // passamos a lista a `children`.
        let mut kids: Vec<gpui::AnyElement> =
            vec![header.into_any_element(), hero.into_any_element()];
        // Recolhido: para no cabeçalho + a meta (faixa fina). Expandido: o card inteiro.
        if !collapsed {
            if let Some(u) = understanding {
                kids.push(u.into_any_element());
            }
            if let Some(s) = strategy_section {
                kids.push(s.into_any_element());
            }
            if let Some(c) = criteria {
                kids.push(c.into_any_element());
            }
            kids.push(bar.into_any_element());
            if let Some(e) = escalation {
                kids.push(e.into_any_element());
            }
            if let Some(a) = actions {
                kids.push(a.into_any_element());
            }
        }

        Panel::surface()
            .gap(Space::Lg)
            .pad(Space::Lg, Space::Lg)
            .children(kids)
    }
}

/// Uma linha de critério: glifo (feito/pendente) + a frase clara. Glifo redundante à intenção
/// (não é só cor — WCAG 1.4.1).
fn criterion_row(t: &Theme, line: &str, achieved: bool) -> impl IntoElement {
    let (glyph, color) = if achieved {
        ("✓", t.state.success)
    } else {
        ("○", t.text.muted)
    };
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(Space::Sm.px(t)))
        .child(
            div()
                .font_family(t.typography.family.ui)
                .text_size(px(f32::from(t.typography.size.body)))
                .text_color(rgb(color))
                .child(gpui::text!(glyph)),
        )
        .child(
            div()
                .font_family(t.typography.family.ui)
                .text_size(px(f32::from(t.typography.size.body)))
                .text_color(rgb(t.text.primary))
                .child(gpui::text!(SharedString::from(line.to_string()))),
        )
}

/// A barra de progresso: rótulo em pt-br + trilho recessado com o preenchimento fracionário
/// (`relative(fraction)` — sem px mágico). A largura É o sinal; o rótulo carrega o mesmo em texto.
fn progress_bar(t: &Theme, prog: &Progress) -> impl IntoElement {
    let fill_color = prog.tone.fill(t);
    div()
        .flex()
        .flex_col()
        .gap(px(Space::Xs.px(t)))
        .child(
            div()
                .font_family(t.typography.family.ui)
                .text_size(px(f32::from(t.typography.size.small)))
                .text_color(rgb(t.text.secondary))
                .child(gpui::text!(SharedString::from(prog.label.clone()))),
        )
        .child(
            // Trilho: largura total, altura de um passo pequeno do token, recessado e arredondado.
            div()
                .w_full()
                .h(px(Space::Sm.px(t)))
                .rounded_full()
                .bg(rgb(t.surface.raised_alt))
                .child(
                    // Preenchimento: fração da largura (clampada na lógica pura), na cor do tom.
                    div()
                        .h_full()
                        .w(relative(prog.fraction.clamp(0.0, 1.0)))
                        .rounded_full()
                        .bg(rgb(fill_color)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;

    fn theme() -> Theme {
        Theme::build(Mode::Dark, "azul")
    }

    /// Cada fase do ciclo vira uma PALAVRA do leigo, sem jargão; o gate (`Interpreted`) e a pausa
    /// (`Escalated`) "precisam de você"; o resto não interrompe.
    #[test]
    fn phase_status_speaks_layman_without_jargon() {
        for phase in [
            GoalPhase::Defined,
            GoalPhase::Interpreted,
            GoalPhase::Confirmed,
            GoalPhase::Decomposed,
            GoalPhase::InLoop,
            GoalPhase::Achieved,
            GoalPhase::Escalated,
        ] {
            let s = phase_status(phase);
            assert!(!s.label.is_empty(), "toda fase tem rótulo");
            let low = s.label.to_lowercase();
            for jargon in [
                "goal",
                "phase",
                "verdict",
                "effort",
                "loop",
                "review",
                "root_cause",
            ] {
                assert!(
                    !low.contains(jargon),
                    "fase {phase:?} vazou jargão: {:?}",
                    s.label
                );
            }
        }
        assert!(
            phase_status(GoalPhase::Interpreted).needs_you,
            "gate humano"
        );
        assert!(
            phase_status(GoalPhase::Escalated).needs_you,
            "pausa pede você"
        );
        assert!(
            !phase_status(GoalPhase::InLoop).needs_you,
            "trabalhando não interrompe"
        );
        assert_eq!(phase_status(GoalPhase::Achieved).tone, BadgeTone::Success);
        // Escalada é PAUSA, não falha: âmbar de atenção, nunca o vermelho de erro.
        assert_eq!(phase_status(GoalPhase::Escalated).tone, BadgeTone::Warning);
    }

    /// `desc` (já sem jargão) é a linha; vazio cai numa frase honesta por tipo de verificação.
    /// E `check_arg` (comando/glob/caminho) JAMAIS vaza para a tela.
    #[test]
    fn criterion_line_is_human_and_never_leaks_check_arg() {
        let with_desc = AcceptanceCriterion {
            desc: "A página abre sem erro".to_string(),
            check_kind: CheckKind::Command,
            check_arg: Some("cargo run --bin abre_pagina --secreto".to_string()),
        };
        let line = criterion_line(&with_desc);
        assert_eq!(line, "A página abre sem erro");
        assert!(
            !line.contains("cargo") && !line.contains("--"),
            "o comando bruto nunca aparece na superfície"
        );

        // Sem desc: frase honesta derivada do tipo — nunca o argumento técnico.
        let bare = AcceptanceCriterion {
            desc: "   ".to_string(),
            check_kind: CheckKind::TestPass,
            check_arg: Some("tests/**/*.rs".to_string()),
        };
        let line = criterion_line(&bare);
        assert_eq!(line, "o teste passa");
        assert!(!line.contains("tests/"), "glob nunca vaza");

        assert_eq!(
            criterion_line(&AcceptanceCriterion {
                desc: String::new(),
                check_kind: CheckKind::FileExists,
                check_arg: None,
            }),
            "o arquivo é criado"
        );
        assert_eq!(
            criterion_line(&AcceptanceCriterion {
                desc: String::new(),
                check_kind: CheckKind::HumanReview,
                check_arg: None,
            }),
            "você confere e aprova"
        );
    }

    /// O CTA de 1 toque só existe onde há decisão humana (interpretar/escalar); senão é `None`.
    #[test]
    fn cta_only_where_a_human_decides() {
        assert_eq!(
            cta_labels(GoalPhase::Interpreted),
            Some(("Sim, é isso", "Quero ajustar"))
        );
        assert_eq!(
            cta_labels(GoalPhase::Escalated),
            Some(("Reforçar o time", "Olhar com você"))
        );
        for phase in [
            GoalPhase::Defined,
            GoalPhase::Confirmed,
            GoalPhase::Decomposed,
            GoalPhase::InLoop,
            GoalPhase::Achieved,
        ] {
            assert_eq!(cta_labels(phase), None, "{phase:?} não pede decisão");
        }
    }

    /// O microcopy de "Quero ajustar" deixa ÓBVIO que o ajuste dispara uma RE-interpretação (o ciclo
    /// propor→corrigir→re-propor, não um mero "salvar texto"), guia as duas teclas e não vaza jargão.
    #[test]
    fn correction_microcopy_signals_the_reinterpret_loop() {
        assert!(!CORRECTION_TITLE.is_empty(), "a seção de ajuste tem título");
        // A dica guia as duas teclas do ciclo (enviar a correção / cancelar).
        assert!(CORRECTION_HINT.contains("Enter"), "diz como enviar");
        assert!(CORRECTION_HINT.contains("Esc"), "diz como cancelar");
        // "envia" (vai ao Maestro REPENSAR), nunca "salva" (mero texto): é o verbo que revela o loop.
        let hint_low = CORRECTION_HINT.to_lowercase();
        assert!(
            hint_low.contains("envia") && !hint_low.contains("salva"),
            "o verbo deixa claro que dispara nova interpretação, não só persiste texto: {CORRECTION_HINT:?}"
        );
        for jargon in [
            "interpret",
            "goal",
            "reinterpret",
            "payload",
            "intent",
            "verdict",
            "loop",
        ] {
            assert!(
                !CORRECTION_TITLE.to_lowercase().contains(jargon),
                "título vazou jargão: {CORRECTION_TITLE:?}"
            );
            assert!(
                !hint_low.contains(jargon),
                "dica vazou jargão: {CORRECTION_HINT:?}"
            );
        }
    }

    /// O aviso de escalada usa o budget vivo, admite a tentativa e oferece ESCOLHA (não erro cru).
    #[test]
    fn escalation_notice_narrates_with_live_budget() {
        let msg = escalation_notice(3);
        assert!(msg.contains('3'), "usa o budget real");
        assert!(msg.contains("prefere olhar"), "oferece olhar junto");
        assert!(
            !msg.to_lowercase().contains("error") && !msg.contains("Fail"),
            "sem jargão de erro"
        );
        assert!(
            escalation_notice(5).contains('5'),
            "parametrizado pelo budget"
        );
    }

    /// A barra preenche POR iteração: a tentativa em curso ocupa sua fatia do budget; antes do laço
    /// fica no começo; ao fechar enche e verde; ao escalar congela no teto com atenção.
    #[test]
    fn progress_fills_per_iteration() {
        let mut g = mock_goal();

        // Pré-laço: nenhuma tentativa ainda.
        g.phase = GoalPhase::Confirmed;
        assert_eq!(progress(&g, 3).fraction, 0.0);
        assert_eq!(progress(&g, 3).tone, ProgressTone::Working);

        // No laço: a barra avança POR rodada (a FRAÇÃO é o sinal), mas o texto fala humano — o leigo
        // lê um ESTADO, nunca "tentativa N de M" (despacho F3-2-7: traduzir, não expor o número).
        g.phase = GoalPhase::InLoop;
        g.iterations = 0;
        let p = progress(&g, 3);
        assert_eq!(
            p.label, "Trabalhando nisso",
            "1ª passada: trabalho, não 'tentativa'"
        );
        assert!((p.fraction - 1.0 / 3.0).abs() < f32::EPSILON);
        g.iterations = 1;
        assert_eq!(
            progress(&g, 3).label,
            "Revisando e tentando de novo",
            "rodada do meio: houve um ajuste, segue tentando"
        );
        g.iterations = 2;
        assert_eq!(
            progress(&g, 3).label,
            "Última tentativa antes de chamar você",
            "última rodada: honesto sobre o limite, prepara a escalada sem assustar"
        );
        assert!((progress(&g, 3).fraction - 1.0).abs() < f32::EPSILON);

        // Fechou: cheia e verde, não importa quantas voltas.
        g.phase = GoalPhase::Achieved;
        let p = progress(&g, 3);
        assert_eq!(p.fraction, 1.0);
        assert_eq!(p.tone, ProgressTone::Done);
        assert_eq!(p.label, "Concluído");

        // Escalou: congela cheia, tom de atenção (pausa, não erro).
        g.phase = GoalPhase::Escalated;
        assert_eq!(progress(&g, 3).tone, ProgressTone::Attention);
    }

    /// O rótulo da barra é estado em pt-br do leigo em TODA fase/rodada: zero jargão e **zero dígito**
    /// — o número técnico da iteração ("tentativa N de M", o `effort`) nunca chega à tela; a FRAÇÃO
    /// carrega o avanço, o TEXTO carrega o significado (despacho F3-2-7 · invariante #6).
    #[test]
    fn progress_label_is_layman_without_jargon_or_digits() {
        let mut g = mock_goal();
        for (phase, iters) in [
            (GoalPhase::Defined, 0),
            (GoalPhase::Interpreted, 0),
            (GoalPhase::Confirmed, 0),
            (GoalPhase::Decomposed, 0),
            (GoalPhase::InLoop, 0),
            (GoalPhase::InLoop, 1),
            (GoalPhase::InLoop, 2),
            (GoalPhase::InLoop, 9),
            (GoalPhase::Achieved, 2),
            (GoalPhase::Escalated, 3),
        ] {
            g.phase = phase;
            g.iterations = iters;
            let label = progress(&g, 3).label;
            assert!(!label.is_empty(), "{phase:?}/{iters} tem rótulo");
            let low = label.to_lowercase();
            for jargon in [
                "iteration",
                "effort",
                "loop",
                "verdict",
                "budget",
                "goal",
                "phase",
                "review",
            ] {
                assert!(!low.contains(jargon), "{phase:?} vazou jargão: {label:?}");
            }
            assert!(
                !label.chars().any(|c| c.is_ascii_digit()),
                "{phase:?}/{iters} expôs número técnico de iteração: {label:?}"
            );
        }
    }

    /// A fração nunca estoura 0..=1 nem divide por zero (budget degenerado).
    #[test]
    fn attempt_fraction_is_bounded() {
        assert_eq!(attempt_fraction(1, 3), 1.0 / 3.0);
        assert_eq!(attempt_fraction(9, 3), 1.0, "clampa no teto");
        assert_eq!(
            attempt_fraction(1, 0),
            1.0,
            "budget 0 degrada honesto, sem dividir por zero"
        );
    }

    /// Cada tom da barra resolve para o token do seu significado (acento/verde/âmbar), nunca cor crua.
    #[test]
    fn progress_tone_maps_to_meaning_token() {
        let t = theme();
        assert_eq!(ProgressTone::Working.fill(&t), t.accent.primary);
        assert_eq!(ProgressTone::Done.fill(&t), t.state.success);
        assert_eq!(ProgressTone::Attention.fill(&t), t.state.warning);
    }

    /// A fixture de tela é coerente: fase de gate, com interpretação e critérios humanos.
    #[test]
    fn mock_goal_is_screen_ready() {
        let g = mock_goal();
        assert_eq!(g.phase, GoalPhase::Interpreted);
        assert!(g.interpretation.is_some());
        assert!(!g.acceptance.is_empty());
        assert!(
            cta_labels(g.phase).is_some(),
            "a fixture exercita o CTA de 1 toque"
        );
    }

    /// `surfaced` esconde as metas FECHADAS e ordena por urgência: quem pede você primeiro
    /// (`Escalated` antes de `Interpreted`), depois o que está em curso, depois o preparo.
    #[test]
    fn surfaced_hides_achieved_and_orders_by_urgency() {
        let mk = |id: &str, phase: GoalPhase| {
            let mut g = mock_goal();
            g.goal_id = id.to_string();
            g.phase = phase;
            g
        };
        let goals = vec![
            mk("done", GoalPhase::Achieved),
            mk("preparo", GoalPhase::Defined),
            mk("gate", GoalPhase::Interpreted),
            mk("curso", GoalPhase::InLoop),
            mk("pausa", GoalPhase::Escalated),
        ];
        let out = surfaced(goals);
        let ids: Vec<&str> = out.iter().map(|g| g.goal_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["pausa", "gate", "curso", "preparo"],
            "Achieved some; urgência: Escalated > Interpreted > InLoop > Defined"
        );
        assert!(
            out.iter().all(|g| g.phase != GoalPhase::Achieved),
            "nenhuma meta fechada na superfície"
        );
    }

    /// Empate de fase preserva a ordem de chegada (estável) — a ordem do log não é embaralhada.
    #[test]
    fn surfaced_is_stable_on_phase_ties() {
        let mk = |id: &str| {
            let mut g = mock_goal();
            g.goal_id = id.to_string();
            g.phase = GoalPhase::InLoop;
            g
        };
        let out = surfaced(vec![mk("a"), mk("b"), mk("c")]);
        let ids: Vec<&str> = out.iter().map(|g| g.goal_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "empate = ordem do log preservada");
    }
}
