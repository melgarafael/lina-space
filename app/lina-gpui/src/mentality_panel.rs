//! **F3-3-5 · O painel "Como o [papel] pensa" — a janela do leigo para o aprendizado do time.**
//!
//! **Invariante #6 do `CLAUDE.md` (não-técnico-first):** zero jargão na superfície. O fundador VÊ,
//! em português dele, o que cada PAPEL aprendeu com ele (a "mentalidade" do papel — spec 35 §4.5),
//! de ONDE cada crença veio ("aprendido em 10/06, quando você corrigiu X"), e mantém o controle:
//! **aposentar uma crença com 1 clique** (humano é o árbitro final — §6.7). `belief_id`/hash/`TTL`/
//! `ReviewVerdict`/`CorrectionObserved` **nunca** aparecem aqui — é a voz da a11y, não `{:?}` de Debug.
//!
//! **Decisão do fundador (2026-06-21 — onda F3-3 §Pendências): PROMOÇÃO AUTOMÁTICA.** Uma crença que
//! atinge evidência suficiente vira regra do papel **sem fila de confirmação** — então o painel mostra
//! estabelecidas como **"já vale"** (não "aguardando seu OK") e provisórias como **"ainda testando"**.
//! O aposentar-1-clique permanece: é o controle *post-hoc* do humano sobre o que o time memorizou.
//!
//! **Identidade = design system F2** (épico 38): toda cor/medida sai de [`crate::theme::active`] (zero
//! literal — a catraca F2-1-5 nasce verde para este arquivo), e o painel reusa o catálogo [`crate::ui`]
//! (`Panel`/`Badge`/`Button`). Fraunces ([`family.display`]) **só** no título do papel — o fundador
//! vendo a mente do seu time é um "momento" autorizado (mesmo idioma do hero de `goal_card`); o corpo
//! é IBM Plex ([`family.ui`]). Esta é a CARA do diferencial da Lina (auto-aprimoramento humano-no-
//! comando): segue a casa, não inventa um visual novo.
//!
//! **Padrão da casa** (igual a `goal_card`/`ui/`): a DECISÃO (estado→rótulo/tom, copy, ordenação) mora
//! em **funções puras testáveis** — gpui não roda headless, então o [`RenderOnce`] é casca fina sobre
//! elas. O view-model ([`RoleMentality`]/[`BeliefView`]) é DESACOPLADO da projeção do core
//! (`lina_core::mentality`, M-PROMO): a costura `view-model ← projeção` é um adaptador fino na fiação
//! (`main.rs`), alinhado com o dono da projeção — o painel não conhece o shape do core.
//!
//! **Disparo de `BeliefRetired`:** o clique "esquecer isso" emite o gesto `belief.retire` pelo canal
//! view→core já existente (`NodeManager::push_human_intent`, igual a `goal.confirm`/`breaker.reset`);
//! o roteamento gesto→evento é costura do `bridge.rs` (M-INJETOR). A view NUNCA escolhe autoridade.
//!
//! [`family.display`]: crate::theme::FontFamilyTokens::display
//! [`family.ui`]: crate::theme::FontFamilyTokens::ui

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, FontWeight, IntoElement, SharedString, Window,
};

use crate::theme::{self, Theme};
use crate::ui::{Badge, BadgeTone, Button, ButtonSize, ClickHandler, Panel, Space};

// ───────────────────────────── Situação da crença → estado sem jargão ─────────────────────────────

/// Quão "firme" é uma crença do papel, na linguagem do leigo. NUNCA o vocabulário do ciclo
/// (`BeliefProposed`/`BeliefEstablished`) na tela — só o que a palavra significa para o fundador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Já virou regra do papel (promoção automática — o fundador decidiu 2026-06-21). "Já vale".
    Established,
    /// Em quarentena — ainda sendo testada na prática. "Ainda testando", convida correção.
    Provisional,
}

/// O estado de uma crença traduzido para o leigo: a PALAVRA do [`Badge`] e seu tom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandingStatus {
    pub label: &'static str,
    pub tone: BadgeTone,
}

/// Traduz a [`Standing`] para o estado de tela do leigo. Pura (a doutrina do mapa vive no teste).
///
/// Estabelecida = **"já vale"** — é o que o papel "já sabe", em tom de sucesso calmo (não "aguardando
/// seu OK": a promoção é automática). Provisória = **"ainda testando"** — âmbar de atenção suave que
/// CONVIDA a confirmar/corrigir, sem alarmar (é hipótese, não erro).
#[must_use]
pub fn standing_status(standing: Standing) -> StandingStatus {
    match standing {
        Standing::Established => StandingStatus {
            label: "já vale",
            tone: BadgeTone::Success,
        },
        Standing::Provisional => StandingStatus {
            label: "ainda testando",
            tone: BadgeTone::Warning,
        },
    }
}

// ───────────────────────────── Microcopy (pt-br, zero jargão) ─────────────────────────────

/// Convite calmo sob uma crença provisória: ela ainda está sendo testada, e o fundador pode validá-la
/// ou corrigi-la quando o caso aparecer. "Confirme ou corrija" espelha o critério da spec 35 §4.4 em
/// linguagem de leigo — sem citar a sentinela `[LINA::BELIEF_CONFIRMED]` (isso é injeção, não tela).
pub const PROVISIONAL_HINT: &str = "Ainda testando — confirme ou corrija quando aparecer.";

/// Rótulo do gesto de aposentar (1 clique → `BeliefRetired`). "Esquecer isso" é calmo e reversível na
/// percepção (o humano é o árbitro; a crença some da injeção, não é apagada do histórico) — não o
/// vermelho de "deletar" (a postura é controle tranquilo, não destruição).
pub const RETIRE_LABEL: &str = "esquecer isso";

/// Nome acessível do botão de aposentar (o rótulo visível já diz, mas o leitor de tela ganha o objeto).
pub const RETIRE_ARIA: &str = "esquecer esta lição";

/// Aviso suave quando a crença nasceu numa sessão que processou conteúdo externo não-confiável
/// (spec 35 §6.3: sinaliza, não bloqueia). Põe o fundador em alerta sem jargão de segurança.
pub const UNTRUSTED_NOTE: &str = "Veio de uma fonte externa — vale conferir antes de confiar.";

/// Estado vazio: o papel ainda não aprendeu nada com o fundador. Honesto e acolhedor — explica o
/// CICLO (corrija → eu memorizo) sem prometer mágica, e nunca mostra uma tela em branco (inv #6).
pub const EMPTY_STATE: &str =
    "Ainda não aprendi nada com você por aqui. Quando eu errar, me corrija — \
     eu memorizo pra não repetir.";

/// Rótulo da seção de proveniência de cada crença (de onde ela veio). Discreto, em pt-br.
pub const PROVENANCE_LABEL: &str = "De onde veio";

/// O título do painel para um papel — o "momento" do fundador (Fraunces autorizado). Segue o
/// template da spec 35 §4.5 ("Como o [papel] pensa"); o `role_label` já chega humanizado em pt-br
/// (ex.: "Backend", "Especialista em Telas") pela fiação — o painel nunca recebe um id de papel cru.
#[must_use]
pub fn role_title(role_label: &str) -> String {
    format!("Como o {role_label} pensa")
}

// ───────────────────────────── View-model (desacoplado do core) ─────────────────────────────

/// Uma crença do papel, JÁ traduzida para a superfície do leigo. É o contrato que o painel consome —
/// a projeção do core (`lina_core::mentality`, M-PROMO) é mapeada para isto por um adaptador fino na
/// fiação. Por design o painel não conhece `belief_id`/hash/contadores: só o `belief_id` viaja (oculto)
/// para o gesto de aposentar carregar QUAL crença, nunca para ser exibido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeliefView {
    /// Id durável da crença — usado SÓ para endereçar o gesto `belief.retire`; NUNCA renderizado.
    pub belief_id: String,
    /// O "como pensar" em linguagem clara (o statement falseável já humanizado). É o que o leigo lê.
    pub statement: String,
    /// Proveniência humanizada ("aprendido em 10/06, quando você corrigiu X") — DADO, não jargão.
    pub provenance: String,
    /// Quão firme: estabelecida ("já vale") ou provisória ("ainda testando").
    pub standing: Standing,
    /// §6.3: nasceu em sessão com conteúdo externo não-confiável → aviso suave (sinaliza, não bloqueia).
    pub untrusted_origin: bool,
}

/// A mentalidade de UM papel: o nome humanizado + as crenças a mostrar. O contrato do painel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleMentality {
    /// Nome do papel em pt-br do leigo (ex.: "Backend"). Já humanizado pela fiação.
    pub role_label: String,
    /// As crenças do papel, na ordem em que devem aparecer ([`ordered`]).
    pub beliefs: Vec<BeliefView>,
}

/// Ordena as crenças para a tela: **estabelecidas primeiro** (o que o papel "já sabe" — fatos calmos),
/// depois as **provisórias** (claramente marcadas, convidando validação). Estável dentro de cada grupo
/// (preserva a ordem de chegada da projeção). Pura — a regra de superfície vive no teste.
#[must_use]
pub fn ordered(mut beliefs: Vec<BeliefView>) -> Vec<BeliefView> {
    beliefs.sort_by_key(|b| standing_rank(b.standing));
    beliefs
}

/// Ordem de exibição (menor = mais acima). Determinística e exaustiva sobre [`Standing`].
#[must_use]
fn standing_rank(standing: Standing) -> u8 {
    match standing {
        Standing::Established => 0,
        Standing::Provisional => 1,
    }
}

// ───────────────────────────── Helpers de tipografia (token-puros) ─────────────────────────────

/// Rótulo de seção discreto (Plex, secundário) — o idioma de rótulo recorrente (mesmo de `goal_card`).
fn section_label(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.secondary))
        .child(gpui::text!(text))
}

/// Parágrafo de corpo (Plex, cor primária) — o statement da crença.
fn body_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.body)))
        .text_color(rgb(t.text.primary))
        .child(gpui::text!(text))
}

/// Texto discreto (Plex, apagado) — proveniência e avisos suaves.
fn muted_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .text_color(rgb(t.text.muted))
        .child(gpui::text!(text))
}

// ───────────────────────────── Fixture de tela (founder session) ─────────────────────────────

/// Uma [`RoleMentality`] de exemplo, em pt-br realista, para os testes e a pré-visualização do gate (h)
/// — uma estabelecida + uma provisória, para exercitar os dois estados + o aposentar. Só fixture: a
/// tela de produção renderiza a projeção VIVA (via a fiação), nunca este mock.
#[cfg(test)]
#[must_use]
pub fn mock_role_mentality() -> RoleMentality {
    RoleMentality {
        role_label: "Backend".to_string(),
        beliefs: vec![
            BeliefView {
                belief_id: "b-pnpm".to_string(),
                statement: "Neste projeto, instalo pacotes com pnpm — não com npm.".to_string(),
                provenance: "Aprendido em 10/06, quando você corrigiu uma instalação com npm."
                    .to_string(),
                standing: Standing::Established,
                untrusted_origin: false,
            },
            BeliefView {
                belief_id: "b-tabs".to_string(),
                statement: "Você prefere mensagens de commit curtas, em português.".to_string(),
                provenance: "Notei ontem, quando você reescreveu um commit meu.".to_string(),
                standing: Standing::Provisional,
                untrusted_origin: false,
            },
        ],
    }
}

// ───────────────────────────── O card de UMA crença (RenderOnce) ─────────────────────────────

/// **O card de uma crença.** builder + [`RenderOnce`], view-agnóstico (o handler de aposentar vem do
/// call-site via `cx.listener(...)`, como o resto do catálogo `ui/`). Construa com [`BeliefCard::new`].
#[derive(IntoElement)]
pub struct BeliefCard {
    belief: BeliefView,
    /// Gesto de aposentar (1 clique → `belief.retire`). `None` = botão inerte (pré-visualização).
    on_retire: Option<ClickHandler>,
}

impl BeliefCard {
    /// Novo card para uma [`BeliefView`].
    #[must_use]
    pub fn new(belief: BeliefView) -> Self {
        Self {
            belief,
            on_retire: None,
        }
    }

    /// Handler do "esquecer isso" (aposentar). Passe `cx.listener(|view, _ev, _w, cx| …)`.
    #[must_use]
    pub fn on_retire(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_retire = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for BeliefCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let status = standing_status(self.belief.standing);
        let provisional = self.belief.standing == Standing::Provisional;

        // Selo de situação ("já vale" / "ainda testando") — Badge anuncia troca via live-region.
        let badge = Badge::new(
            SharedString::from(format!("belief-standing-{}", self.belief.belief_id)),
            status.label,
            status.tone,
        );

        // Botão "esquecer isso" — ghost discreto (controle calmo do humano), só quando há handler.
        let retire = self.on_retire.map(|h| {
            Button::new(
                SharedString::from(format!("belief-retire-{}", self.belief.belief_id)),
                RETIRE_LABEL,
            )
            .ghost()
            .size(ButtonSize::Sm)
            .aria(RETIRE_ARIA)
            .on_click(h)
        });

        // Cabeçalho do card: selo de situação à esquerda, "esquecer isso" à direita.
        let header = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .gap(px(Space::Sm.px(&t)))
            .child(badge)
            .children(retire);

        // Proveniência: de onde a crença veio (humanizada), com rótulo discreto.
        let provenance = div()
            .flex()
            .flex_col()
            .gap(px(Space::Xs.px(&t)))
            .child(section_label(&t, PROVENANCE_LABEL))
            .child(muted_text(
                &t,
                SharedString::from(self.belief.provenance.clone()),
            ));

        // Dica de provisória (convite a confirmar/corrigir) — só quando provisória.
        let hint = provisional.then(|| muted_text(&t, SharedString::from(PROVISIONAL_HINT)));
        // Aviso de origem externa — só quando sinalizada (§6.3).
        let untrusted = self
            .belief
            .untrusted_origin
            .then(|| muted_text(&t, SharedString::from(UNTRUSTED_NOTE)));

        Panel::card()
            .gap(Space::Sm)
            .pad(Space::Md, Space::Md)
            .child(header)
            .child(body_text(
                &t,
                SharedString::from(self.belief.statement.clone()),
            ))
            .children(hint)
            .children(untrusted)
            .child(provenance)
    }
}

// ───────────────────────────── O painel do papel (RenderOnce) ─────────────────────────────

/// **O painel "Como o [papel] pensa".** Cabeçalho com o nome do papel (Fraunces — o "momento") +
/// a lista de crenças (estabelecidas primeiro), ou o estado vazio acolhedor. builder + [`RenderOnce`];
/// os cards já vêm com seus handlers do call-site (a view é dona do gesto). Construa com
/// [`MentalityPanel::new`] e alimente [`MentalityPanel::beliefs`].
#[derive(IntoElement)]
pub struct MentalityPanel {
    role_label: String,
    cards: Vec<BeliefCard>,
}

impl MentalityPanel {
    /// Novo painel para um papel (nome já humanizado em pt-br). Sem crenças = estado vazio.
    #[must_use]
    pub fn new(role_label: impl Into<String>) -> Self {
        Self {
            role_label: role_label.into(),
            cards: Vec::new(),
        }
    }

    /// Os cards de crença a exibir (já construídos com seus handlers, na ordem de [`ordered`]).
    #[must_use]
    pub fn beliefs(mut self, cards: Vec<BeliefCard>) -> Self {
        self.cards = cards;
        self
    }
}

impl RenderOnce for MentalityPanel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();

        // Título = o "momento" do fundador vendo a mente do time (Fraunces autorizado, como o hero
        // da Goal). NUNCA o id do papel — só o rótulo humanizado.
        let title = div()
            .font_family(t.typography.family.display)
            .text_size(px(f32::from(t.typography.size.title)))
            .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
            .text_color(rgb(t.text.bright))
            .child(gpui::text!(SharedString::from(role_title(
                &self.role_label
            ))));

        // Corpo: as crenças, ou o estado vazio acolhedor (nunca tela em branco).
        let body: gpui::AnyElement = if self.cards.is_empty() {
            muted_text(&t, SharedString::from(EMPTY_STATE)).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(Space::Md.px(&t)))
                .children(self.cards)
                .into_any_element()
        };

        Panel::surface()
            .gap(Space::Lg)
            .pad(Space::Lg, Space::Lg)
            .child(title)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada situação vira uma PALAVRA do leigo (decisão do fundador: "já vale" / "ainda testando"),
    /// com o tom certo (sucesso calmo / atenção suave) e sem jargão do ciclo de crença.
    #[test]
    fn standing_speaks_layman_per_founder_decision() {
        let est = standing_status(Standing::Established);
        assert_eq!(
            est.label, "já vale",
            "estabelecida = 'já vale' (promoção automática)"
        );
        assert_eq!(est.tone, BadgeTone::Success);

        let prov = standing_status(Standing::Provisional);
        assert_eq!(prov.label, "ainda testando");
        assert_eq!(
            prov.tone,
            BadgeTone::Warning,
            "provisória = atenção suave que convida correção, não erro"
        );

        // Nenhum rótulo vaza o vocabulário técnico do ciclo de crença (spec §3 / inv #6).
        for s in [Standing::Established, Standing::Provisional] {
            let low = standing_status(s).label.to_lowercase();
            for jargon in [
                "belief",
                "proposed",
                "established",
                "provisional",
                "reinforced",
                "challenged",
                "retired",
                "hash",
                "ttl",
                "verdict",
            ] {
                assert!(
                    !low.contains(jargon),
                    "vazou jargão: {:?}",
                    standing_status(s).label
                );
            }
        }
    }

    /// Toda a microcopy do painel é pt-br do leigo: zero jargão técnico em QUALQUER string de tela.
    #[test]
    fn all_microcopy_is_layman_without_jargon() {
        let surfaces = [
            PROVISIONAL_HINT,
            RETIRE_LABEL,
            RETIRE_ARIA,
            UNTRUSTED_NOTE,
            EMPTY_STATE,
            PROVENANCE_LABEL,
            &role_title("Backend"),
        ];
        for s in surfaces {
            assert!(!s.is_empty(), "toda copy tem conteúdo");
            let low = s.to_lowercase();
            for jargon in [
                "belief",
                "belief_id",
                "correctionobserved",
                "reviewverdict",
                "hash",
                "top-k",
                "ttl",
                "payload",
                "intent",
                "spawn",
                "{:?}",
                "untrusted",
                "provisional",
            ] {
                assert!(
                    !low.contains(jargon),
                    "copy vazou jargão ({jargon:?}): {s:?}"
                );
            }
        }
    }

    /// O título segue o template da spec ("Como o [papel] pensa") e usa o rótulo humanizado, nunca id.
    #[test]
    fn role_title_follows_spec_template() {
        assert_eq!(role_title("Backend"), "Como o Backend pensa");
        assert_eq!(
            role_title("Especialista em Telas"),
            "Como o Especialista em Telas pensa"
        );
    }

    /// O aposentar é descrito como gesto CALMO e reversível ("esquecer isso"), não como "deletar"
    /// destrutivo — a postura é controle tranquilo do humano (§6.7), não destruição.
    #[test]
    fn retire_label_is_calm_not_destructive() {
        let low = RETIRE_LABEL.to_lowercase();
        assert!(
            low.contains("esquecer"),
            "o gesto é 'esquecer', humano no comando"
        );
        for harsh in ["deletar", "apagar", "excluir", "remover", "destruir"] {
            assert!(
                !low.contains(harsh),
                "aposentar não é destruição crua: {RETIRE_LABEL:?}"
            );
        }
    }

    /// Ordenação: estabelecidas (o que o papel "já sabe") aparecem ANTES das provisórias; estável
    /// dentro de cada grupo (preserva a ordem da projeção).
    #[test]
    fn ordered_puts_established_first_stable_within_group() {
        let mk = |id: &str, standing: Standing| BeliefView {
            belief_id: id.to_string(),
            statement: "x".to_string(),
            provenance: "y".to_string(),
            standing,
            untrusted_origin: false,
        };
        let out = ordered(vec![
            mk("p1", Standing::Provisional),
            mk("e1", Standing::Established),
            mk("p2", Standing::Provisional),
            mk("e2", Standing::Established),
        ]);
        let ids: Vec<&str> = out.iter().map(|b| b.belief_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["e1", "e2", "p1", "p2"],
            "estabelecidas primeiro; ordem de chegada preservada em cada grupo"
        );
    }

    /// A fixture de tela é coerente para o gate (h): um papel com uma estabelecida + uma provisória,
    /// cada qual com proveniência humanizada e id (oculto) para o gesto de aposentar.
    #[test]
    fn mock_is_screen_ready() {
        let role = mock_role_mentality();
        assert!(!role.role_label.is_empty());
        assert_eq!(role.beliefs.len(), 2, "exercita os dois estados");
        assert!(
            role.beliefs
                .iter()
                .any(|b| b.standing == Standing::Established),
            "tem uma 'já vale'"
        );
        assert!(
            role.beliefs
                .iter()
                .any(|b| b.standing == Standing::Provisional),
            "tem uma 'ainda testando'"
        );
        for b in &role.beliefs {
            assert!(!b.belief_id.is_empty(), "id endereça o aposentar");
            assert!(!b.statement.is_empty() && !b.provenance.is_empty());
        }
    }
}
