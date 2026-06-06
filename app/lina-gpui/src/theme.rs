//! `theme` — **F1-2-1: design system com tokens nomeados (dark/light + personalização local-first)**.
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
//! **Terminal é superfície escura nos 2 temas** (decisão de curadoria): o conteúdo do grid vem do CLI
//! via ANSI e assume fundo escuro; tematizar o conteúdo além de BG/cursor padrão está fora do escopo
//! (F1-2-1 "NÃO entra"). No modo claro, o terminal fica como um viewport escuro dentro do chrome claro.
//!
//! Âncora `UiHost` (invariante #7): tema é assunto EXCLUSIVO do shell — nenhum token atravessa para
//! `lina-core` (este módulo não importa nada do core nem do gpui; consumidores chamam `gpui::rgb`).

use std::sync::{LazyLock, PoisonError, RwLock};

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

/// O tema resolvido — tokens prontos para `gpui::rgb()`. `Copy` barato (~30 u32): as views leem uma
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
pub fn apply(mode: Mode, accent: &str) {
    *ACTIVE.write().unwrap_or_else(PoisonError::into_inner) = Theme::build(mode, accent);
}

// ═══════════════════════════ export/import JSON (100% local — T7§A Avançado) ═══════════════════════════

/// O tema como arquivo: JSON **legível** em pt-br, local-first (nenhuma rede). Campos desconhecidos
/// de versões futuras são IGNORADOS no import ("aplica o que entende" — T7§A estados de erro).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemePrefs {
    /// `"escuro"` | `"claro"` (mesmas strings do T7).
    #[serde(default)]
    pub modo: String,
    /// Nome do acento curado (ex.: `"azul"`).
    #[serde(default)]
    pub acento: String,
}

/// Serializa as preferências correntes como JSON legível (export local).
#[must_use]
pub fn export_json(mode: Mode, accent: &str) -> String {
    let prefs = ThemePrefs {
        modo: mode.as_setting().to_string(),
        acento: accent.trim().to_lowercase(),
    };
    // ThemePrefs é Serialize de structs planas — `to_string_pretty` não tem caminho de erro real;
    // o fallback mantém a função infalível (best-effort, mesma postura do save_settings).
    serde_json::to_string_pretty(&prefs).unwrap_or_else(|_| "{}".to_string())
}

/// Lê um arquivo de tema. JSON inválido → `Err` com mensagem LEIGA (copy do T7§A); campos
/// desconhecidos são ignorados; acento desconhecido cai no default ao construir ([`Theme::build`]).
pub fn parse_json(json: &str) -> Result<(Mode, String), String> {
    let prefs: ThemePrefs = serde_json::from_str(json)
        .map_err(|_| "Este arquivo não parece um tema do Lina.".to_string())?;
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
