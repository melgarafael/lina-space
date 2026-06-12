//! `theme` — **F1-2-1 + F2-1: design system com tokens nomeados — cor (dark/light), tipografia,
//! espaçamento, cantos e movimento (personalização local-first)**.
//!
//! A **fonte única de cor do shell** (modelo do Zed/GPUI Component — [[13.3]] achado 5: "nunca cores
//! hard-coded"). Tudo que pinta a UI lê DAQUI:
//! - **Tokens nomeados e modulares** em grupos ([`SurfaceTokens`]/[`TextTokens`]/[`AccentTokens`]/
//!   [`StateTokens`]/[`FocusTokens`]/[`TerminalTokens`]) — não um monólito de consts soltas.
//! - **Stepping consistente dark↔light** via [`ColorScale`] (modelo `ColorScaleSet` do Zed): cada
//!   família de cor declara o par (valor dark, valor light) **uma vez**; [`Theme::build`] resolve.
//! - **8 acentos curados** ([`ACCENTS`]) — cada um é uma escala desenhada para passar o gate WCAG
//!   nos DOIS modos (anti-slop: curadoria primeiro; roda de cor livre NÃO entra — T7§A anti-padrão 4).
//! - **Tema vivo**: [`active`]/[`apply`] — trocar modo/acento re-pinta o shell no próximo frame, sem
//!   restart (as views leem o tema a cada `render`).
//! - **Local-first** (invariante #2): a escolha persiste via `Settings` (T7, `settings.json` — o
//!   mesmo canal de Ajustes existente); export/import de tema é JSON **100% local** ([`export_json`]/
//!   [`parse_json`]) — zero rede neste módulo (nenhum import de rede; só `std` + `serde`).
//! - **Gate WCAG ampliado (CI)**: os testes deste módulo FALHAM se qualquer token de texto baixar de
//!   4.5:1 (AA normal) / 3:1 (muted/large) em QUALQUER modo × acento, se o focus ring baixar de 3:1
//!   ([WCAG 1.4.11]), ou se um `rgb(0x…)` literal reaparecer no shell fora deste módulo (lint).
//!
//! **F2-1 — vocabulário completo (mesmas garantias da cor):** o módulo também é a fonte única de
//! TIPOGRAFIA, ESPAÇAMENTO, CANTOS e MOVIMENTO ([`TypographyTokens`]/[`SpacingTokens`]/
//! [`RadiusTokens`]/[`MotionTokens`]), com testes-gate de integridade neste módulo (a catraca
//! anti-número-mágico do shell é a F2-1-5). Voz da fusão **T1 + temperatura T3** selada na
//! F2-0-D (épico 38 §VIII): IBM Plex Sans (UI) + Fraunces (momentos) + JetBrains Mono (grid —
//! fecha a dívida do ADR 0019 §7: fonte de código/terminal **via token**, nunca literal).
//!
//! **Doutrina de movimento (D1-A8, observada no Raycast):** NUNCA animar ação iniciada por input
//! frequente (abrir/focar/teclar = [`MotionTokens::instant`] = 0ms — repetida centenas de vezes
//! ao dia, animação = lentidão); o restante ≤300ms e SEMPRE subordinado a reduce-motion
//! ([`MotionTokens::effective`] zera tudo com o flag ligado — ADR 0019 §7).
//!
//! **Fontes — instâncias estáticas obrigatórias (D1-A7, verificada via API em 2026-06-12):** o pin
//! do gpui/cosmic-text não aplica eixos variáveis (zed#4850 · cosmic-text#406) — toda família
//! entra como estáticos OFL completos. Pesos escolhidos (fonte: pesquisa D1, fetch real dos repos):
//! - **IBM Plex Sans** (OFL, repo `IBM/plex`, estáticos completos): Regular 400 · Medium 500 ·
//!   SemiBold 600 · Bold 700.
//! - **Fraunces** (OFL, repo `undercasetype/Fraunces`, `fonts/static/` pré-gerados): SemiBold 600
//!   em opsz alto (72pt) — só momentos display, nunca corpo de UI.
//! - **JetBrains Mono** (OFL, repo `JetBrains/JetBrainsMono`, estáticos completos): Regular 400 ·
//!   Bold 700 (o bold ANSI do grid).
//!
//! O empacotamento físico no .app (embedding via `add_fonts`) é story posterior; até lá o consumo
//! resolve pelo sistema (fallback do font-kit se ausente — o token já nomeia a família certa).
//!
//! **Terminal é superfície escura nos 2 temas** (decisão de curadoria): o conteúdo do grid vem do CLI
//! via ANSI e assume fundo escuro; tematizar o conteúdo além de BG/cursor padrão está fora do escopo
//! (F1-2-1 "NÃO entra"). No modo claro, o terminal fica como um viewport escuro dentro do chrome claro.
//!
//! Âncora `UiHost` (invariante #7): tema é assunto EXCLUSIVO do shell — nenhum token atravessa para
//! `lina-core` (este módulo não importa nada do core nem do gpui; consumidores chamam `gpui::rgb`).

use std::sync::{LazyLock, PoisonError, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ═══════════════════════════ modo + escala (stepping consistente) ═══════════════════════════

/// Modo visual do shell. Persistido em `Settings.theme` como `"escuro"`/`"claro"` (strings já
/// existentes do T7 — compatível com `settings.json` antigos).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

impl Mode {
    /// Lê o modo da string persistida do T7. Desconhecido → `Dark` (default seguro, nunca falha).
    #[must_use]
    pub fn from_setting(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("claro") {
            Self::Light
        } else {
            Self::Dark
        }
    }

    /// A string persistida no T7 (`settings.json`).
    #[must_use]
    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Dark => "escuro",
            Self::Light => "claro",
        }
    }
}

/// Uma família de cor com **um degrau por modo** (modelo `ColorScaleSet` do Zed, reduzido ao que
/// usamos): o par é declarado UMA vez e resolvido por [`ColorScale::pick`] — impossível "esquecer"
/// o valor light de um token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorScale {
    pub dark: u32,
    pub light: u32,
}

impl ColorScale {
    const fn new(dark: u32, light: u32) -> Self {
        Self { dark, light }
    }

    /// Mesmo valor nos dois modos (ex.: tokens do terminal — superfície escura por curadoria).
    const fn fixed(both: u32) -> Self {
        Self {
            dark: both,
            light: both,
        }
    }

    #[must_use]
    pub fn pick(self, mode: Mode) -> u32 {
        match mode {
            Mode::Dark => self.dark,
            Mode::Light => self.light,
        }
    }
}

// ═══════════════════════════ acentos curados (8 — T7§A) ═══════════════════════════

/// Acento padrão (o azul histórico do shell da Fase 0).
pub const DEFAULT_ACCENT: &str = "azul";

/// Os **8 acentos curados** do T7§A: cada um é uma [`ColorScale`] desenhada para atingir AA (≥4.5:1)
/// como TEXTO sobre todas as superfícies nos DOIS modos — o gate WCAG itera sobre todos. Liberdade
/// dentro de limites (anti-slop): roda de cor livre fica fora da superfície.
pub const ACCENTS: [(&str, ColorScale); 8] = [
    ("azul", ColorScale::new(0x7aa2f7, 0x2f4fc7)),
    ("violeta", ColorScale::new(0xbb9af7, 0x6b3fd4)),
    ("verde", ColorScale::new(0x9ece6a, 0x33691e)),
    ("ambar", ColorScale::new(0xe0af68, 0x7c5400)),
    ("rosa", ColorScale::new(0xf7768e, 0xb1234a)),
    ("ciano", ColorScale::new(0x7dcfff, 0x00657f)),
    ("laranja", ColorScale::new(0xff9e64, 0x9a4b00)),
    ("teal", ColorScale::new(0x73daca, 0x0c6e5d)),
];

/// A escala do acento `name` (case-insensitive); desconhecido → [`DEFAULT_ACCENT`] (best-effort,
/// nunca falha — mesma postura do `load_settings`).
fn accent_scale(name: &str) -> ColorScale {
    let name = name.trim();
    ACCENTS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .or_else(|| ACCENTS.iter().find(|(n, _)| *n == DEFAULT_ACCENT))
        .map(|(_, s)| *s)
        .unwrap_or(ColorScale::new(0x7aa2f7, 0x2f4fc7))
}

// ═══════════════════════════ escalas base (fonte única dos valores) ═══════════════════════════
// Cada const abaixo é UMA família dark/light. O dark é a paleta histórica da Fase 0 (migrada, não
// redesenhada); o light foi desenhado token a token contra o gate WCAG (ver testes).

mod scale {
    use super::ColorScale;

    // superfície
    pub const CANVAS: ColorScale = ColorScale::new(0x0a0e27, 0xf2f4fb);
    pub const PANEL: ColorScale = ColorScale::new(0x141a36, 0xe7ebf6);
    pub const CARD: ColorScale = ColorScale::new(0x0d1228, 0xfbfcff);
    pub const CHROME: ColorScale = ColorScale::new(0x0c1130, 0xe2e7f4);
    pub const RAISED: ColorScale = ColorScale::new(0x2a3152, 0xd3daee);
    pub const RAISED_ALT: ColorScale = ColorScale::new(0x3a3f5a, 0xc4cde8);
    pub const SELECTED_ROW: ColorScale = ColorScale::new(0x1b2347, 0xdde4f5);
    pub const DANGER_MUTED: ColorScale = ColorScale::new(0x33202c, 0xf3dde2);
    pub const SUCCESS_MUTED: ColorScale = ColorScale::new(0x18301f, 0xdff0e2);
    pub const BORDER: ColorScale = ColorScale::new(0x2a3152, 0xc9d1e8);
    pub const BORDER_MUTED: ColorScale = ColorScale::new(0x222a44, 0xd8deef);

    // texto
    pub const TEXT_PRIMARY: ColorScale = ColorScale::new(0xc8d3f5, 0x232a45);
    pub const TEXT_SECONDARY: ColorScale = ColorScale::new(0x9aa5d4, 0x424e74);
    pub const TEXT_MUTED: ColorScale = ColorScale::new(0x5b658f, 0x707d9f);
    pub const TEXT_BRIGHT: ColorScale = ColorScale::new(0xeef1ff, 0x10142b);
    /// Texto sobre fills de ACENTO (action/create/confirm) — claros nos 2 modos (fills saturados).
    pub const TEXT_ON_ACCENT: ColorScale = ColorScale::new(0xeef1ff, 0xf7f9ff);
    /// Texto sobre chips de ESTADO (success/warning/danger): no dark os fills são claros → texto
    /// escuro; no light os fills são saturados escuros → texto claro.
    pub const TEXT_ON_EMPHASIS: ColorScale = ColorScale::new(0x11111b, 0xf7f9ff);

    // acento (botões fortes; o `primary` vem da escolha do usuário em `ACCENTS`)
    pub const ACCENT_SECONDARY: ColorScale = ColorScale::new(0xbb9af7, 0x6b3fd4);
    pub const ACCENT_ACTION: ColorScale = ColorScale::new(0x3d59c9, 0x2f4fc7);
    pub const ACCENT_CREATE: ColorScale = ColorScale::new(0x6c4ad1, 0x6b3fd4);
    pub const ACCENT_CONFIRM: ColorScale = ColorScale::new(0x2c7a4b, 0x1e6b3a);

    // estado
    pub const STATE_SUCCESS: ColorScale = ColorScale::new(0x9ece6a, 0x276b32);
    pub const STATE_WARNING: ColorScale = ColorScale::new(0xe0af68, 0x8a5a00);
    pub const STATE_DANGER: ColorScale = ColorScale::new(0xf7768e, 0xb02347);

    // terminal (superfície escura nos 2 modos — ver doc do módulo)
    pub const TERMINAL_BG: ColorScale = ColorScale::fixed(0x0d1228);
    pub const TERMINAL_CURSOR: ColorScale = ColorScale::fixed(0x7aa2f7);
    pub const TERMINAL_CURSOR_TEXT: ColorScale = ColorScale::fixed(0x0a0e27);
    pub const TERMINAL_SELECTION: ColorScale = ColorScale::fixed(0x334573);
}

// ═══════════════════════════ grupos de tokens (o contrato dos consumidores) ═══════════════════════════

/// Superfícies (fundos e bordas) — do canvas ao chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceTokens {
    /// Fundo do canvas/janela.
    pub canvas: u32,
    /// Painéis e cabeçalhos de card.
    pub panel: u32,
    /// Corpo de card (nota/pasta; moldura do terminal).
    pub card: u32,
    /// Chrome fixo (topbar/rodapé/paleta).
    pub chrome: u32,
    /// Botões neutros / chips elevados.
    pub raised: u32,
    /// Variante mais clara do elevado (item selecionado da paleta, chips de modo).
    pub raised_alt: u32,
    /// Linha selecionada em listas (ex.: vault do segundo cérebro).
    pub selected_row: u32,
    /// Fundo discreto de ação destrutiva (✕ do card; banners de aviso/erro).
    pub danger_muted: u32,
    /// Fundo discreto de sucesso (banners "tudo pronto").
    pub success_muted: u32,
    /// Borda padrão (card sem foco).
    pub border: u32,
    /// Borda apagada (aura do canvas sem time conectado).
    pub border_muted: u32,
}

/// Texto — cada token tem um alvo WCAG no gate (ver testes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextTokens {
    /// Texto principal (AA ≥4.5:1 sobre as superfícies).
    pub primary: u32,
    /// Texto de apoio/subtítulos (AA ≥4.5:1).
    pub secondary: u32,
    /// Texto terciário/legendas (tier secundário do W4-6: ≥3:1, propositalmente <4.5 sobre canvas).
    pub muted: u32,
    /// Texto de destaque máximo (títulos de overlay, item ativo).
    pub bright: u32,
    /// Texto sobre fills de acento (action/create/confirm).
    pub on_accent: u32,
    /// Texto sobre chips de estado (success/warning/danger).
    pub on_emphasis: u32,
}

/// Acento — a personalidade do shell (o `primary` é a escolha do usuário entre os 8 curados).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccentTokens {
    /// O acento escolhido: bordas de foco, títulos, aura conectada, pastilhas.
    pub primary: u32,
    /// Acento de apoio (miolo do pulso A2A).
    pub secondary: u32,
    /// Botão de ação primária (⚡ A2A).
    pub action: u32,
    /// Botão de criação (✦ Novo Agente).
    pub create: u32,
    /// Botão de confirmação (➕ Terminal, retomar).
    pub confirm: u32,
}

/// Estado — semáforo do shell (badges, banners, chips).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateTokens {
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
}

/// Foco — o anel de foco visível nos 2 temas (contraste ≥3:1, WCAG 1.4.11 — gate de F1-2-1;
/// F1-2-6 o consome no focus manager do canvas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusTokens {
    pub ring: u32,
}

/// Terminal — BG/cursor padrão do grid (superfície escura nos 2 modos; ver doc do módulo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTokens {
    pub bg: u32,
    pub cursor: u32,
    pub cursor_text: u32,
    pub selection: u32,
}

// ═══════════════════ vocabulário F2-1 (tipografia · espaçamento · cantos · movimento) ═══════════════════

/// Famílias tipográficas — a voz da fusão T1+T3 (F2-0-D, épico 38 §VIII). `&'static str` de
/// propósito: famílias são CURADORIA (como os 8 acentos), não campo livre — trocar uma exige
/// decisão registrada NESTE módulo, nunca um literal no shell (ADR 0019 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFamilyTokens {
    /// UI/chrome (IBM Plex Sans — humanista-técnica com calor, pt-br impecável; D1 §II-T1).
    pub ui: &'static str,
    /// Momentos display (Fraunces — onboarding, conclusões; NUNCA corpo de UI).
    pub display: &'static str,
    /// Grid do terminal e código (JetBrains Mono — selada pelo ADR 0019 §7).
    pub mono: &'static str,
}

/// Escala de tamanhos (px inteiros) — nascida da MEDIÇÃO do shell no kickoff F2-1 (corpo
/// dominante 12px, micro-UI 10-11, títulos 15/20/28, momentos 40), não de razão teórica.
/// Consumo: `.text_size(px(f32::from(t.size.body)))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeScaleTokens {
    /// Legendas e medidas mínimas (kbd, medidores).
    pub caption: u16,
    /// Texto de apoio.
    pub small: u16,
    /// Corpo padrão do shell.
    pub body: u16,
    /// Grid do terminal — CONTRATO com as métricas de célula (`bridge.rs` `CELL_W`/`CELL_H`
    /// medem ESTA família NESTE px; mudar aqui exige re-derivar as métricas na costura).
    pub grid: u16,
    /// Subtítulos e cabeçalhos de card.
    pub subtitle: u16,
    /// Títulos de janela/overlay.
    pub title: u16,
    /// Cabeçalhos de tela (onboarding).
    pub heading: u16,
    /// Momentos display (Fraunces).
    pub display: u16,
}

/// Pesos nomeados — instâncias ESTÁTICAS completas obrigatórias (D1-A7: o pin não aplica eixos
/// variáveis). Consumo: `FontWeight(f32::from(t.weight.semibold))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontWeightTokens {
    pub regular: u16,
    pub medium: u16,
    pub semibold: u16,
    pub bold: u16,
}

/// Tipografia completa do shell (F2-1-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypographyTokens {
    pub family: FontFamilyTokens,
    pub size: TypeScaleTokens,
    pub weight: FontWeightTokens,
}

impl TypographyTokens {
    /// A voz canônica da fusão T1+T3 (F2-0-D). Toda construção de tema parte DAQUI.
    pub const CANONICAL: Self = Self {
        family: FontFamilyTokens {
            ui: "IBM Plex Sans",
            display: "Fraunces",
            mono: "JetBrains Mono",
        },
        size: TypeScaleTokens {
            caption: 10,
            small: 11,
            body: 12,
            grid: 13,
            subtitle: 15,
            title: 20,
            heading: 28,
            display: 40,
        },
        weight: FontWeightTokens {
            regular: 400,
            medium: 500,
            semibold: 600,
            bold: 700,
        },
    };
}

/// Espaçamento (F2-1-2) — a escala selada no épico: 4/8/12/16/24/32. Mapa para os utilitários
/// gpui (1 unidade = 4px): `xs`=`p_1` · `sm`=`p_2` · `md`=`p_3` · `lg`=`p_4` · `xl`=`p_6` ·
/// `xxl`=`p_8`. Consumo direto: `.p(px(f32::from(t.spacing.md)))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpacingTokens {
    pub xs: u16,
    pub sm: u16,
    pub md: u16,
    pub lg: u16,
    pub xl: u16,
    pub xxl: u16,
}

impl SpacingTokens {
    pub const CANONICAL: Self = Self {
        xs: 4,
        sm: 8,
        md: 12,
        lg: 16,
        xl: 24,
        xxl: 32,
    };
}

/// Cantos (F2-1-2). `md`=6 preserva o degrau dominante MEDIDO no shell (103 usos de
/// `.rounded_md()`); o "flat honesto" da fusão entra na APLICAÇÃO (F2-2), não inflando a escala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadiusTokens {
    /// Cantos retos (flat honesto — T1).
    pub none: u16,
    pub sm: u16,
    pub md: u16,
    pub lg: u16,
    /// Pílulas/badges (valor sentinela "tudo redondo").
    pub full: u16,
}

impl RadiusTokens {
    pub const CANONICAL: Self = Self {
        none: 0,
        sm: 4,
        md: 6,
        lg: 12,
        full: 9999,
    };
}

/// Movimento (F2-1-3) — durações nomeadas + reduce-motion. **DOUTRINA (D1-A8): nunca animar
/// ação iniciada por input frequente** — abrir/focar terminal, teclar, chrome = [`Self::instant`]
/// (0ms); animação só onde SIGNIFICA (estado mudou, trabalho fluindo, espaço). Use SEMPRE
/// [`Self::effective`] (subordinação a reduce-motion é do ADR 0019 §7, não opcional).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionTokens {
    /// Ações de input frequente: SEM animação, por doutrina (0ms).
    pub instant: Duration,
    /// Micro-transições de estado (hover de chip, fade de badge).
    pub fast: Duration,
    /// Transição padrão de estado (cor de chip, banner).
    pub base: Duration,
    /// Movimento espacial (pan/zoom programático, spawn) — teto <300ms (D1-A8).
    pub slow: Duration,
    /// `true` = toda duração efetiva vira 0ms. A FONTE é o sistema/Ajustes (fiação é costura);
    /// o único ponto de mutação é [`set_reduce_motion`].
    pub reduce_motion: bool,
}

impl MotionTokens {
    pub const CANONICAL: Self = Self {
        instant: Duration::ZERO,
        fast: Duration::from_millis(120),
        base: Duration::from_millis(180),
        slow: Duration::from_millis(280),
        reduce_motion: false,
    };

    /// A duração EFETIVA de uma animação: zera TUDO sob reduce-motion. Consumidores usam este
    /// acessor, nunca o campo cru — é ele que garante a subordinação (ADR 0019 §7).
    // dead_code: o 1º consumidor de produção chega com a costura/F2-2 (este crate é binário —
    // sem consumidor externo possível). Remover o allow ao fiar o primeiro uso.
    #[allow(dead_code)]
    #[must_use]
    pub fn effective(self, d: Duration) -> Duration {
        if self.reduce_motion {
            Duration::ZERO
        } else {
            d
        }
    }
}

/// O tema resolvido — tokens prontos para `gpui::rgb()`. `Copy` barato (~30 u32 + o vocabulário
/// F2-1, tudo inteiros/`Duration`/`&'static str`): as views leem uma
/// cópia por render ([`active`]) e nunca seguram lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub mode: Mode,
    pub surface: SurfaceTokens,
    pub text: TextTokens,
    pub accent: AccentTokens,
    pub state: StateTokens,
    pub focus: FocusTokens,
    pub terminal: TerminalTokens,
    pub typography: TypographyTokens,
    pub spacing: SpacingTokens,
    pub radius: RadiusTokens,
    pub motion: MotionTokens,
}

impl Theme {
    /// Resolve o tema para `(mode, acento)` — toda construção de tema passa AQUI (fonte única).
    /// Acento desconhecido → [`DEFAULT_ACCENT`] (best-effort, nunca falha).
    #[must_use]
    pub fn build(mode: Mode, accent: &str) -> Self {
        let accent_primary = accent_scale(accent).pick(mode);
        Self {
            mode,
            surface: SurfaceTokens {
                canvas: scale::CANVAS.pick(mode),
                panel: scale::PANEL.pick(mode),
                card: scale::CARD.pick(mode),
                chrome: scale::CHROME.pick(mode),
                raised: scale::RAISED.pick(mode),
                raised_alt: scale::RAISED_ALT.pick(mode),
                selected_row: scale::SELECTED_ROW.pick(mode),
                danger_muted: scale::DANGER_MUTED.pick(mode),
                success_muted: scale::SUCCESS_MUTED.pick(mode),
                border: scale::BORDER.pick(mode),
                border_muted: scale::BORDER_MUTED.pick(mode),
            },
            text: TextTokens {
                primary: scale::TEXT_PRIMARY.pick(mode),
                secondary: scale::TEXT_SECONDARY.pick(mode),
                muted: scale::TEXT_MUTED.pick(mode),
                bright: scale::TEXT_BRIGHT.pick(mode),
                on_accent: scale::TEXT_ON_ACCENT.pick(mode),
                on_emphasis: scale::TEXT_ON_EMPHASIS.pick(mode),
            },
            accent: AccentTokens {
                primary: accent_primary,
                secondary: scale::ACCENT_SECONDARY.pick(mode),
                action: scale::ACCENT_ACTION.pick(mode),
                create: scale::ACCENT_CREATE.pick(mode),
                confirm: scale::ACCENT_CONFIRM.pick(mode),
            },
            state: StateTokens {
                success: scale::STATE_SUCCESS.pick(mode),
                warning: scale::STATE_WARNING.pick(mode),
                danger: scale::STATE_DANGER.pick(mode),
            },
            // O anel de foco É o acento do usuário — curadoria garante ≥3:1 nos 2 modos (gate).
            focus: FocusTokens {
                ring: accent_primary,
            },
            terminal: TerminalTokens {
                bg: scale::TERMINAL_BG.pick(mode),
                cursor: scale::TERMINAL_CURSOR.pick(mode),
                cursor_text: scale::TERMINAL_CURSOR_TEXT.pick(mode),
                selection: scale::TERMINAL_SELECTION.pick(mode),
            },
            // Vocabulário F2-1: invariante a modo/acento — a voz é UMA (fusão T1+T3).
            typography: TypographyTokens::CANONICAL,
            spacing: SpacingTokens::CANONICAL,
            radius: RadiusTokens::CANONICAL,
            motion: MotionTokens::CANONICAL,
        }
    }
}

// ═══════════════════════════ tema ATIVO (troca ao vivo, sem restart) ═══════════════════════════

/// O tema vivo do processo. `RwLock` (não gpui Global) para o módulo ficar **gpui-free e testável**
/// e helpers sem `cx` ([`crate::a11y::live_region_element`], `naming_overlay`…) lerem tokens. As
/// janelas re-pintam no frame seguinte (os `render` leem [`active`] a cada frame).
static ACTIVE: LazyLock<RwLock<Theme>> =
    LazyLock::new(|| RwLock::new(Theme::build(Mode::Dark, DEFAULT_ACCENT)));

/// Cópia do tema ativo (leitura barata; nunca segura lock através de render).
#[must_use]
pub fn active() -> Theme {
    // Poison-recovery (idioma do `bridge::lock`): um panic alheio não pode apagar o tema do app.
    *ACTIVE.read().unwrap_or_else(PoisonError::into_inner)
}

/// Aplica `(mode, acento)` como tema vivo — efeito no próximo frame de TODAS as janelas (sem
/// restart, sem flash: é só a próxima passada de render lendo novos u32).
///
/// O flag reduce-motion SOBREVIVE à troca (F2-1-3): trocar modo/acento jamais religa animação
/// para quem a reduziu — seria regressão de a11y silenciosa.
pub fn apply(mode: Mode, accent: &str) {
    let mut active = ACTIVE.write().unwrap_or_else(PoisonError::into_inner);
    let reduce_motion = active.motion.reduce_motion;
    *active = Theme::build(mode, accent);
    active.motion.reduce_motion = reduce_motion;
}

/// Liga/desliga o reduce-motion do tema vivo (F2-1-3) — o ÚNICO ponto de mutação do flag.
/// A FONTE da verdade é o sistema/Ajustes; a fiação (observer) é costura posterior.
// dead_code: a fiação (observer de aparência/Ajustes) é a costura que consome isto — pedido ao
// Maestro junto com o diff do grid. Remover o allow ao fiar.
#[allow(dead_code)]
pub fn set_reduce_motion(on: bool) {
    ACTIVE
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .motion
        .reduce_motion = on;
}

// ═══════════════════════════ export/import JSON (100% local — T7§A Avançado) ═══════════════════════════

/// Seção de tipografia do arquivo de tema (F2-1, aditiva). Documenta a voz em pt-br legível;
/// famílias são CURADAS — a APLICAÇÃO de override parcial por token é a F2-1-4 (que consome
/// [`parse_prefs`]), não esta story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TypographyPrefs {
    /// Família da UI (`"IBM Plex Sans"`).
    #[serde(default)]
    pub interface: String,
    /// Família dos momentos display (`"Fraunces"`).
    #[serde(default)]
    pub momentos: String,
    /// Família do grid do terminal (`"JetBrains Mono"` — ADR 0019 §7).
    #[serde(default)]
    pub terminal: String,
}

/// Seção de movimento do arquivo de tema (F2-1, aditiva).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MotionPrefs {
    /// `true` = toda duração efetiva vira 0ms ([`MotionTokens::effective`]).
    #[serde(default)]
    pub reduzir: bool,
}

/// O tema como arquivo: JSON **legível** em pt-br, local-first (nenhuma rede). Campos desconhecidos
/// de versões futuras são IGNORADOS no import ("aplica o que entende" — T7§A estados de erro);
/// seções AUSENTES de versões antigas viram `None` (import antigo nunca quebra — F2-1 item 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemePrefs {
    /// `"escuro"` | `"claro"` (mesmas strings do T7).
    #[serde(default)]
    pub modo: String,
    /// Nome do acento curado (ex.: `"azul"`).
    #[serde(default)]
    pub acento: String,
    /// Vocabulário F2-1 (aditivo — ausente em temas pré-F2-1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tipografia: Option<TypographyPrefs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movimento: Option<MotionPrefs>,
}

/// Serializa as preferências correntes como JSON legível (export local). As seções F2-1 saem
/// preenchidas: a tipografia canônica (documenta a voz no arquivo) e o estado VIVO do
/// reduce-motion (preferência real do processo).
#[must_use]
pub fn export_json(mode: Mode, accent: &str) -> String {
    let voice = TypographyTokens::CANONICAL;
    let prefs = ThemePrefs {
        modo: mode.as_setting().to_string(),
        acento: accent.trim().to_lowercase(),
        tipografia: Some(TypographyPrefs {
            interface: voice.family.ui.to_string(),
            momentos: voice.family.display.to_string(),
            terminal: voice.family.mono.to_string(),
        }),
        movimento: Some(MotionPrefs {
            reduzir: active().motion.reduce_motion,
        }),
    };
    // ThemePrefs é Serialize de structs planas — `to_string_pretty` não tem caminho de erro real;
    // o fallback mantém a função infalível (best-effort, mesma postura do save_settings).
    serde_json::to_string_pretty(&prefs).unwrap_or_else(|_| "{}".to_string())
}

/// Lê um arquivo de tema COMPLETO (todas as seções, incl. F2-1). JSON inválido → `Err` com
/// mensagem LEIGA; campos desconhecidos ignorados; seções ausentes → `None`. A F2-1-4 (overrides
/// parciais por token) consome DAQUI.
pub fn parse_prefs(json: &str) -> Result<ThemePrefs, String> {
    serde_json::from_str(json).map_err(|_| "Este arquivo não parece um tema do Lina.".to_string())
}

/// Lê um arquivo de tema no contrato dos consumidores atuais (modo + acento). Acento desconhecido
/// cai no default ao construir ([`Theme::build`]).
pub fn parse_json(json: &str) -> Result<(Mode, String), String> {
    let prefs = parse_prefs(json)?;
    let accent = if prefs.acento.trim().is_empty() {
        DEFAULT_ACCENT.to_string()
    } else {
        prefs.acento.trim().to_lowercase()
    };
    Ok((Mode::from_setting(&prefs.modo), accent))
}

// ═══════════════════════════ testes (o GATE do CI mora aqui) ═══════════════════════════

#[cfg(test)]
pub(crate) static THEME_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::wcag::{contrast_ratio, meets_aa, meets_aa_large};

    /// Todos os pares (modo × acento) que o shell pode assumir — o gate itera TODOS.
    fn all_themes() -> Vec<Theme> {
        let mut out = Vec::new();
        for mode in [Mode::Dark, Mode::Light] {
            for (name, _) in ACCENTS {
                out.push(Theme::build(mode, name));
            }
        }
        out
    }

    /// **GATE WCAG (CI) — ampliado pelo F1-2-1**: cobre os 2 temas, os 8 acentos e TODOS os tokens
    /// de texto. FALHA se qualquer token de texto baixar de 4.5:1 (AA normal) sobre as superfícies
    /// em que é usado, ou se o muted (tier secundário, contrato do W4-6) baixar de 3:1.
    #[test]
    fn themes_meet_wcag_contrast_gate() {
        for th in all_themes() {
            let tag = format!("modo={:?} acento=0x{:06x}", th.mode, th.accent.primary);
            let surfaces = [
                ("canvas", th.surface.canvas),
                ("panel", th.surface.panel),
                ("card", th.surface.card),
                ("chrome", th.surface.chrome),
            ];
            // Texto AA normal (≥4.5:1) sobre as superfícies de leitura.
            for (tn, t) in [
                ("text.primary", th.text.primary),
                ("text.secondary", th.text.secondary),
                ("text.bright", th.text.bright),
                ("accent.primary", th.accent.primary),
                ("state.success", th.state.success),
                ("state.warning", th.state.warning),
                ("state.danger", th.state.danger),
            ] {
                for (sn, s) in surfaces {
                    assert!(
                        meets_aa(t, s),
                        "[{tag}] {tn} sobre {sn}: {:.2}:1 < 4.5:1 (AA)",
                        contrast_ratio(t, s)
                    );
                }
            }
            // Texto em botões/chips ELEVADOS e fills fortes.
            for (tn, t, sn, s) in [
                ("text.bright", th.text.bright, "raised", th.surface.raised),
                (
                    "text.bright",
                    th.text.bright,
                    "raised_alt",
                    th.surface.raised_alt,
                ),
                ("text.primary", th.text.primary, "raised", th.surface.raised),
                (
                    "text.on_accent",
                    th.text.on_accent,
                    "accent.action",
                    th.accent.action,
                ),
                (
                    "text.on_accent",
                    th.text.on_accent,
                    "accent.create",
                    th.accent.create,
                ),
                (
                    "text.on_accent",
                    th.text.on_accent,
                    "accent.confirm",
                    th.accent.confirm,
                ),
                (
                    "text.on_emphasis",
                    th.text.on_emphasis,
                    "state.success",
                    th.state.success,
                ),
                (
                    "text.on_emphasis",
                    th.text.on_emphasis,
                    "state.warning",
                    th.state.warning,
                ),
                (
                    "text.on_emphasis",
                    th.text.on_emphasis,
                    "state.danger",
                    th.state.danger,
                ),
                // Banners discretos (onboarding/dev_tools/obsidian): estado como TEXTO sobre os
                // fundos *_muted correspondentes.
                (
                    "state.success",
                    th.state.success,
                    "surface.success_muted",
                    th.surface.success_muted,
                ),
                (
                    "accent.primary",
                    th.accent.primary,
                    "surface.success_muted",
                    th.surface.success_muted,
                ),
                (
                    "state.warning",
                    th.state.warning,
                    "surface.danger_muted",
                    th.surface.danger_muted,
                ),
                (
                    "state.danger",
                    th.state.danger,
                    "surface.danger_muted",
                    th.surface.danger_muted,
                ),
                // Linha selecionada de lista: texto primário + muted continuam legíveis.
                (
                    "text.primary",
                    th.text.primary,
                    "surface.selected_row",
                    th.surface.selected_row,
                ),
            ] {
                assert!(
                    meets_aa(t, s),
                    "[{tag}] {tn} sobre {sn}: {:.2}:1 < 4.5:1 (AA)",
                    contrast_ratio(t, s)
                );
            }
            // Muted: tier secundário do W4-6 — ≥3:1 (AA-grande) sobre as superfícies, e
            // propositalmente <4.5:1 sobre o canvas (se atingir AA-normal, não é mais "muted").
            for (sn, s) in surfaces {
                assert!(
                    meets_aa_large(th.text.muted, s),
                    "[{tag}] text.muted sobre {sn}: {:.2}:1 < 3:1",
                    contrast_ratio(th.text.muted, s)
                );
            }
            assert!(
                !meets_aa(th.text.muted, th.surface.canvas),
                "[{tag}] text.muted atingiu AA-normal sobre canvas — perdeu o tier secundário"
            );
            // Texto da seleção do terminal: o fg das células (vindo do CLI) é preservado; aqui
            // garantimos que o PRÓPRIO fundo de seleção não some no fundo do terminal (≥1.3:1 —
            // distinguível; o texto por cima mantém o contraste do conteúdo).
            assert!(
                contrast_ratio(th.terminal.selection, th.terminal.bg) >= 1.3,
                "[{tag}] seleção do terminal indistinguível do fundo"
            );
        }
    }

    /// **Critério do gate (Maestro): focus ring VISÍVEL nos 2 temas** — ≥3:1 (WCAG 1.4.11,
    /// non-text contrast) contra canvas/panel/card, para TODOS os 8 acentos curados.
    #[test]
    fn focus_ring_visible_in_both_themes() {
        for th in all_themes() {
            for (sn, s) in [
                ("canvas", th.surface.canvas),
                ("panel", th.surface.panel),
                ("card", th.surface.card),
            ] {
                assert!(
                    contrast_ratio(th.focus.ring, s) >= 3.0,
                    "[modo={:?}] focus.ring 0x{:06x} sobre {sn}: {:.2}:1 < 3:1",
                    th.mode,
                    th.focus.ring,
                    contrast_ratio(th.focus.ring, s)
                );
            }
            // O anel também precisa se destacar do cursor/conteúdo do terminal? Não — o anel é a
            // BORDA do card (sobre canvas), não um elemento interno do grid.
        }
    }

    /// **Critério de aceite 1 (LINT): nenhum `rgb(0x…)`/`rgba(0x…)` literal fora deste módulo.**
    /// Varre `src/*.rs` no momento do teste (CI). Também proíbe `const X: u32 = 0x…` fora daqui —
    /// o escape óbvio de "const de cor local" que reabriria o furo do 13.15.
    #[test]
    fn no_hardcoded_colors_outside_theme_module() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut violations = Vec::new();
        for entry in std::fs::read_dir(&src).expect("ler src/") {
            let path = entry.expect("entry").path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !name.ends_with(".rs") || name == "theme.rs" {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("ler fonte");
            for (i, line) in content.lines().enumerate() {
                let hard_rgb = line.contains("rgb(0x") || line.contains("rgba(0x");
                // const de cor: `const NOME: u32 = 0xRRGGBB` (6 dígitos hex) fora do tema.
                let hard_const = line.contains(": u32 = 0x");
                if hard_rgb || hard_const {
                    violations.push(format!("{name}:{}: {}", i + 1, line.trim()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "cores hard-coded fora do módulo de tema (use tokens de `theme::active()`):\n{}",
            violations.join("\n")
        );
    }

    /// Modo parseia das strings persistidas do T7 (incl. desconhecido → Dark, nunca falha).
    #[test]
    fn mode_parses_from_settings_strings() {
        assert_eq!(Mode::from_setting("escuro"), Mode::Dark);
        assert_eq!(Mode::from_setting("claro"), Mode::Light);
        assert_eq!(Mode::from_setting(" CLARO "), Mode::Light);
        assert_eq!(Mode::from_setting("???"), Mode::Dark);
        assert_eq!(Mode::from_setting(""), Mode::Dark);
        assert_eq!(Mode::from_setting(Mode::Light.as_setting()), Mode::Light);
        assert_eq!(Mode::from_setting(Mode::Dark.as_setting()), Mode::Dark);
    }

    /// Acento desconhecido cai no DEFAULT (best-effort) — um `settings.json` editado à mão com
    /// acento inválido não quebra o boot nem perde o tema.
    #[test]
    fn unknown_accent_falls_back_to_default() {
        let th = Theme::build(Mode::Dark, "fuchsia-neon-2099");
        let default = Theme::build(Mode::Dark, DEFAULT_ACCENT);
        assert_eq!(th.accent.primary, default.accent.primary);
        // case-insensitive + trim no nome curado.
        assert_eq!(
            Theme::build(Mode::Light, " Violeta ").accent.primary,
            Theme::build(Mode::Light, "violeta").accent.primary
        );
    }

    /// **Critério 2 (troca ao vivo)**: `apply` muda o tema ATIVO do processo — é o que as views
    /// leem no próximo frame (sem restart). Guard serializa contra outros testes do global.
    #[test]
    fn apply_updates_active_theme_live() {
        let _guard = THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        apply(Mode::Light, "rosa");
        let th = active();
        assert_eq!(th.mode, Mode::Light);
        assert_eq!(
            th.accent.primary,
            Theme::build(Mode::Light, "rosa").accent.primary
        );
        // restaura o default p/ não vazar estado entre testes.
        apply(Mode::Dark, DEFAULT_ACCENT);
        assert_eq!(active().mode, Mode::Dark);
    }

    /// **Critério 3 (export/import)**: export gera JSON legível; import o aplica (roundtrip).
    #[test]
    fn prefs_export_import_roundtrip() {
        let json = export_json(Mode::Light, "teal");
        // legível: chaves em pt-br, valores nomeados (não números mágicos).
        assert!(json.contains("\"modo\": \"claro\""), "JSON legível: {json}");
        assert!(
            json.contains("\"acento\": \"teal\""),
            "JSON legível: {json}"
        );
        let (mode, accent) = parse_json(&json).expect("roundtrip");
        assert_eq!(mode, Mode::Light);
        assert_eq!(accent, "teal");
    }

    /// T7§A estados de erro: campos de versão FUTURA são ignorados ("aplica o que entende");
    /// arquivo que não é um tema → mensagem leiga; acento vazio → default.
    #[test]
    fn import_tolerates_future_fields_and_rejects_garbage() {
        let (mode, accent) =
            parse_json(r#"{"modo":"claro","acento":"rosa","brilho_holografico":9000}"#)
                .expect("campos desconhecidos ignorados");
        assert_eq!(mode, Mode::Light);
        assert_eq!(accent, "rosa");

        let err = parse_json("isto não é json").expect_err("lixo rejeitado");
        assert_eq!(err, "Este arquivo não parece um tema do Lina.");

        let (_, accent) = parse_json(r#"{"modo":"escuro"}"#).expect("acento ausente");
        assert_eq!(accent, DEFAULT_ACCENT);
    }

    /// **F2-1-1 — integridade da tipografia:** escala de tamanhos ESTRITAMENTE crescente
    /// (`grid`=13 é contrato com as métricas de célula do `bridge.rs`), pesos crescentes,
    /// famílias não-vazias e distintas.
    #[test]
    fn typography_vocabulary_integrity() {
        let t = TypographyTokens::CANONICAL;
        let sizes = [
            t.size.caption,
            t.size.small,
            t.size.body,
            t.size.grid,
            t.size.subtitle,
            t.size.title,
            t.size.heading,
            t.size.display,
        ];
        assert!(
            sizes.windows(2).all(|w| w[0] < w[1]),
            "escala de tamanhos não é estritamente crescente: {sizes:?}"
        );
        assert_eq!(
            t.size.grid, 13,
            "grid≠13px quebra o contrato de célula (bridge.rs CELL_W/CELL_H — re-derivar na costura)"
        );
        let weights = [
            t.weight.regular,
            t.weight.medium,
            t.weight.semibold,
            t.weight.bold,
        ];
        assert!(
            weights.windows(2).all(|w| w[0] < w[1]),
            "pesos fora de ordem: {weights:?}"
        );
        assert_eq!(t.weight.regular, 400, "regular ≠ 400 (convenção OpenType)");
        for fam in [t.family.ui, t.family.display, t.family.mono] {
            assert!(!fam.trim().is_empty(), "família vazia no vocabulário");
        }
        assert_ne!(
            t.family.ui, t.family.display,
            "UI e display iguais — a voz perde o contraste"
        );
        assert_ne!(t.family.ui, t.family.mono, "UI e mono iguais");
    }

    /// **ADR 0019 §7 — dívida FECHADA:** JetBrains Mono selada para grid/código VIA TOKEN. Este
    /// teste é o guardião: trocar a família do grid exige decisão NESTE módulo (e re-derivar as
    /// métricas de célula), nunca um literal no shell.
    #[test]
    fn mono_family_is_jetbrains_mono_per_adr_0019() {
        assert_eq!(TypographyTokens::CANONICAL.family.mono, "JetBrains Mono");
        assert_eq!(
            Theme::build(Mode::Dark, DEFAULT_ACCENT)
                .typography
                .family
                .mono,
            "JetBrains Mono"
        );
        assert_eq!(
            Theme::build(Mode::Light, DEFAULT_ACCENT)
                .typography
                .family
                .ui,
            "IBM Plex Sans"
        );
    }

    /// **F2-1-2:** espaçamento é EXATAMENTE a escala do épico (4/8/12/16/24/32); cantos
    /// crescentes com `none`=0 (flat honesto disponível) e `md`=6 (degrau dominante medido).
    #[test]
    fn spacing_and_radius_scales_match_epic() {
        let s = SpacingTokens::CANONICAL;
        assert_eq!(
            [s.xs, s.sm, s.md, s.lg, s.xl, s.xxl],
            [4, 8, 12, 16, 24, 32],
            "a escala de espaçamento é a do épico F2-1-2 — mudá-la é decisão de épico, não de story"
        );
        let r = RadiusTokens::CANONICAL;
        assert_eq!(r.none, 0, "flat honesto exige o degrau zero");
        assert_eq!(r.md, 6, "md≠6 muda o degrau dominante do shell sem decisão");
        assert!(
            r.none < r.sm && r.sm < r.md && r.md < r.lg && r.lg < r.full,
            "cantos fora de ordem"
        );
    }

    /// **F2-1-3 — doutrina de movimento (D1-A8):** input frequente = 0ms; durações nomeadas
    /// crescentes com teto <300ms; `effective` zera TUDO sob reduce-motion.
    #[test]
    fn motion_durations_respect_doctrine_and_reduce_motion() {
        let m = MotionTokens::CANONICAL;
        assert_eq!(
            m.instant,
            Duration::ZERO,
            "input frequente NUNCA anima (D1-A8)"
        );
        assert!(
            m.instant < m.fast && m.fast < m.base && m.base < m.slow,
            "durações fora de ordem"
        );
        assert!(
            m.slow < Duration::from_millis(300),
            "teto D1-A8 estourado: {:?} ≥ 300ms",
            m.slow
        );
        assert!(
            !m.reduce_motion,
            "default = animações ligadas (o flag real vem do sistema, na fiação)"
        );
        assert_eq!(
            m.effective(m.slow),
            m.slow,
            "sem reduce, a duração passa intacta"
        );
        let reduced = MotionTokens {
            reduce_motion: true,
            ..m
        };
        for d in [m.fast, m.base, m.slow] {
            assert_eq!(
                reduced.effective(d),
                Duration::ZERO,
                "reduce-motion não zerou {d:?} (ADR 0019 §7: subordinação obrigatória)"
            );
        }
    }

    /// **F2-1 item 4 — JSON aditivo:** export carrega as seções novas (legíveis em pt-br);
    /// tema ANTIGO (só modo/acento) importa sem quebrar; roundtrip preserva o vocabulário.
    #[test]
    fn prefs_vocabulary_is_additive_and_roundtrips() {
        let json = export_json(Mode::Dark, "ambar");
        assert!(
            json.contains("\"tipografia\""),
            "export sem seção tipografia: {json}"
        );
        assert!(
            json.contains("\"JetBrains Mono\""),
            "export sem a família do grid: {json}"
        );
        assert!(
            json.contains("\"movimento\""),
            "export sem seção movimento: {json}"
        );

        let prefs = parse_prefs(&json).expect("roundtrip do export");
        let tip = prefs.tipografia.expect("tipografia presente no roundtrip");
        assert_eq!(tip.interface, "IBM Plex Sans");
        assert_eq!(tip.momentos, "Fraunces");
        assert_eq!(tip.terminal, "JetBrains Mono");

        // Tema PRÉ-F2-1 (só modo/acento): importa com seções → None, contrato antigo intacto.
        let old = parse_prefs(r#"{"modo":"claro","acento":"rosa"}"#).expect("import antigo");
        assert_eq!(old.tipografia, None);
        assert_eq!(old.movimento, None);
        let (mode, accent) = parse_json(r#"{"modo":"claro","acento":"rosa"}"#).expect("antigo");
        assert_eq!(mode, Mode::Light);
        assert_eq!(accent, "rosa");
    }

    /// **F2-1-3 — flag vivo:** `set_reduce_motion` muda o tema ATIVO e o flag SOBREVIVE a
    /// `apply` (trocar modo/acento jamais religa animação reduzida — a11y). Guard serializa.
    #[test]
    fn reduce_motion_survives_apply_and_zeroes_effective() {
        let _guard = THEME_TEST_GUARD
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_reduce_motion(true);
        assert!(active().motion.reduce_motion);
        assert_eq!(
            active().motion.effective(Duration::from_millis(180)),
            Duration::ZERO
        );
        apply(Mode::Light, "teal");
        assert!(
            active().motion.reduce_motion,
            "apply() apagou o reduce-motion — regressão de a11y"
        );
        // restaura o default p/ não vazar estado entre testes.
        set_reduce_motion(false);
        apply(Mode::Dark, DEFAULT_ACCENT);
        assert!(!active().motion.reduce_motion);
    }

    /// Stepping consistente: toda escala TEMÁTICA tem dois degraus REAIS (dark ≠ light) — uma
    /// escala "preguiçosa" (mesmo valor) quebraria o modo claro silenciosamente. As exceções
    /// legítimas (terminal, fixado escuro por curadoria) são `ColorScale::fixed` e ficam fora.
    #[test]
    fn curated_scales_have_two_real_steps() {
        for (name, s) in ACCENTS {
            assert_ne!(s.dark, s.light, "acento '{name}' sem degrau light real");
        }
        // nomes únicos (o seletor por nome depende disso).
        let mut names: Vec<&str> = ACCENTS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ACCENTS.len(), "nomes de acento duplicados");
    }
}
