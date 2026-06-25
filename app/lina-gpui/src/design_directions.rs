//! **F2-4-5 · Onboarding de Direções Visuais — a porta de entrada estética do leigo.**
//!
//! Quando o leigo abre um projeto, ele não sabe que "direção visual" dar. Esta é a **galeria
//! local** onde ele escolhe a personalidade visual do projeto em **1 clique**: a direção oficial
//! do Lina em destaque no topo + a shortlist curada (épico 38 §VIII.3). Cada direção é um
//! `DESIGN.md` no formato **OpenDesign**, vendorizado em `assets/design-directions/` e embutido no
//! binário ([`include_str!`]) — **zero rede por padrão** (inv #2, local-first). O ÚNICO acesso
//! externo é o link **opt-in** a `open-design.ai` ([`OPEN_DESIGN_URL`]): nada sai da máquina sem o
//! clique do usuário (e o que sai é o navegador DELE abrindo, não o app fazendo rede).
//!
//! **Decisão registrada (épico 38 §VIII.1, NÃO re-decidir):** a identidade do Lina é UMA — a fusão
//! T1+T3 "Instrumento de Estúdio com a temperatura do Ateliê". Por isso há **uma** direção da casa
//! no topo (não T1/T2/T4 soltos — T1-puro e T4 foram *rejeitados* na decisão).
//!
//! **Padrão da casa** (igual a `mentality_panel`/`gallery`/`ui`): a DECISÃO (lista, partição
//! Lina↔curadas, seleção, copy) mora em **funções/estado PUROS testáveis** — gpui não roda
//! headless, então o [`RenderOnce`] é casca fina sobre eles. Toda cor/medida sai de
//! [`crate::theme::active`] (zero literal — a catraca F2-1-5 nasce verde aqui), reusando o catálogo
//! [`crate::ui`] (`Panel`/`Badge`/`Button`). Fraunces ([`family.display`]) só no título da galeria
//! (o "momento" do leigo escolhendo a cara do projeto); o corpo é IBM Plex ([`family.ui`]).
//!
//! **A escolha NÃO decide autoridade:** a galeria é um SELETOR — clicar destaca e devolve a escolha
//! ao caller (`on_choose`, fechado no call-site, igual ao `on_retire` do `mentality_panel`). O que
//! fazer com a direção escolhida (persistir como preferência do Espaço) é costura da fiação
//! (`main.rs`), alinhada com o dono — o módulo não conhece o shape do core.
//!
//! [`family.display`]: crate::theme::FontFamilyTokens::display
//! [`family.ui`]: crate::theme::FontFamilyTokens::ui

// Módulo-biblioteca: a API pública é CONSUMIDA pela costura de `main.rs` (a fiar pelo Maestro —
// ver `.entrega-f2-4-design.md`) e, por ora, exercida pelos testes. Mesmo idioma de `gallery.rs`/
// `ui.rs`/`canvas.rs` (`allow(dead_code)` para o que ainda só os testes usam). Remover ao fiar.
#![allow(dead_code)]

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, FontWeight, IntoElement, SharedString, Window,
};

use crate::theme::{self, Theme};
use crate::ui::{Badge, BadgeTone, Button, ButtonSize, ClickHandler, Panel, Space};

// ───────────────────────────── Dados de uma direção (puro) ─────────────────────────────

/// De onde a direção vem — decide o destaque na galeria e o tom do selo. A da **casa** sobe ao
/// topo; as **curadas** são a shortlist vendorizada do OpenDesign (épico 38 §VIII.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A direção oficial do Lina (fusão T1+T3, §VIII.1). Em destaque, sempre primeiro.
    Lina,
    /// Vendorizada da curadoria OpenDesign (Apache-2.0; sem marca — `curadoria-opendesign.md` §II).
    Curated,
}

/// Uma direção visual escolhível — JÁ traduzida para a superfície do leigo. O `design_md` é o
/// `DESIGN.md` (formato OpenDesign) embutido: viaja COM a direção (o "ver detalhes" e o futuro
/// `lina-theme` o consomem), mas o card mostra só `name`/`blurb` em pt-br claro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesignDirection {
    /// Id estável (== nome do arquivo vendorizado, sem extensão). Endereça a escolha; nunca exibido cru.
    pub id: &'static str,
    /// Nome com caráter, em pt-br (não o slug técnico).
    pub name: &'static str,
    /// O que esta direção É e para quem cai bem — 1 frase específica, sem jargão.
    pub blurb: &'static str,
    /// Clima curto que orienta o leigo (o selo das curadas). Vazio para a da casa (tem selo próprio).
    pub climate: &'static str,
    /// Origem (destaque + tom do selo).
    pub origin: Origin,
    /// O `DESIGN.md` vendorizado (formato OpenDesign), embutido no binário — local-first.
    pub design_md: &'static str,
}

/// **Todas as direções, na ordem da galeria: a da casa primeiro, depois a shortlist curada.**
/// Cada `design_md` é o arquivo vendorizado em `assets/design-directions/` (embutido — zero rede).
#[must_use]
pub fn directions() -> &'static [DesignDirection] {
    &[
        // ── A cara do Lina (topo, destaque) — fusão T1+T3, §VIII.1 ──
        DesignDirection {
            id: "lina-estudio",
            name: "Instrumento de Estúdio",
            blurb: "A cara do Lina: cada cor significa um estado do trabalho, superfícies quentes \
                    e o foco sempre vivo. Vivo, honesto e acolhedor.",
            climate: "",
            origin: Origin::Lina,
            design_md: include_str!("../assets/design-directions/lina-estudio.md"),
        },
        // ── Clima "Terminal" — instrumento escuro, leitura de relance ──
        DesignDirection {
            id: "mission-control",
            name: "Centro de Comando",
            blurb: "Centro de comando escuro com telemetria âmbar e tipografia de máquina. \
                    Clareza funcional acima de tudo.",
            climate: "Terminal",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/mission-control.md"),
        },
        DesignDirection {
            id: "hud",
            name: "Painel de Voo",
            blurb: "Verde-fósforo sobre quase-preto, dados em caixa-alta, geometria angular. \
                    Leitura instantânea, zero ambiguidade.",
            climate: "Terminal",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/hud.md"),
        },
        DesignDirection {
            id: "trading-terminal",
            name: "Mesa de Operações",
            blurb: "Escuro e denso como uma mesa de operações: legível de relance a dois metros, \
                    com sinais de cor diretos.",
            climate: "Terminal",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/trading-terminal.md"),
        },
        // ── Clima "Editorial" — papel quente, cara de impresso ──
        DesignDirection {
            id: "atelier-zero",
            name: "Ateliê Editorial",
            blurb: "Revista de alto padrão: papel quente, títulos grandes e anotações finas. \
                    Tem cara de impresso artesanal.",
            climate: "Editorial",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/atelier-zero.md"),
        },
        DesignDirection {
            id: "kami",
            name: "Papel & Tinta",
            blurb: "Pergaminho quente, acento azul-tinta e serifa que conduz a leitura. \
                    Cara de documento impresso de qualidade.",
            climate: "Editorial",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/kami.md"),
        },
        DesignDirection {
            id: "warm-editorial",
            name: "Terracota no Papel",
            blurb: "Estética de revista com serifa: terracota sobre papel creme. \
                    Boa para textos longos e marcas com voz.",
            climate: "Editorial",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/warm-editorial.md"),
        },
        DesignDirection {
            id: "paper",
            name: "Papel",
            blurb: "Textura de papel impressa, poucas cores e tipografia limpa. \
                    Tátil e calmo, sem firula.",
            climate: "Editorial",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/paper.md"),
        },
        DesignDirection {
            id: "editorial",
            name: "Revista",
            blurb: "Layout de revista com serifa refinada e colunas bem estruturadas. \
                    Leitura elegante e organizada.",
            climate: "Editorial",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/editorial.md"),
        },
        // ── Clima "Minimalista" — sóbrio, preciso ──
        DesignDirection {
            id: "refined",
            name: "Refinado",
            blurb: "Minimalismo moderno com serifa elegante e cores sóbrias. \
                    Curado e sofisticado, sem excesso.",
            climate: "Minimalista",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/refined.md"),
        },
        DesignDirection {
            id: "sleek",
            name: "Enxuto",
            blurb: "Linhas limpas, paleta intencional e espaçamento consistente. \
                    Minimalista de verdade, nada sobra.",
            climate: "Minimalista",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/sleek.md"),
        },
        // ── Clima "Estrutura" — organização visível, respiro ──
        DesignDirection {
            id: "bento",
            name: "Bento",
            blurb: "Blocos modulares tipo cartão, hierarquia clara e contraste sutil. \
                    Organizado e fácil de varrer com os olhos.",
            climate: "Estrutura",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/bento.md"),
        },
        DesignDirection {
            id: "spacious",
            name: "Respiro",
            blurb: "Muito espaço em branco, padding generoso e grade. \
                    Interfaces limpas que respiram.",
            climate: "Estrutura",
            origin: Origin::Curated,
            design_md: include_str!("../assets/design-directions/spacious.md"),
        },
    ]
}

/// As direções da casa (Lina), na ordem da galeria — sempre no topo.
#[must_use]
pub fn lina_directions() -> Vec<DesignDirection> {
    directions()
        .iter()
        .copied()
        .filter(|d| d.origin == Origin::Lina)
        .collect()
}

/// As direções curadas (shortlist OpenDesign vendorizada), na ordem da galeria.
#[must_use]
pub fn curated_directions() -> Vec<DesignDirection> {
    directions()
        .iter()
        .copied()
        .filter(|d| d.origin == Origin::Curated)
        .collect()
}

// ───────────────────────────── Estado da galeria (puro, testável) ─────────────────────────────

/// **O estado da galeria — gpui-free, headless.** Lembra QUAL direção está escolhida (índice em
/// [`directions`]); a navegação por teclado e a seleção são puras (o render lê deste estado). Mesmo
/// idioma do `CreateSpaceModal` (`gallery.rs`): seleção 1-clique, índice resolvido sob demanda.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DesignGallery {
    /// Índice da direção escolhida em [`directions`], ou `None` (nenhuma escolhida ainda).
    selected: Option<usize>,
}

impl DesignGallery {
    /// Galeria nova — nada escolhido (o leigo decide; nunca pré-seleciona por ele).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Quantas direções existem (Lina + curadas).
    #[must_use]
    pub fn count(&self) -> usize {
        directions().len()
    }

    /// Escolhe a direção `idx` (1 clique). Índice fora de faixa é ignorado (defensivo).
    pub fn select(&mut self, idx: usize) {
        if idx < self.count() {
            self.selected = Some(idx);
        }
    }

    /// `true` se a direção `idx` está escolhida (para o render destacar o card sem reflow).
    #[must_use]
    pub fn is_selected(&self, idx: usize) -> bool {
        self.selected == Some(idx)
    }

    /// A direção escolhida (resolvida do índice), ou `None` se nada foi escolhido.
    #[must_use]
    pub fn selected_direction(&self) -> Option<DesignDirection> {
        self.selected.and_then(|i| directions().get(i).copied())
    }

    /// Próxima direção (→/↓) — circular. Sem seleção, começa na primeira (a da casa).
    pub fn select_next(&mut self) {
        let n = self.count();
        self.selected = Some(self.selected.map_or(0, |i| (i + 1) % n));
    }

    /// Direção anterior (←/↑) — circular. Sem seleção, começa na última.
    pub fn select_prev(&mut self) {
        let n = self.count();
        self.selected = Some(self.selected.map_or(n - 1, |i| (i + n - 1) % n));
    }
}

// ───────────────────────────── Microcopy (pt-br, zero jargão) ─────────────────────────────

/// Título da galeria — o "momento" do leigo escolhendo a cara do projeto (Fraunces autorizado).
pub const GALLERY_TITLE: &str = "Escolha a cara do seu projeto";

/// Subtítulo: explica a escolha em 1 frase honesta, sem prometer mágica.
pub const GALLERY_SUBTITLE: &str =
    "Cada direção é um jeito de o projeto se parecer. Dá pra trocar depois — escolha a que combina.";

/// Rótulo da seção em destaque (a direção da casa).
pub const LINA_SECTION_LABEL: &str = "A cara do Lina";

/// Selo da direção da casa (no card em destaque).
pub const LINA_BADGE: &str = "feita pela casa";

/// Rótulo da seção das direções curadas.
pub const CURATED_SECTION_LABEL: &str = "Outras direções";

/// Rótulo do gesto de escolher (no card). 1 clique escolhe e destaca.
pub const CHOOSE_LABEL: &str = "Usar esta";

/// Nome acessível do gesto de escolher (o card já diz; o leitor de tela ganha o objeto).
#[must_use]
pub fn choose_aria(name: &str) -> String {
    format!("Usar a direção {name}")
}

/// Aviso honesto do único acesso externo: abre o navegador SÓ no clique (nada sai sozinho).
pub const OPEN_DESIGN_HINT: &str =
    "Quer ver ainda mais direções? Isto abre um site no seu navegador — nada do seu projeto sai daqui.";

/// CTA do link opt-in. Diz o que acontece ao clicar (abre o navegador), na voz da casa.
pub const OPEN_DESIGN_CTA: &str = "Explorar mais em open-design.ai ↗";

/// O ÚNICO endereço externo desta tela — aberto SÓ pelo clique do usuário (`cx.open_url`), opt-in.
pub const OPEN_DESIGN_URL: &str = "https://open-design.ai";

// ───────────────────────────── Helpers de tipografia (token-puros) ─────────────────────────────

/// Título-momento da galeria (Fraunces, destaque máximo) — mesmo idioma do hero de `mentality_panel`.
fn hero_title(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.display)
        .text_size(px(f32::from(t.typography.size.title)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.bright))
        .child(gpui::text!(text))
}

/// Rótulo de seção discreto (Plex, secundário) — o idioma recorrente da casa.
fn section_label(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.secondary))
        .child(gpui::text!(text))
}

/// Nome da direção no card (subtítulo, peso forte) — Plex, cor de destaque.
fn card_name(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.subtitle)))
        .font_weight(FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.bright))
        .child(gpui::text!(text))
}

/// Parágrafo de corpo (Plex, cor primária) — o blurb da direção.
fn body_text(t: &Theme, text: SharedString) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.body)))
        .text_color(rgb(t.text.primary))
        .child(gpui::text!(text))
}

/// Texto discreto (Plex, apagado) — subtítulo e o aviso do link opt-in.
fn muted_text(t: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .font_family(t.typography.family.ui)
        .text_size(px(f32::from(t.typography.size.small)))
        .text_color(rgb(t.text.muted))
        .child(gpui::text!(text))
}

// ───────────────────────────── O card de UMA direção (RenderOnce) ─────────────────────────────

/// **O card de uma direção.** builder + [`RenderOnce`], view-agnóstico — o `on_choose` vem do
/// call-site (`cx.listener(...)`, igual ao resto do catálogo). O card inteiro é clicável: 1 clique
/// escolhe E destaca (geometria uniforme — selecionar não faz reflow, via `Panel::card`).
#[derive(IntoElement)]
pub struct DirectionCard {
    direction: DesignDirection,
    selected: bool,
    /// Gesto de escolher (1 clique). `None` = card inerte (pré-visualização).
    on_choose: Option<ClickHandler>,
}

impl DirectionCard {
    /// Novo card para uma direção, com seu estado de seleção atual.
    #[must_use]
    pub fn new(direction: DesignDirection, selected: bool) -> Self {
        Self {
            direction,
            selected,
            on_choose: None,
        }
    }

    /// Handler do "usar esta" (escolher). Passe `cx.listener(|view, _ev, _w, cx| …)`.
    #[must_use]
    pub fn on_choose(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_choose = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for DirectionCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let d = self.direction;

        // Selo: a da casa veste o acento ("feita pela casa"); as curadas mostram o clima, neutro.
        let (badge_label, badge_tone) = match d.origin {
            Origin::Lina => (LINA_BADGE, BadgeTone::Info),
            Origin::Curated => (d.climate, BadgeTone::Neutral),
        };
        let badge = Badge::new(
            SharedString::from(format!("direction-badge-{}", d.id)),
            badge_label,
            badge_tone,
        );

        // Cabeçalho: nome à esquerda, selo à direita.
        let header = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .gap(px(Space::Sm.px(&t)))
            .child(card_name(&t, SharedString::from(d.name)))
            .child(badge);

        // "Usar esta" — confirma a escolha; só quando há handler (na preview fica fora).
        let choose = self.on_choose.map(|h| {
            Button::new(
                SharedString::from(format!("direction-choose-{}", d.id)),
                CHOOSE_LABEL,
            )
            .confirm()
            .size(ButtonSize::Sm)
            .selected(self.selected)
            .aria(choose_aria(d.name))
            .on_click(h)
        });

        Panel::card()
            .id(SharedString::from(format!("direction-card-{}", d.id)))
            .selected(self.selected)
            .gap(Space::Sm)
            .pad(Space::Md, Space::Md)
            .aria(SharedString::from(d.name))
            .child(header)
            .child(body_text(&t, SharedString::from(d.blurb)))
            .children(choose)
    }
}

// ───────────────────────────── A galeria (RenderOnce) ─────────────────────────────

/// **A galeria de Direções Visuais.** Título-momento + a direção da casa em destaque + a shortlist
/// curada + o rodapé com o link opt-in (sinalizado). builder + [`RenderOnce`]; os cards já vêm com
/// seus handlers do call-site (a view é dona do gesto). Construa com [`DirectionsGallery::new`].
#[derive(IntoElement)]
pub struct DirectionsGallery {
    lina_cards: Vec<DirectionCard>,
    curated_cards: Vec<DirectionCard>,
    /// Abrir o link opt-in (`cx.open_url(OPEN_DESIGN_URL)`). `None` = link inerte (preview).
    on_open_external: Option<ClickHandler>,
}

impl DirectionsGallery {
    /// Galeria nova (sem cards ainda — alimente com [`Self::lina`]/[`Self::curated`]).
    #[must_use]
    pub fn new() -> Self {
        Self {
            lina_cards: Vec::new(),
            curated_cards: Vec::new(),
            on_open_external: None,
        }
    }

    /// Os cards da casa (em destaque, no topo).
    #[must_use]
    pub fn lina(mut self, cards: Vec<DirectionCard>) -> Self {
        self.lina_cards = cards;
        self
    }

    /// Os cards das direções curadas.
    #[must_use]
    pub fn curated(mut self, cards: Vec<DirectionCard>) -> Self {
        self.curated_cards = cards;
        self
    }

    /// Handler do link opt-in para `open-design.ai`. Passe `cx.listener(|_, _, _, cx|
    /// cx.open_url(OPEN_DESIGN_URL))` — o navegador do usuário abre SÓ no clique (local-first).
    #[must_use]
    pub fn on_open_external(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_external = Some(Box::new(handler));
        self
    }
}

impl Default for DirectionsGallery {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for DirectionsGallery {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();

        // Cabeçalho: título-momento (Fraunces) + subtítulo honesto.
        let head = div()
            .flex()
            .flex_col()
            .gap(px(Space::Xs.px(&t)))
            .child(hero_title(&t, GALLERY_TITLE))
            .child(muted_text(&t, GALLERY_SUBTITLE));

        // Seção da casa (destaque, topo).
        let lina_section = div()
            .flex()
            .flex_col()
            .gap(px(Space::Sm.px(&t)))
            .child(section_label(&t, LINA_SECTION_LABEL))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Space::Md.px(&t)))
                    .children(self.lina_cards),
            );

        // Seção das curadas.
        let curated_section = div()
            .flex()
            .flex_col()
            .gap(px(Space::Sm.px(&t)))
            .child(section_label(&t, CURATED_SECTION_LABEL))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Space::Md.px(&t)))
                    .children(self.curated_cards),
            );

        // Rodapé: o único acesso externo, claramente opt-in e sinalizado.
        let external = self.on_open_external.map(|h| {
            Button::new("design-directions-open-external", OPEN_DESIGN_CTA)
                .ghost()
                .size(ButtonSize::Sm)
                .aria(OPEN_DESIGN_CTA)
                .on_click(h)
        });
        let footer = div()
            .flex()
            .flex_col()
            .gap(px(Space::Xs.px(&t)))
            .child(muted_text(&t, OPEN_DESIGN_HINT))
            .children(external);

        Panel::surface()
            .gap(Space::Lg)
            .pad(Space::Lg, Space::Lg)
            .child(head)
            .child(lina_section)
            .child(curated_section)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A direção da casa (Lina) é exatamente UMA e vem PRIMEIRO — a decisão §VIII.1 é uma fusão
    /// única, não T1/T2/T4 soltos. As demais são todas curadas.
    #[test]
    fn lina_direction_is_single_and_first() {
        let all = directions();
        assert_eq!(all[0].origin, Origin::Lina, "a da casa abre a galeria");
        assert_eq!(all[0].id, "lina-estudio");
        assert_eq!(
            lina_directions().len(),
            1,
            "uma identidade da casa (a fusão T1+T3), não vários territórios soltos"
        );
        assert!(
            all[1..].iter().all(|d| d.origin == Origin::Curated),
            "depois da casa, tudo é curado"
        );
    }

    /// A shortlist curada é exatamente a do épico §VIII.3 (`curadoria-opendesign.md` §II): os 12
    /// sistemas SEM marca — e exibidos **agrupados por clima** (direções parecidas juntas, decisão
    /// de UX para o leigo; a distinção núcleo/complemento da curadoria é prioridade interna, não
    /// ordem de tela). Provo o que importa: o conjunto dos 12 + cada clima num bloco contíguo, na
    /// ordem dos blocos — robusto a reordenações futuras de UX, mas cai se faltar/sobrar uma direção.
    #[test]
    fn curated_shortlist_is_the_twelve_grouped_by_climate() {
        use std::collections::BTreeSet;
        let curated = curated_directions();

        // (a) Exatamente os 12 da shortlist, nenhum a mais ou a menos (conjunto, ignora ordem).
        let ids: BTreeSet<&str> = curated.iter().map(|d| d.id).collect();
        let expected: BTreeSet<&str> = [
            "mission-control",
            "hud",
            "trading-terminal",
            "atelier-zero",
            "kami",
            "warm-editorial",
            "refined",
            "sleek",
            "paper",
            "editorial",
            "bento",
            "spacious",
        ]
        .into_iter()
        .collect();
        assert_eq!(ids, expected, "exatamente os 12 sem marca da shortlist §VIII.3");

        // (b) Agrupadas por clima: a sequência de climas distintos na ordem, sem repetir um clima já
        // fechado (cada clima é um bloco contíguo). `blocks` sem duplicata ⇒ contíguo.
        let mut blocks: Vec<&str> = Vec::new();
        for &c in curated.iter().map(|d| &d.climate) {
            if blocks.last() != Some(&c) {
                blocks.push(c);
            }
        }
        assert_eq!(
            blocks,
            vec!["Terminal", "Editorial", "Minimalista", "Estrutura"],
            "climas em blocos contíguos, na ordem de exibição"
        );
        let distinct: BTreeSet<&str> = blocks.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            blocks.len(),
            "cada clima aparece num único bloco (contíguo), não espalhado"
        );
    }

    /// Cada direção carrega seu `DESIGN.md` vendorizado (formato OpenDesign), embutido e não-vazio —
    /// prova que o picker é LOCAL (zero rede): o conteúdo da direção já está no binário.
    #[test]
    fn every_direction_embeds_its_opendesign_doc() {
        for d in directions() {
            assert!(!d.id.is_empty() && !d.name.is_empty() && !d.blurb.is_empty());
            assert!(
                d.design_md.trim_start().starts_with("# "),
                "{}: DESIGN.md no formato OpenDesign (abre com título)",
                d.id
            );
            assert!(
                d.design_md.contains("> Category:"),
                "{}: o cabeçalho OpenDesign declara a categoria",
                d.id
            );
        }
    }

    /// Local-first (inv #2): o ÚNICO endereço externo da tela é `open-design.ai`, e ele é opt-in
    /// (aberto só pelo clique). Nenhuma outra URL vaza na copy de tela.
    #[test]
    fn only_external_address_is_the_optin_link() {
        assert_eq!(OPEN_DESIGN_URL, "https://open-design.ai");
        let surfaces = [
            GALLERY_TITLE,
            GALLERY_SUBTITLE,
            LINA_SECTION_LABEL,
            LINA_BADGE,
            CURATED_SECTION_LABEL,
            CHOOSE_LABEL,
            OPEN_DESIGN_HINT,
            OPEN_DESIGN_CTA,
        ];
        for s in surfaces {
            let low = s.to_lowercase();
            // Nenhum protocolo de rede embutido na copy (a URL mora só na const, usada no clique).
            for url_marker in ["http://", "https://", "www.", ".com", "ftp:"] {
                assert!(
                    !low.contains(url_marker),
                    "copy de tela não embute endereço de rede ({url_marker:?}): {s:?}"
                );
            }
        }
        // O aviso do opt-in deixa claro que o navegador abre só no clique (nada sai sozinho).
        assert!(OPEN_DESIGN_HINT.contains("navegador"));
    }

    /// Toda a microcopy é pt-br do leigo: zero jargão técnico de design/implementação em QUALQUER
    /// string de tela (molde `mentality_panel`). "Direção visual", nunca "design token".
    #[test]
    fn all_microcopy_is_layman_without_jargon() {
        let fixed = [
            GALLERY_TITLE,
            GALLERY_SUBTITLE,
            LINA_SECTION_LABEL,
            LINA_BADGE,
            CURATED_SECTION_LABEL,
            CHOOSE_LABEL,
            OPEN_DESIGN_HINT,
            OPEN_DESIGN_CTA,
        ];
        // Inclui também os nomes/blurbs/climas das direções + o aria dinâmico (é tudo texto de tela).
        let mut all: Vec<String> = fixed.iter().map(|s| (*s).to_string()).collect();
        all.push(choose_aria("Centro de Comando"));
        for d in directions() {
            all.push(d.name.to_string());
            all.push(d.blurb.to_string());
            all.push(d.climate.to_string());
        }
        for s in &all {
            let low = s.to_lowercase();
            for jargon in [
                "design token",
                "frontmatter",
                "markdown",
                "css",
                "hex",
                "rgb(",
                "fontweight",
                "px(",
                "serde",
                "viewport",
                "{:?}",
            ] {
                assert!(
                    !low.contains(jargon),
                    "copy vazou jargão ({jargon:?}): {s:?}"
                );
            }
        }
    }

    /// O estado da galeria: seleção 1-clique, índice resolvido para a direção, defensivo a índice
    /// inválido, e navegação circular (←→/↑↓). Não-vacuoso: sem o estado, cada asserção cai.
    #[test]
    fn gallery_state_selects_resolves_and_navigates() {
        let mut g = DesignGallery::new();
        assert!(
            g.selected_direction().is_none(),
            "nasce sem escolha — o leigo decide"
        );
        assert_eq!(g.count(), directions().len());

        // 1 clique escolhe e resolve para a direção certa.
        g.select(0);
        assert!(g.is_selected(0));
        assert_eq!(g.selected_direction().map(|d| d.id), Some("lina-estudio"));

        // Índice fora de faixa é ignorado (não muda a escolha).
        g.select(9999);
        assert!(g.is_selected(0), "índice inválido não rouba a seleção");

        // Navegação circular.
        g.select(g.count() - 1);
        g.select_next();
        assert!(g.is_selected(0), "wrap para frente volta à primeira");
        g.select_prev();
        assert!(g.is_selected(g.count() - 1), "wrap para trás vai à última");
    }

    /// Pré-visualização do gate de tela: os cards e a galeria montam sem handler (inertes) — prova
    /// que o render compila contra o contrato, mesmo antes da fiação do `main.rs`.
    #[test]
    fn cards_and_gallery_build_inert_for_preview() {
        let lina: Vec<DirectionCard> = lina_directions()
            .into_iter()
            .enumerate()
            .map(|(i, d)| DirectionCard::new(d, i == 0))
            .collect();
        let curated: Vec<DirectionCard> = curated_directions()
            .into_iter()
            .map(|d| DirectionCard::new(d, false))
            .collect();
        assert_eq!(lina.len(), 1);
        assert_eq!(curated.len(), 12);
        let _gallery = DirectionsGallery::new().lina(lina).curated(curated);
    }
}
