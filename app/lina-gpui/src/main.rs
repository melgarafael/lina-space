//! `lina-gpui` — o **walking skeleton com render de terminal de verdade** (Onda 2). Canvas
//! gpui com 2 terminais reais do core; cada card pinta o **snapshot do grid VISÍVEL**
//! (`VtBackend::screen()`) a cada frame — cols×rows com **cores por célula + cursor** —
//! então TUIs ricas (Claude Code, vim) aparecem corretas. **Scroll** (scrollback),
//! **teclas especiais** (setas/Home/End/PageUp·Down/F-keys → CSI), **resize** (PTY+grid),
//! e o diferencial **A2A com pulso "sem fios"**. Tradução core↔UI em [`bridge`] (gpui-free).

mod bridge;
// W4-2: modelo de cena do canvas T4 (zonas foco/periferia/suspenso + badge) — gpui-free, testável.
mod canvas;
// F3-1-7: card da Goal na tela do leigo (meta + "o que entendi" + critérios + barra de iteração +
// confirmar/escalar). Componente sobre o catálogo `ui/` + tokens F2; lógica de tela em funções puras.
// A projeção `Goal` viva é ligada na integração do Maestro (DEP do despacho).
mod goal_card;
// F3-3-5: painel "Como o [papel] pensa" — a janela do leigo para o que cada papel APRENDEU com ele
// (proveniência humanizada + aposentar-1-clique → `BeliefRetired`). Componente sobre o catálogo `ui/`
// + tokens F2; lógica de tela em funções puras. A projeção viva (`lina_core::mentality`, M-PROMO) é
// mapeada para o view-model por um adaptador fino em `mentality_views` (costura alinhada com o dono).
mod mentality_panel;
// W4-2 · M1: paleta de comandos (Cmd-K) sobre o canvas — modelo puro testável + render gpui.
mod palette;
// W4-1: onboarding turno-0 (T0→T3 + "Instalar para mim"). Módulo isolado; disjunto do canvas.
mod onboarding;
// Tela "Ferramentas de desenvolvimento" do onboarding (git/gh/vercel/node/python): reusa o instalador
// dos assistentes (PTY oculto + re-hidratação de PATH). Lógica gpui-free; render fina sobre OnboardingView.
mod dev_tools;
// Tela "Seu segundo cérebro" (Obsidian) do onboarding: detecta/instala o app, escolhe vault(s),
// gera o PageIndex determinístico e persiste em `.lina/vault.json`. Lógica gpui-free; render fina.
mod obsidian;
// W4-3: chrome de conexão "sem fios" — freio (pausa de orquestração), selo por membership, reduce-motion.
mod wiring;
// W3-7c · TETO DE CUSTO REAL: mede o output dos PTYs e emite TokenUsageReported (alimenta o CostLedger).
mod cost;
// W4-4: chrome de persistência (salvo✓ / recuperação T8 / Espaços T6 / Ajustes T7). Janela própria.
mod persistence_ui;
/// F1-4-1 (fiação app): decide qual Espaço o boot abre + auto-inscrição no registry global.
mod workspace_boot;
// M8 fatia (i): o boot por-Espaço extraído em construtor reutilizável (`boot_ws_runtime`) —
// `WsRuntime` (tudo por-Espaço, com dreno próprio) + `SharedInfra` (por-processo). Fundação
// do switch vivo F1-4-4; com 1 runtime o comportamento é idêntico ao boot inline anterior.
mod runtime;
// W4-5: galeria de Focos (T3) — presets que montam o Espaço com time + papéis; gpui-free, testável.
mod gallery;
// W4-6: acessibilidade (AccessibleBuffer sem ANSI, live-region "resposta pronta", WCAG, reduce-motion).
mod a11y;
// F2-2-2 (ADR 0028): o live-region (custom Element) foi PROMOVIDO de spike a produção — saiu do
// `#[path] pub mod live` do a11y.rs para `mod a11y_live;` no crate root. Consumidores (toast/badge
// do E, a11y.rs::live_region_element) usam `crate::a11y_live::*`. Registro é do dono do main.rs.
mod a11y_live;
// W4-2 · P4: inspetor do nó selecionado (nome/papel/activity/perfil/status) — gpui-free, testável.
mod inspector;
// W4-2 · M3/M4: criadores de NOTA e PASTA (apenda Note/FolderCreated + persiste em .lina/) — gpui-free, testável.
mod creators;
// F1-2-1: design system tokenizado (dark/light + 8 acentos curados) — a FONTE ÚNICA de cor do shell.
// Todo pixel pintado lê `theme::active()`; o gate WCAG + o lint anti-cor-hardcoded moram lá. gpui-free.
mod theme;
// F1-2-2/F1-2-3: contrato de fronteira do co-piloto de papel (nome → sugestão + porquê). gpui-free.
mod role_suggester;
// F1-2-2 (M6/M6-E): modal de criar/editar Agente — evolui o M2. Modelo gpui-free + render fina.
mod agent_modal;
// F1-1-5 (P6 §Atividade): dashboard vivo por terminal — estado/motor/custo~/atividade. gpui-free.
mod dashboard;
// F1-1-7: fila de atenção UNIFICADA + toast (permissão + custódia). Lógica de apresentação em
// funções puras (gpui-free, testável headless); render fina. ZERO write no PTY (ADR 0021 §6).
mod attention_ui;
// F1-4-4 · M8 (T4): rail esquerdo de Espaços — estado headless + render. SÓ a declaração
// vive aqui; o mount, os atalhos ⌘O/⌘1..9 e o switch real são fiação do Maestro (integração).
mod sidebar;
// F1-5-1: sonda [PROF] — decompõe o frametime que a [FPS] mede agregado (poll/assemble por
// painel/chrome + layout_paint/present via sentinela de paint). Lógica gpui-free, testável.
mod prof;
// F1-4-6: copy congelada + estado headless da ativação do Lina PRO (campo de colar,
// erros leigos por variante, painel do plano) — compartilhado pelo M9 (1b) e T7§P.
mod license_ui;
// F2-2-1 · catálogo de componentes núcleo (botão/painel/input/modal) da fusão T1+T3. Registro de
// módulo apenas — a costura de boot/wiring do main.rs segue do Maestro (despacho r4).
mod ui;
// F4-0-2 (UI): modal de upload de credencial de canal — modelo gpui-free + render fina (padrão M6).
// O valor é coletado, enviado ao cofre via o contrato core (F4-0-2/credential.rs) e some da tela.
mod credential_modal;
// F4-WA-2c (UI): modal "Receber um aviso de fora" (webhook) — modelo gpui-free + render fina (padrão
// credential_modal). Coleta terminal+instrução+nível → gesto `webhook.configure` → `WebhookConfigured`.
mod webhook_modal;
// F4-0-5: badge "este Espaço está falando com o mundo" — projeção PURA do log (canal com credencial
// viva), sem relógio nem I/O. 0 canais → 0 badge; ≥1 → diz QUAL canal (doc 40 §10).
mod exposure;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gpui::{
    div, point, prelude::*, px, rgb, size, text, App, Bounds, ClickEvent, ClipboardItem, Context,
    Entity, ExternalPaths, FocusHandle, FontWeight, InputHandler, KeyDownEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point,
    Render, Rgba, Role, ScrollDelta, ScrollWheelEvent, TitlebarOptions, UTF16Selection, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;

use lina_core::{build_paste, DomainEvent, PtyManager, VtRgb, VtScreen};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use lina_bootstrap::Autonomy;

use bridge::{
    card_visible, cell_h, cell_in_selection, cell_w, encode_pointer, hit_test, lock, normalize_sel,
    quote_dropped_paths, screen_to_cell, shell_cmd, A2aTrigger, AgentEngine, AttentionHub, Camera,
    CwdPolicy, Desk, Grid, NodeAdmission, NodeManager, PtrAction, CARD_H, CARD_W,
};

/// Tamanho da fonte do grid, do token (contrato grid=13 — teste no theme.rs).
fn grid_font_px() -> f32 {
    f32::from(theme::active().typography.size.grid)
}

/// Token `0xRRGGBB` do tema → [`VtRgb`] (cores do shell entrando no pipeline de células do grid:
/// cursor padrão e fundo de seleção — únicos pontos onde o tema toca o conteúdo do terminal).
fn vt(token: u32) -> VtRgb {
    VtRgb {
        r: ((token >> 16) & 0xff) as u8,
        g: ((token >> 8) & 0xff) as u8,
        b: (token & 0xff) as u8,
    }
}

/// Quantas cols×rows cabem na área de grid de um card (>= 80×24, "tamanho útil").
fn fit_dims() -> (u16, u16) {
    let pad = 8.0;
    let title = 34.0;
    let cols = (((CARD_W - 2.0 * pad) / cell_w()).floor() as u16).max(80);
    let rows = (((CARD_H - title - 2.0 * pad) / cell_h()).floor() as u16).max(24);
    (cols, rows)
}

/// Traduz uma `Keystroke` em bytes para o PTY — APENAS teclas de controle/navegação: Ctrl+letra;
/// Enter/Backspace/Tab/Escape; setas como CSI (ou SS3 em `app_cursor`/DECCKM); Home/End/PageUp·Down/
/// Insert/Delete/F1-F12. **Imprimíveis (letras, dígitos, espaço, acentos, CJK) NÃO saem por aqui:**
/// fluem pelo IME do SO (`InputHandler::replace_text_in_range` → [`composed_text_to_pty_bytes`]),
/// espelhando o `to_esc_str` do terminal do Zed. Emiti-los aqui duplicaria o caractere (o gpui já o
/// injeta via `insertText`); ver o roteamento em `gpui_macos::window::handle_key_event`.
fn keystroke_to_bytes(ks: &Keystroke, app_cursor: bool) -> Vec<u8> {
    let key = ks.key.as_str();
    if ks.modifiers.control && key.len() == 1 {
        if let Some(c) = key.chars().next() {
            if c.is_ascii_alphabetic() {
                return vec![(c.to_ascii_uppercase() as u8) & 0x1f];
            }
        }
    }
    // Setas/Home/End: ESC O X em application-cursor-keys, ESC [ X em modo normal.
    let csi = |c: u8| -> Vec<u8> {
        if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    match key {
        // Cmd+Delete (mac) / Ctrl+Backspace (windows) → LIMPAR todo o input: `Ctrl+U` (`0x15`), o
        // kill-line do readline que o shell e o Claude Code respeitam. No Mac, a tecla "delete"
        // (apaga p/ trás) é reportada pelo gpui como `"backspace"`; cobrimos ambos os nomes sob
        // `platform`, e `"backspace"` sob `control` p/ o Ctrl+Backspace do Windows. DEVE vir antes
        // dos ramos `"backspace"`/`"delete"` PUROS (match é top-down). PENDENTE TELA: ver o input
        // esvaziar num Claude real.
        "backspace" | "delete" if ks.modifiers.platform => return vec![0x15],
        "backspace" if ks.modifiers.control => return vec![0x15],
        // BUG A — Shift+Enter insere NOVA LINHA (não submete). Enviamos ESC+CR (`0x1b 0x0D`) num
        // ÚNICO write: parsers de TUI leem `ESC`+char contíguo como Meta/Alt+char, e o Claude Code
        // (Ink) mapeia Meta/Alt+Enter → quebra de linha no input multilinha — é o MESMO byte do
        // atalho documentado "Option+Enter" / "Esc depois Enter", e funciona SEM `/terminal-setup`
        // (a Lina É o emulador: ela sintetiza os bytes do PTY, então manda o que o Meta+Enter manda).
        // Rejeitados: `\n` (0x0A) puro — muitos editores de linha o tratam como SUBMIT; CSI-u
        // `\x1b[13;2u` — exigiria o protocolo de teclado Kitty negociado (o Claude Code não liga por
        // padrão), viraria lixo literal. Enter PURO segue submetendo (`0x0D`). PENDENTE TELA: a
        // confirmação da quebra de linha num Claude real (gpui não roda headless).
        "enter" | "return" if ks.modifiers.shift => return vec![0x1b, 0x0D],
        "enter" | "return" => return vec![0x0D],
        "backspace" => return vec![0x7f],
        "tab" => return vec![0x09],
        "escape" => return vec![0x1b],
        "up" => return csi(b'A'),
        "down" => return csi(b'B'),
        "right" => return csi(b'C'),
        "left" => return csi(b'D'),
        "home" => return csi(b'H'),
        "end" => return csi(b'F'),
        "pageup" => return vec![0x1b, b'[', b'5', b'~'],
        "pagedown" => return vec![0x1b, b'[', b'6', b'~'],
        "insert" => return vec![0x1b, b'[', b'2', b'~'],
        "delete" => return vec![0x1b, b'[', b'3', b'~'],
        "f1" => return vec![0x1b, b'O', b'P'],
        "f2" => return vec![0x1b, b'O', b'Q'],
        "f3" => return vec![0x1b, b'O', b'R'],
        "f4" => return vec![0x1b, b'O', b'S'],
        "f5" => return vec![0x1b, b'[', b'1', b'5', b'~'],
        "f6" => return vec![0x1b, b'[', b'1', b'7', b'~'],
        "f7" => return vec![0x1b, b'[', b'1', b'8', b'~'],
        "f8" => return vec![0x1b, b'[', b'1', b'9', b'~'],
        "f9" => return vec![0x1b, b'[', b'2', b'0', b'~'],
        "f10" => return vec![0x1b, b'[', b'2', b'1', b'~'],
        "f11" => return vec![0x1b, b'[', b'2', b'3', b'~'],
        "f12" => return vec![0x1b, b'[', b'2', b'4', b'~'],
        _ => {}
    }
    Vec::new()
}

/// **MODELO B (decisão do fundador) — roteamento do mouse-down sobre o grid de um terminal.** O
/// destino do arrasto:
/// - `LocalSelect`: arrasto PURO **sempre** seleciona texto LOCAL — MESMO com o Claude em
///   mouse-reporting (que ele liga por padrão). Era ESTA a causa raiz REAL do bug: com mouse-reporting
///   ON, `send_pointer` roubava todo arrasto pro TUI (`report_node`) e `self.sel` nunca nascia → a
///   "seleção parcial que desseleciona" era o PRÓPRIO Claude desenhando ao receber os eventos.
/// - `ForwardToTui`: só sob ⌥ Option segurado E quando o TUI aceitou o reporting (`tui_accepted`).
///
/// Pura → trava a política por teste (gpui não roda headless).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerRoute {
    LocalSelect,
    ForwardToTui,
}

#[must_use]
fn pointer_route(option_held: bool, tui_accepted: bool) -> PointerRoute {
    if option_held && tui_accepted {
        PointerRoute::ForwardToTui
    } else {
        PointerRoute::LocalSelect
    }
}

/// W2-4 (dívida IME): texto COMPOSTO pelo IME do SO (dead-keys pt-br ´~^, cedilha, CJK) → bytes UTF-8
/// para o PTY do nó focado. É o análogo de `key_char` em [`keystroke_to_bytes`], mas a partir do
/// caractere FINAL (já composto) que o sistema entrega via `InputHandler::replace_text_in_range`.
/// Texto vazio → sem bytes (o chamador NÃO submete um `WriteOp`).
fn composed_text_to_pty_bytes(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// Ponte IME→PTY (W2-4): o IME do SO entrega aqui o TEXTO COMPOSTO (dead-keys pt-br ´~^, cedilha,
/// CJK) e nós o escrevemos no PTY do nó focado. Espelha o `TerminalInputHandler` do terminal do Zed
/// (mesmo SHA de gpui): o terminal não tem buffer de texto local — o pre-edit (marcado) é rastreado
/// só p/ o protocolo `NSTextInputClient`, e o caractere final vai ao PTY no `replace_text_in_range`.
///
/// É REGISTRADO 1×/frame no `paint` de um `gpui::canvas` (única fase válida p/ `Window::handle_input`),
/// atrelado ao foco do canvas (`self.focus`). Lê/escreve o estado IME via o handle `Entity`.
struct ImeHandler {
    view: Entity<WorkspaceView>,
}

impl InputHandler for ImeHandler {
    /// Ponto de inserção (sem seleção) → sinaliza ao SO que há um contexto de texto ativo; sem isto a
    /// composição de dead-keys não inicia. (Não distinguimos alt-screen aqui — ver nota na entrega.)
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    /// Faixa do texto MARCADO (pre-edit) em UTF-16 — o SO usa-a p/ saber o que substituir ao compor.
    fn marked_text_range(
        &mut self,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<std::ops::Range<usize>> {
        self.view.read(cx).ime_marked_range_utf16()
    }

    /// Não expomos o documento (o "texto" é o scrollback do PTY, não editável pelo IME).
    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<String> {
        None
    }

    /// COMMIT: o caractere final composto chega aqui (ex. "á") → vira bytes no PTY do nó focado.
    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, view_cx| {
            view.ime_clear_marked(view_cx);
            view.ime_commit(text);
        });
    }

    /// COMPOSIÇÃO em curso: o SO marca o pre-edit (ex. o "´" pendente). Rastreamos sem enviar ao PTY.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, view_cx| {
            view.ime_set_marked(new_text.to_string(), view_cx);
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        self.view
            .update(cx, |view, view_cx| view.ime_clear_marked(view_cx));
    }

    /// Sem janela de candidatos posicionada — dead-keys não a usam (MVP; CJK apareceria na origem).
    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        None
    }

    /// Terminal: queremos REPETIÇÃO de tecla (e dead-keys compõem via IME de qualquer forma), não o
    /// popup de acentos do macOS (press-and-hold). Espelha o terminal do Zed.
    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
}

#[inline]
fn rgba(c: VtRgb) -> Rgba {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
}

/// Pinta UMA linha do grid: agrupa células de mesmo estilo em runs (flex-row de spans),
/// com a célula do cursor como bloco invertido (accent). Monoespaçado → tudo alinha.
/// F1-5-1: `runs_out` (sonda [PROF] ligada) acumula quantos runs a linha montou — proxy de
/// quads no shell (cada run ≈ 1 quad de fundo + 1 run de texto). `None` = custo zero.
fn render_line(
    cells: &[lina_core::VtCell],
    cursor_col: Option<usize>,
    row: usize,
    sel: Option<(usize, usize, usize, usize)>,
    runs_out: Option<&Cell<usize>>,
) -> impl IntoElement {
    struct Run {
        fg: VtRgb,
        bg: VtRgb,
        bold: bool,
        text: String,
    }
    // F1-2-1: cursor padrão + fundo de seleção vêm dos tokens de TERMINAL (superfície escura nos
    // 2 temas — o conteúdo ANSI do CLI não é tematizado; só BG/cursor padrão, escopo da story).
    let term = theme::active().terminal;
    let mut runs: Vec<Run> = Vec::new();
    for (col, cell) in cells.iter().enumerate() {
        let selected =
            sel.is_some_and(|(sc, sr, ec, er)| cell_in_selection(col, row, sc, sr, ec, er));
        let (fg, bg, bold) = if cursor_col == Some(col) {
            // Bloco do cursor: bg accent, fg escuro.
            (vt(term.cursor_text), vt(term.cursor), false)
        } else if selected {
            // Célula SELECIONADA: bg de destaque, mantém fg/negrito (texto legível).
            (cell.fg, vt(term.selection), cell.bold)
        } else {
            (cell.fg, cell.bg, cell.bold)
        };
        let ch = match cell.c {
            '\0' | '\t' => ' ',
            other => other,
        };
        match runs.last_mut() {
            Some(r) if r.fg == fg && r.bg == bg && r.bold == bold => r.text.push(ch),
            _ => runs.push(Run {
                fg,
                bg,
                bold,
                text: ch.to_string(),
            }),
        }
    }
    if let Some(c) = runs_out {
        c.set(c.get() + runs.len());
    }
    div().flex().flex_row().children(runs.into_iter().map(|r| {
        let span = div()
            .bg(rgba(r.bg))
            .text_color(rgba(r.fg))
            .child(text!(r.text));
        if r.bold {
            span.font_weight(FontWeight::BOLD)
        } else {
            span
        }
    }))
}

/// Fator de zoom para um evento de scroll (gentil; o zoom absoluto é clampado na [`Camera`]).
fn scroll_zoom_factor(delta: ScrollDelta) -> f32 {
    let dy: f32 = match delta {
        ScrollDelta::Lines(p) => p.y,
        ScrollDelta::Pixels(p) => p.y / px(40.0),
    };
    (1.0 + dy * 0.08).clamp(0.5, 1.5)
}

/// Pinta o GRID VISÍVEL inteiro (snapshot atual) — uma linha por linha do viewport. `scale` =
/// zoom do canvas (a fonte monoespaçada escala junto, mantendo o alinhamento do grid).
fn render_grid(
    screen: &VtScreen,
    scale: f32,
    sel: Option<(usize, usize, usize, usize)>,
    runs_out: Option<&Cell<usize>>,
) -> impl IntoElement {
    let cur = screen.cursor;
    div()
        .flex()
        .flex_col()
        .font_family(theme::active().typography.family.mono)
        .text_size(px(grid_font_px() * scale))
        // BUG A: line-height EXPLÍCITA = CELL_H. A natural da fonte do grid (~18-19px) é MAIOR que
        // CELL_H=17, então as 26 rows (`fit_dims`) ocupavam mais que o body do card e a ÚLTIMA linha (a
        // barra de input "❯ …" da TUI) era CLIPADA pelo `overflow_hidden`. Travando em CELL_H, as 26
        // rows cabem exatamente (== o que `fit_dims` calcula) → o input APARECE.
        .line_height(px(cell_h() * scale))
        .children((0..screen.rows).map(move |r| {
            let cursor_col = (cur.visible && cur.line == r).then_some(cur.col);
            render_line(screen.row(r), cursor_col, r, sel, runs_out)
        }))
}

/// Seleção de texto ativa (W2-4): em qual nó e o range de células `anchor → head` do viewport.
#[derive(Clone, Copy)]
struct Sel {
    node: NodeId,
    anchor: (usize, usize),
    head: (usize, usize),
}

// === W5-1 (instrumentação de FPS de render) ===========================================
// O gpui NÃO roda headless, então o FPS de render só sai NA TELA, com o app vivo. Instrumentamos o
// caminho de render do `WorkspaceView` no MESMO canal (stderr) das demais sondas (cards/pulso): mede
// o FRAMETIME como o intervalo entre dois `render` consecutivos — a cadência REAL na tela (incl. a
// espera de vsync), não o custo-de-CPU de montar a cena (que subestima: layout/paint/present do gpui
// vêm DEPOIS que `render` retorna). Custo desprezível: 1 subtração + 1 push por frame.

/// Janela de amostragem do frametime, em FRAMES. ~120 frames ≈ 1s a 120fps / ~2s a 60fps. Loga uma
/// linha `[FPS]` por janela cheia (ou ao atingir [`FPS_WINDOW_MS`] de render ativo — o que vier antes).
const FPS_WINDOW: usize = 120;
/// Teto de TEMPO ATIVO (ms) por janela. A soma do ring É o tempo de parede dos frames medidos (as
/// lacunas ociosas já foram descartadas), então logamos ao acumular ~2s de render ATIVO mesmo abaixo
/// de [`FPS_WINDOW`] frames — cobre o caso de FPS baixo sem segurar o relatório.
const FPS_WINDOW_MS: f64 = 2000.0;
/// Teto de intervalo entre frames. Acima disto NÃO é "frame lento": é OCIOSIDADE — o loop de animação
/// quiesce quando nada muda (`request_animation_frame` só se auto-sustenta enquanto chegam frames),
/// então o "intervalo" vira a duração da quietude. Descartado p/ não envenenar p50/p95/p99 (medimos
/// FPS *enquanto desenha*). 250ms = 4fps; abaixo disto é desenho contínuo real.
const IDLE_GAP_MS: f64 = 250.0;

// O percentil nearest-rank vive em [`prof::percentile_sorted`] (F1-5-1: fonte única — as
// sondas [FPS] e [PROF] usam a MESMA aritmética, testada headless lá).

/// O canvas gpui — a ÚNICA parte que conhece o toolkit. O estado dos nós (add/remove, model,
/// grids) vive no [`NodeManager`] gpui-free; a view só renderiza, roteia input e foca.
/// F3-1-7: gesto de ARRASTAR um card de Goal (overlay de tela, FORA do canvas/câmera — por isso não
/// reusa o [`canvas_drag::CardDrag`], que opera em coordenadas de mundo). Guarda a âncora do gesto
/// para o `on_mouse_move` do root computar o novo deslocamento (mouse atual − mouse inicial).
struct GoalDrag {
    goal_id: String,
    start_mouse: (f32, f32),
    start_offset: (f32, f32),
}

struct WorkspaceView {
    // SEAM-1: `Arc` — o `NodeManager` (funil de admissão) é COMPARTILHADO com a `MailboxPump`/
    // `AttentionHub` (threads gpui-free) para `SpawnApproved`/banner criarem terminais pelo MESMO
    // funil (ADR 0022). `admit_node` é `&self`/gpui-free (estado `Arc<Mutex>`) — sharing seguro.
    nodes: Arc<NodeManager>,
    input: Arc<dyn InputSink>,
    // `None` em produção (sem A/B semeados) → o botão ⚡ A2A nem renderiza. `Some` no demo (A→B).
    a2a: Option<Arc<A2aTrigger>>,
    focused: NodeId,
    focus: FocusHandle,
    /// Câmera 2D do canvas (pan + zoom). Transform world↔screen, culling e hit-test usam ela.
    camera: Camera,
    /// Arrasto do FUNDO (pan): `(mouse_inicial_em_tela, pan_inicial)` em px de tela.
    drag: Option<((f32, f32), (f32, f32))>,
    /// F2-3-1: gesto de MOVER um card (arrasto pela barra de título). `None` = sem gesto ativo.
    card_drag: Option<canvas::drag::CardDrag>,
    /// F3-1-7: cards de Goal RECOLHIDOS (por `goal_id`) — estado de sessão (não persiste no log).
    /// Recolhido = só cabeçalho + meta, para o card não cobrir o canvas.
    collapsed_goals: std::collections::HashSet<String>,
    /// F3-1-7: arrasto em curso de um card de Goal (`None` = sem gesto).
    goal_drag: Option<GoalDrag>,
    /// F3-1-7: deslocamento (px de tela) de cada card de Goal pela posição base, por `goal_id` —
    /// estado de sessão (o usuário arrastou o card para fora do caminho; não persiste no log).
    goal_offsets: std::collections::HashMap<String, (f32, f32)>,
    /// F3-1-7: editor inline do entendimento de uma Goal ("Quero ajustar") — `Some((goal_id, buffer))`
    /// enquanto o humano DIGITA a correção. Enter re-envia `goal.interpret` pelo canal humano; Esc fecha.
    editing_goal: Option<(String, String)>,
    /// F3-1-7: metas FECHADAS (dispensadas) do canvas pelo usuário — DURÁVEL (espelha os settings). O
    /// card não é mostrado; a meta segue no log (preferência de visualização). Carregado no boot.
    dismissed_goals: std::collections::HashSet<String>,
    /// Z-order (profundidade) por nó: maior = mais à frente. O focado é bombeado ao topo.
    z_order: BTreeMap<NodeId, u64>,
    z_next: u64,
    /// W2-4: seleção de texto atual (destacada no render; copiada no ⌘C).
    sel: Option<Sel>,
    /// `true` enquanto um arraste de seleção está em curso.
    dragging_sel: bool,
    /// `Some(node)` enquanto um "press" de mouse reporting está ativo nesse nó (motion/release).
    report_node: Option<NodeId>,
    /// W3-6c: mesa de custódia compartilhada com a [`BrokerPump`]. O banner do gate humano é lido
    /// daqui; ⌘⏎ sinaliza a confirmação (tecla na janela = gate inforjável pelo PTY do agente).
    desk: Desk,
    /// F1-2-2 · M6/M6-E: o modal de criar/editar Agente (evoluiu o modo-nomeação M2 da Fase 0).
    /// `Some` = aberto (teclado vai pro modal; eventos SÓ no Criar/Salvar).
    agent_modal: Option<agent_modal::AgentModal>,
    /// F1-2-2: resultado da varredura de motores (descoberta W4-1 roda FORA da thread de UI e
    /// deposita aqui; o loop do modal o consome — lição: discovery off-thread + injetável).
    modal_scan: Option<Arc<Mutex<Option<Vec<agent_modal::Engine>>>>>,
    /// F4-0-2 (UI): o modal de upload de credencial de canal. `Some` = aberto (teclado vai pro modal;
    /// o segredo só sai no Guardar, rumo ao cofre; some da tela ao fechar).
    credential_modal: Option<credential_modal::CredentialModal>,
    /// F4-WA-2c (UI): o modal "Receber um aviso de fora". `Some` = aberto (teclado vai pro modal). O
    /// endereço/senha gerados pelo servidor chegam por `webhook_present_outcome` e somem ao fechar.
    webhook_modal: Option<webhook_modal::WebhookModal>,
    /// F4-0-5: cache da projeção de exposição (`(event_count, ExposureState)`) — reprojetada SÓ
    /// quando o log muda (disciplina `projection_cache_is_stale`, igual ao cache de goals).
    exposure_cache: Option<(u64, exposure::ExposureState)>,
    /// FIX-3: nível de autonomia EFETIVO do Espaço ([`workspace_autonomy`] — a mesma fonte do
    /// wiring/guard). Alimenta o badge do card e o prefill da seção AUTONOMIA do modal; vira
    /// por-Agente quando a costura do funil (bridge) carimbar `LINA_AUTONOMY` por nó.
    ws_autonomy: Autonomy,
    /// W4-3: FREIO da auto-orquestração, compartilhado com a [`MailboxPump`]. A UI lê `paused` p/ o
    /// rótulo do rodapé e sinaliza `toggle_requested` ao clicar.
    brake: wiring::Brake,
    /// W4-3: reduce-motion (a11y). `true` → o pulso A→B NÃO anima (a entrega ocorre igual). Toggle no
    /// rodapé; a W4-6 o inicializa do setting do SO/env (`a11y::os_reduce_motion`).
    reduce_motion: bool,
    /// W4-6: anunciador da live-region ("resposta pronta" 1×/turno na transição →Idle, não por byte).
    a11y_live: a11y::LiveRegion,
    /// W4-2 · M1: paleta de comandos (Cmd-K). Fechada por padrão.
    palette: palette::PaletteState,
    /// F1-1-5 (P6/fluxo c): painel "Atividade e custos" aberto? (toggle pela paleta).
    dashboard_open: bool,
    /// F1-1-5: fios vivos do dashboard (timeline de hooks + sessões + identidades).
    /// `Arc` (F1-4-4 fatia ii): re-apontado pelo switch para o dash do runtime ATIVO.
    dash: Arc<dashboard::DashWiring>,
    /// F1-4-4 fatia (ii): TODOS os runtimes do processo (fundos VIVOS) + qual está ativo.
    /// A view cacheia os handles do ativo (campos acima); o switch os re-aponta.
    runtimes: runtime::Runtimes,
    /// Infra por-processo (PTY/fábrica/dims) para montar runtime sob demanda na troca.
    shared: runtime::SharedInfra,
    /// F1-4-4 · M8: o rail de Espaços (estado headless + render — T4) com os callbacks fiados.
    sidebar: sidebar::Sidebar,
    /// F1-4-4 · M8: cache do mini-status por Espaço (projeção 1× por abertura do rail — T2).
    mini_cache: dashboard::MiniStatusCache,
    /// F1-4-4 · M9: modal Criar Espaço (estado headless — T5). `Some` = aberto.
    create_space_modal: Option<gallery::CreateSpaceModal>,
    /// Spec M8 (a11y): arquivamento PENDENTE com `[ Desfazer ]` — o `WorkspaceArchived` só
    /// é gravado quando a janela do toast fecha (commit adiado; desfazer = cancelar).
    archive_toast: Option<sidebar::ArchiveToast>,
    /// F1-1-5 (sonda `[DASH]`): último `ts` de hook reportado por nó — loga a latência
    /// evento→card 1× por evento (na MUDANÇA), sem spammar o stderr a 60fps.
    dash_lat_reported: BTreeMap<String, u64>,
    /// W4-2 · M3/M4: `Some((kind, buffer))` enquanto o humano DIGITA o título da nota/pasta a criar
    /// (disparado pela paleta). Enter cria + foca; Esc cancela. `None` = fora do modo criação.
    creating: Option<(creators::CreatorKind, String)>,
    /// BUG A/B (instrumentação): última linha de diagnóstico de cards logada — loga só na MUDANÇA
    /// (rows/zona/dim), sem spammar o stderr a 60fps. O Maestro lê o stderr p/ validar por DADOS.
    diag_last_frame: String,
    /// PULSO (instrumentação): último diagnóstico do pulso A→B logado — loga só na MUDANÇA de estado
    /// dos gates (reduce-motion, id∈cards, progress), sem spammar a 60fps. Revela por DADOS qual gate
    /// corta a animação no caminho do `lina ask` real (o `eprintln` do nascimento fica na ponte).
    diag_last_pulse: String,
    /// W2-4 (dívida IME): texto MARCADO (pre-edit) da composição em curso — ex. o "´" pendente entre
    /// a dead-key e a vogal. Rastreado só p/ o protocolo `NSTextInputClient` (o SO pergunta a faixa
    /// marcada); NÃO é renderizado inline (a TUI não tem onde mostrá-lo). `None` = fora de composição.
    ime_marked: Option<String>,
    /// W5-1 (instrumentação de FPS): instante do ÚLTIMO `render`. O FRAMETIME é o intervalo até o
    /// próximo frame — a cadência REAL na tela. `Instant` monotônico (NUNCA `SystemTime`). `None` no
    /// 1º frame (ainda sem intervalo a medir).
    last_frame_at: Option<Instant>,
    /// W5-1: ring das últimas amostras de frametime (ms) p/ os percentis. Esvazia a cada janela
    /// (~[`FPS_WINDOW`] frames OU ~[`FPS_WINDOW_MS`] de render ativo). Lacunas de OCIOSIDADE (intervalo
    /// > [`IDLE_GAP_MS`], sem redraw) são descartadas p/ não envenenar p50/p95/p99.
    frametime_ms: Vec<f64>,
    /// F1-1-7: fiação da Fila de Atenção (projeção `AttentionQueue` do core ↔ log ↔ desk).
    attention: Arc<AttentionHub>,
    /// F1-1-7: snapshot da fila do frame (sincronizado pelo heartbeat — ordenação do CORE).
    attention_items: Vec<lina_core::AttentionItem>,
    /// F1-1-7: memória do toast (countdown/«Depois»/snooze do colapsado) — gpui-free, testada.
    attention_machine: attention_ui::ToastMachine,
    /// F1-1-7: painel compacto da fila aberto? (badge 🔔 / ⌘J).
    attention_panel_open: bool,
    /// F1-1-7: instante do último lembrete sonoro (1×/30s enquanto fila >0).
    attention_sound_last: Option<Instant>,
    /// F1-1-7: cache do mute persistido (settings.json — recarregado pelo heartbeat).
    attention_sound_muted: bool,
    /// F1-1-7: último refresh do cache do mute (re-lê ~2s quando a fila está viva).
    attention_settings_at: Option<Instant>,
    /// F1-1-7 (sonda `[ATT]`): último diagnóstico da fila logado — loga só na MUDANÇA
    /// (items/toast/badge), sem spammar o stderr (idioma diag_last_frame/pulse). O
    /// Maestro valida por DADOS o que está na tela sem precisar vê-la.
    attention_diag_last: String,
    /// F1-1-7: onde settings.json mora (o MESMO dir do event log — padrão T7).
    settings_dir: PathBuf,
    /// F1-5-1: sonda [PROF] (decomposição do frametime). `LINA_PROF=1` liga; desligada, o
    /// custo por frame é UM check de bool (o protocolo de overhead compara o [FPS] 0×1).
    prof: prof::Probe,
    /// F3-1-7: cache das Goals VIVAS do painel — `(event_count, goals, iteration_budget)`. As Goals
    /// e o budget (`goal_max_iterations` dos `SystemParams`) são META do log (fora da projeção do
    /// canvas), então o painel os reconstrói varrendo o log com a MESMA disciplina de cache do
    /// custo/effort: re-varre SÓ com evento novo (`projection_cache_is_stale`), NUNCA por frame.
    goals_cache: Option<(u64, Vec<lina_core::Goal>, u32)>,
    /// F3-3-5: cache do painel "Como o [papel] pensa" — `(event_count, mentalidades por papel)`. A
    /// projeção `Mentality` é reconstruída por REPLAY (O(N)); como `render` roda por frame, re-projeta
    /// SÓ com evento novo (`projection_cache_is_stale`), NUNCA por frame — mesma disciplina do
    /// custo/effort/goals (fix do freeze do `lina ask`: store ocupado mantém o cache).
    mentality_cache: Option<(u64, Vec<mentality_panel::RoleMentality>)>,
}

impl WorkspaceView {
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)] // construtor único do root; os fios entram aqui.
    fn new(
        nodes: Arc<NodeManager>,
        input: Arc<dyn InputSink>,
        a2a: Option<Arc<A2aTrigger>>,
        focused: NodeId,
        desk: Desk,
        brake: wiring::Brake,
        dash: Arc<dashboard::DashWiring>,
        attention: Arc<AttentionHub>,
        settings_dir: PathBuf,
        runtimes: runtime::Runtimes,
        shared: runtime::SharedInfra,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // W4-3 FIX (pulso A→B do `lina ask`): o pulso nasce numa thread gpui-free (MailboxPump/
        // `deliver_fn` e a ProjectionPump), que NÃO acorda o event loop do gpui. Sem input de UI
        // recente, o `request_animation_frame` (que só se auto-sustenta enquanto CHEGAM frames) não
        // roda → o pulso fica parado (`progressing=false`, provado pela instrumentação) e some
        // invisível. Este loop de animação dirige re-renders ENQUANTO houver pulso vivo: detecta o
        // nascimento por poll BARATO do model (sem render) e força `cx.notify()` a ~60fps até
        // `Pulse::progress()` expirar (~1100ms). Cobre os DOIS caminhos (lina-ask e Bus) sem tocar a
        // ponte gpui-free e sem quebrar o caminho do botão A2A.
        // F1-1-5 (wiring): heartbeat do painel "Atividade e custos" — timeline/sessões
        // mudam em threads gpui-free (não acordam o event loop); enquanto o painel está
        // aberto, ~2 ticks/s marcam dirty (lição: animação precisa de dirty). Barato
        // quando fechado (1 check booleano a cada 500ms).
        cx.spawn(async move |this, cx| loop {
            if this
                .update(cx, |view, cx| {
                    if view.dashboard_open {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(alive) = this.update(cx, |view, cx| {
                    let alive = lock(&view.nodes.model)
                        .pulse
                        .is_some_and(|p| p.progress().is_some());
                    if alive {
                        cx.notify(); // marca dirty → o frame re-renderiza e anima o dot
                    }
                    alive
                }) else {
                    break; // a view foi dropada (app fechando) → encerra o loop.
                };
                cx.background_executor()
                    .timer(Duration::from_millis(if alive { 16 } else { 50 }))
                    .await;
            }
        })
        .detach();
        // F1-1-7: HEARTBEAT da Fila de Atenção — a fila muda em threads gpui-free
        // (detector/pumps NÃO acordam o event loop; lição: animação precisa de dirty).
        // Ritmo adaptativo: ~30fps com toast/pulso vivos (countdown/badge suaves);
        // ~4Hz de dados quando só há pendência parada; o sync re-replaya SÓ quando o
        // event_count mudou (padrão CostLedger — barato no ocioso).
        cx.spawn(async move |this, cx| loop {
            let Ok(animating) = this.update(cx, |view, cx| view.attention_heartbeat(cx)) else {
                break;
            };
            cx.background_executor()
                .timer(Duration::from_millis(if animating { 33 } else { 250 }))
                .await;
        })
        .detach();
        // F4-WA-2c (UI) — POLL do RETORNO do webhook: o `(url,secret)` nasce na thread do servidor
        // (handler gpui-free de `lina-webhooks`, F4-WA-1), que NÃO acorda o event loop do gpui nem
        // alcança o `Context` da View (main-thread). Este loop drena o retorno na main-thread (via o
        // acessor `take_webhook_outcome` do `NodeManager`) e o injeta no modal aberto. **Take-once:** o
        // `(url,secret)` é efêmero (consumido 1×, NUNCA logado/`{:?}` — o secret tem `Debug` redigido).
        // Cadência adaptativa: rápida enquanto o modal aguarda (Pending), ociosa de resto (um
        // lock+Option é barato no ocioso, padrão dos heartbeats acima).
        cx.spawn(async move |this, cx| loop {
            let Ok(pending) = this.update(cx, |view, cx| view.drain_webhook_outcome(cx)) else {
                break;
            };
            cx.background_executor()
                .timer(Duration::from_millis(if pending { 50 } else { 400 }))
                .await;
        })
        .detach();
        let mut view = Self {
            nodes,
            input,
            a2a,
            focused,
            focus,
            camera: Camera::default(),
            drag: None,
            card_drag: None,
            collapsed_goals: std::collections::HashSet::new(),
            goal_drag: None,
            goal_offsets: std::collections::HashMap::new(),
            editing_goal: None,
            // F3-1-7: restaura as metas que o usuário fechou em sessões anteriores (durável).
            dismissed_goals: persistence_ui::load_settings(&settings_dir)
                .dismissed_goals
                .into_iter()
                .collect(),
            z_order: BTreeMap::new(),
            z_next: 0,
            sel: None,
            dragging_sel: false,
            report_node: None,
            desk,
            agent_modal: None,
            modal_scan: None,
            credential_modal: None,
            webhook_modal: None,
            exposure_cache: None,
            ws_autonomy: workspace_autonomy(),
            brake,
            // W4-6 gap3: este campo é o OVERRIDE do usuário (rodapé); o SO/env entra via
            // `a11y::reduce_motion_effective` (fonte única) no pulso/rótulo. Começa sem override.
            reduce_motion: false,
            a11y_live: a11y::LiveRegion::default(),
            palette: palette::PaletteState::default(),
            // LINA_DASH=1 abre o painel no BOOT (validação por dados/roteiro do fundador,
            // mesmo idioma de LINA_DEMO). Produção: fechado; abre pela paleta (Cmd+K).
            dashboard_open: matches!(
                std::env::var("LINA_DASH").ok().as_deref().map(str::trim),
                Some("1")
            ),
            dash,
            dash_lat_reported: BTreeMap::new(),
            creating: None,
            diag_last_frame: String::new(),
            diag_last_pulse: String::new(),
            ime_marked: None,
            last_frame_at: None,
            frametime_ms: Vec::with_capacity(FPS_WINDOW),
            attention,
            attention_items: Vec::new(),
            attention_machine: attention_ui::ToastMachine::default(),
            attention_panel_open: false,
            attention_sound_last: None,
            attention_sound_muted: false,
            attention_settings_at: None,
            attention_diag_last: String::new(),
            settings_dir,
            runtimes,
            shared,
            // F1-4-4 · M8: o rail nasce com os 7 callbacks da integração fiados — cliques do
            // componente despacham para os métodos da view (o componente não se auto-muta).
            sidebar: sidebar::Sidebar::new(
                std::rc::Rc::new(|v: &mut Self, root: &Path, _w, cx| {
                    v.switch_to_workspace(root.to_path_buf(), cx);
                    v.refresh_sidebar_rows();
                }),
                std::rc::Rc::new(|v: &mut Self, _w, cx| v.open_create_space_modal(cx)),
                std::rc::Rc::new(|v: &mut Self, root: &Path, novo: &str, _w, cx| {
                    v.rename_workspace(root, novo, cx);
                }),
                std::rc::Rc::new(|v: &mut Self, root: &Path, _w, cx| {
                    v.archive_workspace(root, cx);
                }),
                std::rc::Rc::new(|v: &mut Self, _w, cx| v.toggle_sidebar(cx)),
                std::rc::Rc::new(|v: &mut Self, _w, _cx| {
                    // Vista de arquivados: pendente da costura `WorkspaceUnarchived` (spec
                    // §6-C2) — degradação VISÍVEL no log, nunca silêncio.
                    eprintln!(
                        "lina-gpui: [WS] vista de arquivados ainda não fiada \
                         (aguarda WorkspaceUnarchived — spec §6-C2)"
                    );
                    let _ = v;
                }),
                std::rc::Rc::new(|v: &mut Self, root: &Path, _w, cx| {
                    // r5 BUG 2: a ação age na linha CLICADA (as ações agora são visíveis
                    // em toda linha — renomear pelo roving renomearia a errada).
                    v.sidebar.state.start_rename_for(root);
                    cx.notify();
                }),
            ),
            mini_cache: dashboard::MiniStatusCache::default(),
            create_space_modal: None,
            archive_toast: None,
            prof: prof::Probe::from_env(),
            goals_cache: None,
            mentality_cache: None,
        };
        // r5 · plug do Descarregar (contrato combinado C↔Core A2A): botão do rail →
        // executor `runtime::unload_workspace` (Busy preservado por re-checagem no seam);
        // narração leiga dos números REAIS + refresh. Com o callback plugado, a dupla
        // guarda do rail (`row_can_unload`) passa a exibir a ação nos Espaços de fundo.
        view.sidebar
            .set_on_unload(std::rc::Rc::new(|v: &mut Self, root: &Path, _w, cx| {
                match runtime::unload_workspace(&v.runtimes, root) {
                    Ok(o) => {
                        v.a11y_live
                            .announce(sidebar::copy_unload_narration(o.parked, o.kept));
                        v.refresh_sidebar_rows();
                    }
                    Err(e) => v.a11y_live.announce(e),
                }
                cx.notify();
            }));
        // F2-2-4 · plug da PORTA VISÍVEL da paleta: o botão rotulado do rail abre a MESMA paleta do
        // ⌘K. Sem este plug o botão nem aparece (Option None) — porta sem dono não se desenha.
        view.sidebar
            .set_on_open_palette(std::rc::Rc::new(|v: &mut Self, _w, cx| {
                v.open_command_palette(cx);
            }));
        view
    }

    /// F2-2-4 — abre a paleta de comandos (ponto ÚNICO: ⌘K e o botão do rail caem aqui). Rebuild dos
    /// comandos com o roster do momento + injeção do contexto de ranking (MRU já vive no estado da
    /// paleta entre aberturas; o hit-count é projeção do event log — costura `events.rs`).
    fn open_command_palette(&mut self, cx: &mut Context<Self>) {
        let cmds = self.palette_commands();
        // F2-2-4 (costura): MRU durável (settings.json) + hit-count como PROJEÇÃO do event
        // log (fold de `PaletteCommandInvoked` no abrir — ⌘K não é hot path; o modelo nunca
        // fabrica contagem, inv#4). Store ilegível ⇒ HashMap vazio (ranking degrada).
        let settings = persistence_ui::load_settings(&self.settings_dir);
        self.palette.set_mru(settings.palette_mru);
        let mut hits: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let active_root = lock(&self.runtimes).active.clone();
        let mounted = {
            let r = lock(&self.runtimes);
            r.map.get(&active_root).map(|rt| Arc::clone(&rt.store))
        };
        if let Some(store) = mounted {
            if let Ok(events) = lock(&store).events() {
                for rec in events {
                    // EventRecord é (kind + payload JSON) — filtra por kind e lê a key crua.
                    if rec.kind == "PaletteCommandInvoked" {
                        if let Some(key) = rec.payload.get("key").and_then(|v| v.as_str()) {
                            *hits.entry(key.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        self.palette.set_hits(hits);
        self.palette.open(cmds);
        cx.notify();
    }

    // ──────────────────── F1-4-4 · troca VIVA de Espaço (fatia ii do M8) ────────────────────

    /// Troca o Espaço ATIVO: monta o alvo sob demanda (boot + restore F1-4-3 dele), carimba o
    /// foco durável (log do alvo + ponteiro global) e re-aponta a view. Os PIDs do Espaço que
    /// sai NÃO são tocados (decisão do fundador: fundos VIVOS — pumps/PTYs/threads seguem).
    /// Falha NÃO troca nada: o Espaço atual segue na tela (inv#6).
    fn switch_to_workspace(&mut self, target_root: PathBuf, cx: &mut Context<Self>) {
        if lock(&self.runtimes).active == target_root {
            return;
        }
        let t0 = Instant::now();
        if let Err(e) =
            runtime::activate_workspace(&self.runtimes, target_root.clone(), &self.shared)
        {
            eprintln!(
                "lina-gpui: [WS] troca para {} falhou ({e}); permanecendo no Espaço atual",
                target_root.display()
            );
            return;
        }
        self.point_to_active();
        // F1-4-4 crit 1 (<1s): o log diz quanto a troca levou — validação por DADOS.
        eprintln!(
            "lina-gpui: [WS] troca para {} em {:?} (PIDs preservados — fundos vivos)",
            target_root.display(),
            t0.elapsed()
        );
        cx.notify();
    }

    /// Re-aponta os handles cacheados da view para o runtime ATIVO e re-deriva o estado de UI
    /// POR-Espaço (câmera/z-order/seleção/fila resetam; o render re-aponta o foco ao 1º card —
    /// "nunca tela em branco"). Ativo ausente do mapa = impossível por construção
    /// (`runtimes_with_boot`/`activate_workspace` inserem antes de apontar); degrada com log.
    fn point_to_active(&mut self) {
        {
            let r = lock(&self.runtimes);
            let Some(rt) = r.map.get(&r.active) else {
                eprintln!("lina-gpui: [WS] runtime ativo ausente do mapa — re-aponte abortado");
                return;
            };
            self.nodes = Arc::clone(&rt.nodes);
            self.input = rt.input.clone() as Arc<dyn InputSink>;
            self.desk = Arc::clone(&rt.desk);
            self.brake = Arc::clone(&rt.brake);
            self.dash = Arc::clone(&rt.dash);
            self.attention = Arc::clone(&rt.attention);
            self.settings_dir = rt.mailbox_dir.join("events");
            self.ws_autonomy = rt.autonomy;
        }
        // Estado de UI por-Espaço — re-deriva do runtime novo:
        self.a2a = None; // o ⚡ demo pertence ao Espaço semeado; fora dele, some (gate ready()).
        self.focused = NodeId::default();
        self.camera = Camera::default();
        self.drag = None;
        self.sel = None;
        self.dragging_sel = false;
        self.report_node = None;
        self.z_order.clear();
        self.z_next = 0;
        self.agent_modal = None;
        self.modal_scan = None;
        self.creating = None;
        self.attention_items.clear();
        self.attention_machine = attention_ui::ToastMachine::default();
        self.attention_settings_at = None;
        self.dash_lat_reported.clear();
    }

    // ──────────────── F1-4-4 · M8/M9: fiação da sidebar + criar Espaço (integração) ────────────────

    /// Abre/recolhe o rail; ABRIR recarrega linhas + mini-status (1× por abertura — T2 §6-B5).
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if !self.sidebar.state.expanded {
            self.refresh_sidebar_rows();
            self.sidebar.state.open();
            // r5: anunciado na live-region (consistência a11y) — e o anúncio ensina os
            // gestos, já que abrir NÃO move o foco (BUG 4(e)).
            self.a11y_live
                .announce("Lista de Espaços aberta — ⌘O recolhe; ⌘F busca.");
        } else {
            self.sidebar.state.close();
            self.a11y_live.announce("Lista de Espaços recolhida.");
        }
        cx.notify();
    }

    /// Linhas do rail: registry canônico (ordem de criação = ⌘n estável) + varredura T6 como
    /// fallback de recuperação + mini-status honesto por Espaço (cache com staleness declarada).
    fn refresh_sidebar_rows(&mut self) {
        let now_ms = lina_core::now_ms();
        let active_root = lock(&self.runtimes).active.clone();
        let registry = lina_core::default_registry_path()
            .and_then(|p| lina_core::WorkspaceRegistry::load(p).ok());
        let mut entries: Vec<lina_core::WorkspaceEntry> = registry
            .as_ref()
            .map(|r| r.entries().to_vec())
            .unwrap_or_default();
        // Arquivamento PENDENTE (janela do Desfazer): a linha já sai da lista padrão como se
        // arquivada — marcado na CÓPIA local (o registry só muda no commit), preservando a
        // posição do `⌘{n}` (arquivado segura a posição — contrato T4 §4).
        if let Some(t) = &self.archive_toast {
            for e in &mut entries {
                if e.path == t.root {
                    e.archived = true;
                }
            }
        }
        let scan = active_root
            .parent()
            .map(|base| {
                persistence_ui::list_workspaces(base, &sidebar::events_dir_of(&active_root))
            })
            .unwrap_or_default();
        // Chaves do cache = MESMA derivação do build_rows (contrato T4 §2 — byte a byte).
        let dirs: Vec<PathBuf> = entries
            .iter()
            .map(|e| sidebar::events_dir_of(&e.path))
            .chain(scan.iter().map(|s| s.events_dir.clone()))
            .collect();
        let sessions = lock(&self.dash.sessions).clone();
        let today = dashboard::utc_day_prefix(now_ms);
        self.mini_cache.refresh(&dirs, &sessions, &today, now_ms);
        self.sidebar.state.set_rows(sidebar::build_rows(
            &entries,
            &scan,
            &active_root,
            &self.mini_cache,
        ));
        // F1-4-6: o selo «PRO» do `+ Novo Espaço…` segue o gate REAL da licença
        // (vitrine §1a ANTES do esforço — o stub `Pro` morreu nesta fatia).
        let active = registry
            .as_ref()
            .map_or(0, lina_core::WorkspaceRegistry::active_count);
        self.sidebar.create_blocked = self.license_gate_blocked_copy(active).is_some();
    }

    /// F1-4-6: o GATE REAL de criação — veredito do `lina-license` (assinatura ed25519
    /// local; free=1/PRO=N data-driven) traduzido na linha-fato da vitrine 1b. `None` =
    /// pode criar. Carrega o estado do disco a cada PONTO de gating (critério 7 de
    /// F1-4-5: o relógio é avaliado aqui, nunca num timer).
    fn license_gate_blocked_copy(&self, active_count: usize) -> Option<String> {
        let state = lina_license::LicenseState::load_default();
        match state.can_create_workspace(active_count as u32, license_ui::now_epoch_s()) {
            lina_license::WorkspaceGate::Allowed => None,
            lina_license::WorkspaceGate::Blocked { limit, reason, .. } => {
                let expiry = state.effective(license_ui::now_epoch_s()).expiry;
                Some(license_ui::blocked_copy(reason, limit, expiry))
            }
        }
    }

    /// Abre o M9 — o gating roda NA ABERTURA (antes do esforço, spec §3) com o gate
    /// REAL da licença (F1-4-6); bloqueado ⇒ vitrine 1b com o campo de colar a chave.
    fn open_create_space_modal(&mut self, cx: &mut Context<Self>) {
        let registry = lina_core::default_registry_path()
            .and_then(|p| lina_core::WorkspaceRegistry::load(p).ok());
        let active = registry
            .as_ref()
            .map_or(0, lina_core::WorkspaceRegistry::active_count);
        let existing: Vec<String> = registry
            .as_ref()
            .map(|r| r.entries().iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default();
        let default_cwd = {
            let s = persistence_ui::load_settings(&self.settings_dir);
            let trimmed = s.default_cwd.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        };
        let blocked = self.license_gate_blocked_copy(active);
        self.create_space_modal = Some(gallery::CreateSpaceModal::new(
            blocked,
            default_cwd,
            existing,
        ));
        // a11y (spec §4): nascer bloqueado (vitrine 1b) já é anunciado — o foco nasce no upsell.
        self.m9_announce();
        cx.notify();
    }

    /// `[[ Ativar ]]` da vitrine 1b (M9): valida a chave colada 100% offline e, em
    /// sucesso, DESTRAVA o modal sem restart (colar → confirmar → criar, ≤3 interações).
    /// Erro fica inline no campo + live-region (critério 5). Side effect único: gravar
    /// `~/.lina/license.json` via `lina_license::activate`.
    fn m9_activate_key(&mut self, cx: &mut Context<Self>) {
        let Some(m) = self.create_space_modal.as_mut() else {
            return;
        };
        let path = lina_license::default_license_path().unwrap_or_default();
        let activated = m.key.activate(
            &path,
            &lina_license::Verifier::official(),
            license_ui::now_epoch_s(),
        );
        if let Some(msg) = m.key.announcement() {
            self.a11y_live.announce(msg);
        }
        if activated {
            if let Some(m) = self.create_space_modal.as_mut() {
                m.unblock_after_activation();
            }
            // O selo «PRO» do rail e a vitrine seguem o estado novo imediatamente.
            self.refresh_sidebar_rows();
        } else if let Some(m) = self.create_space_modal.as_mut() {
            // E1: foco volta ao campo (saída da tabela de erros); E2/E3/E4 mantêm o texto.
            m.focus = gallery::M9Focus::KeyField;
        }
        cx.notify();
    }

    /// Spec §4 (M9 a11y): lê o `announcement()` do modal para o FOCO corrente (erro do
    /// Diretório ao focar o campo; vitrine inteira no upsell) e o manda à live-region —
    /// é a fiação que o T5 deixou pendente (.entrega-m8-t5 §contrato item 2).
    fn m9_announce(&mut self) {
        if let Some(msg) = self
            .create_space_modal
            .as_ref()
            .and_then(gallery::CreateSpaceModal::announcement)
        {
            self.a11y_live.announce(msg);
        }
    }

    /// `[ Procurar… ]` do M9 — picker nativo só-pastas (mesmo padrão do M6); a validação
    /// na SELEÇÃO acontece dentro de `set_workdir_picked` (T5).
    fn m9_pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Usar esta pasta".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update(cx, |view, cx| {
                        if let Some(m) = view.create_space_modal.as_mut() {
                            m.set_workdir_picked(&p);
                            // a11y: a validação NA SELEÇÃO falhou → anuncia já (o foco ainda
                            // está no [ Procurar… ], então `announcement()` não cobriria).
                            if let Some(e) = m.error.as_ref() {
                                view.a11y_live.announce(e.message.clone());
                            }
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// `[[ Criar Espaço ]]` — cria via seam T3 e TROCA para o novo (aterrissagem povoada,
    /// nunca tela em branco — spec §2.3). Erro/bloqueio fica INLINE (modal segue aberto).
    fn m9_submit(&mut self, cx: &mut Context<Self>) {
        let Some(mut m) = self.create_space_modal.take() else {
            return;
        };
        let Some(parent) = lock(&self.runtimes).active.parent().map(Path::to_path_buf) else {
            eprintln!("lina-gpui: [WS] raiz ativa sem diretório-pai — criação abortada");
            return;
        };
        let Some(mut registry) = lina_core::default_registry_path()
            .and_then(|p| lina_core::WorkspaceRegistry::load(p).ok())
        else {
            eprintln!("lina-gpui: [WS] ponteiro global indisponível — criação abortada");
            self.create_space_modal = Some(m);
            return;
        };
        // F1-4-6: re-check de corrida do submit com o gate REAL (outro processo pode
        // ter criado um Espaço entre abrir e submeter) — Allowed→Pro, Blocked→Free.
        // Nota honesta: PRO com teto finito N>1 projeta em `Pro` (ilimitado no core);
        // o teto data-driven é garantido pelo gate de ABERTURA, que é o do produto.
        let tier = if self
            .license_gate_blocked_copy(registry.active_count())
            .is_none()
        {
            lina_core::LicenseTier::Pro
        } else {
            lina_core::LicenseTier::Free
        };
        let mut created: Option<PathBuf> = None;
        let ok = m.submit(&parent, &mut registry, lina_core::now_ms(), tier, |root| {
            created = Some(root)
        });
        if ok {
            if let Some(root) = created {
                // F1-4-4 crit 2: criado → o foco vai pro Espaço NOVO já povoado pelo preset.
                self.switch_to_workspace(root, cx);
                self.refresh_sidebar_rows();
            }
        } else {
            self.create_space_modal = Some(m); // erro de diretório/vitrine — inline, sem beco
            self.m9_announce(); // a11y: o submit falho focou o campo do erro → anuncia.
        }
        cx.notify();
    }

    /// Teclado do M9 (spec §4): Tab/Shift-Tab percorre na ordem da spec (anúncio via
    /// aria-label do elemento focado); ←→/↑↓ navegam os cards de Foco; imprimível digita
    /// no campo focado; **Enter SUBMETE sempre** (exceto com foco no `[ Procurar… ]`, que
    /// abre o picker — é o "ativar o controle focado"); Esc fecha sem criar (estado é RAM).
    fn m9_handle_key(&mut self, ks: &gpui::Keystroke, cx: &mut Context<Self>) {
        match ks.key.as_str() {
            "escape" => {
                self.create_space_modal = None;
            }
            "enter" | "return" => {
                // F1-4-6: na vitrine bloqueada (1b), Enter ATIVA o controle focado
                // (expandir campo / Ativar / abrir o site / fechar) — não há form a
                // submeter. Fora dela, vale a spec §4: Enter SUBMETE sempre.
                let upsell = self
                    .create_space_modal
                    .as_ref()
                    .map_or(gallery::UpsellAction::None, |m| m.upsell_enter());
                match upsell {
                    gallery::UpsellAction::OpenKeyField => {
                        if let Some(m) = self.create_space_modal.as_mut() {
                            m.open_key_field();
                        }
                        self.m9_announce();
                    }
                    gallery::UpsellAction::Activate => self.m9_activate_key(cx),
                    gallery::UpsellAction::OpenStore => {
                        // Único toque externo do fluxo: navegador padrão, por gesto
                        // explícito (inv#2) — o app em si nunca faz requisição.
                        cx.open_url(license_ui::STORE_URL);
                    }
                    gallery::UpsellAction::Close => {
                        self.create_space_modal = None;
                    }
                    gallery::UpsellAction::None => {
                        let focus_on_browse = self
                            .create_space_modal
                            .as_ref()
                            .is_some_and(|m| matches!(m.focus, gallery::M9Focus::Browse));
                        if focus_on_browse {
                            self.m9_pick_folder(cx);
                        } else {
                            self.m9_submit(cx);
                        }
                    }
                }
            }
            "tab" => {
                if let Some(m) = self.create_space_modal.as_mut() {
                    if ks.modifiers.shift {
                        m.focus_prev();
                    } else {
                        m.focus_next();
                    }
                }
                // a11y (spec §4): após mover o foco, o announcement do alvo novo vai à
                // live-region — é assim que o erro do Diretório é FALADO ao focar o campo.
                self.m9_announce();
            }
            "left" | "up" => {
                if let Some(m) = self.create_space_modal.as_mut() {
                    if matches!(m.focus, gallery::M9Focus::Cards) {
                        m.select_prev_preset();
                    }
                }
            }
            "right" | "down" => {
                if let Some(m) = self.create_space_modal.as_mut() {
                    if matches!(m.focus, gallery::M9Focus::Cards) {
                        m.select_next_preset();
                    }
                }
            }
            "backspace" => {
                if let Some(m) = self.create_space_modal.as_mut() {
                    m.backspace();
                }
            }
            _ => {
                if !ks.modifiers.platform && !ks.modifiers.control {
                    if let (Some(m), Some(ch)) =
                        (self.create_space_modal.as_mut(), ks.key_char.as_deref())
                    {
                        m.type_char(ch);
                    }
                }
            }
        }
    }

    /// Casca de render do M9 (T5 entregou o ESTADO headless; a casca é fiação da
    /// integração). Form: cards de Foco → Nome (+aviso de duplicado) → Diretório de
    /// Trabalho (+erro com a saída POR variante) → Procurar/Criar/Cancelar. `blocked`
    /// troca o form pela vitrine §1a (free=1). Cores 100% `theme::active()` (lint F1-2-1);
    /// o anel de foco segue `m.focus` (o teclado dirige — `m9_handle_key`).
    fn render_create_space(
        &self,
        m: &gallery::CreateSpaceModal,
        th: &theme::Theme,
        viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use gallery::M9Focus;
        let w = 560.0_f32;
        let x = (f32::from(viewport.width) - w) / 2.0;
        let ring = |on: bool| if on { th.focus.ring } else { th.surface.border };
        let field = |label: &str, value: &str, focused: bool, note: Option<(String, u32)>| {
            let mut f = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(th.text.secondary))
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(ring(focused)))
                        .bg(rgb(th.terminal.bg))
                        .text_sm()
                        .text_color(rgb(th.text.primary))
                        .child(if value.is_empty() {
                            " ".to_string()
                        } else {
                            value.to_string()
                        }),
                );
            if let Some((texto, cor)) = note {
                f = f.child(div().text_xs().text_color(rgb(cor)).child(texto));
            }
            f
        };
        // Clamp à janela: sem teto o form cresce além do viewport e os botões somem
        // (mesma lição do bugB2 em `agent_modal.rs`); excedente ROLA dentro do painel.
        let max_h = (f32::from(viewport.height) - 96.0 - 24.0).max(160.0);
        let mut panel = div()
            .id("m9-panel")
            .absolute()
            .left(px(x.max(60.0)))
            .top(px(96.0))
            .w(px(w))
            .max_h(px(max_h))
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(rgb(th.surface.border))
            .bg(rgb(th.surface.panel))
            .child(
                div()
                    .text_lg()
                    .text_color(rgb(th.text.bright))
                    .child("Criar Espaço"),
            );
        if let Some(blocked) = &m.blocked {
            // F1-4-6 — vitrine 1b NO LUGAR do form (limite ANTES do esforço): título +
            // explicação honesta (copy §1b congelada), a linha-fato do motivo, «Já tenho
            // uma chave» expandindo o campo ALI MESMO, e a saída de compra no navegador.
            let mut card = div()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(rgb(ring(matches!(m.focus, M9Focus::Upsell))))
                .bg(rgb(th.surface.raised))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(th.text.bright))
                        .child(license_ui::COPY_BLOCK_TITLE),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(th.text.primary))
                        .child(license_ui::COPY_BLOCK_BODY),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(th.text.muted))
                        .child(blocked.clone()),
                );
            if m.key_field_open {
                // Campo de colar (rótulo a11y «Sua Chave do Lina PRO») + [[ Ativar ]].
                let shown = if m.key.input.is_empty() {
                    license_ui::COPY_KEY_PLACEHOLDER.to_string()
                } else {
                    format!("{}▌", m.key.input)
                };
                let shown_fg = if m.key.input.is_empty() {
                    th.text.muted
                } else {
                    th.text.bright
                };
                card = card
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(th.text.secondary))
                            .child(license_ui::COPY_KEY_LABEL),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_2()
                            .border_color(rgb(ring(matches!(m.focus, M9Focus::KeyField))))
                            .bg(rgb(th.surface.card))
                            .text_sm()
                            .text_color(rgb(shown_fg))
                            .child(shown),
                    )
                    .child(
                        div()
                            .id("m9-key-activate")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_2()
                            .border_color(rgb(ring(matches!(m.focus, M9Focus::Activate))))
                            .bg(rgb(th.accent.action))
                            .text_color(rgb(th.text.on_accent))
                            .cursor_pointer()
                            .on_click(cx.listener(|v, _ev: &gpui::ClickEvent, _w, cx| {
                                v.m9_activate_key(cx);
                            }))
                            .child(format!("[[ {} ]]", license_ui::COPY_KEY_ACTIVATE)),
                    );
                // Erro acionável inline (E1–E4) — campo mantém o texto para corrigir.
                if let Some(license_ui::KeyFeedback::Error { message, renewable }) = &m.key.feedback
                {
                    card = card.child(
                        div()
                            .text_sm()
                            .text_color(rgb(th.state.warning))
                            .child(message.clone()),
                    );
                    if *renewable {
                        card = card.child(
                            div()
                                .text_xs()
                                .text_color(rgb(th.text.muted))
                                .child(license_ui::COPY_RENEW_NOTE),
                        );
                    }
                }
            } else {
                card = card.child(
                    div()
                        .id("m9-key-toggle")
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_2()
                        .border_color(rgb(ring(matches!(m.focus, M9Focus::KeyToggle))))
                        .bg(rgb(th.surface.raised_alt))
                        .text_color(rgb(th.text.bright))
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _ev: &gpui::ClickEvent, _w, cx| {
                            if let Some(m) = v.create_space_modal.as_mut() {
                                m.open_key_field();
                            }
                            v.m9_announce();
                            cx.notify();
                        }))
                        .child(format!("[[ {} ]]", license_ui::COPY_BLOCK_HAVE_KEY)),
                );
            }
            card = card
                .child(
                    div()
                        .id("m9-key-buy")
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_2()
                        .border_color(rgb(ring(matches!(m.focus, M9Focus::Buy))))
                        .bg(rgb(th.surface.raised))
                        .text_color(rgb(th.accent.action))
                        .cursor_pointer()
                        .on_click(cx.listener(|_v, _ev: &gpui::ClickEvent, _w, cx| {
                            cx.open_url(license_ui::STORE_URL);
                        }))
                        .child(format!("[ {} ]", license_ui::COPY_BLOCK_BUY)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(th.text.muted))
                        .child(license_ui::COPY_BLOCK_NOTE),
                )
                // «Agora não» (§1b): a saída neutra — fecha sem criar, nada insiste.
                .child(
                    div()
                        .id("m9-key-not-now")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .border_2()
                        .border_color(rgb(ring(matches!(m.focus, M9Focus::Cancel))))
                        .text_color(rgb(th.text.muted))
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _ev: &gpui::ClickEvent, _w, cx| {
                            v.create_space_modal = None;
                            cx.notify();
                        }))
                        .child(format!("[ {} ]", license_ui::COPY_BLOCK_NOT_NOW)),
                );
            panel = panel.child(card);
        } else {
            // F1-4-6: chave acabou de ativar NESTE modal → banner discreto «o que mudou»
            // sobre o form já destravado (sem restart; sem tela extra no caminho ≤3).
            if let Some(license_ui::KeyFeedback::Success {
                changed,
                valid_until,
            }) = &m.key.feedback
            {
                let mut ok = div()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(th.state.success))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(th.state.success))
                            .child(license_ui::COPY_SUCCESS_TITLE),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(th.text.secondary))
                            .child(license_ui::COPY_SUCCESS_CHANGED),
                    );
                for item in changed {
                    ok = ok.child(
                        div()
                            .text_xs()
                            .text_color(rgb(th.text.primary))
                            .child(item.clone()),
                    );
                }
                if let Some(v) = valid_until {
                    ok = ok.child(
                        div()
                            .text_xs()
                            .text_color(rgb(th.text.muted))
                            .child(v.clone()),
                    );
                }
                panel = panel.child(ok);
            }
            // Cards de Foco (3 presets — os 5 do ux-flows são backlog §6-B4).
            let mut cards = div().flex().flex_row().gap_2();
            for (idx, preset) in gallery::FocusPreset::all().into_iter().enumerate() {
                // F3-5-10: um gabarito escolhido APAGA a seleção de Foco (mutuamente exclusivos).
                let selected = !m.has_template() && idx == m.preset_idx;
                cards = cards.child(
                    div()
                        .id(("m9-card", idx))
                        .flex_1()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if selected {
                            ring(matches!(m.focus, M9Focus::Cards))
                        } else {
                            th.surface.border
                        }))
                        .bg(rgb(if selected {
                            th.surface.raised
                        } else {
                            th.surface.card
                        }))
                        .cursor_pointer()
                        .on_click(cx.listener(move |v, _ev: &gpui::ClickEvent, _w, cx| {
                            if let Some(m) = v.create_space_modal.as_mut() {
                                m.select_preset(idx);
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(th.text.primary))
                                .child(preset.label().to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(th.text.muted))
                                .child(preset.blurb().to_string()),
                        ),
                );
            }
            // F3-5-10: galeria de gabaritos prontos (doc-fonte 59) — escolher um SEMEIA o Espaço
            // (roster + config + pistas) pelo MESMO funil gated; mouse-clicável (teclado é follow-up).
            let mut tpl_cards = div().flex().flex_row().gap_2();
            for (idx, t) in gallery::CreateSpaceModal::templates()
                .into_iter()
                .enumerate()
            {
                let selected = m.is_template_selected(idx);
                tpl_cards = tpl_cards.child(
                    div()
                        .id(("m9-tpl-card", idx))
                        .flex_1()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if selected {
                            th.accent.action
                        } else {
                            th.surface.border
                        }))
                        .bg(rgb(if selected {
                            th.surface.raised
                        } else {
                            th.surface.card
                        }))
                        .cursor_pointer()
                        .aria_label(format!("modelo pronto: {}", t.name))
                        .on_click(cx.listener(move |v, _ev: &gpui::ClickEvent, _w, cx| {
                            if let Some(m) = v.create_space_modal.as_mut() {
                                m.select_template(idx);
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(th.text.primary))
                                .child(t.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(th.text.muted))
                                .child(gallery::template_blurb(&t.slug).to_string()),
                        ),
                );
            }
            let tpl_section = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(th.text.muted))
                        .child(gallery::COPY_M9_TEMPLATES_LABEL.to_string()),
                )
                .child(tpl_cards);
            panel = panel
                .child(cards)
                .child(tpl_section)
                .child(
                    field(
                        gallery::COPY_M9_NAME_LABEL,
                        &m.name,
                        matches!(m.focus, M9Focus::Name),
                        m.dup_note().map(|n| (n, th.state.warning)),
                    )
                    .id("m9-name")
                    .cursor_pointer()
                    .on_click(cx.listener(
                        |v, _ev: &gpui::ClickEvent, _w, cx| {
                            if let Some(m) = v.create_space_modal.as_mut() {
                                m.focus = gallery::M9Focus::Name;
                            }
                            cx.notify();
                        },
                    )),
                )
                .child(
                    field(
                        gallery::COPY_M9_WORKDIR_LABEL,
                        &m.workdir,
                        matches!(m.focus, M9Focus::Workdir),
                        m.error
                            .as_ref()
                            .map(|e| (format!("{} · [ {} ]", e.message, e.exit), th.state.danger)),
                    )
                    .id("m9-workdir")
                    .cursor_pointer()
                    .on_click(cx.listener(
                        |v, _ev: &gpui::ClickEvent, _w, cx| {
                            if let Some(m) = v.create_space_modal.as_mut() {
                                m.focus = gallery::M9Focus::Workdir;
                            }
                            cx.notify();
                        },
                    )),
                );
            // Botões: Procurar… · Criar Espaço · Cancelar (ordem de tab da spec §4).
            let botao = |id: &'static str, rotulo: String, focused: bool, primario: bool| {
                div()
                    .id(id)
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(ring(focused)))
                    .bg(rgb(if primario {
                        th.accent.create
                    } else {
                        th.surface.raised
                    }))
                    .text_sm()
                    .text_color(rgb(if primario {
                        th.text.on_accent
                    } else {
                        th.text.primary
                    }))
                    .cursor_pointer()
                    .child(rotulo)
            };
            panel = panel.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .justify_end()
                    .child(
                        botao(
                            "m9-browse",
                            gallery::COPY_M9_BROWSE.to_string(),
                            matches!(m.focus, M9Focus::Browse),
                            false,
                        )
                        .on_click(
                            cx.listener(|v, _ev: &gpui::ClickEvent, _w, cx| v.m9_pick_folder(cx)),
                        ),
                    )
                    .child(
                        botao(
                            "m9-create",
                            gallery::COPY_M9_CREATE.to_string(),
                            matches!(m.focus, M9Focus::Create),
                            true,
                        )
                        .on_click(cx.listener(|v, _ev: &gpui::ClickEvent, _w, cx| v.m9_submit(cx))),
                    )
                    .child(
                        botao(
                            "m9-cancel",
                            gallery::COPY_M9_CANCEL.to_string(),
                            matches!(m.focus, M9Focus::Cancel),
                            false,
                        )
                        .on_click(cx.listener(
                            |v, _ev: &gpui::ClickEvent, _w, cx| {
                                v.create_space_modal = None;
                                cx.notify();
                            },
                        )),
                    ),
            );
        }
        // Véu transparente que captura o clique: o modal Criar Espaço não vaza pro canvas atrás
        // (mesma doutrina M2 do ui::modal::Modal — occlude por default).
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .occlude(),
            )
            .child(panel)
            .into_any_element()
    }

    /// Apenda um evento no log de um Espaço: runtime MONTADO usa o handle vivo; de fundo
    /// não-montado abre o store 1× (busy_timeout do open cobre a concorrência — F1-4-1 crit 5).
    fn append_to_workspace(&mut self, root: &Path, ev: &lina_core::DomainEvent) -> bool {
        let mounted = {
            let r = lock(&self.runtimes);
            r.map.get(root).map(|rt| Arc::clone(&rt.store))
        };
        let res = match mounted {
            Some(store) => lock(&store)
                .append(ev)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            None => lina_core::EventStore::open(sidebar::events_dir_of(root))
                .and_then(|mut s| s.append(ev).map(|_| ()))
                .map_err(|e| e.to_string()),
        };
        if let Err(e) = res {
            eprintln!(
                "lina-gpui: [WS] evento não persistiu no Espaço {}: {e}",
                root.display()
            );
            return false;
        }
        true
    }

    /// Pós-mutação de um Espaço (rename/arquivar): re-deriva a entrada do PRÓPRIO log
    /// (a autoridade — ADR 0010) e atualiza o ponteiro global, sem mexer no foco.
    fn sync_registry_entry(&mut self, root: &Path) {
        if let Some(Ok(mut reg)) =
            lina_core::default_registry_path().map(lina_core::WorkspaceRegistry::load)
        {
            match lina_core::WorkspaceRegistry::rederive_entry(root) {
                Ok(entry) => {
                    reg.upsert(entry);
                    if let Err(e) = reg.save() {
                        eprintln!("lina-gpui: [WS] ponteiro global não salvo ({e}); seguindo");
                    }
                }
                Err(e) => {
                    eprintln!(
                        "lina-gpui: [WS] re-derivação de {} falhou ({e})",
                        root.display()
                    );
                }
            }
        }
    }

    /// `[ Renomear ]` confirmado: `WorkspaceRenamed` no log do alvo + ponteiro re-derivado.
    fn rename_workspace(&mut self, root: &Path, novo: &str, cx: &mut Context<Self>) {
        let novo = novo.trim();
        if novo.is_empty() {
            return;
        }
        if self.append_to_workspace(
            root,
            &lina_core::DomainEvent::WorkspaceRenamed {
                name: novo.to_string(),
            },
        ) {
            self.sync_registry_entry(root);
            self.refresh_sidebar_rows();
        }
        cx.notify();
    }

    /// `[ Arquivar ]` com Desfazer (spec §M8): a linha some JÁ, mas o `WorkspaceArchived`
    /// só é gravado quando a janela do toast fecha (commit adiado — `commit_pending_archive`).
    /// Desfazer dentro da janela = cancelar a gravação; nenhum evento novo no core (o
    /// `WorkspaceUnarchived` da spec §6-C2 segue pendente para a vista de arquivados).
    /// Anunciado na live-region (o arquivamento é AUDÍVEL, não só visual).
    fn archive_workspace(&mut self, root: &Path, cx: &mut Context<Self>) {
        if lock(&self.runtimes).active == root {
            eprintln!("lina-gpui: [WS] arquivar o Espaço ATIVO não é permitido (troque antes)");
            return;
        }
        // Um arquivamento pendente por vez: iniciar o 2º COMMITA o 1º (não o perde).
        self.commit_pending_archive(cx);
        let name = lina_core::default_registry_path()
            .and_then(|p| lina_core::WorkspaceRegistry::load(p).ok())
            .and_then(|r| {
                r.entries()
                    .iter()
                    .find(|e| e.path == root)
                    .map(|e| e.name.clone())
            })
            .unwrap_or_else(|| root.display().to_string());
        let toast = sidebar::ArchiveToast::new(root.to_path_buf(), name, lina_core::now_ms());
        self.a11y_live
            .announce(sidebar::copy_archive_announce(&toast.name));
        self.archive_toast = Some(toast);
        self.refresh_sidebar_rows();
        cx.notify();
    }

    /// Fecha a janela do Desfazer: grava o `WorkspaceArchived` no log do alvo + ponteiro
    /// re-derivado — some da lista padrão, NADA se perde (replay recupera — F1-4-4 crit 3).
    /// Se o usuário TROCOU para o Espaço pendente durante a janela (⌘n), arquivar o ativo
    /// é proibido → vira Desfazer (anunciado).
    fn commit_pending_archive(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.archive_toast.take() else {
            return;
        };
        if lock(&self.runtimes).active == t.root {
            self.a11y_live
                .announce(sidebar::copy_archive_undone(&t.name));
        } else if self.append_to_workspace(&t.root, &lina_core::DomainEvent::WorkspaceArchived) {
            self.sync_registry_entry(&t.root);
        }
        self.refresh_sidebar_rows();
        cx.notify();
    }

    /// `[ Desfazer ]` (clique, Tab+Enter ou ⌘Z): cancela o arquivamento pendente — a linha
    /// volta à lista; nada foi gravado, nada a reverter. Anunciado na live-region.
    fn undo_pending_archive(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.archive_toast.take() else {
            return;
        };
        self.a11y_live
            .announce(sidebar::copy_archive_undone(&t.name));
        self.refresh_sidebar_rows();
        cx.notify();
    }

    // ───────────────────────── F1-1-7 · Fila de Atenção (gestos + heartbeat) ─────────────────────────

    /// Um tick da fila: sincroniza com o hub (core ordena; replay só com evento novo),
    /// avança a máquina do toast, decide o lembrete sonoro (1×/30s, mute persistido) e
    /// devolve se há ANIMAÇÃO viva (countdown/pulso → o loop sobe p/ ~30fps).
    fn attention_heartbeat(&mut self, cx: &mut Context<Self>) -> bool {
        let now = lina_core::now_ms();
        // Spec §M8: janela do Desfazer fechou → grava o arquivamento pendente (carona no
        // heartbeat, que roda sempre — ≥4Hz; o countdown do toast também anima por ele).
        if self.archive_toast.as_ref().is_some_and(|t| t.expired(now)) {
            self.commit_pending_archive(cx);
        }
        let items = self.attention.sync(now);
        let changed = items != self.attention_items;
        if changed {
            self.attention_items = items;
        }
        self.attention_machine.tick(&self.attention_items, now);
        // Cache do mute (settings.json — o T7 pode mudá-lo ao vivo): re-lê ~2s
        // enquanto a fila está viva; fila vazia não toca o disco.
        if !self.attention_items.is_empty() {
            let stale = self
                .attention_settings_at
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(2));
            if stale {
                self.attention_sound_muted =
                    persistence_ui::load_settings(&self.settings_dir).attention_sound_muted;
                self.attention_settings_at = Some(Instant::now());
            }
        }
        let elapsed = self
            .attention_sound_last
            .map(|t| t.elapsed().as_millis() as u64);
        if attention_ui::should_play_sound(
            self.attention_items.len(),
            self.attention_sound_muted,
            elapsed,
        ) {
            attention_ui::play_attention_sound();
            self.attention_sound_last = Some(Instant::now());
        }
        let visible = self.attention_machine.visible(&self.attention_items, now);
        let toast_alive = matches!(visible, Some(attention_ui::ToastView::Single(_)));
        let badge = attention_ui::badge_view(&self.attention_items);
        let pulsing = badge.pulsing && !a11y::reduce_motion_effective(self.reduce_motion);
        // Sonda [ATT]: estado da fila/toast/badge no stderr, SÓ na mudança (sem spam) —
        // o Maestro valida por dados (toast em <2s, colapso >1, escalação) sem ver a tela.
        let diag = format!(
            "items={} toast={visible:?} badge_count={} badge_pulsing={}",
            self.attention_items.len(),
            badge.count,
            badge.pulsing
        );
        if diag != self.attention_diag_last {
            eprintln!("lina-gpui: [ATT] fila · {diag}");
            self.attention_diag_last = diag;
        }
        let animating = toast_alive || pulsing || self.archive_toast.is_some();
        if changed || animating {
            cx.notify();
        }
        animating
    }

    /// Re-sincroniza JÁ (pós-gesto): o evento acabou de ser apendado — a tela reflete
    /// no mesmo frame, sem esperar o próximo heartbeat.
    fn attention_refresh_now(&mut self, cx: &mut Context<Self>) {
        let now = lina_core::now_ms();
        self.attention_items = self.attention.sync(now);
        self.attention_machine.tick(&self.attention_items, now);
        cx.notify();
    }

    fn attention_toggle_panel(&mut self, cx: &mut Context<Self>) {
        self.attention_panel_open = !self.attention_panel_open;
        cx.notify();
    }

    fn attention_open_panel(&mut self, cx: &mut Context<Self>) {
        self.attention_panel_open = true;
        cx.notify();
    }

    /// «Depois»/Esc-sem-decidir: recolhe o toast SEM decidir (item segue na fila+badge).
    fn attention_snooze(&mut self, cx: &mut Context<Self>) {
        self.attention_machine.snooze(&self.attention_items);
        cx.notify();
    }

    /// Pause-on-hover do countdown (ponteiro sobre o toast).
    fn attention_hover(&mut self, hovering: bool) {
        self.attention_machine.hover(hovering, lina_core::now_ms());
    }

    /// Aprova uma PERMISSÃO: SÓ registra a decisão (`PermissionResolved{approve,human}`)
    /// — nenhum byte vai ao PTY (ADR 0021 §6; o write é F1-1-8). Custódia nunca passa
    /// por aqui (o hub/core devolvem `false` — o gesto dela é o ⌘⏎ existente).
    fn attention_approve(&mut self, stable_id: &str, cx: &mut Context<Self>) {
        // SEAM-1: um banner de SPAWN (cascata) → deixa o terminal nascer (funil único, dedupe M3).
        if self.attention.resolve_spawn(stable_id, true) {
            self.attention_refresh_now(cx);
            return;
        }
        if self.attention.resolve(
            stable_id,
            lina_core::ApprovalDecision::Approve,
            lina_core::ResolutionVia::Human,
        ) {
            self.attention_refresh_now(cx);
        }
    }

    /// Recusa uma PERMISSÃO (registro apenas — simetria do aprovar).
    fn attention_deny(&mut self, stable_id: &str, cx: &mut Context<Self>) {
        // SEAM-1: recusar um banner de SPAWN → nada nasce (`SpawnDeclined`).
        if self.attention.resolve_spawn(stable_id, false) {
            self.attention_refresh_now(cx);
            return;
        }
        if self.attention.resolve(
            stable_id,
            lina_core::ApprovalDecision::Deny,
            lina_core::ResolutionVia::Human,
        ) {
            self.attention_refresh_now(cx);
        }
    }

    /// «Não era um pedido» (mitigação de FP): `PermissionDismissed` — não é deny.
    fn attention_dismiss(&mut self, stable_id: &str, cx: &mut Context<Self>) {
        if self.attention.dismiss(stable_id) {
            self.attention_refresh_now(cx);
        }
    }

    /// Tela do fundador (2026-06-25): «Limpar fila» — dispensa TODOS os itens visíveis de uma vez.
    /// Cada um vira `PermissionDismissed{stable_id}` (o fold genérico remove de qualquer fila), dando
    /// ao usuário o controle de zerar pendências velhas que nenhum encerramento automático alcança
    /// (ex.: avisos de mensagens não entregues a alvos que nunca existiram). Gesto humano, durável.
    fn attention_clear_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .attention_items
            .iter()
            .map(|i| i.stable_id.clone())
            .collect();
        let mut changed = false;
        for id in &ids {
            changed |= self.attention.dismiss(id);
        }
        if changed {
            self.attention_refresh_now(cx);
        }
    }

    /// «Silenciar/Reativar detecção deste terminal» (allowlist por nó — reversível).
    fn attention_toggle_mute(&mut self, node_id: &str, cx: &mut Context<Self>) {
        let muted = self.attention.is_node_muted(node_id);
        if self.attention.set_node_muted(node_id, !muted) {
            self.attention_refresh_now(cx);
        }
    }

    /// R2b: «Ir até o terminal» — ação primária de pergunta (`Choice`/`Trust`): foca e
    /// REVELA o nó pelo NOME (o `node_id` da fila é o nome do roster). O gesto resolve
    /// LÁ (escolher no próprio terminal); nada é decidido/injetado daqui. Nome que não
    /// resolve (nó fechou entre o toast e o clique) → no-op logado, nunca panic.
    fn attention_goto_node(&mut self, node_name: &str, window: &Window, cx: &mut Context<Self>) {
        let found = lock(&self.nodes.model)
            .nodes
            .iter()
            .find(|(_, v)| v.name == node_name)
            .map(|(id, _)| *id);
        match found {
            Some(id) => {
                self.focus(id);
                self.reveal(id, window);
                // O convite foi aceito: recolhe o toast (o item segue na fila/badge
                // até o bloqueio resolver no terminal — sinal de resolução é do
                // detector, F1-1-6/Dev 01).
                self.attention_machine.snooze(&self.attention_items);
                cx.notify();
                eprintln!("lina-gpui: [ATT] goto — foco/zoom no nó {node_name:?}");
            }
            None => eprintln!(
                "lina-gpui: [ATT] goto — nó {node_name:?} não está mais no canvas (fechou?)"
            ),
        }
    }

    /// R2b: nº de terminais TRABALHANDO agora (Busy/Running — `Blocked` do core projeta
    /// como Busy na UI) — alimenta o empty-state honesto da fila (1b).
    fn busy_terminal_count(&self) -> usize {
        lock(&self.nodes.model)
            .nodes
            .values()
            .filter(|v| {
                matches!(v.kind, NodeKind::Terminal)
                    && matches!(v.status, NodeStatus::Busy | NodeStatus::Running)
            })
            .count()
    }

    // === IME (W2-4): a composição do SO (dead-keys ´~^, cedilha, CJK) entra por estes ganchos, ===
    // === chamados pelo `ImeHandler` no `paint`. Espelham os helpers do terminal do Zed.         ===

    /// COMMIT: escreve o texto FINAL composto (ex. "á") nos bytes do PTY do nó focado — mesma rota de
    /// um keystroke imprimível, mas a partir do caractere já montado pelo IME. Vazio → não submete.
    fn ime_commit(&mut self, text: &str) {
        let bytes = composed_text_to_pty_bytes(text);
        if !bytes.is_empty() {
            self.input.submit(self.focused, WriteOp::HumanKeys(bytes));
        }
    }

    /// Marca o pre-edit (composição em curso). Texto vazio é equivalente a limpar a marca.
    fn ime_set_marked(&mut self, text: String, cx: &mut Context<Self>) {
        if text.is_empty() {
            return self.ime_clear_marked(cx);
        }
        self.ime_marked = Some(text);
        cx.notify();
    }

    /// Sai do estado de composição (commit, cancelamento ou `unmarkText` do SO).
    fn ime_clear_marked(&mut self, cx: &mut Context<Self>) {
        if self.ime_marked.is_some() {
            self.ime_marked = None;
            cx.notify();
        }
    }

    /// Faixa do texto marcado em UNIDADES UTF-16 (o que o `NSTextInputClient` espera).
    fn ime_marked_range_utf16(&self) -> Option<std::ops::Range<usize>> {
        self.ime_marked
            .as_ref()
            .map(|t| 0..t.encode_utf16().count())
    }

    /// W4-2 · M1: a lista de comandos da paleta (rebuild a cada Cmd-K — o roster muda). Comandos
    /// estáticos + "Focar: <nó>" por nó vivo.
    fn palette_commands(&self) -> Vec<palette::Command> {
        use palette::{Command, PaletteAction as A};
        // Base pura (inclui "Aparência: tema e cores" — guardião em palette::tests) + roster vivo.
        let mut cmds = palette::base_commands();
        for (id, nv) in self.nodes.cards() {
            cmds.push(Command::new(
                format!("🎯 Focar: {}", nv.name),
                A::FocusNode(id),
            ));
            // F1-2-2 (M6-E): editar um Agente vivo pela paleta (entry point do fluxo (a)).
            if matches!(nv.kind, NodeKind::Terminal) {
                cmds.push(Command::new(
                    format!("✏️ Editar Agente: {}", nv.name),
                    A::EditAgent(id),
                ));
            }
        }
        cmds
    }

    /// W4-2 · M1: executa a ação escolhida na paleta (abre o modal M6, foca, alterna o freio,
    /// ou abre o modo CRIAÇÃO de nota/pasta — M3/M4).
    fn run_palette_action(
        &mut self,
        action: palette::PaletteAction,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        use palette::PaletteAction as A;
        match action {
            A::NewAgent => self.open_agent_modal_create(cx),
            A::ConnectChannel => self.open_credential_modal(cx),
            A::ConfigureWebhook => self.open_webhook_modal(cx),
            A::EditAgent(node) => self.open_agent_modal_edit(node, cx),
            A::FocusNode(node) => {
                self.focus(node);
                self.reveal(node, window);
            }
            A::ToggleBrake => lock(&self.brake).toggle_requested = true,
            A::OpenSettings => self.open_settings_window(cx),
            // M3/M4: abre o modo CRIAÇÃO (digita o título → Enter cria + foca; ver `handle_key`).
            A::NewNote => self.creating = Some((creators::CreatorKind::Note, String::new())),
            A::NewFolder => self.creating = Some((creators::CreatorKind::Folder, String::new())),
            A::ToggleDashboard => self.toggle_dashboard(cx),
            // F2-2-5 (toolbar contextual): ⚑ Atender — NAVEGA até o nó que precisa de você
            // (foca + revela), SEM aprovar nada (a autorização mantém autoridade única — segurança
            // do ADR 0029). É a paridade do "ir até o terminal" da fila, agora na toolbar do card.
            A::GotoAtencao(node) => {
                self.focus(node);
                self.reveal(node, window);
                cx.notify();
            }
            // F2-2-5 (toolbar): ✕ Encerrar — encerra o CLI do nó (o mesmo que o ✕ do header faz).
            A::CloseNode(node) => self.close_node(node, window),
        }
    }

    /// F1-1-5 (wiring): abre/fecha o painel "Atividade e custos" (P6/fluxo c). Enquanto
    /// aberto, um loop de tick (~500ms) marca dirty — timeline/sessões mudam em threads
    /// gpui-free e NÃO acordam o event loop (lição: animação precisa de dirty).
    fn toggle_dashboard(&mut self, cx: &mut Context<Self>) {
        self.dashboard_open = !self.dashboard_open;
        cx.notify(); // o heartbeat do painel (new) cuida do refresh enquanto aberto
    }

    /// F3-1-7: o `AutonomyLevel` vivo do workspace (espelha `bridge::autonomy_to_level`, privado da
    /// bomba) — alimenta `resolve_from_store` para ler o `goal_max_iterations` vivo.
    fn autonomy_level(&self) -> lina_core::AutonomyLevel {
        match self.ws_autonomy {
            Autonomy::Manual => lina_core::AutonomyLevel::Manual,
            Autonomy::Assisted => lina_core::AutonomyLevel::Assisted,
            Autonomy::Autonomous => lina_core::AutonomyLevel::Autonomous,
        }
    }

    /// F3-1-7: re-projeta as Goals VIVAS + o budget de iteração SÓ quando o log mudou (disciplina
    /// `projection_cache_is_stale` — NUNCA por frame; mesma do custo/effort). Goals e budget são META
    /// (fora da projeção do canvas), reconstruídos por replay do event log.
    fn refresh_goals_cache(&mut self) {
        let level = self.autonomy_level(); // só lê o enum de autonomia — NÃO toca o store.
        let cached = self.goals_cache.as_ref().map(|(c, _, _)| *c);
        // NÃO-BLOQUEANTE na thread de RENDER (fix freeze do `lina ask`): a entrega A2A faseada
        // segura o store por até ~`ready_timeout` (~2s, por design). Via `try_with_store`, store
        // ocupado → mantém o cache e re-tenta no próximo quadro; o render nunca trava nesse lock.
        let fresh = self.nodes.try_with_store(|store| {
            let count = store.event_count().ok()?;
            if !dashboard::projection_cache_is_stale(cached, Some(count)) {
                return None; // cache fresco → nada a atualizar.
            }
            let recs = store.events().ok()?;
            let goals = lina_core::project_goals(&recs);
            // `goal_max_iterations` vivo (SystemParams, spec 51); falha de replay → default da spec.
            let budget = lina_core::resolve_from_store(store, level)
                .map(|r| r.router_config.goal_max_iterations)
                .unwrap_or(goal_card::DEFAULT_ITERATION_BUDGET);
            Some((count, goals, budget))
        });
        if let Some(v) = fresh {
            self.goals_cache = Some(v);
        }
    }

    /// F4-0-5: reprojeta a exposição (canais com credencial viva) SÓ quando o log muda — mesma
    /// disciplina `projection_cache_is_stale` (NUNCA por frame) e `try_with_store` não-bloqueante
    /// do cache de goals (o badge é META, derivado do event log; inv #4).
    fn refresh_exposure_cache(&mut self) {
        let cached = self.exposure_cache.as_ref().map(|(c, _)| *c);
        let fresh = self.nodes.try_with_store(|store| {
            let count = store.event_count().ok()?;
            if !dashboard::projection_cache_is_stale(cached, Some(count)) {
                return None;
            }
            let recs = store.events().ok()?;
            Some((count, exposure::ExposureState::from_records(&recs)))
        });
        if let Some(v) = fresh {
            self.exposure_cache = Some(v);
        }
    }

    /// F4-0-5: o badge persistente "este Espaço está falando com o mundo" — `None` quando 0 canais
    /// expostos (0 → 0 badge). Cor de alerta SÓBRIA (`state.warning`, doc 40 §10: feedback
    /// subconsciente, não decoração) e diz QUAL(is) canal(is), nunca um genérico.
    fn render_exposure_badge(&mut self) -> Option<gpui::AnyElement> {
        self.refresh_exposure_cache();
        // Extrai os nomes (chips) + a frase completa (aria) e libera o empréstimo antes de pintar.
        let (chips, aria): (Vec<gpui::SharedString>, String) = {
            let (_, st) = self.exposure_cache.as_ref()?;
            if st.is_empty() {
                return None; // 0 canais → 0 badge.
            }
            let chips = st
                .channels()
                .iter()
                .map(|c| gpui::SharedString::from(c.display.clone()))
                .collect();
            let aria = st.headline().unwrap_or_default();
            (chips, aria)
        };
        let t = theme::active();
        let mut row = div()
            .id("exposure-badge")
            .absolute()
            .top(px(f32::from(t.spacing.md)))
            .right(px(f32::from(t.spacing.md)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(f32::from(t.spacing.sm)))
            .px(px(f32::from(t.spacing.md)))
            .py(px(f32::from(t.spacing.xs)))
            .rounded_full()
            .bg(rgb(t.surface.card))
            .border_1()
            .border_color(rgb(t.state.warning))
            .aria_label(gpui::SharedString::from(aria))
            .child(
                // Ponto de status (token de alerta) — o sinal de cor que o doc 40 §10 pede.
                div()
                    .w(px(f32::from(t.spacing.sm)))
                    .h(px(f32::from(t.spacing.sm)))
                    .rounded_full()
                    .bg(rgb(t.state.warning)),
            )
            .child(
                div()
                    .text_size(px(f32::from(t.typography.size.small)))
                    .font_family(t.typography.family.ui)
                    .text_color(rgb(t.text.bright))
                    .child(exposure::BADGE_HEADLINE),
            );
        // Um chip por canal exposto — "diz QUAL canal" (doc 40 §10), nunca um genérico.
        for name in chips {
            row = row.child(
                div()
                    .px(px(f32::from(t.spacing.sm)))
                    .py(px(f32::from(t.spacing.xs)))
                    .rounded_md()
                    .bg(rgb(t.surface.raised))
                    .text_size(px(f32::from(t.typography.size.small)))
                    .font_family(t.typography.family.ui)
                    .text_color(rgb(t.text.primary))
                    .child(name),
            );
        }
        Some(row.into_any_element())
    }

    /// F3-1-7: o painel das Goals VIVAS sobre o canvas — um card por meta que merece rosto
    /// ([`goal_card::surfaced`]), em pt-br sem jargão, na identidade F2. `None` quando não há meta
    /// viva (sem painel). Coluna top-center, sob os modais/paleta. Confirmar/ajustar é 1 toque →
    /// `confirm_goal`/`correct_goal`.
    fn render_goals_panel(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        self.refresh_goals_cache();
        let (mut goals, budget) = match &self.goals_cache {
            Some((_, g, b)) => (goal_card::surfaced(g.clone()), *b),
            None => return None,
        };
        // F3-1-7: respeita as metas FECHADAS pelo usuário (durável) — somem da tela, seguem no log.
        goals.retain(|g| !self.dismissed_goals.contains(&g.goal_id));
        if goals.is_empty() {
            return None;
        }
        let mut col = div()
            .absolute()
            .top_16()
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap_3();
        for goal in goals {
            let gid_confirm = goal.goal_id.clone();
            let gid_correct = goal.goal_id.clone();
            let gid_toggle = goal.goal_id.clone();
            let gid_dismiss = goal.goal_id.clone();
            let gid_drag = goal.goal_id.clone();
            let is_collapsed = self.collapsed_goals.contains(&goal.goal_id);
            let (ox, oy) = self
                .goal_offsets
                .get(&goal.goal_id)
                .copied()
                .unwrap_or((0.0, 0.0));
            let editing = self.editing_goal.as_ref().and_then(|(gid, buf)| {
                (gid == &goal.goal_id).then(|| gpui::SharedString::from(buf.clone()))
            });
            let card = goal_card::GoalCard::new(goal)
                .iteration_budget(budget)
                .collapsed(is_collapsed)
                .editing(editing)
                .on_confirm(cx.listener(move |view, _ev, window, cx| {
                    view.confirm_goal(&gid_confirm, window, cx);
                }))
                .on_correct(cx.listener(move |view, _ev, _window, cx| {
                    view.correct_goal(&gid_correct, cx);
                }))
                .on_toggle(cx.listener(move |view, _ev, _window, cx| {
                    view.toggle_goal_collapsed(&gid_toggle);
                    cx.notify();
                }))
                .on_dismiss(cx.listener(move |view, _ev, _window, cx| {
                    view.dismiss_goal(&gid_dismiss);
                    cx.notify();
                }));
            // Largura amigável (≈42% da viewport). `relative` + left/top desloca o card pelo arrasto
            // SEM tirá-lo do empilhamento (os vizinhos não se mexem). `on_mouse_down` ancora o gesto e
            // PARA a propagação (senão o root iniciaria o pan da câmera embaixo); o move/up vivem no
            // root (captura global) — o card não "gruda" quando o cursor escapa dele.
            col = col.child(
                div()
                    .relative()
                    .left(px(ox))
                    .top(px(oy))
                    .w(gpui::relative(0.42))
                    .child(card)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, ev: &MouseDownEvent, _w, cx| {
                            let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                            let start_offset = view
                                .goal_offsets
                                .get(&gid_drag)
                                .copied()
                                .unwrap_or((0.0, 0.0));
                            view.goal_drag = Some(GoalDrag {
                                goal_id: gid_drag.clone(),
                                start_mouse: pos,
                                start_offset,
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        Some(col.into_any_element())
    }

    /// F3-3-5: re-projeta a Mentality VIVA por papel SÓ quando o log mudou (disciplina
    /// `projection_cache_is_stale` — NUNCA por frame; mesma do custo/effort/goals; fix do freeze do
    /// `lina ask`). Enumera os papéis pelo ROSTER vivo (`cards`→`node_role`) — a fonte certa para
    /// "Como o [papel] pensa": os papéis do time. Para cada um, delega à projeção do core
    /// (`lina_core::mentality`, M-PROMO/Terminal I) o QUÊ mostrar via `top_k_for_role` (estabelecidas
    /// primeiro, exclui aposentadas/expiradas/não-capturáveis), humaniza o nome do papel
    /// (`role_suggester::humanize`) e MAPEIA `Belief`→`BeliefView` (o painel não conhece o shape do
    /// core). Não-bloqueante: store ocupado mantém o cache (entrega NÃO trava a UI).
    fn refresh_mentality_cache(&mut self) {
        // Teto de exibição por papel: generoso (o fundador vê essencialmente todas as crenças ativas
        // do papel, já ordenadas). É o mesmo `top_k` do injetor — deve casar/superar o K dele.
        const PANEL_CAP: usize = 24;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // Papéis distintos do roster vivo (fora do borrow do store).
        let roles: std::collections::BTreeSet<String> = self
            .nodes
            .cards()
            .into_iter()
            .filter_map(|(id, _)| self.nodes.node_role(id))
            .collect();
        let cached = self.mentality_cache.as_ref().map(|(c, _)| *c);
        if let Some(v) = self.nodes.try_with_store(|store| {
            let count = store.event_count().ok()?;
            if !dashboard::projection_cache_is_stale(cached, Some(count)) {
                return None;
            }
            let recs = store.events().ok()?;
            let mentality = lina_core::mentality::Mentality::from_records(
                &recs,
                lina_core::mentality::PromotionPolicy::default(),
            );
            let views: Vec<mentality_panel::RoleMentality> = roles
                .iter()
                .filter_map(|role| {
                    let beliefs: Vec<mentality_panel::BeliefView> = mentality
                        .top_k_for_role(role, PANEL_CAP, now_ms)
                        .into_iter()
                        .map(|b| mentality_panel::BeliefView {
                            belief_id: b.belief_id.clone(),
                            statement: b.statement.clone(),
                            provenance: b.provenance.clone(),
                            // `top_k_for_role` já exclui aposentadas; só `Established`/`Provisional`
                            // chegam aqui. O `_` cobre `Provisional` (e o `Retired` impossível).
                            standing: match b.status {
                                lina_core::mentality::BeliefStatus::Established => {
                                    mentality_panel::Standing::Established
                                }
                                _ => mentality_panel::Standing::Provisional,
                            },
                            untrusted_origin: b.untrusted_origin,
                        })
                        .collect();
                    if beliefs.is_empty() {
                        // Papel sem crença ativa não vira painel (sem tela vazia forçada).
                        return None;
                    }
                    Some(mentality_panel::RoleMentality {
                        // Nome do papel humanizado em pt-br (nunca o código cru na tela, inv #6).
                        role_label: role_suggester::humanize(role).0,
                        beliefs: mentality_panel::ordered(beliefs),
                    })
                })
                .collect();
            Some((count, views))
        }) {
            self.mentality_cache = Some(v);
        }
    }

    /// F3-3-5: o painel "Como o [papel] pensa" sobre o canvas — um painel por papel que aprendeu algo
    /// com o fundador, em pt-br sem jargão, na identidade F2. `None` quando nenhum papel tem crença
    /// ativa (sem tela vazia forçada). O gesto "esquecer isso" dispara `belief.retire` via
    /// [`Self::retire_belief`]. A posição (flanco direito) é provisória — o lugar/abertura final do
    /// painel (toggle/inspector) é polimento de UX para quando o ciclo completo estiver na tela.
    fn render_mentality_panel(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        self.refresh_mentality_cache();
        let roles = match &self.mentality_cache {
            Some((_, v)) if !v.is_empty() => v.clone(),
            _ => return None,
        };
        let mut col = div()
            .absolute()
            .top_16()
            .right_0()
            .flex()
            .flex_col()
            .items_end()
            .gap_3();
        for role in roles {
            // As crenças já vêm ordenadas (estabelecidas primeiro) do cache.
            let cards: Vec<mentality_panel::BeliefCard> = role
                .beliefs
                .into_iter()
                .map(|belief| {
                    let bid = belief.belief_id.clone();
                    mentality_panel::BeliefCard::new(belief).on_retire(cx.listener(
                        move |view, _ev, _window, cx| {
                            view.retire_belief(&bid, cx);
                        },
                    ))
                })
                .collect();
            let panel = mentality_panel::MentalityPanel::new(role.role_label).beliefs(cards);
            col = col.child(div().w(gpui::relative(0.42)).child(panel));
        }
        Some(col.into_any_element())
    }

    /// F3-3-5 (humano árbitro, spec 35 §6.7): o fundador clicou "esquecer isso" numa crença. MESMA
    /// costura do [`Self::confirm_goal`]/[`Self::release_breaker`] (ADR 0036): a view só DISPARA o gesto
    /// pelo canal in-process (`belief.retire`); a `MailboxPump` carimba a autoridade SERVER-SIDE e
    /// roteia para emitir `BeliefRetired{reason:"retired_by_human"}` — a view NUNCA escolhe autoridade
    /// nem `reason` (doutrina de segurança: campo escrito pela UI jamais decide). O roteamento
    /// `belief.retire`→`BeliefRetired` é costura do `bridge.rs::drain_human_intents` (M-INJETOR).
    /// `belief_id` é id cunhado server-side (vem da projeção) — sem escape JSON, como o `goal_id`.
    fn retire_belief(&mut self, belief_id: &str, cx: &mut Context<Self>) {
        self.nodes
            .push_human_intent("belief.retire", format!(r#"{{"belief_id":"{belief_id}"}}"#));
        cx.notify();
    }

    /// F3-1-7 (gate humano da meta): o fundador confirmou a interpretação com 1 toque. O efeito REAL
    /// (`goal.confirm` → `GoalConfirmed`, com `by` carimbado SERVER-SIDE pelo supervisor) depende de
    /// uma costura de boot que o app ainda NÃO tem: o workspace-shell não é um nó (logo não há
    /// identidade autenticada de outbox para o confirm humano), e o `Mailbox` vive na bomba, fora da
    /// view. Doutrina (spec 52 §Segurança 2): `by` JAMAIS é escolhido pelo cliente — então NÃO
    /// forjamos um aqui. Por ora registra a decisão por DADOS (`[GOAL]`, o Maestro valida o fluxo na
    /// tela); o disparo autenticado é costura do Maestro (DEP do despacho). [SEAM: identidade do confirm].
    fn confirm_goal(&mut self, goal_id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        // ADR 0036: a view DISPARA o gesto pelo canal in-process; a `MailboxPump` carimba `by="human"`
        // SERVER-SIDE (a view NÃO escolhe `by`). `goal_id` é UUID cunhado server-side — sem escape JSON.
        self.nodes
            .push_human_intent("goal.confirm", format!(r#"{{"goal_id":"{goal_id}"}}"#));
        cx.notify();
    }

    /// F3-CONF-2 (#21): o fundador clicou "liberar" no nó pausado pelo disjuntor. MESMA costura do
    /// [`Self::confirm_goal`] (ADR 0036): a view só DISPARA o gesto pelo canal in-process; a
    /// `MailboxPump` carimba `by="human"` SERVER-SIDE (a view NUNCA escolhe `by` — a doutrina de
    /// segurança: nenhum campo escrito pela UI decide autoridade). O gesto `breaker.reset` é roteado
    /// pela `MailboxPump` (`drain_human_intents`, bridge) para `Router::reset_breaker` — NÃO pelo
    /// `human_intent` (que é de Goal e o rejeitaria): fecha o disjuntor e tira o nó de `Blocked`.
    /// `node` é UUID cunhado server-side — sem escape JSON.
    fn release_breaker(&mut self, node: NodeId, _window: &mut Window, cx: &mut Context<Self>) {
        self.nodes
            .push_human_intent("breaker.reset", format!(r#"{{"node":"{node}"}}"#));
        cx.notify();
    }

    /// F3-1-7: o fundador quer AJUSTAR a interpretação (re-abrir `goal.interpret`). Mesma costura
    /// pendente do [`Self::confirm_goal`] (intent autenticado pelo `Mailbox` da bomba). Registra por DADOS.
    fn correct_goal(&mut self, goal_id: &str, cx: &mut Context<Self>) {
        // ADR 0036: abre o editor inline do entendimento (pré-preenchido com a interpretação atual). O
        // usuário reescreve e o Enter re-envia `goal.interpret` pelo canal humano (ver `handle_key`).
        let current = self
            .goals_cache
            .as_ref()
            .and_then(|(_, goals, _)| goals.iter().find(|g| g.goal_id == goal_id))
            .and_then(|g| g.interpretation.clone())
            .unwrap_or_default();
        self.editing_goal = Some((goal_id.to_string(), current));
        cx.notify();
    }

    /// F3-1-7: recolhe/expande o card de uma Goal (estado de sessão, por `goal_id`) — toggle puro
    /// (presente no conjunto = recolhido). Não persiste no log: é preferência de visualização.
    fn toggle_goal_collapsed(&mut self, goal_id: &str) {
        if !self.collapsed_goals.remove(goal_id) {
            self.collapsed_goals.insert(goal_id.to_string());
        }
    }

    /// F3-1-7: o usuário FECHA (dispensa) o card de uma meta — some da tela DURAVELMENTE (settings), sem
    /// apagar a meta do log (segue auditável; é preferência de visualização). Idempotente: só re-grava
    /// os settings na 1ª vez que aquela meta é fechada.
    fn dismiss_goal(&mut self, goal_id: &str) {
        if self.dismissed_goals.insert(goal_id.to_string()) {
            let mut settings = persistence_ui::load_settings(&self.settings_dir);
            settings.dismissed_goals = self.dismissed_goals.iter().cloned().collect();
            persistence_ui::save_settings(&self.settings_dir, &settings);
        }
    }

    /// F1-1-5 (P6/fluxo c — wiring): o painel "Atividade e custos". Hierarquia do
    /// wireframe: 1) estado (palavra+cor) → 2) custo (dinheiro, `~estimado` obrigatório)
    /// → 3) atividade. Tokens 100% do `theme` (zero cor hardcoded); `ElementId` único
    /// por linha (lição AccessKit); sonda `[DASH]` loga a latência evento→card 1× por
    /// evento — o Maestro valida por DADOS, o fundador pelo olho.
    /// Geometria: `dashboard::dashboard_panel_rect` (PURA, clampada à janela — bug da
    /// rodada: o painel fixo vazava a borda direita) + clip + scroll interno das linhas.
    /// F3-0-6: mapa `NodeId.to_string() → EffortBadge` (modelo·effort por nó). `EffortAssigned` é
    /// META (fora da projeção do canvas), então é reconstruído varrendo o log com a MESMA disciplina
    /// de cache do custo (re-varre SÓ com evento novo; nunca por frame). NÃO-BLOQUEANTE no render
    /// (fix do freeze do `lina ask`): store ocupado → mantém o cache e re-varre no próximo quadro.
    /// Fonte ÚNICA consumida pelo PAINEL (linha do nó) E pelo HEADER do card (pílula).
    fn effort_badges_cached(&self) -> BTreeMap<String, dashboard::EffortBadge> {
        let mut cache = lock(&self.dash.effort);
        let cached = cache.as_ref().map(|(c, _)| *c);
        if let Some(v) = self.nodes.try_with_store(|store| {
            let count = store.event_count().ok()?;
            if !dashboard::projection_cache_is_stale(cached, Some(count)) {
                return None;
            }
            Some((count, dashboard::effort_badges(&store.events().ok()?)))
        }) {
            *cache = Some(v);
        }
        cache.as_ref().map(|(_, m)| m.clone()).unwrap_or_default()
    }

    fn render_dashboard(
        &mut self,
        th: &theme::Theme,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let today = dashboard::utc_day_prefix(now_ms);
        let sessions = lock(&self.dash.sessions).clone();
        let engine = dashboard::EngineBadge::from_cli(Some(&self.dash.cli_id));

        // Linhas por terminal: nome + estado canônico (VIVO, do pump — o log não persiste
        // toda transição) + atividade da timeline. O custo POR NÓ entra abaixo, dos cards.
        struct Row {
            node: NodeId,
            name: String,
            state: dashboard::UiState,
            activity: String,
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut activities: BTreeMap<NodeId, crate::inspector::Activity> = BTreeMap::new();
        {
            let tl = lock(&self.dash.timeline);
            let model = lock(&self.nodes.model);
            for (node, view) in model.nodes.iter() {
                if !matches!(view.kind, NodeKind::Terminal) {
                    continue;
                }
                // `lina_host::NodeStatus` (UI) → string do vocabulário do core (o mapeamento
                // canônico vive em `dashboard::UiState::from_projection`). `Crashed` ≈ hang real.
                let status_str = match view.status {
                    NodeStatus::Busy => "Busy",
                    NodeStatus::Dead => "Dead",
                    NodeStatus::Crashed => "Crashed",
                    NodeStatus::Idle => "Idle",
                    // #21: o disjuntor passou a chegar aqui (o bridge parou de colapsar Blocked→Busy);
                    // sem este braço cairia em "Running" — mentira. `from_projection` já o lê como
                    // "Esperando você" (a voz do painel; ≠ "pausado por segurança" do card).
                    NodeStatus::Blocked => "Blocked",
                    NodeStatus::Starting | NodeStatus::Running => "Running",
                    _ => "Running", // variantes futuras: vivo sem distinção (fallback honesto)
                };
                let state = dashboard::UiState::from_projection(
                    Some(status_str),
                    view.status == NodeStatus::Crashed,
                );
                let live = tl.activity(&view.name, now_ms);
                activities.insert(
                    *node,
                    live.as_ref()
                        .map(|(a, _)| *a)
                        .unwrap_or(crate::inspector::Activity::Idle),
                );
                let activity = match live {
                    Some((_, Some(tool))) => format!("rodando: {tool}"),
                    Some((crate::inspector::Activity::Idle, None)) => {
                        "terminou a resposta".to_owned()
                    }
                    Some((_, None)) => "trabalhando".to_owned(),
                    None => "—".to_owned(),
                };
                // Sonda [DASH]: latência evento→card, 1× por evento novo (na mudança).
                if let Some(ts) = tl.last_ts(&view.name) {
                    if self.dash_lat_reported.get(&view.name) != Some(&ts) {
                        eprintln!(
                            "lina-gpui: [DASH] card node={} lat_evento->card_ms={}",
                            view.name,
                            now_ms.saturating_sub(ts)
                        );
                        self.dash_lat_reported.insert(view.name.clone(), ts);
                    }
                }
                rows.push(Row {
                    node: *node,
                    name: view.name.clone(),
                    state,
                    activity,
                });
            }
        }

        // Rodada 360 (passo 7): custo POR NÓ pelo binding node↔cwd do LOG (ADR 0022 §3).
        // Replay do log SÓ quando há evento novo (cache por `event_count`, padrão
        // `refresh_cost_paused`) — NUNCA por frame; store ilegível conserva o snapshot.
        let (cards, adopted) = {
            let mut cache = lock(&self.dash.projected);
            // NÃO-BLOQUEANTE no render (fix freeze do `lina ask`): store ocupado pela entrega A2A
            // → conserva o snapshot; re-projeta no próximo quadro. (Antes: `store_event_count` +
            // `projected` travavam o store na thread de render → congelava durante a entrega.)
            let cached = cache.as_ref().map(|(c, _)| *c);
            if let Some(v) = self.nodes.try_with_store(|store| {
                let count = store.event_count().ok()?;
                if !dashboard::projection_cache_is_stale(cached, Some(count)) {
                    return None;
                }
                Some((count, store.project().ok()?))
            }) {
                *cache = Some(v);
            }
            match cache.as_ref() {
                Some((_, st)) => (
                    dashboard::build_cards(st, &sessions, &self.dash.ws_root, &activities, &today),
                    dashboard::adopted_cwds(st),
                ),
                None => (Vec::new(), std::collections::BTreeSet::new()),
            }
        };
        let cards_by_node: BTreeMap<NodeId, dashboard::AgentCard> =
            cards.into_iter().map(|c| (c.node, c)).collect();
        // F3-0-6: badge modelo·effort por nó (fonte única — painel E header do card).
        let effort_by_node = self.effort_badges_cached();
        // Total "Hoje:": sessões sob o ws_root + sessões dos cwds ADOTADOS pelos nós do
        // Espaço (parcela de fora rotulada no próprio texto — honesto, nunca soma muda).
        let total =
            dashboard::workspace_cost_today(&sessions, &self.dash.ws_root, &adopted, &today);

        let state_color = |s: dashboard::UiState| -> u32 {
            match s.color_token() {
                "verde" => th.state.success,
                "ambar" => th.state.warning,
                "vermelho-suave" => th.state.danger,
                "azul-escuro" => th.accent.primary,
                _ => th.text.muted, // cinza / cinza-escuro
            }
        };

        // Clamp aos bounds da janela (função pura, testada headless) + clip: o painel
        // NUNCA vaza — texto/linhas além do retângulo são cortados ou rolam, não vazam.
        let rect = dashboard::dashboard_panel_rect(viewport.0, viewport.1);
        let panel = div()
            .id("dash-panel")
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.w))
            .h(px(rect.h))
            .overflow_hidden()
            .bg(rgb(th.surface.panel))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            // Fusão: painel de chrome em IBM Plex Sans, escala de token (cascateia aos filhos).
            .font_family(th.typography.family.ui)
            .child(
                div()
                    .text_size(px(f32::from(th.typography.size.body)))
                    .text_color(rgb(th.text.primary))
                    .child("Atividade e custos do time"),
            )
            .child(
                // Custo do Espaço HOJE — dinheiro primeiro, com o marcador honesto
                // (inclui a parcela ADOTADA fora da pasta do Espaço, rotulada).
                div()
                    .id("dash-total")
                    .text_size(px(f32::from(th.typography.size.subtitle)))
                    .text_color(rgb(th.text.primary))
                    .child(format!("Hoje: {}", total.line.text)),
            )
            .child(
                div()
                    .text_size(px(f32::from(th.typography.size.caption)))
                    .text_color(rgb(th.text.muted))
                    .child(format!(
                        "motor: {} · {}",
                        engine.label,
                        dashboard::COST_TOOLTIP
                    )),
            );
        // Linhas por terminal num viewport ROLÁVEL próprio (lição do onboarding: root de
        // scroll é BLOCO `overflow_y_scroll`; o flex fica num wrapper interno). Muitos
        // terminais → rola DENTRO do painel; nunca vaza a base da janela.
        let mut rows_box = div().flex().flex_col().gap_2();
        for (i, row) in rows.into_iter().enumerate() {
            let color = state_color(row.state);
            // Custo do NÓ (hierarquia P6: estado → custo → atividade): valor + ressalva
            // de atribuição na MESMA linha (fora do Espaço/compartilhado/não rastreável);
            // sem projeção do log para o nó → "—" honesto, nunca chute.
            let card = cards_by_node.get(&row.node);
            let cost_text = match card {
                Some(c) => match c.attribution.note() {
                    Some(note) => format!("{} · {}", c.cost_today.text, note),
                    None => c.cost_today.text.clone(),
                },
                None => "—".to_owned(),
            };
            let cost_color = if card.is_some_and(|c| c.cost_today.has_data) {
                th.text.primary
            } else {
                th.text.muted
            };
            // F1-2-4 (entry point): "✎ Editar" abre a edição de 1ª classe (M6-E) deste Agente. Só
            // terminais (nota/pasta não têm papel/doutrina). PENDENTE TELA DO FUNDADOR (gpui não-headless).
            let node_id = row.node;
            let is_terminal = lock(&self.nodes.model)
                .nodes
                .get(&node_id)
                .is_some_and(|v| matches!(v.kind, NodeKind::Terminal));
            let mut name_line = div().flex().flex_row().items_center().gap_2().child(
                div()
                    .flex_1()
                    .text_size(px(f32::from(th.typography.size.body)))
                    .text_color(rgb(th.text.primary))
                    .child(format!("{}  ● {}", row.name, row.state.label())),
            );
            // F3-0-6: pílula do nível de raciocínio (modelo·effort) em pt-br leigo — chip neutro
            // (não compete com a cor SEMÂNTICA do estado), modelo muted quando o log o correlaciona.
            if let Some(badge) = effort_by_node.get(&node_id.to_string()) {
                name_line = name_line.child(
                    div()
                        .id(("dash-effort", i as u64))
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .text_size(px(f32::from(th.typography.size.small)))
                        .text_color(rgb(th.text.secondary))
                        .child(text!(badge.surface_text())),
                );
            }
            if is_terminal {
                name_line = name_line.child(
                    div()
                        .id(("dash-edit", i as u64))
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .text_size(px(f32::from(th.typography.size.small)))
                        .text_color(rgb(th.accent.primary))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, cx| {
                            view.open_agent_modal_edit(node_id, cx);
                        }))
                        .child(text!("✎ Editar")),
                );
            }
            rows_box = rows_box.child(
                div()
                    .id(("dash-row", i as u64))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .bg(rgb(th.surface.card))
                    .child(name_line)
                    .child(
                        div()
                            .text_size(px(f32::from(th.typography.size.small)))
                            .text_color(rgb(cost_color))
                            .child(cost_text),
                    )
                    .child(
                        div()
                            .text_size(px(f32::from(th.typography.size.small)))
                            .text_color(rgb(color))
                            .child(row.activity),
                    ),
            );
        }
        panel.child(
            div()
                .id("dash-rows")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(rows_box),
        )
    }

    /// **Fix de tela (F1-2-1/inv#6)**: abre a janela de Ajustes (T7 — Aparência/Espaços/T8) SOB
    /// DEMANDA — engrenagem na barra, `⌘,` e paleta. Antes só existia atrás de
    /// `LINA_PERSIST_PANEL=1` (dev) e o tema era INALCANÇÁVEL em produção. Janela única: se já
    /// há um painel aberto, reativa em vez de duplicar.
    fn open_settings_window(&mut self, cx: &mut Context<Self>) {
        let existing = cx
            .windows()
            .into_iter()
            .find_map(|w| w.downcast::<persistence_ui::PersistenceView>());
        if let Some(handle) = existing {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        persistence_ui::open_window(
            cx,
            Arc::clone(&self.nodes.model),
            self.nodes.store_handle(),
            self.nodes.lina_home().join("events"),
        );
    }

    // ───────────────────────── F1-2-2 · M6/M6-E (modal de Agente) ─────────────────────────

    /// Nomes vivos do Espaço (dedup "(2)" do modal).
    fn live_names(&self) -> Vec<String> {
        self.nodes
            .cards()
            .into_iter()
            .map(|(_, v)| v.name)
            .collect()
    }

    /// Abre o M6 em modo CRIAR.
    fn open_agent_modal_create(&mut self, cx: &mut Context<Self>) {
        let mut modal = agent_modal::AgentModal::new_create(
            Arc::new(role_suggester::W31Suggester),
            self.live_names(),
        );
        // FIX-3: prefill = o nível EFETIVO do Espaço (honesto: é o que o novo Agente terá hoje).
        modal.set_autonomy(self.ws_autonomy);
        self.open_agent_modal(modal, cx);
    }

    /// Abre o M6-E em modo EDITAR (pré-preenchido com nome + papel correntes do nó).
    fn open_agent_modal_edit(&mut self, node: NodeId, cx: &mut Context<Self>) {
        let Some((name, node_autonomy)) = lock(&self.nodes.model)
            .nodes
            .get(&node)
            .map(|v| (v.name.clone(), v.autonomy))
        else {
            return; // nó sumiu entre o build da paleta e o clique — nada a editar.
        };
        let role = self.nodes.node_role(node);
        // ADR 0037: o cwd REAL do nó (projeção do log) — o modal mostra a pasta de verdade e a
        // usa para detectar a troca no Salvar.
        let cwd = self.nodes.node_cwd(node);
        // ADR 0039: o motor atual do nó — pré-seleciona o chip certo e detecta a troca de motor.
        let cli = self.nodes.node_cli(node);
        let modal = agent_modal::AgentModal::new_edit(
            Arc::new(role_suggester::W31Suggester),
            node,
            &name,
            role.as_deref(),
            // FIX-3 costura: o nível REAL do nó (per-Agente, `NodeView.autonomy`) — o modal de
            // edição mostra o MESMO que o badge do card (consistência badge↔modal).
            node_autonomy,
            cwd.as_deref().map(std::path::Path::new),
            cli.as_deref(),
            self.live_names(),
        );
        self.open_agent_modal(modal, cx);
    }

    /// Abre o modal + dispara a VARREDURA de motores fora da thread de UI (lição W4-1: discovery
    /// off-thread) + o loop de ticks (~50ms) que dirige o debounce do co-piloto e consome a
    /// varredura. O loop morre sozinho quando o modal fecha.
    fn open_agent_modal(&mut self, modal: agent_modal::AgentModal, cx: &mut Context<Self>) {
        let scan: Arc<Mutex<Option<Vec<agent_modal::Engine>>>> = Arc::new(Mutex::new(None));
        self.modal_scan = Some(Arc::clone(&scan));
        let lina_home = self.nodes.lina_home().to_path_buf();
        thread::spawn(move || {
            // Profiles TOML do DISCO (critério 7) + binários no PATH (W4-1). Nunca na thread de UI.
            let profiles = agent_modal::load_profiles(&agent_modal::profiles_dir(&lina_home));
            let found = lina_core::discover_clis();
            *lock(&scan) = Some(agent_modal::engines_from(&found, &profiles));
        });
        self.agent_modal = Some(modal);
        cx.spawn(async move |this, cx| loop {
            let Ok(open) = this.update(cx, |view, cx| {
                let Some(m) = view.agent_modal.as_mut() else {
                    view.modal_scan = None;
                    return false;
                };
                let mut changed = m.tick();
                if let Some(scan) = &view.modal_scan {
                    if let Some(engines) = lock(scan).take() {
                        m.set_engines(engines);
                        changed = true;
                    }
                }
                if changed {
                    cx.notify();
                }
                true
            }) else {
                break;
            };
            if !open {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
        })
        .detach();
        cx.notify();
    }

    fn modal_select_engine(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.select_engine(idx);
            cx.notify();
        }
    }

    /// FIX-3: clique numa opção da seção AUTONOMIA (criar E editar).
    fn modal_set_autonomy(&mut self, a: Autonomy, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.set_autonomy(a);
            cx.notify();
        }
    }

    /// F3-0-6: clique numa opção do controle nomeado de Raciocínio (espelha `modal_set_autonomy`).
    fn modal_set_effort(&mut self, e: lina_core::Effort, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.set_effort(e);
            cx.notify();
        }
    }

    fn modal_accept_suggestion(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.accept_suggestion();
            cx.notify();
        }
    }

    fn modal_toggle_why(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.toggle_why();
            cx.notify();
        }
    }

    fn modal_open_gallery(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.open_gallery();
            cx.notify();
        }
    }

    fn modal_gallery_pick(&mut self, idx: Option<usize>, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.gallery_pick(idx);
            cx.notify();
        }
    }

    fn modal_toggle_advanced(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.toggle_advanced();
            cx.notify();
        }
    }

    /// Rodada 360 (ADR 0022 §4): liga/desliga o CONSENTIMENTO do kit de integração na
    /// pasta própria escolhida no modal (a Lina só escreve na pasta do usuário com isto).
    fn modal_toggle_kit_consent(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.toggle_kit_consent();
            cx.notify();
        }
    }

    /// ADR 0037: liga/desliga "reiniciar agora na nova pasta" (toggle do modal de edição).
    fn modal_toggle_restart_now(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.toggle_restart_now();
            cx.notify();
        }
    }

    fn modal_focus(&mut self, f: agent_modal::FocusField, cx: &mut Context<Self>) {
        if let Some(m) = self.agent_modal.as_mut() {
            m.set_focus(f);
            cx.notify();
        }
    }

    fn modal_cancel(&mut self, cx: &mut Context<Self>) {
        self.agent_modal = None;
        cx.notify();
    }

    // ── F4-0-2 (UI) · modal de credencial de canal ──
    /// Abre o modal de upload de credencial (paleta "Conectar um canal").
    fn open_credential_modal(&mut self, cx: &mut Context<Self>) {
        self.credential_modal = Some(credential_modal::CredentialModal::new());
        cx.notify();
    }

    /// Move o foco entre os campos do modal de credencial (clique na tela).
    fn credential_focus(&mut self, f: credential_modal::CredField, cx: &mut Context<Self>) {
        if let Some(m) = self.credential_modal.as_mut() {
            m.set_focus(f);
            cx.notify();
        }
    }

    /// Fecha o modal de credencial sem guardar — o que foi digitado some (estado é RAM).
    fn credential_cancel(&mut self, cx: &mut Context<Self>) {
        self.credential_modal = None;
        cx.notify();
    }

    /// Guarda a credencial: leva o segredo UMA vez ao cofre pelo canal view→core
    /// ([`push_human_intent`](bridge) → `MailboxPump` carimba `by="human"` → o handler do core chama
    /// `credential::store_channel_credential`, que grava no `SecretVault` e emite
    /// `CredentialStored{channel,scope,key_ref}` — **o VALOR nunca entra no log nem no env**). Modal
    /// incompleto registra um erro leve e segue aberto; completo fecha (o segredo não persiste na
    /// view). `serde_json` escapa o texto livre do usuário com segurança.
    fn credential_commit(&mut self, cx: &mut Context<Self>) {
        let Some(m) = self.credential_modal.as_mut() else {
            return;
        };
        let Some(plan) = m.commit() else {
            cx.notify(); // incompleto: o modal já registrou o erro leve.
            return;
        };
        let payload = serde_json::json!({
            "channel": plan.channel,
            "scope": plan.scope,
            "key": plan.key,
            "value": plan.value,
        })
        .to_string();
        self.nodes.push_human_intent("credential.store", payload);
        self.credential_modal = None;
        cx.notify();
    }

    // ── F4-WA-2c (UI) · modal "Receber um aviso de fora" (webhook) ──
    /// Nomes dos terminais VIVOS com CLI (alvos possíveis do aviso) — exclui notas/pastas e mortos.
    fn live_terminal_names(&self) -> Vec<String> {
        self.nodes
            .cards()
            .into_iter()
            .filter(|(_, v)| {
                matches!(v.kind, NodeKind::Terminal) && !matches!(v.status, NodeStatus::Dead)
            })
            .map(|(_, v)| v.name)
            .collect()
    }

    /// Abre o modal de webhook (paleta "Receber um aviso de fora"), já com a lista de terminais vivos.
    fn open_webhook_modal(&mut self, cx: &mut Context<Self>) {
        self.webhook_modal = Some(webhook_modal::WebhookModal::new(self.live_terminal_names()));
        cx.notify();
    }

    /// Move o foco entre terminal e instrução (clique/Tab).
    fn webhook_focus(&mut self, f: webhook_modal::WebhookField, cx: &mut Context<Self>) {
        if let Some(m) = self.webhook_modal.as_mut() {
            m.set_focus(f);
            cx.notify();
        }
    }

    /// Escolhe o terminal que recebe o aviso (clique numa linha).
    fn webhook_select_node(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.webhook_modal.as_mut() {
            m.select_node(idx);
            cx.notify();
        }
    }

    /// Fixa o nível "Me avisa antes" / "Pode agir" (clique num segmento).
    fn webhook_set_autonomy(
        &mut self,
        level: webhook_modal::WebhookAutonomy,
        cx: &mut Context<Self>,
    ) {
        if let Some(m) = self.webhook_modal.as_mut() {
            m.set_autonomy(level);
            cx.notify();
        }
    }

    /// Fecha o modal de webhook (Cancelar / Pronto / ✕) — o estado é RAM, some na hora.
    fn webhook_cancel(&mut self, cx: &mut Context<Self>) {
        self.webhook_modal = None;
        cx.notify();
    }

    /// Cria o aviso: emite o gesto `webhook.configure` pelo canal view→core
    /// ([`push_human_intent`](bridge) → `MailboxPump` carimba `by="human"` → o handler do core monta a
    /// engine `lina-webhooks`, gera `hook_id`+secret e emite `WebhookConfigured{target_node,instruction,
    /// autonomy_level}`). O modal NÃO fecha: entra em "aguardando" até o servidor devolver o
    /// endereço/senha por [`Self::webhook_present_outcome`]. Modal incompleto registra erro leve e segue
    /// aberto. **O payload NÃO carrega secret** (o servidor o gera — nada sensível trafega aqui).
    fn webhook_commit(&mut self, cx: &mut Context<Self>) {
        let Some(m) = self.webhook_modal.as_mut() else {
            return;
        };
        let Some(plan) = m.commit() else {
            cx.notify(); // incompleto: o modal já registrou o erro leve.
            return;
        };
        let payload = serde_json::json!({
            "target_node": plan.target_node,
            "instruction": plan.instruction,
            "autonomy_level": plan.autonomy_level,
        })
        .to_string();
        self.nodes.push_human_intent("webhook.configure", payload);
        cx.notify();
    }

    /// **Fio de retorno do webhook (F4-WA-1 ↔ 2c).** Drena o `(url,secret)` que o handler do servidor
    /// depositou (`NodeManager::take_webhook_outcome`) e o injeta no modal aberto via
    /// [`Self::webhook_present_outcome`]. **Take-once:** consumido 1× (o secret não fica residente na
    /// fila nem é logado). Devolve `true` enquanto o modal aguarda o retorno (Pending) — o loop de poll
    /// usa isso para acelerar a cadência só quando há algo a esperar. Drenar com o modal fechado
    /// (usuário cancelou) **descarta** o retorno: o `webhook_present_outcome` é no-op e o `(url,secret)`
    /// é dropado (efêmero — nunca persiste).
    fn drain_webhook_outcome(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((url, secret)) = self.nodes.take_webhook_outcome() {
            self.webhook_present_outcome(url, secret, cx);
        }
        self.webhook_modal
            .as_ref()
            .is_some_and(|m| m.stage() == webhook_modal::WebhookStage::Pending)
    }

    /// O servidor gerou o endereço + a senha e os devolve à tela — o modal passa ao estágio de
    /// resultado (exibe 1×). Chamado pelo [`Self::drain_webhook_outcome`] (loop de poll); no-op se o
    /// modal já foi fechado.
    pub(crate) fn webhook_present_outcome(
        &mut self,
        url: String,
        secret: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(m) = self.webhook_modal.as_mut() {
            m.present_outcome(webhook_modal::WebhookOutcome { url, secret });
            cx.notify();
        }
    }

    /// Copia o endereço do aviso para a área de transferência (botão "Copiar endereço").
    fn webhook_copy_url(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self
            .webhook_modal
            .as_ref()
            .and_then(webhook_modal::WebhookModal::outcome)
            .map(|o| o.url.clone())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(url));
        }
    }

    /// Copia a senha de segurança (botão "Copiar senha") — a única forma de levá-la antes de sumir.
    fn webhook_copy_secret(&mut self, cx: &mut Context<Self>) {
        if let Some(secret) = self
            .webhook_modal
            .as_ref()
            .and_then(webhook_modal::WebhookModal::outcome)
            .map(|o| o.secret.clone())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(secret));
        }
    }

    /// "+ Instalar outro motor…"/estado vazio: REUSA o fluxo W4-1 (T1) — abre a janela do
    /// onboarding com o instalador humano. (Instalação EMBUTIDA no modal é aprofundamento da
    /// story — ver DONE; o caminho do leigo nunca tem beco.)
    fn modal_install_engine(&mut self, cx: &mut Context<Self>) {
        let lina_home = self.nodes.lina_home().to_path_buf();
        let onboarding_dir = lina_home
            .parent()
            .map(|p| p.join("onboarding"))
            .unwrap_or_else(|| lina_home.join("onboarding"));
        onboarding::open_window(cx, onboarding_dir, lina_home);
    }

    /// Picker NATIVO de pasta (só-pastas), fora da thread de UI — padrão do segundo cérebro.
    fn modal_pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Usar esta pasta".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update(cx, |view, cx| {
                        if let Some(m) = view.agent_modal.as_mut() {
                            m.set_cwd(p);
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// `[[ Criar Agente ]]` / `[[ Salvar ]]` — o ÚNICO ponto onde o modal vira eventos no log
    /// (critério 6: kill -9 antes daqui não deixa rastro). Erro de validação fica INLINE.
    fn modal_commit(&mut self, window: &Window, cx: &mut Context<Self>) {
        use agent_modal::ModalMode;
        // 1) plano sob o borrow do modal; 2) efeito sobre self com o modal solto.
        enum Plan {
            Create(agent_modal::CreatePlan),
            Save(agent_modal::SavePlan),
        }
        let plan = {
            let Some(m) = self.agent_modal.as_mut() else {
                return;
            };
            let r = match m.mode {
                ModalMode::Create => m.create_plan().map(Plan::Create),
                ModalMode::Edit { .. } => m.save_plan().map(Plan::Save),
            };
            match r {
                Ok(p) => p,
                Err(msg) => {
                    m.set_error(msg);
                    cx.notify();
                    return;
                }
            }
        };
        let result: Result<Option<NodeId>, String> = match &plan {
            Plan::Create(p) => self
                .nodes
                .create_agent_with_autonomy(
                    &p.name,
                    Some(&p.engine),
                    p.cwd.as_deref(),
                    p.role.as_deref(),
                    // ADR 0022 §4: consentimento explícito do modal — sem ele, pasta própria
                    // nasce SEM kit e o card mostra o badge (degradação visível).
                    p.kit_consent,
                    // FIX-3 costura: a autonomia ESCOLHIDA no modal vira `LINA_AUTONOMY`
                    // por-Agente (o guard honra) e a projeção (`NodeView.autonomy`) reflete o badge.
                    p.autonomy,
                )
                .map(Some),
            Plan::Save(p) => {
                // ADR 0037: valida a nova Pasta de Trabalho ANTES de qualquer mutação —
                // atomicidade (não renomeia para depois falhar na pasta) E segurança (NUNCA
                // remove o nó vivo do `restart` se a pasta for inválida → o terminal não some).
                // `validate_workdir` canonicaliza + PROVA escrita (mesma garantia do M9). `None`
                // = a pasta não mudou.
                let cwd_validated: Result<Option<std::path::PathBuf>, String> = match &p.cwd {
                    Some(c) => workspace_boot::validate_workdir(c.as_path())
                        .map(Some)
                        .map_err(|e| e.to_string()),
                    None => Ok(None),
                };
                match cwd_validated {
                    Err(e) => Err(e),
                    Ok(cwd) => {
                        let renamed =
                            self.nodes
                                .rename_node(p.node, &p.name)
                                .and_then(|()| match &p.role {
                                    Some(role) => self.nodes.assign_role(p.node, role),
                                    None => Ok(()),
                                });
                        if let Err(e) = renamed {
                            Err(e)
                        } else if let Some(new_engine) = p.engine.clone() {
                            // ADR 0039: o MOTOR mudou → re-ergue o Agente com o novo CLI (e na nova
                            // pasta, se também trocou; senão `None` mantém a pasta atual). Trocar o
                            // cérebro de um terminal vivo é encerrá-lo e recriá-lo com o novo motor.
                            let profiles = agent_modal::load_profiles(&agent_modal::profiles_dir(
                                self.nodes.lina_home(),
                            ));
                            self.nodes
                                .restart_node_in_dir(
                                    p.node,
                                    cwd.as_deref(),
                                    Some(new_engine),
                                    &profiles,
                                )
                                .map(Some)
                        } else if let Some(c) = cwd {
                            // ADR 0037: `restart_now` re-ergue na nova pasta AGORA (o novo
                            // `TerminalSpawned{cwd}` persiste); senão só grava `NodeCwdSet` (a pasta
                            // passa a valer na próxima abertura — o caminho "só na próxima vez").
                            if p.restart_now {
                                let profiles = agent_modal::load_profiles(
                                    &agent_modal::profiles_dir(self.nodes.lina_home()),
                                );
                                self.nodes
                                    .restart_node_in_dir(p.node, Some(&c), None, &profiles)
                                    .map(Some)
                            } else {
                                self.nodes.set_node_cwd(p.node, &c).map(|()| None)
                            }
                        } else {
                            Ok(None)
                        }
                    }
                }
            }
        };
        match result {
            Ok(created) => {
                if let Plan::Create(p) = &plan {
                    if let Some(note) = &p.dup_note {
                        eprintln!("lina-gpui: M6 — {note}");
                    }
                }
                // FIX-3 costura: a CRIAÇÃO (⌘N) agora carimba `LINA_AUTONOMY` por-Agente
                // (`create_agent_with_autonomy`) e o badge reflete `NodeView.autonomy` — FECHADO.
                // A EDIÇÃO (SavePlan) ainda aguarda a porta de autonomia no `NodeManager` (análoga
                // a `assign_role`, Pedido 3 da costura): até lá, mudar a autonomia no Editar
                // registra mas não re-carimba o env do nó VIVO. NUNCA silencioso (eprintln honesto).
                let pending = match &plan {
                    Plan::Create(_) => None,
                    Plan::Save(p) => p.autonomy,
                };
                if let Some(a) = pending {
                    eprintln!(
                        "lina-gpui: M6 — nova autonomia ({}) registrada na edição; aplicação ao \
                         nó VIVO aguarda a porta de autonomia no NodeManager (Pedido 3 da costura \
                         FIX-3). Recriar o agente já aplica imediatamente.",
                        agent_modal::autonomy_surface_label(a),
                    );
                }
                self.agent_modal = None;
                // M6-E3: o foco vai direto pro input do Agente recém-criado (zero cliques).
                if let Some(node) = created {
                    self.focus(node);
                    self.reveal(node, window);
                }
            }
            Err(e) => {
                // Tabela de erros do fluxo (a): nunca stack trace na tela — copy leiga inline
                // (erros de validação do plano já são leigos; falha de spawn/persistência ganha
                // a mensagem genérica e o detalhe técnico vai ao stderr).
                eprintln!("lina-gpui: M6 — commit falhou: {e}");
                if let Some(m) = self.agent_modal.as_mut() {
                    // Mensagens LEIGAS de pasta (spawn ou `WorkdirError` da edição ADR 0037)
                    // aparecem inteiras; o resto cai na genérica (detalhe técnico só no stderr).
                    let lower = e.to_lowercase();
                    let leiga = if lower.starts_with("não consigo trabalhar nessa pasta")
                        || lower.starts_with("essa pasta não está acessível")
                        || lower.starts_with("não consegui criar a pasta")
                    {
                        e
                    } else {
                        agent_modal::COPY_ERR_COMMIT.to_string()
                    };
                    m.set_error(leiga);
                }
            }
        }
        cx.notify();
    }

    // ───────────────────────── W2-4: seleção · copiar/colar · mouse ─────────────────────────

    /// Posição (mundo) de um nó na projeção.
    fn node_pos(&self, node: NodeId) -> Option<(f32, f32)> {
        lock(&self.nodes.model).nodes.get(&node).map(|v| (v.x, v.y))
    }

    /// Clona o handle do grid de um nó (solta o lock do mapa antes de lockar o grid).
    fn grid_of(&self, node: NodeId) -> Option<Grid> {
        lock(&self.nodes.grids).get(&node).cloned()
    }

    /// Célula `(col,row)` do viewport sob um ponto de TELA, ciente de pan/zoom.
    fn screen_cell(&self, node: NodeId, screen_pt: (f32, f32)) -> Option<(usize, usize)> {
        let pos = self.node_pos(node)?;
        let g = self.grid_of(node)?;
        let dims = lock(&g).dims();
        Some(screen_to_cell(&self.camera, pos, screen_pt, dims))
    }

    /// Traduz uma ação de ponteiro em bytes SGR e injeta no PTY. Devolve `true` se o TUI pediu
    /// mouse reporting E o evento gerou bytes — checa e codifica sob o MESMO lock do grid
    /// (`encode_mouse` testa o reporting por dentro), então é ATÔMICO: sem janela de corrida com
    /// a thread leitora. `false` = a UI trata (seleciona/rola).
    fn send_pointer(
        &self,
        node: NodeId,
        action: PtrAction,
        cell: (usize, usize),
        mods: (bool, bool, bool),
    ) -> bool {
        let Some(g) = self.grid_of(node) else {
            return false;
        };
        let bytes = {
            let guard = lock(&g);
            encode_pointer(&**guard, action, cell, mods)
        };
        match bytes {
            Some(b) => {
                self.input.submit(node, WriteOp::HumanKeys(b));
                true
            }
            None => false,
        }
    }

    /// Limpa a seleção (no backend e na UI).
    fn clear_selection_state(&mut self) {
        if let Some(sel) = self.sel.take() {
            if let Some(g) = self.grid_of(sel.node) {
                lock(&g).clear_selection();
            }
        }
    }

    /// Estende a seleção em curso até a célula sob `screen_pt` (chamado no arraste). Só toca o
    /// backend quando a CÉLULA muda (o mousemove dispara a ~60Hz, muitas vezes na mesma célula).
    /// Devolve `true` se a seleção MUDOU — o chamador então marca a view suja (`cx.notify()`), sem o
    /// qual a view quiescente (terminal IDLE) NÃO repinta o highlight do arraste (BUG C, ver o
    /// `on_mouse_move`).
    fn extend_selection(&mut self, screen_pt: (f32, f32)) -> bool {
        let Some((node, anchor, head)) = self.sel.as_ref().map(|s| (s.node, s.anchor, s.head))
        else {
            return false;
        };
        let Some(cell) = self.screen_cell(node, screen_pt) else {
            return false;
        };
        if cell == head {
            return false;
        }
        if let Some(sel) = self.sel.as_mut() {
            sel.head = cell;
        }
        if let Some(g) = self.grid_of(node) {
            lock(&g).set_selection(anchor.0, anchor.1, cell.0, cell.1);
        }
        // BUG C (instrumentação de-dupada por CÉLULA, não por frame): o fundador grepa `sel ·` no
        // stderr p/ confirmar, sem ver a tela, que o head ESTENDE contínuo (anchor fixo; norm cresce).
        eprintln!(
            "lina-gpui: sel · node={node} anchor=({},{}) head=({},{}) norm={:?}",
            anchor.0,
            anchor.1,
            cell.0,
            cell.1,
            normalize_sel(anchor, cell)
        );
        true
    }

    /// Texto da seleção atual (do backend, respeitando wrap), ou `None`.
    fn selection_text(&self) -> Option<String> {
        let sel = self.sel.as_ref()?;
        let g = self.grid_of(sel.node)?;
        let t = lock(&g).selection_text();
        t
    }

    /// O nó focado está em modo bracketed-paste?
    fn focused_bracketed(&self) -> bool {
        self.grid_of(self.focused)
            .is_some_and(|g| lock(&g).mode().bracketed_paste)
    }

    /// Mouse-down sobre o grid de um nó (MODELO B): arrasto PURO inicia SELEÇÃO local sempre; ⌥
    /// Option encaminha o mouse ao TUI (se ele pediu reporting). Ver [`pointer_route`].
    fn pointer_down(&mut self, node: NodeId, ev: &MouseDownEvent) {
        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
        let Some(cell) = self.screen_cell(node, pos) else {
            return;
        };
        let mods = (ev.modifiers.shift, ev.modifiers.alt, ev.modifiers.control);
        self.focus(node);
        // Toda nova interação no grid zera a seleção anterior (de qualquer card).
        self.clear_selection_state();
        // MODELO B: arrasto PURO SEMPRE seleciona local; ⌥ Option encaminha o mouse ao agente. Só
        // consultamos/encaminhamos ao TUI sob Option — `send_pointer` é ATÔMICO (checa reporting +
        // codifica sob o MESMO lock), então só o invocamos quando Option está segurado (curto-circuito
        // do `&&`). Sem Option, o arrasto NUNCA vira mouse-report → cai na seleção local, mesmo com o
        // Claude (mouse-reporting ON). Ver [`pointer_route`] p/ a política (testada).
        let option_held = ev.modifiers.alt;
        let tui_accepted = option_held && self.send_pointer(node, PtrAction::Press, cell, mods);
        match pointer_route(option_held, tui_accepted) {
            PointerRoute::ForwardToTui => self.report_node = Some(node),
            PointerRoute::LocalSelect => {
                self.sel = Some(Sel {
                    node,
                    anchor: cell,
                    head: cell,
                });
                self.dragging_sel = true;
            }
        }
    }

    /// ⌘C: copia o texto da seleção para o clipboard do sistema.
    fn copy_selection(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// ⌘V: cola o clipboard no PTY focado via bracketed-paste (sanitizado por `build_paste`).
    fn paste_clipboard(&self, cx: &mut Context<Self>) {
        // Guard: só cola se o nó focado ainda está vivo — o pump pode tê-lo removido (NodeDied)
        // entre o frame e este keystroke. Evita colagem perdida num nó fantasma.
        if !lock(&self.nodes.model).nodes.contains_key(&self.focused) {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let bytes = build_paste(&text, self.focused_bracketed());
        if !bytes.is_empty() {
            self.input.submit(self.focused, WriteOp::HumanKeys(bytes));
        }
    }

    /// Marca `node` como focado e o traz para a FRENTE (z mais alto): clicar/abrir um card o põe
    /// no topo da pilha (z-order).
    fn focus(&mut self, node: NodeId) {
        self.focused = node;
        self.z_next = self.z_next.wrapping_add(1);
        self.z_order.insert(node, self.z_next);
    }

    /// z (profundidade) de um nó: maior = mais à frente; nós nunca focados ficam no fundo (0).
    fn z_of(&self, node: &NodeId) -> u64 {
        self.z_order.get(node).copied().unwrap_or(0)
    }

    /// Cards em ordem de z CRESCENTE (último = topo) — base do render (desenha o topo por
    /// último) e do hit-test (resolve sobreposição pelo maior z).
    fn cards_z_asc(&self) -> Vec<(NodeId, (f32, f32))> {
        let mut cz: Vec<(NodeId, (f32, f32))> = self
            .nodes
            .cards()
            .iter()
            .map(|(id, nv)| (*id, (nv.x, nv.y)))
            .collect();
        cz.sort_by_key(|(id, _)| self.z_of(id));
        cz
    }

    /// Auto-pan: traz o card do nó `node` INTEIRAMENTE para dentro da viewport visível. É o que
    /// faz o terminal recém-criado **aparecer** — o [`next_free_slot`] pode posicioná-lo além da
    /// borda da janela (3 cards de 680px não cabem numa janela de 1450px), e o pan o revela.
    fn reveal(&mut self, node: NodeId, window: &Window) {
        let card = lock(&self.nodes.model).nodes.get(&node).map(|v| (v.x, v.y));
        if let Some(card) = card {
            let vp = window.viewport_size();
            self.camera.reveal(
                card,
                (CARD_W, CARD_H),
                (f32::from(vp.width), f32::from(vp.height)),
            );
        }
    }

    /// Encerra `node` (mata o CLI), limpa o z-order e — SE era o nó focado — reposiciona o foco num
    /// nó vivo (nunca tela em branco) e o revela. Fonte única do "fechar nó": [`close_focused`], o ✕
    /// do header do card e a ação `CloseNode` da toolbar (F2-2-5) passam todos por aqui.
    fn close_node(&mut self, node: NodeId, window: &Window) {
        if let Err(e) = self.nodes.remove_node(node) {
            eprintln!("lina-gpui: fechar nó: {e}");
            return;
        }
        self.z_order.remove(&node);
        // Fechar um nó PERIFÉRICO não rouba o foco; só reposiciona se quem saiu era o focado.
        if self.focused == node {
            // `first` num let SEPARADO: solta o lock do model ANTES do reveal (que re-locka o
            // model) — em edition 2021 o guard temporário de um `if let` viveria por todo o bloco,
            // causando re-lock na mesma thread (deadlock).
            let first = lock(&self.nodes.model).order.first().copied();
            if let Some(first) = first {
                self.focus(first);
                self.reveal(first, window);
            }
        }
    }

    /// Fecha o nó focado (⌘⌫). Delega ao [`close_node`] (fonte única).
    fn close_focused(&mut self, window: &Window) {
        self.close_node(self.focused, window);
    }

    /// **F1-2-4 — ⌘R abre a EDIÇÃO de 1ª classe (M6-E) do Agente focado.** Substitui o stub de dev
    /// que ciclava 5 nomes fixos: agora o atalho é descobrível e edita nome/papel pelo modal completo
    /// (re-injeção de doutrina + confirmação no caminho do Salvar). Só para terminais — nota/pasta não
    /// têm papel/doutrina. Devolve `true` se abriu (foco era um terminal).
    fn edit_focused_agent(&mut self, cx: &mut Context<Self>) -> bool {
        let is_terminal = lock(&self.nodes.model)
            .nodes
            .get(&self.focused)
            .is_some_and(|v| matches!(v.kind, NodeKind::Terminal));
        if is_terminal {
            self.open_agent_modal_edit(self.focused, cx);
        }
        is_terminal
    }

    fn handle_key(&mut self, ev: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        // F1-2-2 · M6/M6-E — MODAL DE AGENTE: enquanto aberto, o teclado o dirige (digitar monta o
        // campo focado; Tab aceita a sugestão do co-piloto; Enter cria/salva; Esc arma/descarta).
        // EXCEÇÃO de precedência ABSOLUTA (fluxo b): ⌘⏎ com Pedido pendente na Fila NÃO cria —
        // cai no gate humano abaixo (a custódia vence o atalho de Criar; M6-E3).
        if self.agent_modal.is_some() {
            let custody_pending = lock(&self.desk).front().is_some();
            let cmd_enter = ks.modifiers.platform && (ks.key == "enter" || ks.key == "return");
            if !(cmd_enter && custody_pending) {
                // Seletor de papéis aberto ([Trocar]): ↑↓ navega, Enter escolhe, Esc fecha (só o
                // seletor), digitação filtra — F1-2-3 + a11y do gate (teclado de ponta a ponta).
                if self.agent_modal.as_ref().is_some_and(|m| m.gallery_open()) {
                    if let Some(m) = self.agent_modal.as_mut() {
                        match ks.key.as_str() {
                            "escape" => m.gallery_close(),
                            "up" => m.gallery_move(-1),
                            "down" => m.gallery_move(1),
                            "enter" | "return" => {
                                m.gallery_pick(None);
                            }
                            "backspace" => m.gallery_backspace(),
                            _ => {
                                if !ks.modifiers.platform && !ks.modifiers.control {
                                    if let Some(kc) = ks.key_char.as_ref().filter(|c| {
                                        !c.is_empty() && !c.chars().any(char::is_control)
                                    }) {
                                        m.gallery_type(kc);
                                    }
                                }
                            }
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                match ks.key.as_str() {
                    "escape" => {
                        if self.agent_modal.as_mut().is_some_and(|m| m.escape()) {
                            self.agent_modal = None;
                        }
                    }
                    "tab" => {
                        if let Some(m) = self.agent_modal.as_mut() {
                            m.accept_suggestion();
                        }
                    }
                    "enter" | "return" => self.modal_commit(window, cx),
                    "backspace" => {
                        if let Some(m) = self.agent_modal.as_mut() {
                            m.backspace();
                        }
                    }
                    _ => {
                        // Imprimível → campo focado (ignora chords ⌘/Ctrl e teclas-controle).
                        if !ks.modifiers.platform && !ks.modifiers.control {
                            if let Some(kc) = ks
                                .key_char
                                .as_ref()
                                .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                            {
                                if let Some(m) = self.agent_modal.as_mut() {
                                    m.type_char(kc);
                                }
                            }
                        }
                    }
                }
                // Modal: a janela é dona do teclado. `stop_propagation` impede o gpui de rotear a
                // tecla ao `insertText` do IME (senão a letra VAZARIA ao PTY focado — lição M2).
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        // F4-0-2 (UI) — MODAL DE CREDENCIAL: enquanto aberto, o teclado o dirige (digitar monta o
        // campo focado; Tab cicla; Enter guarda; Esc arma/descarta; Backspace apaga). Mesma
        // precedência/`stop_propagation` do modo criação — a letra do segredo NUNCA vaza ao PTY.
        if self.credential_modal.is_some() {
            match ks.key.as_str() {
                "escape" => {
                    if self.credential_modal.as_mut().is_some_and(|m| m.escape()) {
                        self.credential_modal = None;
                    }
                }
                "tab" => {
                    let dir = if ks.modifiers.shift { -1 } else { 1 };
                    if let Some(m) = self.credential_modal.as_mut() {
                        m.cycle_focus(dir);
                    }
                }
                "enter" | "return" => self.credential_commit(cx),
                "backspace" => {
                    if let Some(m) = self.credential_modal.as_mut() {
                        m.backspace();
                    }
                }
                _ => {
                    if !ks.modifiers.platform && !ks.modifiers.control {
                        if let Some(kc) = ks
                            .key_char
                            .as_ref()
                            .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                        {
                            if let Some(m) = self.credential_modal.as_mut() {
                                m.type_char(kc);
                            }
                        }
                    }
                }
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // F4-WA-2c (UI) — MODAL "RECEBER UM AVISO DE FORA": enquanto aberto, o teclado o dirige (digitar
        // monta a instrução; Tab cicla terminal↔instrução; ↑/↓ escolhem o terminal quando a lista está
        // focada; Enter cria — ou fecha no resultado; Esc arma/descarta; Backspace apaga). Mesma
        // precedência/`stop_propagation` do modal de credencial.
        if self.webhook_modal.is_some() {
            let stage = self
                .webhook_modal
                .as_ref()
                .map(webhook_modal::WebhookModal::stage);
            match ks.key.as_str() {
                "escape" => {
                    if self.webhook_modal.as_mut().is_some_and(|m| m.escape()) {
                        self.webhook_modal = None;
                    }
                }
                "tab" => {
                    let dir = if ks.modifiers.shift { -1 } else { 1 };
                    if let Some(m) = self.webhook_modal.as_mut() {
                        m.cycle_focus(dir);
                    }
                }
                "up" | "down" => {
                    if let Some(m) = self.webhook_modal.as_mut() {
                        if m.focus_field() == webhook_modal::WebhookField::Terminal {
                            m.cycle_node(if ks.key == "up" { -1 } else { 1 });
                        }
                    }
                }
                "enter" | "return" => {
                    if stage == Some(webhook_modal::WebhookStage::Outcome) {
                        self.webhook_cancel(cx);
                    } else {
                        self.webhook_commit(cx);
                    }
                }
                "backspace" => {
                    if let Some(m) = self.webhook_modal.as_mut() {
                        m.backspace();
                    }
                }
                _ => {
                    if !ks.modifiers.platform && !ks.modifiers.control {
                        if let Some(kc) = ks
                            .key_char
                            .as_ref()
                            .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                        {
                            if let Some(m) = self.webhook_modal.as_mut() {
                                m.type_char(kc);
                            }
                        }
                    }
                }
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // F3-1-7 — EDITOR DO ENTENDIMENTO da Goal ("Quero ajustar"): digita a correção; Enter re-envia
        // `goal.interpret` pelo canal humano (ADR 0036); Esc cancela; Backspace apaga. Modal leve, mesma
        // precedência do modo criação (consome a tecla p/ o IME não vazar a letra ao PTY focado).
        if self.editing_goal.is_some() {
            match ks.key.as_str() {
                "escape" => self.editing_goal = None,
                "enter" | "return" => {
                    if let Some((goal_id, buf)) = self.editing_goal.take() {
                        let understanding = buf.trim();
                        if !understanding.is_empty() {
                            // `by="human"` é carimbado SERVER-SIDE na MailboxPump; a view só enfileira o
                            // intent. `serde_json` escapa o texto livre do usuário (aspas etc.) com segurança.
                            let payload = serde_json::json!({
                                "goal_id": goal_id,
                                "interpretation": understanding,
                                "strategy": "ajustado por voce",
                            })
                            .to_string();
                            self.nodes.push_human_intent("goal.interpret", payload);
                        }
                    }
                }
                "backspace" => {
                    if let Some((_, buf)) = self.editing_goal.as_mut() {
                        buf.pop();
                    }
                }
                _ => {
                    if !ks.modifiers.platform && !ks.modifiers.control {
                        if let Some(kc) = ks
                            .key_char
                            .as_ref()
                            .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                        {
                            if let Some((_, buf)) = self.editing_goal.as_mut() {
                                buf.push_str(kc);
                            }
                        }
                    }
                }
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // W4-2 · M3/M4 — MODO CRIAÇÃO de nota/pasta (disparado pela paleta): digita o título; Enter cria
        // (semeia o nó VIVO + foca); Esc cancela; Backspace apaga. Modal leve, irmão do naming (M2).
        if self.creating.is_some() {
            match ks.key.as_str() {
                "escape" => self.creating = None,
                "enter" | "return" => {
                    if let Some((kind, buf)) = self.creating.take() {
                        let title = buf.trim().to_string();
                        if !title.is_empty() {
                            match self.nodes.create_artifact(kind, &title) {
                                Ok(node) => {
                                    self.focus(node);
                                    self.reveal(node, window);
                                }
                                Err(e) => {
                                    eprintln!("lina-gpui: M3/M4 criar {kind:?} '{title}': {e}")
                                }
                            }
                        }
                    }
                }
                "backspace" => {
                    if let Some((_, buf)) = self.creating.as_mut() {
                        buf.pop();
                    }
                }
                _ => {
                    if !ks.modifiers.platform && !ks.modifiers.control {
                        if let Some(kc) = ks
                            .key_char
                            .as_ref()
                            .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                        {
                            if let Some((_, buf)) = self.creating.as_mut() {
                                buf.push_str(kc);
                            }
                        }
                    }
                }
            }
            // Modal: consome a tecla p/ o IME não vazar a letra do título ao PTY (ver naming acima).
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // W4-2 · M1 — PALETA DE COMANDOS: se aberta, as teclas a DIRIGEM (digita filtra · ↑↓ navega ·
        // Enter executa · Esc fecha). Modificador (⌘/Ctrl) NÃO entra na query (evita ⌘V virar texto).
        if self.palette.is_open() {
            let ch = if ks.modifiers.platform || ks.modifiers.control {
                None
            } else {
                ks.key_char.as_deref()
            };
            if let Some(action) = self.palette.handle_key(ks.key.as_str(), ch) {
                // F2-2-4 (costura): o Enter acabou de registrar a escolha no MRU do modelo
                // (frente = a key executada). Persiste o fato no log (hit-count por replay)
                // e o MRU nos settings ANTES de rodar a ação (ações podem trocar de Espaço).
                if let Some(chosen) = self.palette.mru().first().cloned() {
                    let active_root = lock(&self.runtimes).active.clone();
                    self.append_to_workspace(
                        &active_root,
                        &lina_core::DomainEvent::PaletteCommandInvoked { key: chosen },
                    );
                    let mut settings = persistence_ui::load_settings(&self.settings_dir);
                    settings.palette_mru = self.palette.mru().to_vec();
                    persistence_ui::save_settings(&self.settings_dir, &settings);
                }
                self.run_palette_action(action, window, cx);
            }
            // Modal: consome a tecla de filtro p/ o IME não vazar a query ao PTY (ver naming acima).
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // F1-4-4 · M9 — modal Criar Espaço aberto: o teclado o DIRIGE (precedência de modal,
        // spec §4: tab percorre, ←→/↑↓ nos cards, Enter SUBMETE SEMPRE, Esc fecha sem criar).
        if self.create_space_modal.is_some() {
            self.m9_handle_key(ks, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // Spec §M8 (a11y) — toast de arquivar vivo: `⌘Z` desfaz de QUALQUER lugar; `Tab`
        // foca o `[ Desfazer ]` e `Enter` o ativa SÓ com o rail aberto (de onde o gesto
        // partiu) — fora dele, Tab/Enter seguem ao terminal focado (não rouba digitação).
        if let Some(t) = &self.archive_toast {
            // r5 BUG 4: Tab/Enter do toast SÓ quando o rail é dono do teclado (`kbd`) —
            // rail meramente aberto não rouba mais Tab/Enter do terminal. ⌘Z segue global.
            if self.sidebar.state.kbd || ks.modifiers.platform {
                match sidebar::archive_toast_key(
                    ks.key.as_str(),
                    ks.modifiers.platform,
                    t.undo_focused,
                ) {
                    sidebar::ArchiveToastKey::Undo => {
                        self.undo_pending_archive(cx);
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    sidebar::ArchiveToastKey::FocusUndo => {
                        if let Some(t) = self.archive_toast.as_mut() {
                            t.undo_focused = true;
                        }
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    sidebar::ArchiveToastKey::None => {}
                }
            }
        }
        // r5 BUG 4 · M8 — o rail NUNCA rouba o teclado: o roteador puro decide o destino
        // pela dupla (expanded, kbd). Rail aberto SEM foco deixa tudo FLUIR ao terminal
        // (inclusive Esc); `⌘F` foca a busca; `⌘O` alterna de qualquer estado (BUG 1).
        match sidebar::rail_key_route(
            self.sidebar.state.expanded,
            self.sidebar.state.kbd,
            ks.key.as_str(),
            ks.modifiers.platform,
        ) {
            sidebar::RailKey::ToggleRail => {
                self.toggle_sidebar(cx);
                cx.stop_propagation();
                return;
            }
            sidebar::RailKey::FocusSearch => {
                self.sidebar.state.focus_search();
                cx.stop_propagation();
                cx.notify();
                return;
            }
            sidebar::RailKey::ToRail => {
                match ks.key.as_str() {
                    "escape" => {
                        // Esc fecha o rail SÓ porque o foco está nele (BUG 4(c)).
                        if self.sidebar.state.renaming.is_some() {
                            self.sidebar.state.cancel_rename();
                        } else {
                            self.sidebar.state.close();
                            self.a11y_live.announce("Lista de Espaços recolhida.");
                        }
                    }
                    "up" => self.sidebar.state.select_prev(),
                    "down" => self.sidebar.state.select_next(),
                    // r5 BUG 2 (teclado): ⌘⌫ arquiva a linha selecionada — mesmo destino
                    // do botão [ Arquivar ] (toast com Desfazer da rodada r4).
                    "backspace" if ks.modifiers.platform => {
                        if let Some(row) = self.sidebar.state.selected_row() {
                            let root = row.ws_root.clone();
                            self.archive_workspace(&root, cx);
                        }
                    }
                    "backspace" => self.sidebar.state.backspace(),
                    "enter" | "return" => {
                        if let Some((root, novo)) = self.sidebar.state.commit_rename() {
                            self.rename_workspace(&root, &novo, cx);
                        } else if let Some(row) = self.sidebar.state.selected_row() {
                            let root = row.ws_root.clone();
                            self.switch_to_workspace(root, cx);
                            self.refresh_sidebar_rows();
                        }
                    }
                    _ => {
                        if !ks.modifiers.platform && !ks.modifiers.control {
                            if let Some(ch) = ks.key_char.as_deref() {
                                self.sidebar.state.type_char(ch);
                            }
                        }
                    }
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
            sidebar::RailKey::PassThrough => {}
        }
        // F1-4-4 · ⌘1..9 troca pelo índice ESTÁVEL do registry (funciona com o M8 FECHADO —
        // posição de criação, contando arquivados; arquivado/ausente = no-op). Contrato T4 §4.
        if ks.modifiers.platform {
            if let Ok(n) = ks.key.parse::<usize>() {
                if (1..=9).contains(&n) {
                    let target = lina_core::default_registry_path()
                        .and_then(|p| lina_core::WorkspaceRegistry::load(p).ok())
                        .and_then(|r| {
                            r.entries()
                                .get(n - 1)
                                .filter(|e| !e.archived)
                                .map(|e| e.path.clone())
                        });
                    if let Some(root) = target {
                        self.switch_to_workspace(root, cx);
                        self.refresh_sidebar_rows();
                    }
                    return;
                }
            }
        }
        // ⌘, abre Ajustes (padrão macOS — entry point do T7; fix de tela: tema alcançável).
        if ks.modifiers.platform && ks.key == "," {
            self.open_settings_window(cx);
            return;
        }
        // ⌘N abre o M6 "Novo Agente" (atalho canônico do fluxo (a)).
        if ks.modifiers.platform && ks.key == "n" {
            self.open_agent_modal_create(cx);
            return;
        }
        // ⌘K abre a paleta (rebuild dos comandos com o roster do momento) — mesmo ponto que o botão
        // visível do rail (F2-2-4: a porta tem duas entradas, uma fiação só).
        if ks.modifiers.platform && ks.key == "k" {
            self.open_command_palette(cx);
            return;
        }
        // F1-1-7: ⌘J abre/fecha a Fila de Atenção (proposta do ux-flows; sem colisão).
        if ks.modifiers.platform && ks.key == "j" {
            self.attention_toggle_panel(cx);
            return;
        }
        // W3-6c (ADR 0004) — GATE HUMANO na FRENTE da fila (custódia OU retomada do teto). Teclas NA
        // JANELA do app: o PTY do agente não as sintetiza → inforjável. ⌘⏎ APROVA; ⌘⇧⏎ RECUSA/dispensa
        // (saída p/ limpar flood/erro sem executar — hole 2). Confirma/recusa SÓ a frente. NÃO vai p/ o PTY.
        if ks.modifiers.platform && (ks.key == "enter" || ks.key == "return") {
            let front_id = lock(&self.desk).front().map(|p| p.id().to_string());
            if ks.modifiers.shift {
                match front_id {
                    Some(id) => {
                        lock(&self.desk).reject_requested = Some(id);
                        eprintln!(
                            "lina-gpui: GATE HUMANO — pedido da frente RECUSADO na janela (⌘⇧⏎)"
                        );
                    }
                    None => eprintln!("lina-gpui: ⌘⇧⏎ sem pedido pendente (nada a recusar)"),
                }
            } else {
                match front_id {
                    Some(id) => {
                        lock(&self.desk).confirm_requested = Some(id);
                        eprintln!(
                            "lina-gpui: GATE HUMANO — pedido da frente confirmado na janela (⌘⏎)"
                        );
                    }
                    // F1-1-7: SEM custódia na frente (precedência custódia > permissão
                    // intacta — o bloco acima é o caminho histórico, intocado), o ⌘⏎
                    // decide o TOAST visível de permissão ("foco implícito", ux-flows)
                    // ou ABRE a fila — NUNCA decide o que não está na tela (regra de
                    // ouro). Só REGISTRA a decisão (ADR 0021 §6); o write é F1-1-8.
                    None => {
                        let now = lina_core::now_ms();
                        match self.attention_machine.visible(&self.attention_items, now) {
                            // R2b: pergunta (Choice/Trust) no toast — ⌘⏎ NÃO aprova
                            // (não há o que aprovar): executa a ação primária, IR ATÉ
                            // O TERMINAL (navegação, não decisão).
                            Some(attention_ui::ToastView::Single(idx))
                                if attention_ui::is_goto_only(&self.attention_items[idx]) =>
                            {
                                let node = self.attention_items[idx].node_id.clone();
                                self.attention_goto_node(&node, window, cx);
                            }
                            Some(attention_ui::ToastView::Single(idx))
                                if self.attention_items[idx].kind
                                    == lina_core::AttentionKind::Permission =>
                            {
                                let sid = self.attention_items[idx].stable_id.clone();
                                self.attention_approve(&sid, cx);
                                eprintln!(
                                    "lina-gpui: [ATT] ⌘⏎ aprovou a permissão do toast (registro; sem write)"
                                );
                            }
                            _ if !self.attention_items.is_empty() => {
                                self.attention_open_panel(cx);
                            }
                            _ => eprintln!("lina-gpui: ⌘⏎ sem pedido pendente (nada a confirmar)"),
                        }
                    }
                }
            }
            return;
        }
        // ⌘C copia a seleção; ⌘V cola no PTY focado (bracketed-paste). Ctrl+C/V seguem p/ o PTY.
        if ks.modifiers.platform && ks.key == "c" {
            self.copy_selection(cx);
            return;
        }
        if ks.modifiers.platform && ks.key == "v" {
            self.paste_clipboard(cx);
            return;
        }
        // ⌘R (F1-2-4): abre a EDIÇÃO de 1ª classe (M6-E) do Agente focado — editar nome/papel de um
        // terminal VIVO sem matá-lo (PID preservado), com re-injeção de doutrina no Salvar. Atalho
        // agora descobrível (substitui o stub que ciclava nomes). Registrado no mapa de atalhos do
        // ux-flows.md (ver .entrega-f1-2-4.md — ux-flows é de outro dono).
        if ks.modifiers.platform && ks.key == "r" {
            self.edit_focused_agent(cx);
            return;
        }
        // Atalhos do canvas (NÃO vão para o PTY): ⌘T adiciona (foca+revela); ⌘⌫ fecha o focado;
        // ⌘+ / ⌘- dão zoom in/out no centro da viewport.
        if shortcut_modifier(ks) && ks.key == "t" {
            match self.nodes.add_node() {
                Ok(node) => {
                    self.focus(node);
                    self.reveal(node, window);
                }
                Err(e) => eprintln!("lina-gpui: add_node (⌘T): {e}"),
            }
            return;
        }
        // ⌘W fecha o terminal focado (padrão Mac; o ✕ no card continua). MIGRADO de ⌘Backspace p/
        // liberar Cmd+Delete/⌘Backspace → LIMPAR o input (decisão do fundador): no Mac a tecla
        // "delete" é o `"backspace"` do gpui, então ⌘Backspace agora cai no `keystroke_to_bytes`
        // (Ctrl+U `0x15`). PENDENTE TELA: confirmar que o SO não intercepta ⌘W antes da view.
        if ks.modifiers.platform && ks.key == "w" {
            self.close_focused(window);
            return;
        }
        if ks.modifiers.platform && (ks.key == "=" || ks.key == "+" || ks.key == "-") {
            let vp = window.viewport_size();
            let center = (f32::from(vp.width) / 2.0, f32::from(vp.height) / 2.0);
            let factor = if ks.key == "-" { 0.8 } else { 1.25 };
            self.camera.zoom_by(center, factor);
            return;
        }
        if ks.modifiers.platform && ks.key == "0" {
            self.camera.reset(); // ⌘0: volta ao home (resgate da vista).
            return;
        }
        // F2-3-5 ⌘1: enquadrar TODOS os terminais (zoom-to-fit) — o fundador nunca fica perdido no
        // vazio. No-op se não há cards. Matemática pura e testada em `canvas::zoom`.
        if ks.modifiers.platform && ks.key == "1" {
            let vp = window.viewport_size();
            let viewport = (f32::from(vp.width), f32::from(vp.height));
            let cards: Vec<(f32, f32)> = self.cards_z_asc().into_iter().map(|(_, p)| p).collect();
            if let Some(b) = canvas::zoom::bounds_of(&cards, CARD_W, CARD_H) {
                self.camera = canvas::zoom::zoom_to_fit(
                    b,
                    viewport,
                    48.0,
                    crate::bridge::ZOOM_MIN,
                    crate::bridge::ZOOM_MAX,
                );
                cx.notify();
            }
            return;
        }
        // F2-3-5 ⌘2: focar a SELEÇÃO (hoje = o card focado; multi-seleção é story futura — aí o
        // call-site passa `bounds_of(&selecionados)`, a função não muda).
        if ks.modifiers.platform && ks.key == "2" {
            let vp = window.viewport_size();
            let viewport = (f32::from(vp.width), f32::from(vp.height));
            if let Some((_, (x, y))) = self
                .cards_z_asc()
                .into_iter()
                .find(|(id, _)| *id == self.focused)
            {
                let b = canvas::zoom::Rect {
                    x,
                    y,
                    w: CARD_W,
                    h: CARD_H,
                };
                self.camera = canvas::zoom::zoom_to_selection(
                    b,
                    viewport,
                    48.0,
                    crate::bridge::ZOOM_MIN,
                    crate::bridge::ZOOM_MAX,
                );
                cx.notify();
            }
            return;
        }
        // F2-3-1: Esc cancela um gesto de MOVER card em curso — reverte o NodeView à origem (o
        // preview o moveu em memória) e CONSOME a tecla (não recolhe toast nem vai ao PTY). ZERO
        // NodeMoved: o `finish` nem é chamado. Tem prioridade sobre o Esc-recolhe-toast abaixo.
        if ks.key == "escape" && self.card_drag.is_some() {
            if let Some(g) = self.card_drag.take() {
                let (ox, oy) = g.origin();
                let node = g.node();
                if let Some(nv) = lock(&self.nodes.model).nodes.get_mut(&node) {
                    nv.x = ox;
                    nv.y = oy;
                }
                cx.notify();
            }
            return;
        }
        // F1-1-7: Esc com TOAST visível — **ARBITRAGEM do Maestro (ux-flows vence a
        // copy da story): Esc NUNCA decide**, em TODAS as superfícies. Fail-safe:
        // recusar vira write no terminal na F1-1-8 e pode matar operação longa; tecla
        // reflexa não decide — decisão é clique/⌘⏎ explícito. Esc só RECOLHE o toast
        // p/ fila/badge, SEM evento de decisão (teste não-vacuoso no bridge:
        // `esc_snooze_never_emits_decision_event`). Sem toast: fecha o painel da fila,
        // se aberto. Senão segue ao PTY (intacto).
        if ks.key == "escape" && !ks.modifiers.platform && !ks.modifiers.control {
            let now = lina_core::now_ms();
            if self
                .attention_machine
                .visible(&self.attention_items, now)
                .is_some()
            {
                self.attention_snooze(cx);
                return;
            }
            if self.attention_panel_open {
                self.attention_panel_open = false;
                cx.notify();
                return;
            }
        }
        let grid = lock(&self.nodes.grids).get(&self.focused).cloned();
        let app_cursor = grid.map(|g| lock(&g).mode().app_cursor).unwrap_or(false);
        let bytes = keystroke_to_bytes(ks, app_cursor);
        if !bytes.is_empty() {
            self.input.submit(self.focused, WriteOp::HumanKeys(bytes));
        }
    }
}

#[cfg(windows)]
fn shortcut_modifier(ks: &Keystroke) -> bool {
    ks.modifiers.control || ks.modifiers.platform
}

#[cfg(not(windows))]
fn shortcut_modifier(ks: &Keystroke) -> bool {
    ks.modifiers.platform
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        // F1-2-1: tokens VIVOS do design system — uma cópia por frame (trocar tema/acento no T7
        // re-pinta este render no frame seguinte, sem restart). Nenhuma cor hard-coded abaixo.
        let th = theme::active();

        // W5-1 (instrumentação de FPS): amostra o FRAMETIME = intervalo desde o frame anterior. O loop
        // de animação (`request_animation_frame` + a tarefa `cx.spawn` do pulso) só dirige frames
        // ENQUANTO há atividade (pan/zoom/pulso vivo), então o intervalo é a cadência REAL de desenho.
        // Intervalo > IDLE_GAP_MS = janela ociosa (sem redraw pedido), NÃO frame lento: descarta p/ os
        // percentis medirem FPS *enquanto desenha*. `Instant` monotônico (lição: NUNCA `SystemTime`).
        let frame_now = Instant::now();
        // F1-5-1 ([PROF]): fecha o frame ANTERIOR (só agora o frametime e o carimbo do sentinela
        // de paint existem) e emite o bloco da janela quando ela fecha (~120 frames). Sonda
        // desligada (`LINA_PROF` ausente/0) = um check de bool e nada mais. Atribuição
        // DELIBERADA: como `frame_now` (t0) já foi capturado, o custo de fechar a janela
        // (sort+format+eprintln, 1×/~120 frames) cai na fase `poll` DESTE frame — 1 amostra
        // inflada por janela (p100; p50/p95 de poll seguem robustos). A alternativa (t0 depois)
        // vazaria o custo para o present_vsync do frame anterior — pior.
        for line in self.prof.begin_frame(frame_now) {
            eprintln!("lina-gpui: {line}");
        }
        // F2-0-1 (costura a): carimba o scale factor na sonda — loga só na MUDANÇA (e avisa
        // em LoDPI <1.5x); custo por frame = 1 comparação de f64 quando nada mudou.
        if let Some(line) = self
            .prof
            .observe_scale_factor(f64::from(window.scale_factor()))
        {
            eprintln!("lina-gpui: {line}");
        }
        // F2-0-1 (costura b): janelas fechadas viram fato no log do Espaço ativo (inv#4 — a
        // régua D0 re-deriva o gate por replay). Vec vazio em ~119/120 frames = custo nulo;
        // sonda desligada nunca acumula. Falha de append já é logada pelo próprio helper.
        let summaries = self.prof.take_window_summaries();
        if !summaries.is_empty() {
            let active_root = lock(&self.runtimes).active.clone();
            for s in summaries {
                self.append_to_workspace(
                    &active_root,
                    &lina_core::DomainEvent::ProfWindowMetric {
                        frames: s.frames,
                        budget_ms: s.budget_ms,
                        active_ms: s.active_ms,
                        excess_ms: s.excess_ms,
                        frames_over_budget: s.frames_over_budget,
                        frame_p50_ms: s.frame_p50_ms,
                        frame_p95_ms: s.frame_p95_ms,
                        frame_p99_ms: s.frame_p99_ms,
                        scale_factor: s.scale_factor,
                    },
                );
            }
        }
        if let Some(prev) = self.last_frame_at {
            let dt_ms = frame_now.saturating_duration_since(prev).as_secs_f64() * 1000.0;
            if dt_ms <= IDLE_GAP_MS {
                self.frametime_ms.push(dt_ms);
            }
        }
        self.last_frame_at = Some(frame_now);

        // Os cards desenhados DERIVAM do NodeManager (não de 2 fixos) — add/remove refletem aqui.
        let mut cards = self.nodes.cards();
        let (pulse, event_count, recovering, cost_paused) = {
            let m = lock(&self.nodes.model);
            (m.pulse, m.event_count, m.recovering, m.cost_paused)
        };
        // W4-3: SELO "Time conectado" = full-mesh LÓGICO por MEMBERSHIP (≥2 nós no Espaço), derivado da
        // presença — aparece SEM nenhum tráfego/arraste de cabo (decisão arq §2.2).
        let connected = wiring::team_connected(cards.len());
        // O foco deve apontar SEMPRE a um nó vivo: se o focado saiu (✕/⌘⌫/pump), o destaque
        // sumiria e as teclas iriam p/ um nó morto. Reaponta para o primeiro card vivo.
        if !cards.iter().any(|(id, _)| *id == self.focused) {
            if let Some((first, _)) = cards.first() {
                self.focused = *first;
            }
        }
        // Limpa z_order de nós mortos: remoções pelo pump (NodeDied) não passam pelos handlers
        // de UI, então a entrada ficaria órfã (vazamento ao longo de uma sessão longa).
        self.z_order
            .retain(|id, _| cards.iter().any(|(cid, _)| cid == id));
        // W2-4: invalida seleção / mouse-report se o nó referenciado morreu (mesma disciplina) —
        // senão um ⌘C copiaria de um nó fantasma e o estado ficaria stale.
        if self
            .sel
            .is_some_and(|s| !cards.iter().any(|(id, _)| *id == s.node))
        {
            self.clear_selection_state();
        }
        if self
            .report_node
            .is_some_and(|n| !cards.iter().any(|(id, _)| *id == n))
        {
            self.report_node = None;
        }

        let vp = window.viewport_size();
        let viewport = (f32::from(vp.width), f32::from(vp.height));
        // NUNCA tela em branco: se NADA está visível e o usuário NÃO está arrastando, recentra a
        // câmera (⌘0 / botão 🏠 fazem o mesmo sob demanda). Durante o arrasto, deixa rolar livre.
        if self.drag.is_none()
            && !cards.is_empty()
            && !cards
                .iter()
                .any(|(_, nv)| card_visible(&self.camera, (nv.x, nv.y), (CARD_W, CARD_H), viewport))
        {
            self.camera.reset();
        }
        // IDs de elemento ESTÁVEIS por nó (independem de cull/z-order): ordena por NodeId, que é
        // total e estável p/ o mesmo conjunto de nós — o gpui reusa o elemento certo entre frames.
        let eid: BTreeMap<NodeId, usize> = {
            let mut ids: Vec<NodeId> = cards.iter().map(|(id, _)| *id).collect();
            ids.sort();
            ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect()
        };
        // Z-ORDER: desenha do fundo p/ o topo — o focado/maior z por último, sobre os demais.
        cards.sort_by_key(|(id, _)| self.z_of(id));
        let cam = self.camera;
        let focused = self.focused;

        // W4-6 a11y: live-region "resposta pronta" — anuncia 1×/turno na transição →Idle (status é
        // turn-level), NUNCA a cada byte/`GridDelta`. Observa os status correntes dos nós.
        let a11y_nodes: Vec<(NodeId, String, NodeStatus)> = cards
            .iter()
            .map(|(id, nv)| (*id, nv.name.clone(), nv.status))
            .collect();
        self.a11y_live.observe(&a11y_nodes);
        let a11y_announce: Option<String> = self.a11y_live.current().map(str::to_string);

        let aura_color = if connected {
            rgb(th.accent.primary)
        } else {
            rgb(th.surface.border_muted)
        };
        let mut root = div()
            .id("canvas")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(rgb(th.surface.canvas))
            .border_2()
            .border_dashed()
            .border_color(aura_color)
            .text_color(rgb(th.text.primary))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, ev: &MouseDownEvent, window, cx| {
                    let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                    let v = window.viewport_size();
                    let vp = (f32::from(v.width), f32::from(v.height));
                    let hit =
                        hit_test(&view.camera, pos, &view.cards_z_asc(), (CARD_W, CARD_H), vp);
                    match canvas::drag::drag_mode(hit) {
                        // F2-3-1: card sob o cursor → MOVER, mas só pela BARRA DE TÍTULO (top ~34px
                        // em mundo, escalado por zoom — mesma altura de `fit_dims`). O CORPO segue
                        // para a seleção de texto do terminal (handler interno do grid): arrastar o
                        // corpo NÃO move o card (invariante de janela — pega-se pelo título).
                        canvas::drag::DragMode::MoveCard(node) => {
                            let card_world = {
                                let m = lock(&view.nodes.model);
                                m.nodes.get(&node).map(|nv| (nv.x, nv.y))
                            };
                            if let Some((cx0, cy0)) = card_world {
                                let (_, top_y) = view.camera.world_to_screen((cx0, cy0));
                                let header_px = 34.0 * view.camera.zoom;
                                if pos.1 >= top_y && pos.1 <= top_y + header_px {
                                    view.focus(node); // z-bump: fica na frente ao pegar
                                    view.card_drag =
                                        Some(canvas::drag::CardDrag::begin(node, (cx0, cy0), pos));
                                    cx.notify();
                                }
                            }
                        }
                        // Fundo vazio → PAN da câmera (comportamento histórico).
                        canvas::drag::DragMode::PanCamera => {
                            view.drag = Some((pos, view.camera.pan));
                        }
                    }
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, window, cx| {
                let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                // F3-1-7: arrasto de card de Goal (overlay) tem PRIORIDADE sobre o gesto do canvas. O
                // root captura o move globalmente → o offset acompanha o cursor mesmo fora do card.
                if let Some(g) = view.goal_drag.as_ref() {
                    if ev.dragging() {
                        let offset = (
                            g.start_offset.0 + pos.0 - g.start_mouse.0,
                            g.start_offset.1 + pos.1 - g.start_mouse.1,
                        );
                        let gid = g.goal_id.clone();
                        view.goal_offsets.insert(gid, offset);
                        cx.notify();
                    }
                    return;
                }
                let zoom = view.camera.zoom;
                // F2-3-1: gesto de MOVER card tem prioridade — recalcula o PREVIEW (move só o
                // NodeView em memória; NÃO emite evento). O event log só recebe no `mouse_up`.
                if let Some(g) = view.card_drag.as_mut() {
                    if ev.dragging() {
                        g.update(pos, zoom);
                        let raw = g.preview();
                        let node = g.node();
                        // F2-3-3 (snap PASSIVO): imanta o preview nos alinhamentos dos cards
                        // VISÍVEIS (8px de tela → mundo). Idempotente; nunca move sozinho.
                        let vw = window.viewport_size();
                        let vp = (f32::from(vw.width), f32::from(vw.height));
                        let others: Vec<(f32, f32)> = {
                            let m = lock(&view.nodes.model);
                            m.nodes
                                .iter()
                                .filter(|(id, _)| **id != node)
                                .map(|(_, nv)| (nv.x, nv.y))
                                .filter(|p| card_visible(&view.camera, *p, (CARD_W, CARD_H), vp))
                                .collect()
                        };
                        let (sx, sy) = canvas::snap::snap_position(
                            raw,
                            &others,
                            CARD_W,
                            CARD_H,
                            crate::bridge::CARD_GAP,
                            8.0 / zoom,
                        );
                        if let Some(nv) = lock(&view.nodes.model).nodes.get_mut(&node) {
                            nv.x = sx;
                            nv.y = sy;
                        }
                        cx.notify();
                    }
                } else if let Some((mouse0, pan0)) = view.drag {
                    if ev.dragging() {
                        view.camera.pan = (pan0.0 + pos.0 - mouse0.0, pan0.1 + pos.1 - mouse0.1);
                        // BUG C (raiz): o gpui só auto-repinta num mousemove quando há `active_drag`
                        // NATIVO (window.rs `dispatch_mouse_event`). Pan/seleção aqui usam estado
                        // MANUAL → sem `cx.notify()` a view quiescente (terminal idle, sem animação)
                        // não redesenha o gesto: o pan "congela" e a seleção parece não estender /
                        // desselecionar. Marcar dirty resolve a causa raiz.
                        cx.notify();
                    }
                } else if view.dragging_sel {
                    // Estende a seleção até a célula sob o cursor (mesmo fora do card). Só repinta
                    // quando a célula MUDA (extend devolve `true`) — evita re-render a 60Hz à toa.
                    if view.extend_selection(pos) {
                        cx.notify();
                    }
                } else if let Some(node) = view.report_node {
                    // Mouse reporting: movimento com botão (drag) → o TUI recebe (se quiser).
                    let mods = (ev.modifiers.shift, ev.modifiers.alt, ev.modifiers.control);
                    if let Some(cell) = view.screen_cell(node, pos) {
                        view.send_pointer(node, PtrAction::Motion, cell, mods);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, ev: &MouseUpEvent, _w, cx| {
                    // F3-1-7: solta o arrasto de um card de Goal (o offset já foi persistido no move).
                    if view.goal_drag.take().is_some() {
                        cx.notify();
                        return;
                    }
                    // F2-3-1: encerra o gesto de MOVER card e EMITE no FIM (posição final). `finish`
                    // devolve None se cancelado (Esc) ou clique sem deslocamento → ZERO NodeMoved.
                    if let Some(g) = view.card_drag.take() {
                        if let Some(mv) = g.finish() {
                            // F2-3-3: persiste a posição IMANTADA. O último `mouse_move` já snapou o
                            // NodeView; `mv` é a posição CRUA do gesto (o snap vive no NodeView, não
                            // no CardDrag). Lê a posição atual do NodeView → log e tela coincidem.
                            let final_pos = lock(&view.nodes.model)
                                .nodes
                                .get(&mv.node)
                                .map(|nv| (nv.x, nv.y))
                                .unwrap_or((mv.x, mv.y));
                            let active_root = lock(&view.runtimes).active.clone();
                            view.append_to_workspace(
                                &active_root,
                                &lina_core::DomainEvent::NodeMoved {
                                    node: mv.node,
                                    x: final_pos.0 as f64,
                                    y: final_pos.1 as f64,
                                },
                            );
                            cx.notify();
                        }
                    }
                    // BUG C: o fim do gesto também muda o visual (limpa clique-simples / fixa o
                    // estado final). Sem `active_drag` nativo o gpui não repinta no mouseup → marca
                    // dirty se houve gesto ativo.
                    let was_active = view.drag.is_some() || view.dragging_sel;
                    view.drag = None;
                    if view.dragging_sel {
                        view.dragging_sel = false;
                        // "Clicar simples limpa": sem arraste (anchor==head) → sem seleção.
                        if view.sel.is_some_and(|s| s.anchor == s.head) {
                            view.clear_selection_state();
                        }
                    }
                    if let Some(node) = view.report_node.take() {
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        let mods = (ev.modifiers.shift, ev.modifiers.alt, ev.modifiers.control);
                        if let Some(cell) = view.screen_cell(node, pos) {
                            view.send_pointer(node, PtrAction::Release, cell, mods);
                        }
                    }
                    if was_active {
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|view, ev: &ScrollWheelEvent, window, _cx| {
                // BUG 2 (scroll): a roda sobre um TERMINAL rola o SCROLLBACK daquele nó (ver o output
                // que passou); no FUNDO vazio = ZOOM do canvas; ⌘/Ctrl+roda = ZOOM em qualquer lugar.
                // TUDO aqui, no handler do ROOT (que SEMPRE dispara) — antes "o card cuidava", mas o
                // gesto não chegava nele de forma confiável e o root caía no zoom.
                let cursor = (f32::from(ev.position.x), f32::from(ev.position.y));
                let v = window.viewport_size();
                let vp = (f32::from(v.width), f32::from(v.height));
                if ev.modifiers.platform || ev.modifiers.control {
                    view.camera.zoom_by(cursor, scroll_zoom_factor(ev.delta));
                    return;
                }
                let Some(node) = hit_test(
                    &view.camera,
                    cursor,
                    &view.cards_z_asc(),
                    (CARD_W, CARD_H),
                    vp,
                ) else {
                    // Fundo vazio → zoom do canvas (pan/zoom de antes intactos).
                    view.camera.zoom_by(cursor, scroll_zoom_factor(ev.delta));
                    return;
                };
                let dy: f32 = match ev.delta {
                    ScrollDelta::Lines(p) => p.y,
                    ScrollDelta::Pixels(p) => p.y / px(cell_h()),
                };
                if dy == 0.0 {
                    return;
                }
                // Mouse reporting atômico: se o TUI sob o cursor CONSOME a roda, NÃO rola o scrollback.
                let mods = (ev.modifiers.shift, ev.modifiers.alt, ev.modifiers.control);
                if let Some(cell) = view.screen_cell(node, cursor) {
                    let action = if dy > 0.0 {
                        PtrAction::WheelUp
                    } else {
                        PtrAction::WheelDown
                    };
                    if view.send_pointer(node, action, cell, mods) {
                        return;
                    }
                }
                // Rola o SCROLLBACK do terminal: `delta > 0` sobe p/ o PASSADO (lina-vt display_offset;
                // o próximo `screen()` reflete → o usuário VÊ o histórico).
                let lines = dy.round() as i32;
                if lines != 0 {
                    if let Some(g) = lock(&view.nodes.grids).get(&node).cloned() {
                        lock(&g).scroll(lines);
                    }
                }
            }))
            .on_key_down(cx.listener(|view, ev: &KeyDownEvent, window, cx| {
                view.handle_key(ev, window, cx);
            }));

        // W2-4 (dívida IME): registra o handler de IME do gpui ATRELADO ao foco do canvas — sem ele as
        // dead-keys (´~^) compõem no SO mas o caractere final NUNCA chega ao PTY. Um `gpui::canvas` de
        // área ZERO existe só p/ chamar `handle_input` no `paint` (única fase válida). O loop de animação
        // (`request_animation_frame`, topo do render) re-registra o handler a cada frame. Mirror do Zed.
        let ime_view = cx.entity();
        let ime_focus = self.focus.clone();
        root = root.child(
            gpui::canvas(
                move |_bounds, _window, _cx| {},
                move |_bounds, _prepaint, window, cx| {
                    window.handle_input(&ime_focus, ImeHandler { view: ime_view }, cx);
                },
            )
            .absolute()
            .size(px(0.0)),
        );

        // BUG A/B (instrumentação): acumula 1 linha por card (rows/zona/dim) e loga após o loop SÓ se
        // mudou (sem spammar a 60fps). O Maestro confirma por DADOS que as rows batem + zona/dim ok.
        let mut frame_diag = String::new();
        // W5-1 (instrumentação de FPS): paineis DESENHADOS = os não-suspensos (mesma classificação de
        // zona da sonda de cards). Conta no loop; `cards.len()` dá os VIVOS.
        let mut panels_drawn = 0usize;
        // F1-5-1 ([PROF]): fronteira poll→assemble (tudo acima é poll/update do model) + total
        // de runs de grid montados no frame (proxy de quads). Desligada → None, custo zero.
        let prof_poll_end = self.prof.enabled.then(Instant::now);
        let mut prof_frame_runs = 0usize;
        // F2-2-5: rect+contexto do card FOCADO, capturado no loop e consumido na ancoragem da
        // toolbar contextual (overlay screen-space) após o desenho dos cards.
        let mut focused_toolbar: Option<(ui::CardRect, palette::NodeCtx, String)> = None;
        // F3-0-6: modelo·effort por nó (mesmo cache do painel) — o header do card mostra com que
        // motor cada terminal pensa, num relance. Reconstruído 1× por frame (cache não-bloqueante).
        let effort_by_node = self.effort_badges_cached();
        for (idx, (id, nv)) in cards.iter().enumerate() {
            // F1-5-1: cronômetro POR PAINEL da fase assemble (inclui lock do grid + screen()).
            let prof_panel_start = self.prof.enabled.then(Instant::now);
            let prof_panel_runs = Cell::new(0usize);
            let node_id = *id;
            // ID de elemento ESTÁVEL por nó (não o idx do loop, que muda com cull/z-order).
            let card_eid = eid.get(&node_id).copied().unwrap_or(idx);
            // W4-2: a ZONA do nó (foco/periferia/suspenso) decide COMO desenhar. Fonte do `in_viewport`
            // é o MESMO culling (W2-3). Suspenso (fora da viewport) → 0 draw-calls (continue).
            let in_viewport = card_visible(&cam, (nv.x, nv.y), (CARD_W, CARD_H), viewport);
            let zone = canvas::classify_zone(node_id == focused, in_viewport);
            // BUG B: `dim` = periferia E IDLE de verdade (sem atividade). NUNCA escurece só por falta de
            // foco — a periferia ATIVA fica legível/normal.
            let dim =
                matches!(zone, canvas::Zone::Periphery) && matches!(nv.status, NodeStatus::Idle);
            // Instrumentação: rows do grid (== o que o PTY tem) + zona + dim, por card.
            let pty_rows = lock(&self.nodes.grids)
                .get(&node_id)
                .map_or(0, |g| lock(g).dims().1);
            let drawn_rows = if zone == canvas::Zone::Suspended {
                0
            } else {
                pty_rows
            };
            frame_diag.push_str(&format!(
                "[{} pty_rows={pty_rows} drawn_rows={drawn_rows} zone={zone:?} dim={dim}] ",
                nv.name
            ));
            if zone == canvas::Zone::Suspended {
                continue;
            }
            panels_drawn += 1; // W5-1: passou da guarda de suspenso → este painel é DESENHADO.
            let (sx, sy) = cam.world_to_screen((nv.x, nv.y));
            let z = cam.zoom;
            // OP-1 (fusão T1+T3, §VIII): "precisa de você" é o sinal MAIS forte — um gate de
            // custódia pendente para este nó VENCE o status e veste o vermelho do acionável (a cor do
            // INDICADOR é a cor do PEDIDO). Computado aqui (antes do dot/borda) para a cor semântica.
            let needs_human = lock(&self.desk)
                .queue
                .iter()
                .any(|p| p.requester() == nv.name);
            // #22 (tela honesta): este nó recebeu um handoff e voltou a Idle SEM começar? O W3
            // publica isso na fila de atenção (`DeliveredNoProgress`); o card consome a projeção e
            // mostra um badge honesto — um Idle "engolido" deixa de parecer um Ocioso qualquer.
            let delivery_stalled = self.attention_items.iter().any(|i| {
                i.node_id == nv.name && i.kind == lina_core::AttentionKind::DeliveredNoProgress
            });
            // F2-2-5 (toolbar contextual): captura o rect (espaço de tela) + o contexto do card
            // FOCADO — a toolbar é ancorada DEPOIS do loop (overlay screen-space, só no nó em
            // Zone::Focus; o `continue` de Suspenso acima já garante que o focado aqui está visível).
            if node_id == focused {
                focused_toolbar = Some((
                    ui::CardRect {
                        x: sx,
                        y: sy,
                        w: CARD_W * z,
                        h: CARD_H * z,
                    },
                    palette::NodeCtx {
                        id: node_id,
                        kind: nv.kind,
                        status: nv.status,
                        needs_human,
                        roster_len: cards.len(),
                    },
                    nv.name.clone(),
                ));
            }
            // Borda SEMÂNTICA: precisa-de-você → `danger` (o card inteiro pede você, mesmo na
            // periferia — fusão "o que precisa de você salta"); senão foco → `focus.ring`; senão
            // neutra. (Tokens ≥3:1 nos 2 temas — gate F1-2-1; F1-2-6 reusa no teclado.)
            let card_border = if needs_human {
                rgb(th.state.danger)
            } else if node_id == focused {
                rgb(th.focus.ring)
            } else {
                rgb(th.surface.border)
            };
            // Indicador pela cor SEMÂNTICA FIXA da fusão (OP-1): vermelho=precisa-de-você ·
            // âmbar=trabalhando · verde=pronto · vermelho=encerrado. Universal no app inteiro.
            let status_dot = if needs_human {
                rgb(th.state.danger)
            } else {
                match nv.status {
                    NodeStatus::Idle => rgb(th.state.success),
                    NodeStatus::Busy | NodeStatus::Running => rgb(th.state.warning),
                    // #21: pausado pelo disjuntor = cor de ATENÇÃO própria (≠ âmbar de "trabalhando",
                    // ≠ vermelho de "morto"); o chip clicável abaixo carrega o estado + a saída.
                    NodeStatus::Blocked => rgb(th.accent.primary),
                    NodeStatus::Crashed | NodeStatus::Dead => rgb(th.state.danger),
                    _ => rgb(th.accent.primary),
                }
            };

            let mut title = div()
                .id(("title", card_eid))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(th.surface.panel))
                // Fusão: chrome em IBM Plex Sans (cascateia aos chips-filho) na escala de token.
                .font_family(th.typography.family.ui)
                .text_size(px(f32::from(th.typography.size.body) * z))
                .text_color(rgb(th.accent.primary))
                .child(
                    div()
                        .size(px(9.0 * z))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(status_dot),
                )
                // BUG S1 (top-bar do card): o nome CEDE com ellipsis (flex_1 + min_w(0) + truncate),
                // não empurra os chips finais pra fora do overflow_hidden do card. Mesmo padrão da
                // sidebar (fatia bf-sidebar). Os chips de meta/ação são flex_shrink_0 → nunca somem.
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .child(text!(nv.name.clone())),
                );
            // Tela honesta (#22/#21): estado em pt-br do leigo, a MESMA voz do leitor de tela —
            // antes era o `Debug` cru do enum (`· Terminal · Busy`), jargão em inglês na tela.
            // #21: a pausa do disjuntor é um chip CLICÁVEL na cor de atenção que dispara a
            // liberação (gesto humano, igual ao `confirm_goal`); os demais estados ficam em texto
            // fosco. `node_status_label` = `aggregate_badge(..).label()` — UI e a11y, MESMA palavra.
            if matches!(nv.status, NodeStatus::Blocked) {
                title = title
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(th.text.muted))
                            .child(text!(format!("· {} ·", node_kind_label(nv.kind)))),
                    )
                    .child(
                        div()
                            .id(("card-breaker", card_eid))
                            .flex_shrink_0()
                            .px_2()
                            .rounded_md()
                            .bg(rgb(th.accent.primary))
                            .text_color(rgb(th.text.on_accent))
                            .text_size(px(f32::from(th.typography.size.caption) * z))
                            .cursor_pointer()
                            // A11y: o gesto de liberação anuncia o estado + a saída.
                            .aria_label(CIRCUIT_BREAKER_LABEL)
                            .on_click(cx.listener(move |view, _ev: &ClickEvent, window, cx| {
                                view.release_breaker(node_id, window, cx);
                            }))
                            .child(text!(CIRCUIT_BREAKER_LABEL)),
                    );
            } else {
                title = title.child(div().flex_shrink_0().text_color(rgb(th.text.muted)).child(
                    text!(format!(
                        "· {} · {}",
                        node_kind_label(nv.kind),
                        node_status_label(nv.status)
                    )),
                ));
            }
            // F3-0-6: pílula modelo·effort no HEADER — com que MOTOR (e quanto cuidado) este
            // terminal pensa, num relance (antes só no painel). Chip neutro (não compete com a cor
            // SEMÂNTICA do estado); a chave do mapa é `NodeId.to_string()` e só nós com
            // `EffortAssigned` (terminais) aparecem — nota/pasta caem fora naturalmente. A11y leva a
            // frase rica (nível + porquê + origem + modelo).
            if let Some(badge) = effort_by_node.get(&node_id.to_string()) {
                title = title.child(
                    div()
                        .id(("card-effort", card_eid))
                        .flex_shrink_0()
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .text_size(px(f32::from(th.typography.size.caption) * z))
                        .text_color(rgb(th.text.secondary))
                        .aria_label(badge.a11y_label())
                        .child(text!(badge.surface_text())),
                );
            }
            // P0 (F2-2-2 / ADR 0028 — prioridade soberana da r6): o estado MAIS crítico do produto —
            // "precisa de você" (gate de custódia humano pendente) — ganha VOZ SEM FOCO via `Badge`
            // Danger (vermelho + "precisa de você"), que compõe o live-region. Os dot/borda vermelhos
            // da r5 PERMANECEM (o visual); o Badge soma o anúncio (a assimetria visual-vs-a11y que a r5
            // ampliou fecha aqui). Id estável por node (`card-needs`+card_eid): a troca de estado
            // anuncia no MESMO nó. CANAL ÚNICO: o `aria_label` do corpo (abaixo) NÃO repete "precisa de
            // você" — passa `needs_human=false` ao `node_label` (o Badge é o dono do anúncio; o corpo
            // carrega nome + status bruto).
            // CORTESIA = POLITE (refinamento da verificação dupla do QA, r6 — locator vs ator): o Badge
            // per-card é LOCALIZADOR (diz QUAL card pede você — anuncia sem interromper); o ATOR é o
            // toast AGREGADO da fila de atenção (Assertive — interrompe com a decisão + ação). Assertar
            // nos DOIS pro mesmo evento treina a ignorar (doutrina do próprio badge.rs); Polite aqui
            // preserva o P0 (anuncia sem foco) E respeita o snooze do toast (o badge não fica martelando
            // depois que você adiou). O default do Badge JÁ é Polite — basta não chamar `needs_you()`.
            if needs_human {
                title = title.child(ui::Badge::new(
                    ("card-needs", card_eid),
                    "precisa de você",
                    ui::BadgeTone::Danger,
                ));
            }
            // #22: o despacho engolido ganha VOZ no card — âmbar (atenção, não erro): o nó recebeu e
            // ainda não começou. Distinto de "precisa de você" (custódia): aqui ninguém está bloqueado
            // num gate; é o Maestro que deve re-despachar. Badge informacional (Polite, como os demais).
            if delivery_stalled {
                title = title.child(ui::Badge::new(
                    ("card-stalled", card_eid),
                    DELIVERY_STALLED_BADGE,
                    ui::BadgeTone::Warning,
                ));
            }
            // BUG D: affordance ✎ DESCOBRÍVEL no próprio CARD (antes só no painel ⌘K → o fundador não
            // achava). Chip clicável no cabeçalho do Agente que abre a edição de 1ª classe
            // (`open_agent_modal_edit`). Mantém os entry points existentes (⌘K + duplo-clique no
            // título). Só terminais (nota/pasta não têm papel/doutrina). Mesma estrutura do chip ✕
            // (parent `.id()` único por card → sem colisão de ElementId no AccessKit). Layout nos 2
            // temas = PENDENTE TELA DO FUNDADOR.
            if matches!(nv.kind, NodeKind::Terminal) {
                // FIX-3: a autonomia atual À VISTA no card (badge discreto, tokens do design
                // system — par de cores na fonte única `autonomy_badge_colors`, gateada por
                // WCAG). Em "assistido" o texto carrega o mini-aviso "pede sim/não" e o rótulo
                // anunciável explica de onde vêm os pedidos; o CLIQUE abre o Editar (é lá que
                // se ajusta — copy da entrega 3). Chip de 1 linha na barra do título: NUNCA uma
                // linha extra no corpo (encolheria o grid `flex_1` → rows do PTY clipadas).
                let (auto_fg, auto_bg) = agent_modal::autonomy_badge_colors(&th);
                title = title.child(
                    div()
                        .id(("card-autonomy", card_eid))
                        // FIX-3 costura (Pedido 2): per-Agente — o badge lê `nv.autonomy` (o
                        // `LINA_AUTONOMY` carimbado no env deste nó), não o nível do Espaço.
                        // UI e enforcement (guard) nunca divergem.
                        .aria_label(agent_modal::card_autonomy_aria(nv.autonomy))
                        .flex_shrink_0()
                        .px_2()
                        .rounded_md()
                        .bg(rgb(auto_bg))
                        .border_1()
                        .border_color(rgb(th.surface.border))
                        .text_size(px(f32::from(th.typography.size.caption) * z))
                        .text_color(rgb(auto_fg))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, cx| {
                            view.open_agent_modal_edit(node_id, cx);
                        }))
                        .child(text!(agent_modal::card_autonomy_badge(nv.autonomy))),
                );
                title = title.child(
                    div()
                        .id(("card-edit", card_eid))
                        .flex_shrink_0()
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .text_size(px(f32::from(th.typography.size.small) * z))
                        .text_color(rgb(th.accent.primary))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, cx| {
                            view.open_agent_modal_edit(node_id, cx);
                        }))
                        .child(text!("✎ Editar")),
                );
            }
            // Rodada 360 (ADR 0022 §4): degradação VISÍVEL — agente em pasta própria SEM o kit
            // de integração (sem consentimento no modal, ou arquivo do usuário preservado).
            if nv.kit_missing {
                title = title.child(
                    div()
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .text_size(px(f32::from(th.typography.size.caption) * z))
                        .text_color(rgb(th.state.warning))
                        .child(text!("⚠ sem doutrina/observabilidade")),
                );
            }
            // FIX-1 (dogfooding 2026-06-10): VÁRIOS agentes dividem ESTA pasta de trabalho (um
            // time no mesmo projeto do usuário — default F1-4-1). A identidade A2A é isolada pelo
            // env (ADR 0026); o badge só torna o compartilhamento VISÍVEL ao leigo — informacional,
            // não alarme (`text.muted`, não `warning`). Chip de 1 linha na barra do título (nunca
            // no corpo: clipa as rows do PTY — família de bugs A/B desta base).
            if nv.cwd_shared {
                title = title.child(
                    div()
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .text_size(px(f32::from(th.typography.size.caption) * z))
                        .text_color(rgb(th.text.muted))
                        .child(text!("vários agentes nesta pasta")),
                );
            }
            // O ✕ só aparece quando há >1 card: o ÚLTIMO nó nunca pode ser fechado (o canvas
            // jamais fica em branco), então não mostramos um botão que recusaria a ação.
            if cards.len() > 1 {
                title = title.child(
                    div()
                        .id(("close", card_eid))
                        .flex_shrink_0()
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.danger_muted))
                        .text_color(rgb(th.state.danger))
                        .cursor_pointer()
                        // Fonte única do "fechar nó" (DRY com close_focused/CloseNode da toolbar).
                        .on_click(cx.listener(move |view, _ev: &ClickEvent, window, _cx| {
                            view.close_node(node_id, window);
                        }))
                        .child(text!("✕")),
                );
            }
            // F1-2-4 (entry point): DUPLO-CLIQUE no cabeçalho de um Agente abre a edição de 1ª classe
            // (M6-E). Só terminais (nota/pasta não têm papel/doutrina). Clique simples não faz nada
            // aqui (não rouba o foco/seleção do corpo). PENDENTE TELA DO FUNDADOR (gpui não-headless).
            if matches!(nv.kind, NodeKind::Terminal) {
                title = title.cursor_pointer().on_click(cx.listener(
                    move |view, ev: &ClickEvent, _w, cx| {
                        if ev.click_count() == 2 {
                            view.open_agent_modal_edit(node_id, cx);
                        }
                    },
                ));
            }

            // O conteúdo do terminal: o SNAPSHOT do grid lido AGORA (não deltas).
            let mut body = div()
                .id(("grid", card_eid))
                .role(Role::Terminal)
                // CANAL ÚNICO (r6 P0): o corpo carrega nome + STATUS bruto; o "precisa de você" é
                // dono do Badge::needs_you() acima (live-region Assertive). Por isso `false` aqui —
                // sem o override de needs_human o leitor de tela não fala "precisa de você" 2×.
                .aria_label(a11y::node_label(&nv.name, nv.status, false))
                .flex_1()
                .overflow_hidden()
                .p_1()
                // F1-2-1: TERMINAL é superfície escura nos 2 temas (conteúdo ANSI assume fundo
                // escuro); nós-ARTEFATO (nota/pasta) usam a superfície de card TEMÁTICA.
                .bg(rgb(
                    if matches!(nv.kind, NodeKind::Note | NodeKind::Folder) {
                        th.surface.card
                    } else {
                        th.terminal.bg
                    },
                ))
                // BUG B: um leve dim SÓ quando o nó está IDLE de verdade (periferia sem atividade) —
                // legível, não "apagado". A periferia ATIVA (ou focada) fica em opacidade plena.
                .opacity(if dim { 0.82 } else { 1.0 })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, ev: &MouseDownEvent, _w, cx| {
                        // Sobre o grid: mouse reporting (se o TUI pediu) OU inicia seleção.
                        view.pointer_down(node_id, ev);
                        // BUG C: repinta ao iniciar — fixa a âncora e LIMPA qualquer highlight
                        // anterior (de outro card) na tela quiescente.
                        cx.notify();
                    }),
                );
            // W4-2 · M3/M4: nós-ARTEFATO (Note/Folder) não têm PTY/grid — desenham ÍCONE + nome (não a
            // lógica de zona/grid dos terminais). Aparecem VIVOS no canvas assim que criados (semeados
            // no model por `create_artifact`).
            if matches!(nv.kind, NodeKind::Note | NodeKind::Folder) {
                let (icon, what) = if matches!(nv.kind, NodeKind::Note) {
                    ("📝", "nota")
                } else {
                    ("📁", "pasta")
                };
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .size_full()
                        // Card-artefato (nota/pasta) em IBM Plex Sans, escala de token (cascateia).
                        .font_family(th.typography.family.ui)
                        .child(
                            div()
                                .text_size(px(f32::from(th.typography.size.display) * z))
                                .child(text!(icon)),
                        )
                        .child(
                            div()
                                .text_color(rgb(th.text.primary))
                                .text_size(px(f32::from(th.typography.size.subtitle) * z))
                                .child(text!(nv.name.clone())),
                        )
                        .child(
                            div()
                                .text_color(rgb(th.text.muted))
                                .text_size(px(f32::from(th.typography.size.small) * z))
                                .child(text!(what)),
                        ),
                );
            } else {
                // BUG B: foco E periferia desenham o GRID ao vivo (LEGÍVEL) — a periferia NÃO é "escura"
                // só por estar sem foco. O que distingue é a BORDA (foco=azul, periferia=cinza, acima) +
                // o leve `dim` (no body, acima) APENAS quando IDLE de verdade. Suspenso (fora da
                // viewport) já deu `continue` (0 draw-calls — culling W2-3, P0 preservado).
                match zone {
                    canvas::Zone::Focus | canvas::Zone::Periphery => {
                        let grid = lock(&self.nodes.grids).get(&node_id).cloned();
                        if let Some(g) = grid {
                            let screen = lock(&g).screen();
                            // BUG 3: o highlight vem da SELEÇÃO DO GRID já reprojetada por frame
                            // (`screen.selection`, scroll-aware), não de `self.sel` (coords de
                            // viewport congeladas no gesto, que ficariam "uma camada acima" ao rolar).
                            // É per-node por construção: só o grid focado tem `term.selection` ativa
                            // (`pointer_down` → `clear_selection_state` zera os demais). `self.sel`
                            // continua, mas só p/ DIRIGIR o gesto (extend/copy/clear).
                            body = body.child(render_grid(
                                &screen,
                                z,
                                screen.selection,
                                self.prof.enabled.then_some(&prof_panel_runs),
                            ));
                        }
                    }
                    canvas::Zone::Suspended => {}
                }
            }

            let card = div()
                .id(("card", card_eid))
                .absolute()
                .left(px(sx))
                .top(px(sy))
                .w(px(CARD_W * z))
                .h(px(CARD_H * z))
                .flex()
                .flex_col()
                .bg(rgb(th.surface.card))
                .border_2()
                .border_color(card_border)
                .rounded_md()
                .overflow_hidden()
                .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, _cx| {
                    // Só foca se o nó ainda existe (o ✕ pode tê-lo removido neste clique). Focar
                    // bombeia o card ao topo (z-order).
                    if lock(&view.nodes.model).nodes.contains_key(&node_id) {
                        view.focus(node_id);
                    }
                }))
                // BUG 2 (scroll): o gesto de roda é tratado no handler do ROOT (que sempre dispara),
                // roteando por `hit_test` — scrollback sobre o terminal, zoom no fundo. (Antes ficava
                // aqui no card e não pegava o gesto de forma confiável.)
                // BUG B: arquivo(s)/mídia/PDF solto(s) NESTE card → injeta o(s) caminho(s) CITADO(s)
                // no PTY do nó, pelo MESMO caminho do teclado humano (fila serial `WriteOp::HumanKeys`,
                // como `paste_clipboard`/`handle_key`). Só nós com PTY (terminal) recebem; nota/pasta
                // não têm grid → ignora. O drop VISUAL = PENDENTE TELA DO FUNDADOR.
                .on_drop(cx.listener(move |view, paths: &ExternalPaths, _w, cx| {
                    if lock(&view.nodes.grids).get(&node_id).is_none() {
                        return;
                    }
                    let quoted = quote_dropped_paths(paths.paths());
                    if quoted.is_empty() {
                        return;
                    }
                    view.focus(node_id);
                    view.input
                        .submit(node_id, WriteOp::HumanKeys(quoted.into_bytes()));
                    cx.notify();
                }))
                .child(title)
                .child(body);

            root = root.child(card);
            // F1-5-1: amostra por-painel (só painéis DESENHADOS chegam aqui — suspensos deram
            // `continue` acima e não geram amostra; o custo deles fica na fase assemble total).
            if let Some(t) = prof_panel_start {
                prof_frame_runs += prof_panel_runs.get();
                self.prof
                    .panel_done(&nv.name, t.elapsed(), prof_panel_runs.get());
            }
        }
        // F1-5-1: fronteira assemble→chrome (daqui até o fim do render é chrome da cena).
        let prof_assemble_end = self.prof.enabled.then(Instant::now);

        // inv#6 (nunca tela em branco): workspace SEM terminais (produção fresca, ou o raro spawn que
        // falhou) → guia centralizada (⌘T) em vez de canvas vazio. Adicionada ANTES do topbar (que vem
        // depois), então os botões do topo continuam clicáveis por cima. Some sozinho ao surgir o 1º card.
        if cards.is_empty() {
            root = root.child(empty_canvas_hint());
        }

        // BUG A/B (instrumentação): loga a linha de diagnóstico SÓ quando muda (rows/zona/dim) — o
        // Maestro lê o stderr e confirma, SEM ver a tela, que as rows do PTY batem com o desenhado e
        // que a zona/dim estão certas (periferia NÃO escurece por falta de foco).
        if !frame_diag.is_empty() && frame_diag != self.diag_last_frame {
            eprintln!("lina-gpui: cards · {frame_diag}");
            self.diag_last_frame = frame_diag;
        }

        // W5-1 (instrumentação de FPS): fecha a janela a cada ~FPS_WINDOW frames OU ~FPS_WINDOW_MS de
        // render ATIVO (a soma do ring É o tempo de parede dos frames medidos — lacunas ociosas já
        // foram descartadas). Loga paineis VIVOS/DESENHADOS + p50/p95/p99 do frametime + FPS estimado
        // (= 1000/média). O fundador faz grep por `[FPS]`. Esvazia o ring p/ a próxima janela.
        let active_ms: f64 = self.frametime_ms.iter().sum();
        if self.frametime_ms.len() >= FPS_WINDOW || active_ms >= FPS_WINDOW_MS {
            if !self.frametime_ms.is_empty() {
                let mut sorted = self.frametime_ms.clone();
                sorted.sort_by(f64::total_cmp);
                let n = sorted.len() as f64;
                let mean = active_ms / n;
                let p50 = prof::percentile_sorted(&sorted, 50.0);
                let p95 = prof::percentile_sorted(&sorted, 95.0);
                let p99 = prof::percentile_sorted(&sorted, 99.0);
                let fps = if mean > 0.0 { 1000.0 / mean } else { 0.0 };
                let panels_live = cards.len();
                eprintln!(
                    "lina-gpui: [FPS] panels_live={panels_live} panels_drawn={panels_drawn} p50={p50:.1}ms p95={p95:.1}ms p99={p99:.1}ms fps={fps:.0}"
                );
            }
            self.frametime_ms.clear();
        }

        // PULSO efêmero A→B (a metáfora "sem fios"). W4-3: respeita REDUCE-MOTION — com ele ligado, a
        // animação é suprimida (a entrega A2A ocorre igual; o pulso é decorativo).
        // W4-6 gap3: reduce-motion lido da FONTE ÚNICA `a11y::reduce_motion_effective` (SO/env sempre
        // respeitado + override do rodapé) — o W4-3 só inverte o bool.
        if let Some(p) = pulse {
            // INSTRUMENTAÇÃO (de-dup): por que o pulso A→B (não) ANIMA na tela. O Maestro lê o stderr
            // p/ validar por DADOS qual gate corta — reduce-motion (`animate=false`), id fora de `cards`
            // (`from_in_cards`/`to_in_cards=false`), ou já expirado (`progressing=false`). Tudo `true` +
            // sem dot visto ⇒ o problema é de paint/percepção, não de estado.
            let rm_eff = a11y::reduce_motion_effective(self.reduce_motion);
            let animate = wiring::animate_pulse(rm_eff);
            let prog = p.progress();
            let from_in = cards.iter().any(|(n, _)| *n == p.from);
            let to_in = cards.iter().any(|(n, _)| *n == p.to);
            let diag = format!(
                "from={} to={} reduce_motion_eff={rm_eff} animate={animate} progressing={} from_in_cards={from_in} to_in_cards={to_in} n_cards={}",
                p.from,
                p.to,
                prog.is_some(),
                cards.len()
            );
            if diag != self.diag_last_pulse {
                eprintln!("lina-gpui: pulso · {diag}");
                self.diag_last_pulse = diag;
            }
            if let Some(t) = prog.filter(|_| animate) {
                let center = |id: NodeId| -> Option<Point<Pixels>> {
                    cards.iter().find(|(n, _)| *n == id).map(|(_, nv)| {
                        let (cx2, cy2) =
                            cam.world_to_screen((nv.x + CARD_W / 2.0, nv.y + CARD_H / 2.0));
                        point(px(cx2), px(cy2))
                    })
                };
                if let (Some(a), Some(b)) = (center(p.from), center(p.to)) {
                    let dot_x = a.x + (b.x - a.x) * t;
                    let dot_y = a.y + (b.y - a.y) * t;
                    let alpha = (1.0 - t).clamp(0.0, 1.0);
                    root = root.child(
                        div()
                            .absolute()
                            .left(dot_x - px(13.0))
                            .top(dot_y - px(13.0))
                            .size(px(26.0))
                            .rounded_full()
                            .bg(rgb(th.accent.primary))
                            .opacity(alpha * 0.35),
                    );
                    root = root.child(
                        div()
                            .absolute()
                            .left(dot_x - px(6.0))
                            .top(dot_y - px(6.0))
                            .size(px(12.0))
                            .rounded_full()
                            .bg(rgb(th.accent.secondary))
                            .opacity(alpha),
                    );
                }
            }
        }

        // Barra superior. BUG 3: `flex_wrap` → os botões QUEBRAM em 2+ linhas quando não cabem na
        // largura (nada é "comido"/cortado; tudo acessível em qualquer tamanho de janela, sem scroll
        // escondido). `gap_2` (mais compacto que `gap_4`) cabe mais por linha.
        let mut topbar = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .bg(rgb(th.surface.chrome))
            .text_color(rgb(th.accent.primary))
            .child(text!(
                "Lina Space · terminais reais (clique p/ focar · digite · scroll)"
            ));

        if connected {
            topbar = topbar.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(8.0)).rounded_full().bg(rgb(th.state.success)))
                    .child(
                        div()
                            .text_color(rgb(th.state.success))
                            .child(text!("Time conectado · todos se falam")),
                    ),
            );
        }

        topbar = topbar.child(
            div()
                .text_color(rgb(th.text.muted))
                .child(text!(format!("log: {event_count} eventos"))),
        );

        // F1-1-7: o sino 🔔 da Fila de Atenção — contagem de pendências reais; pulsa
        // com item Escalated (≥5min), suprimido sob reduce-motion (fonte única W4-6).
        let att_now = lina_core::now_ms();
        let att_reduce = a11y::reduce_motion_effective(self.reduce_motion);
        topbar = topbar.child(attention_ui::render_badge(
            &self.attention_items,
            self.attention_panel_open,
            att_reduce,
            att_now,
            &th,
            cx,
        ));

        // O ⚡ A2A (A→B) é demo-only e só aparece com AMBOS os alvos vivos: em produção `a2a` é `None`
        // (sem A/B) e o botão nem existe; no demo, se o fundador fechar A ou B, `ready()` o esconde.
        if self.a2a.as_ref().is_some_and(|a| a.ready()) {
            topbar = topbar.child(
                div()
                    .id("a2a-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(th.accent.action))
                    .text_color(rgb(th.text.on_accent))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                        // O B é um shell real: injeta um COMANDO válido (roda limpo nele).
                        if let Some(a) = &view.a2a {
                            a.fire(
                                "echo '📨 A2A recebido de Terminal A · cooperacao sem fios'"
                                    .to_string(),
                            );
                        }
                    }))
                    .child(text!("⚡ Enviar A2A (A→B)")),
            );
        }

        topbar = topbar.child(
            div()
                .id("add-terminal-btn")
                .px_3()
                .py_1()
                .rounded_md()
                .bg(rgb(th.accent.confirm))
                .text_color(rgb(th.text.on_accent))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, cx| {
                    // Criar abre o MODAL de configuração (motor/papel/pasta) — a porta RICA. Antes
                    // criava um shell puro DIRETO (sem escolher CLI), e o leigo ficava sem o claude
                    // (bug de tela 2026-06-20). O atalho ⌘T segue criando o shell rápido p/ avançados.
                    view.open_agent_modal_create(cx);
                }))
                .child(text!("➕ Novo Terminal")),
        );

        // F1-2-2 · M6 "Novo Agente": o botão abre o MODAL (evoluiu o modo-nomeação M2; ⌘N idem).
        topbar = topbar.child(
            div()
                .id("new-agent-btn")
                .px_3()
                .py_1()
                .rounded_md()
                .bg(rgb(th.accent.create))
                .text_color(rgb(th.text.on_accent))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, cx| {
                    view.open_agent_modal_create(cx);
                }))
                .child(text!("✦ Novo Agente")),
        );

        // W4-2 · M3/M4: modo CRIAÇÃO ativo → banner do título da nota/pasta sendo digitado.
        if let Some((kind, buf)) = &self.creating {
            let (icon, what) = match kind {
                creators::CreatorKind::Note => ("📝", "Nova nota"),
                creators::CreatorKind::Folder => ("📁", "Nova pasta"),
            };
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(th.surface.raised_alt))
                    .text_color(rgb(th.text.bright))
                    .child(text!(format!(
                        "{icon} {what}: {buf}▌  (Enter cria · Esc cancela)"
                    ))),
            );
        }

        // 🏠 Centralizar: resgata a vista (pan/zoom → home). Sempre visível p/ o não-técnico
        // nunca ficar perdido no canvas (mesmo efeito do ⌘0).
        topbar = topbar.child(
            div()
                .id("home-btn")
                .px_3()
                .py_1()
                .rounded_md()
                .bg(rgb(th.surface.raised))
                .text_color(rgb(th.text.primary))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, cx| {
                    view.camera.reset();
                    cx.notify(); // sem isto, a câmera reseta mas o frame não re-renderiza (clique "não faz nada")
                }))
                .child(text!("🏠 Centralizar")),
        );

        // Fix de tela (F1-2-1/inv#6): Ajustes DESCOBRÍVEL — engrenagem ao lado de Centralizar
        // (⌘, e a paleta também abrem; o env LINA_PERSIST_PANEL segue só como auto-open de dev).
        topbar = topbar.child(
            div()
                .id("settings-btn")
                .px_3()
                .py_1()
                .rounded_md()
                .bg(rgb(th.surface.raised))
                .text_color(rgb(th.text.primary))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, cx| {
                    view.open_settings_window(cx);
                }))
                .child(text!("⚙️ Ajustes")),
        );

        if recovering {
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(th.state.danger))
                    .text_color(rgb(th.text.on_emphasis))
                    .child(text!("⟳ Recuperando…")),
            );
        }

        // W3-7c · TETO DE CUSTO atingido → workspace PAUSADO (visível). Só o gate humano reabre:
        // `lina resume` (de um terminal) → confirmação na janela (⌘⏎) → CostCeilingResumed.
        if cost_paused {
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(th.state.danger))
                    .text_color(rgb(th.text.on_emphasis))
                    .child(text!(
                        "🛑 teto de custo atingido · workspace PAUSADO — rode `lina resume` e confirme (⌘⏎)"
                    )),
            );
        }

        // W3-6c (ADR 0004) — BANNER DO GATE HUMANO: VISÍVEL na tela. Âmbar = pedido na frente da fila
        // aguardando ⌘⏎; senão, o último resultado da execução por alguns segundos.
        let (custody_banner, custody_pending) = {
            let d = lock(&self.desk);
            (d.banner(), d.front().is_some())
        };
        if let Some(banner) = custody_banner {
            // Pares (bg, fg) cobertos pelo gate WCAG: warning+on_emphasis / confirm+on_accent
            // (o antigo texto escuro fixo violava AA sobre o verde — corrigido na migração F1-2-1).
            let (bg, fg) = if custody_pending {
                (th.state.warning, th.text.on_emphasis)
            } else {
                (th.accent.confirm, th.text.on_accent)
            };
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(bg))
                    .text_color(rgb(fg))
                    .child(text!(banner)),
            );
        }

        // W4-3 · RODAPÉ — o FREIO da auto-orquestração (sempre acessível) + toggle de reduce-motion.
        let paused = lock(&self.brake).paused;
        // W4-6 gap3: o rótulo do rodapé reflete o EFETIVO (fonte única) — não o override cru.
        let reduce_motion = a11y::reduce_motion_effective(self.reduce_motion);
        // BUG 5: rótulo em linguagem de leigo (sem "orquestração" cru) + legenda explicando o que faz.
        // Pares (bg, fg) em tokens cobertos pelo gate WCAG (F1-2-1).
        let (freio_bg, freio_fg, freio_txt) = if paused {
            (
                th.accent.confirm,
                th.text.on_accent,
                "▶ Retomar cooperação dos agentes",
            )
        } else {
            (
                th.state.warning,
                th.text.on_emphasis,
                "⏸ Pausar cooperação dos agentes",
            )
        };
        // BUG 5: o toggle deixa EXPLÍCITO o que liga/desliga (ANIMAÇÕES) + cor de estado óbvia (âmbar
        // quando desligadas = "redução ativa"). `reduce_motion` já é o EFETIVO (fonte única, W4-6).
        let (anim_bg, anim_fg, anim_txt) = if reduce_motion {
            (
                th.state.warning,
                th.text.on_emphasis,
                "🎞 Animações: DESLIGADAS",
            )
        } else {
            (th.surface.raised, th.text.primary, "🎞 Animações: ligadas")
        };
        let mut footer = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            // BUG 3: o rodapé também QUEBRA em linhas quando não cabe (freio + animações + legenda).
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .bg(rgb(th.surface.chrome))
            .child(
                div()
                    .id("freio-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(freio_bg))
                    .text_color(rgb(freio_fg))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                        // Só SINALIZA: a MailboxPump aplica Router::pause/resume no próximo tick.
                        lock(&view.brake).toggle_requested = true;
                    }))
                    .child(text!(freio_txt)),
            )
            .child(
                div()
                    .id("reduce-motion-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(anim_bg))
                    .text_color(rgb(anim_fg))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                        view.reduce_motion = !view.reduce_motion;
                    }))
                    .child(text!(anim_txt)),
            )
            // BUG 5: legenda SEMPRE visível explicando o freio em linguagem de leigo.
            .child(div().text_color(rgb(th.text.muted)).child(text!(
                "ℹ Cooperação = os agentes se delegam tarefas sozinhos · Pausar segura isso (nada se perde, retoma quando quiser)"
            )));
        if paused {
            footer = footer.child(div().text_color(rgb(th.state.warning)).child(text!(
                "⏸ pausado · novas delegações ficam na FILA (nada se perde; nenhum trabalho some)"
            )));
        }

        // W4-6: a live-region (Role::Status) entra na cena — anunciada ao leitor de tela quando muda.
        if let Some(msg) = &a11y_announce {
            root = root.child(
                div()
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .child(a11y::live_region_element(msg)),
            );
        }

        let root = root.child(topbar).child(footer);
        let mut root = root;

        // F2-2-5: a TOOLBAR contextual do card FOCADO — overlay screen-space (chrome, não escala com
        // o zoom), ancorada ACIMA do card (flip sob a topbar via toolbar_anchor). Consome o registry
        // ÚNICO node_commands(NodeCtx) através do NodeToolbar do catálogo; o clique EXECUTA a ação E
        // apenda PaletteCommandInvoked (paridade de ranking com a paleta — Command::key é pub). F1-2-6
        // intocado: o teclado faz o mesmo pela paleta (⌘K).
        if let Some((card_rect, ctx, node_name)) = focused_toolbar {
            // Chrome (não-token, como CARD_W/CARD_H): altura aprox. da barra + área reservada sob a
            // topbar; afinadas no gate-de-tela. Passadas como f32 ao anchor PURO (não são px() literais).
            const TOOLBAR_H: f32 = 40.0;
            const SAFE_TOP: f32 = 56.0;
            let gap = f32::from(th.spacing.sm);
            if let Some((tx, ty)) = ui::toolbar_anchor(card_rect, TOOLBAR_H, gap, SAFE_TOP, true) {
                root = root.child(div().absolute().left(px(tx)).top(px(ty)).occlude().child(
                    ui::NodeToolbar::new(ctx, node_name).on_invoke(cx.listener(
                        |view, action: &palette::PaletteAction, window, cx| {
                            // Paridade com a paleta: apenda PaletteCommandInvoked (ranking íntegro)
                            // e então executa a ação. `Command::key` deriva a key (API pub do registry).
                            let key = palette::Command::new("", action.clone()).key();
                            let active_root = lock(&view.runtimes).active.clone();
                            view.append_to_workspace(
                                &active_root,
                                &lina_core::DomainEvent::PaletteCommandInvoked { key },
                            );
                            view.run_palette_action(action.clone(), window, cx);
                        },
                    )),
                ));
            }
        }

        // Spec §M8: TOAST de arquivamento (canto inferior esquerdo, perto do rail de onde o
        // gesto partiu) — «Espaço “N” arquivado · [ Desfazer ] (Ns)». O countdown anima pelo
        // heartbeat; o anúncio audível já foi à live-region no `archive_workspace`.
        if let Some(t) = &self.archive_toast {
            let secs = t.remaining_secs(lina_core::now_ms());
            let undo_ring = if t.undo_focused {
                th.focus.ring
            } else {
                th.surface.border
            };
            root = root.child(
                div()
                    .absolute()
                    .bottom_8()
                    .left_8()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(th.surface.raised))
                    .border_1()
                    .border_color(rgb(th.surface.border))
                    .text_color(rgb(th.text.primary))
                    .child(text!(format!("Espaço “{}” arquivado", t.name)))
                    .child(
                        div()
                            .id("archive-undo")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_2()
                            .border_color(rgb(undo_ring))
                            .bg(rgb(th.surface.raised_alt))
                            .text_color(rgb(th.text.bright))
                            .cursor_pointer()
                            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                                v.undo_pending_archive(cx);
                            }))
                            .child(text!(format!("{} ({secs}s)", sidebar::COPY_ARCHIVE_UNDO))),
                    ),
            );
        }

        // F1-1-7: TOAST da Fila de Atenção (canto inferior direito) — single com
        // countdown (pause-on-hover) ou colapsado «+N» (countdown desativado). A
        // visibilidade vem da MÁQUINA testada (expirado/«Depois» não renascem; o item
        // segue no badge/fila — nada se perde).
        if let Some(tv) = self
            .attention_machine
            .visible(&self.attention_items, att_now)
        {
            // Costura bf-toasts (T2): render_toast agora recebe viewport + dashboard_open para
            // posicionar o toast à ESQUERDA da coluna do dashboard (sem sobreposição).
            let vp = window.viewport_size();
            root = root.child(attention_ui::render_toast(
                &self.attention_items,
                tv,
                self.attention_machine.timer(),
                att_now,
                (f32::from(vp.width), f32::from(vp.height)),
                self.dashboard_open,
                &th,
                cx,
            ));
        }
        // F1-1-7: painel compacto da fila (badge 🔔 / ⌘J) — decide permissão (registro
        // apenas), apresenta custódia com o gesto EXISTENTE; sob os modais (M6/paleta).
        if self.attention_panel_open {
            let muted_nodes: BTreeMap<String, bool> = self
                .attention_items
                .iter()
                .map(|i| (i.node_id.clone(), self.attention.is_node_muted(&i.node_id)))
                .collect();
            let muted_set: std::collections::BTreeSet<String> = muted_nodes
                .into_iter()
                .filter_map(|(n, m)| m.then_some(n))
                .collect();
            let desk_front = lock(&self.desk).front().map(|p| p.id().to_string());
            let vp = window.viewport_size();
            // R2b 1b: empty-state honesto — o painel sabe quantos terminais trabalham.
            let busy = self.busy_terminal_count();
            root = root.child(attention_ui::render_panel(
                &self.attention_items,
                &muted_set,
                desk_front.as_deref(),
                busy,
                (f32::from(vp.width), f32::from(vp.height)),
                att_now,
                &th,
                cx,
            ));
        }
        // F3-1-7: painel das Goals VIVAS (card que o leigo confirma com 1 toque). Sobre o canvas,
        // sob os modais/paleta; ausente quando não há meta viva (sem tela vazia forçada).
        if let Some(panel) = self.render_goals_panel(cx) {
            root = root.child(panel);
        }
        // F3-3-5: painel "Como o [papel] pensa" — o que cada papel APRENDEU com o fundador (estabelecidas
        // "já vale", provisórias "ainda testando", aposentar-1-clique). Ausente até a projeção viva
        // alimentar a tela (Camada 2); sem tela vazia forçada.
        if let Some(panel) = self.render_mentality_panel(cx) {
            root = root.child(panel);
        }
        // F1-1-5 (P6/fluxo c): painel "Atividade e custos" — zona lateral direita, sob os modais.
        // Geometria clampada ao viewport REAL (fix: o painel fixo vazava a borda direita).
        let root = if self.dashboard_open {
            let vp = window.viewport_size();
            let panel = self.render_dashboard(&th, (f32::from(vp.width), f32::from(vp.height)), cx);
            root.child(panel)
        } else {
            root
        };
        // F1-4-4 · M8: o RAIL de Espaços no flanco esquerdo (52px colapsado ↔ 280px expandido;
        // o componente se dimensiona — T4). Por cima do canvas, por baixo dos modais.
        let root = root.child(self.sidebar.render(&th, cx));
        // F4-0-5: badge de exposição "este Espaço está falando com o mundo" — overlay persistente
        // sobre o canvas, sob os modais. `None` = 0 canais expostos (0 → 0 badge).
        let root = match self.render_exposure_badge() {
            Some(badge) => root.child(badge),
            None => root,
        };
        // F1-4-4 · M9: modal Criar Espaço (galeria de Focos + Diretório de Trabalho — T5).
        let root = match &self.create_space_modal {
            Some(m) => root.child(self.render_create_space(m, &th, window.viewport_size(), cx)),
            None => root,
        };
        // F1-2-2 · M6/M6-E: o modal de Agente é o overlay central (evoluiu o overlay do M2 — BUG 4
        // resolvido por um modal de verdade). A paleta, quando aberta, continua mais ao topo.
        let root = match &self.agent_modal {
            Some(m) => root.child(agent_modal::render(m, window.viewport_size(), cx)),
            None => root,
        };
        // F4-0-2 (UI): o modal de credencial de canal — mesmo nível de overlay do modal de Agente.
        let root = match &self.credential_modal {
            Some(m) => root.child(credential_modal::render(m, window.viewport_size(), cx)),
            None => root,
        };
        // F4-WA-2c (UI): o modal "Receber um aviso de fora" — mesmo nível de overlay.
        let root = match &self.webhook_modal {
            Some(m) => root.child(webhook_modal::render(m, window.viewport_size(), cx)),
            None => root,
        };
        // W4-2 · M1: a PALETA, quando aberta, é o overlay mais ao TOPO (modal sobre o canvas/chrome).
        let root = if self.palette.is_open() {
            root.child(self.palette.render())
        } else {
            root
        };
        // F1-5-1 ([PROF]): SENTINELA de paint — canvas de tamanho ZERO como ÚLTIMO filho da
        // cena: seu closure de paint roda DENTRO da fase de paint do gpui (depois do layout,
        // ao fim do paint em ordem de árvore) e carimba o instante no slot atômico. É o que
        // separa `layout_paint` de `present_vsync` — a melhor aproximação do custo pós-render
        // disponível no gpui sem forkar o framework. Só existe com a sonda LIGADA.
        let root = if self.prof.enabled {
            let slot = self.prof.paint_slot();
            root.child(
                gpui::canvas(
                    move |_bounds, _window, _cx| {},
                    move |_bounds, _prepaint, _window, _cx| slot.stamp(),
                )
                .absolute()
                .size(px(0.0)),
            )
        } else {
            root
        };
        // F1-5-1: registra as marcas de CPU deste frame (fechado no próximo begin_frame, quando
        // o frametime e o carimbo do sentinela existirem).
        if let (Some(poll_end), Some(assemble_end)) = (prof_poll_end, prof_assemble_end) {
            self.prof.finish_frame(prof::RenderMarks {
                t0: frame_now,
                poll_end,
                assemble_end,
                end: Instant::now(),
                live: cards.len(),
                drawn: panels_drawn,
                runs: prof_frame_runs,
            });
        }
        root
    }
}

/// PRODUÇÃO vs DEMO. `LINA_DEMO` (1/true/on/yes) liga o modo demo do fundador: semeia Terminal A/B,
/// arma o botão ⚡ A2A A→B e grava o workspace em `temp`. Sem a env → PRODUÇÃO (default do aluno).
/// FIX-3: a FONTE ÚNICA do nível de autonomia do Espaço (`LINA_AUTONOMY`, default `assistido` —
/// o default do produto, decisão do fundador). Consumida pelo wiring do `main` (BootstrapWriter +
/// RouterConfig) E pela UI (badge do card, prefill do modal) — superfície e enforcement nunca
/// divergem porque leem a mesma função.
fn workspace_autonomy() -> Autonomy {
    match std::env::var("LINA_AUTONOMY")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("manual") => Autonomy::Manual,
        Some("autonomo" | "autonomous") => Autonomy::Autonomous,
        _ => Autonomy::Assisted,
    }
}

fn lina_demo_enabled() -> bool {
    matches!(
        std::env::var("LINA_DEMO").ok().as_deref().map(str::trim),
        Some("1") | Some("true") | Some("on") | Some("yes")
    )
}

/// DEMO DE PAPEL (`LINA_DEMO_ROLE`): quando setado a um papel CANÔNICO (ex.: `DEVELOPER`), o modo
/// demo semeia UM terminal vivo COM ESSE PAPEL no lugar do par A/B — é o roster coerente que o
/// painel "Como o [papel] pensa" (F3-3 gate h) precisa: um papel VIVO que case as crenças semeadas
/// pelo `mentality_demo_seed`. Vazio/ausente → demo padrão (A→B). **Verbatim** (sem uppercase): o
/// papel canônico tem caixa própria (`DEVELOPER` mas `reviewer`) e precisa bater EXATO com a `role`
/// das crenças; quem chama (o `demo-mentality.sh`) passa o token exato.
fn demo_role() -> Option<String> {
    std::env::var("LINA_DEMO_ROLE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Base PERSISTENTE por-usuário (macOS: `~/Library/Application Support`). Sem `$HOME` (raríssimo): cai
/// no `temp` como degradação VISÍVEL — a alternativa (panic) violaria inv#6 ("app NUNCA quebrar").
fn app_support_dir() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home)
            .join("Library")
            .join("Application Support"),
        None => {
            eprintln!(
                "lina-gpui: HOME ausente — usando diretório temporário (o estado NÃO persistirá)"
            );
            std::env::temp_dir()
        }
    }
}

/// Onde o workspace do aluno é GRAVADO. Prioridade: `LINA_WS_ROOT` (override de dev) > DEMO (`temp`,
/// caminho histórico do fundador) > PRODUÇÃO (Application Support persistente). NUNCA `temp` em
/// produção: o SO o limpa no reboot = perda do trabalho do aluno, viola inv#6.
fn resolve_ws_root(demo: bool) -> PathBuf {
    if let Ok(p) = std::env::var("LINA_WS_ROOT") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if demo {
        return std::env::temp_dir().join("lina-space-ws3");
    }
    app_support_dir().join("Lina").join("walking-skeleton")
}

/// inv#6 (nunca tela em branco): quando o canvas não tem nenhum terminal (workspace fresco/produção, ou
/// o raro caso de spawn que falhou), uma guia centralizada substitui o vazio — o aluno sabe o próximo
/// passo (⌘N, a porta RICA: modal com papel+motor; ⌘T continua funcionando, só não é ensinado —
/// quick win da rodada 360: UMA porta na superfície). Some sozinho assim que o 1º card existe.
fn empty_canvas_hint() -> impl IntoElement {
    let th = theme::active();
    div()
        .absolute()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .font_family(th.typography.family.display)
                .text_size(px(f32::from(th.typography.size.display)))
                .child(text!("✨")),
        )
        .child(
            // MOMENTO da fusão (directive #4): Fraunces no acolhimento — a 1ª vez que o fundador vê
            // o Espaço pronto. Fraunces é a "decoração" autorizada: UMA aparição, no momento, não no corpo.
            div()
                .font_family(th.typography.family.display)
                .text_size(px(f32::from(th.typography.size.heading)))
                .font_weight(FontWeight(f32::from(th.typography.weight.semibold)))
                .text_color(rgb(th.text.bright))
                .child(text!("Seu Espaço está pronto")),
        )
        .child(
            // Instrução = UI em IBM Plex Sans (Fraunces só no momento, nunca em rótulo recorrente).
            div()
                .font_family(th.typography.family.ui)
                .text_size(px(f32::from(th.typography.size.subtitle)))
                .text_color(rgb(th.text.secondary))
                .child(text!("Pressione ⌘N para criar seu primeiro agente")),
        )
}

/// `true` se o `PATH` já contém alguma localização "de usuário/dev" (Homebrew, nvm, volta, cargo,
/// `~/.local/bin`, `/usr/local/bin`) — sinal de que foi aberto via terminal OU já hidratado. **Puro**
/// (testável sem env). Quando falso, estamos no PATH mínimo do `launchd` e precisamos hidratar.
#[must_use]
fn path_looks_hydrated(path: &str) -> bool {
    path.split(':').any(|p| {
        p.contains("/homebrew/")
            || p == "/usr/local/bin"
            || p.contains("/.nvm/")
            || p.ends_with("/.local/bin")
            || p.contains("/.volta/")
            || p.contains("/.cargo/")
            || p.contains("/.bun/")
            || p.contains("/.deno/")
    })
}

/// Candidatos CONHECIDOS de diretórios onde CLIs de IA/npm vivem — a contraparte exata das
/// famílias que `path_looks_hydrated` reconhece. **Puro** (só monta caminhos a partir do home).
/// nvm fica de fora de propósito (dirs versionados, sem `current/bin` estável).
fn known_cli_dir_candidates(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    vec![
        home.join(".local/bin"), // claude, agy (instaladores oficiais) — bug de tela 2026-06-20
        std::path::PathBuf::from("/opt/homebrew/bin"),
        std::path::PathBuf::from("/usr/local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
        home.join(".deno/bin"),
        home.join(".opencode/bin"), // opencode (instalador próprio) — fora do PATH do Dock
    ]
}

/// Filtra os candidatos que EXISTEM no disco. É o que torna o fallback determinístico:
/// zero shell, zero rc do usuário, zero espera — só `is_dir`.
fn existing_dirs(candidates: &[std::path::PathBuf]) -> Vec<String> {
    candidates
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// **Garante os dirs de CLI conhecidos no PATH — SEMPRE, não só quando o PATH "parece pobre".**
///
/// Bug de tela 2026-06-11: o `zsh -lic` é best-effort com timeout de 2s; sob carga ele estoura e
/// o app ficava com o PATH mínimo do launchd — nenhum motor spawnava no restore.
///
/// Bug de tela 2026-06-20 (a razão do `path_looks_hydrated` ter saído daqui): "parece hidratado"
/// (tem `/opt/homebrew/bin`) NÃO garante `~/.local/bin` (onde mora o `claude`/`agy`) nem
/// `~/.opencode/bin`. Um login shell não-interativo costuma exportar o Homebrew mas não esses —
/// então a DESCOBERTA de CLIs (modal) e o restore só enxergavam codex/gemini, e claude/opencode/
/// antigravity SUMIAM da lista de motores. Agora unimos SEMPRE os dirs conhecidos que EXISTEM,
/// adicionando só os que FALTAM no PATH atual. Determinístico (`is_dir`, sem shell).
#[cfg(unix)]
fn ensure_known_cli_dirs_in_path() {
    let effective = std::env::var("PATH").unwrap_or_default();
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let missing = missing_cli_dirs(&effective, std::path::Path::new(&home));
    if missing.is_empty() {
        return;
    }
    let augmented = augmented_verify_path(&effective, &missing);
    // edition 2021: `set_var` é seguro (e rodamos antes de qualquer thread/janela subir).
    std::env::set_var("PATH", augmented.as_str());
    eprintln!(
        "lina-gpui: PATH completado com {} dir(s) de CLI ({}) — descoberta e spawn agora os enxergam",
        missing.len(),
        missing.join(", ")
    );
}

/// Núcleo PURO de [`ensure_known_cli_dirs_in_path`]: os dirs de CLI conhecidos que EXISTEM e
/// FALTAM no `path_env`. Puro (depende só de `path_env` + `home` + FS) → testável sem mexer no
/// ambiente. É o que prova a regressão do bug de tela 2026-06-20: `~/.local/bin` precisa entrar
/// AINDA QUE o PATH já tenha o Homebrew ("parecer rico" ≠ ter o dir do `claude`).
#[cfg(unix)]
fn missing_cli_dirs(path_env: &str, home: &std::path::Path) -> Vec<String> {
    let known = existing_dirs(&known_cli_dir_candidates(home));
    let present: std::collections::HashSet<String> = std::env::split_paths(path_env)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    known.into_iter().filter(|d| !present.contains(d)).collect()
}

/// **ADR 0037 / Melhoria CLI-no-boot** — os dirs `bin` das versões de Node instaladas via **nvm**
/// (`~/.nvm/versions/node/<v>/bin`). O fallback de hidratação os exclui de propósito (não há
/// `current/bin` estável), MAS é exatamente aí que um `claude` instalado por `npm i -g` sob nvm
/// vive — e onde o restore via Dock o perdia (PATH mínimo do launchd + login shell estourando o
/// timeout). Determinístico (`read_dir`, sem shell). Vazio se não houver nvm. **Puro** (depende
/// só do home + FS), testável injetando um `home` sintético.
#[cfg(unix)]
fn nvm_bin_dirs(home: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path().join("bin"))
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    out.sort(); // determinístico (qualquer versão serve — é o mesmo CLI; só precisamos achar UM).
    out
}

/// **ADR 0037 / Melhoria CLI-no-boot** — resolve o programa de um CLI (nome nu como `claude`)
/// para o seu CAMINHO ABSOLUTO, buscando ALÉM do `PATH` do processo: dirs canônicos de CLI
/// (`known_cli_dir_candidates`) + os dirs versionados do nvm. É o que faz o restore re-lançar o
/// `claude` mesmo quando o app sobe pelo Dock com o PATH mínimo e o login shell falhou. O `PATH`
/// vem PRIMEIRO (quando o login shell funcionou, ele já é a verdade); os extras são fallback.
/// `None` = nada a trocar (já tem `/` → o exec resolve sozinho; ou não foi encontrado em lugar
/// nenhum → o restore degrada honestamente como hoje). Best-effort; `find_in_path` é puro.
#[cfg(unix)]
pub(crate) fn resolve_cli_program(program: &str) -> Option<String> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    resolve_cli_program_in(program, &path_env, home.as_deref())
}

/// Núcleo PURO de [`resolve_cli_program`] (injeta `path_env` + `home` p/ teste determinístico,
/// sem tocar o ambiente do processo). O `PATH` vem primeiro; os dirs canônicos + nvm são fallback.
#[cfg(unix)]
fn resolve_cli_program_in(
    program: &str,
    path_env: &str,
    home: Option<&std::path::Path>,
) -> Option<String> {
    if program.contains('/') {
        return None; // caminho (absoluto/relativo) já é resolvível pelo exec — não mexe.
    }
    let mut search = path_env.to_string();
    if let Some(home) = home {
        let mut extra = existing_dirs(&known_cli_dir_candidates(home));
        extra.extend(nvm_bin_dirs(home));
        for d in extra {
            if !d.is_empty() {
                search.push(':');
                search.push_str(&d);
            }
        }
    }
    lina_core::find_in_path(program, &search).map(|p| p.to_string_lossy().into_owned())
}

/// No-op fora de unix (Windows resolve o exe pelo PATH/extensões do SO — sem o problema do launchd).
#[cfg(not(unix))]
pub(crate) fn resolve_cli_program(_program: &str) -> Option<String> {
    None
}

/// **ADR 0037 / Melhoria CLI-no-boot** — reescreve o programa (`command[0]`) de cada terminal a
/// re-erguer para o caminho ABSOLUTO resolvido por [`resolve_cli_program`], quando encontrável.
/// Aplicado entre `plan_restore` e `restore_terminals` nos DOIS caminhos de restore (boot e
/// religação de Espaço descarregado): blinda o re-lançamento do CLI contra o PATH pobre do Dock.
pub(crate) fn absolutize_restore_commands(plans: &mut [bridge::RestoredTerminal]) {
    for p in plans {
        if let Some(first) = p.command.first() {
            if let Some(abs) = resolve_cli_program(first) {
                if abs != *first {
                    eprintln!(
                        "lina-gpui: [RESTORE] CLI '{first}' resolvido para caminho absoluto \
                         (PATH do boot não o cobria): {abs}"
                    );
                    p.command[0] = abs;
                }
            }
        }
    }
}

/// Extrai o PATH carimbado pela sentinela `marker` no stdout do shell de login (ignora ruído de rc).
/// **Puro** (testável sem subprocesso).
#[must_use]
fn parse_marked_path<'a>(stdout: &'a str, marker: &str) -> Option<&'a str> {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix(marker))
        .map(str::trim)
        .filter(|p| !p.is_empty())
}

/// Resolve o PATH REAL do **shell de login** do usuário (sourceia `.zprofile`/`.zshrc` etc.), onde
/// Homebrew/nvm/volta exportam o PATH. **BOUNDED** (~2.5s) e best-effort: um rc pendurado/erro → `None`.
/// É a base de duas coisas: hidratar o PATH no boot ([`hydrate_path_from_login_shell`]) e RE-resolver
/// após "Instalar para mim" (o instalador acabou de escrever o seu dir no rc → um shell de login novo
/// já o enxerga). Não toca o `env` — só devolve a string; o caller decide.
#[cfg(unix)]
pub(crate) fn resolve_login_shell_path() -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    // A sentinela isola o PATH do ruído impresso por rc interativo.
    const MARK: &str = "__LINA_PATH__=";
    let script = format!("printf '%s%s\\n' '{MARK}' \"$PATH\"");
    let mut child = Command::new(&shell)
        .args(["-lic", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| eprintln!("lina-gpui: não resolvi o PATH do shell de login ({shell}): {e}"))
        .ok()?;

    let mut out = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf); // canal pode ter sido abandonado (timeout) → ignore.
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(buf) => {
            let _ = child.wait();
            parse_marked_path(&String::from_utf8_lossy(&buf), MARK).map(str::to_string)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("lina-gpui: PATH do shell de login demorou demais — sigo com o PATH atual");
            None
        }
    }
}

/// No-op fora de unix (Windows não tem o problema do PATH de `launchd`).
#[cfg(not(unix))]
pub(crate) fn resolve_login_shell_path() -> Option<String> {
    None
}

/// macOS/Linux: um app aberto pelo **Finder/Dock** herda só o PATH mínimo do `launchd`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`) — SEM Homebrew, nvm, volta, `~/.local/bin` nem o bin global do
/// npm. Resultado: a varredura de CLIs (lê `PATH` do env) não acha `claude`/`codex` mesmo instalados,
/// e o "Instalar para mim" falha com *"No viable candidates found in PATH"* (o `npm` é invisível).
/// Aqui resolvemos o PATH REAL do shell de login e o injetamos no processo: a descoberta e os PTYs
/// filhos (portable-pty semeia o env de `std::env::vars_os()`) passam a enxergar tudo. Best-effort.
#[cfg(unix)]
fn hydrate_path_from_login_shell() {
    let current = std::env::var("PATH").unwrap_or_default();
    // Resolução via login shell só quando o PATH parece POBRE (evita o custo do subshell quando
    // já estamos num terminal). NÃO retornamos aqui: mesmo com o PATH "rico" (Homebrew presente),
    // ~/.local/bin (claude/agy) e ~/.opencode/bin podem estar de fora — o `ensure` abaixo SEMPRE
    // roda. (bug de tela 2026-06-20: o `return` antigo aqui pulava o `ensure` e claude sumia.)
    if !path_looks_hydrated(&current) {
        if let Some(resolved) = resolve_login_shell_path() {
            if resolved != current {
                // edition 2021: `set_var` é seguro (rodamos antes de qualquer thread/janela subir).
                std::env::set_var("PATH", resolved.as_str());
                eprintln!(
                    "lina-gpui: PATH hidratado do shell de login ({} entradas) — CLIs/npm agora visíveis",
                    resolved.split(':').count()
                );
            }
        }
    }
    // SEMPRE garante os dirs de CLI conhecidos — independente de o PATH "parecer hidratado".
    ensure_known_cli_dirs_in_path();
}

/// No-op fora de unix (Windows não tem o problema do PATH de `launchd`; o de-risk do ConPTY é à parte).
#[cfg(not(unix))]
fn hydrate_path_from_login_shell() {}

/// Expande `~`/`~/...` (e `~\...` no Windows) para o diretório do usuário (`HOME`/`USERPROFILE`).
/// **Puro** (depende só do env de home). Usado p/ os `verify_paths` das receitas (ex.: `~/.opencode/bin`).
#[must_use]
pub(crate) fn expand_tilde(p: &str) -> String {
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if p == "~" {
        if let Some(h) = home() {
            return h.to_string_lossy().into_owned();
        }
    } else if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(h) = home() {
            return std::path::Path::new(&h)
                .join(rest)
                .to_string_lossy()
                .into_owned();
        }
    }
    p.to_string()
}

/// Une `extra_dirs` (com prioridade, já que o binário recém-instalado pode estar lá) ao `base` PATH,
/// removendo duplicatas e vazios, respeitando o separador do SO (`:`/`;`). **Puro/determinístico**
/// (usa `std::env::{split_paths,join_paths}`). É o PATH ampliado sobre o qual a verificação pós-install
/// roda a descoberta.
#[must_use]
pub(crate) fn augmented_verify_path(base: &str, extra_dirs: &[String]) -> String {
    use std::collections::HashSet;
    use std::path::PathBuf;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !p.as_os_str().is_empty() && seen.insert(p.clone()) {
            out.push(p);
        }
    };
    for d in extra_dirs {
        push(PathBuf::from(expand_tilde(d)));
    }
    for p in std::env::split_paths(base) {
        push(p);
    }
    std::env::join_paths(&out)
        .map(|os| os.to_string_lossy().into_owned())
        .unwrap_or_else(|_| base.to_string())
}

/// Após "Instalar para mim": RE-resolve o PATH do shell de login (o instalador acabou de escrever seu
/// dir no rc → um login shell novo já o vê) e une os `verify_paths` fixos da receita, gravando no
/// processo. Assim a verificação acha o binário recém-instalado E o canvas já consegue spawná-lo sem
/// reabrir o app. Best-effort: se a re-resolução falhar, parte do PATH atual. Roda na thread de
/// instalação, logo após o instalador sair (set_var pontual, sem leitores concorrentes nesse instante).
pub(crate) fn refresh_path_after_install(verify_paths: &[String]) {
    let base =
        resolve_login_shell_path().unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    let augmented = augmented_verify_path(&base, verify_paths);
    std::env::set_var("PATH", augmented);
}

fn main() {
    // PRIMEIRA coisa: hidrata o PATH ANTES de qualquer varredura de CLI ou spawn de PTY (ver doc da
    // função). Sem isto, um app aberto pelo Finder não acha `claude`/`codex`/`npm` instalados.
    hydrate_path_from_login_shell();

    // **Lina universal (a base do produto):** instala a doutrina + a skill `lina-agent-bus` no config
    // GLOBAL de cada CLI (`~/.claude`, `~/.codex`, `~/.gemini`) — ADITIVO, IDEMPOTENTE, AUTO-GATED.
    // Sem isto, só AGENTES (cwd gerenciado) conheciam o Lina; um terminal PURO ou um `claude` rodado
    // em qualquer pasta ficava cego pro Lina → o leigo tinha de saber "novo agente vs novo terminal".
    // Agora as capacidades estão acessíveis em TUDO, em qualquer pasta. Best-effort (não aborta o boot).
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        match lina_bootstrap::ensure_lina_globally_available(&home) {
            Ok(r) => eprintln!(
                "lina-gpui: Lina universal instalado — doutrina em {} CLIs + {} skills (sob {})",
                r.doctrine_files.len(),
                r.skill_dirs.len(),
                home.display()
            ),
            Err(e) => eprintln!(
                "lina-gpui: instalação global do Lina falhou parcialmente ({e}) — agentes gerenciados seguem com a ficha por-cwd"
            ),
        }
    }
    let (cols, rows) = fit_dims();
    // BUG A (instrumentação de boot): o PTY é spawnado com EXATAMENTE estas `rows`, e o render trava a
    // line-height em CELL_H → as `rows` cabem inteiras no card (a ÚLTIMA = barra de input da TUI). O
    // Maestro confere: pty_rows (nos logs por-card) == estas `rows`.
    let (cw, ch, fpx) = (cell_w(), cell_h(), grid_font_px());
    eprintln!(
        "lina-gpui: layout · card {CARD_W}x{CARD_H} · cell {cw}x{ch} · font {fpx}px (line-height travada={ch}) · PTY {cols}x{rows} → {rows} linhas cabem (a ultima = input)"
    );

    // #22 (inv#4 · fonte-única forense): o app e o bin `lina` (do/guard/ask) compartilham UM ÚNICO
    // event log. O bin grava em `events_dir() = <LINA_HOME>/events` (LINA_HOME é setado logo abaixo,
    // = `<ws_root>/.lina`); aqui o app abre EXATAMENTE o mesmo diretório `<ws_root>/.lina/events`.
    // ANTES o app abria um store paralelo em `temp/lina-space-ws2`, DISJUNTO do log do bin (ws3): a
    // forense ficava partida em dois logs (ActionGated/BrokerDenied do bin vs. eventos do supervisor),
    // violando o invariante "o event log é a fonte da verdade". `ws_root`/`mailbox_dir` precisam
    // existir antes desta abertura; `EventStore::open` faz `create_dir_all` do diretório.
    // PRODUÇÃO vs DEMO + onde o workspace mora (item 6 / addendum): produção grava num diretório
    // PERSISTENTE por-usuário (Application Support), nunca em `temp` (que o reboot limpa = perda de
    // dados, viola inv#6). Demo/override mantêm o caminho histórico em `temp`.
    let demo = lina_demo_enabled();
    let ws_root = resolve_ws_root(demo);
    // F1-4-1 (fiação app): em PRODUÇÃO sem override de dev, o ponteiro global
    // `~/.lina/workspaces.json` decide QUAL Espaço abre — o focado por último (é o que o
    // switcher M8 carimba ao trocar). Ponteiro vazio/quebrado degrada para o default acima
    // (inv#6); `LINA_WS_ROOT` e DEMO seguem soberanos (escapes de dev intactos).
    let ws_root = if !demo && std::env::var("LINA_WS_ROOT").map_or(true, |v| v.trim().is_empty()) {
        match lina_core::default_registry_path().map(lina_core::WorkspaceRegistry::load) {
            Some(Ok(reg)) => workspace_boot::pick_production_root(&reg, ws_root),
            _ => ws_root,
        }
    } else {
        ws_root
    };
    // F1-2-1: aplica o tema PERSISTIDO (T7 `settings.json`, ao lado do event log) ANTES de abrir
    // qualquer janela — a escolha dark/light + acento sobrevive ao restart, 100% local (inv #2).
    // (Tema é GLOBAL do processo — aplicado aqui, fora do runtime por-Espaço.)
    // F2-1-4: `"sistema"` resolve com chute dark AQUI (não há janela ainda — appearance real
    // é inacessível); o 1º callback do observer (registrado na abertura da janela, abaixo)
    // re-resolve com o appearance REAL — janela claro-no-SO re-pinta no 1º frame, sem restart.
    {
        let s = persistence_ui::load_settings(&ws_root.join(".lina").join("events"));
        let setting = theme::ModeSetting::from_setting(&s.theme);
        theme::set_overrides(s.theme_overrides.clone()); // ajustes por token persistidos (F2-1-4)
        theme::set_reduce_motion(s.reduce_motion); // a11y persistido (F2-1-3)
        theme::apply_setting(setting, &s.accent); // carimbo interno default=dark (chute honesto)
    }

    // M8 fatia (i) · infra POR-PROCESSO (`SharedInfra`): o `PtyManager` (processos do SO), a
    // fábrica de shell e as dimensões do grid — os Espaços compartilham ESTES handles; todo o
    // resto nasce POR-Espaço dentro do `boot_ws_runtime`.
    let pty = Arc::new(Mutex::new(PtyManager::new()));
    // Fábrica de nós: todo terminal é um SHELL INTERATIVO REAL e COMPLETO — idêntico em
    // capacidade (o B aceita teclado e roda claude/vim igual ao A). Add/remove em runtime
    // reusa EXATAMENTE esta fábrica via NodeManager.
    let shared = runtime::SharedInfra {
        pty: Arc::clone(&pty),
        cmd_factory: Arc::new(shell_cmd),
        cols,
        rows,
    };

    // M8 fatia (i): TODO o boot por-Espaço (passos [2]-[17] do mapa) vive no `boot_ws_runtime`;
    // o main só resolve QUAL Espaço abre (acima) e opera demo/load-gen/janela SOBRE o runtime.
    // `Err` = o que antes encerrava o processo dentro do boot (inv#6: mensagem clara, nunca
    // backtrace) — quem encerra é o main, único dono dessa decisão.
    let rt = match runtime::boot_ws_runtime(ws_root, &shared, demo) {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("lina-gpui: {e} — encerrando.");
            std::process::exit(1);
        }
    };
    // F1-4-4 fatia (ii): o runtime do boot NÃO é desmontado — entra INTEIRO no mapa de
    // runtimes do processo (fundos VIVOS). Demo/load-gen/janela operam por handles CLONADOS
    // (tudo Arc, barato); a view lê sempre o runtime ATIVO e o switch re-aponta.
    let ws_root = rt.ws_root.clone();
    let mailbox_dir = rt.mailbox_dir.clone();
    let store = Arc::clone(&rt.store);
    let sup = Arc::clone(&rt.sup);
    let nodes = Arc::clone(&rt.nodes);
    let model = Arc::clone(&rt.model);
    let grids = Arc::clone(&rt.grids);
    let input: Arc<dyn InputSink> = rt.input.clone();
    let attention = Arc::clone(&rt.attention);
    let desk = Arc::clone(&rt.desk);
    let brake = Arc::clone(&rt.brake);
    let injection_profile = rt.injection_profile.clone();
    let profile_registry = Arc::clone(&rt.profile_registry);
    let dash = Arc::clone(&rt.dash);
    let runtimes = runtime::runtimes_with_boot(rt);
    let dir = mailbox_dir.join("events");
    // Estado PERSISTENTE do onboarding (progresso + log próprio), co-locado no workspace (subdir, não
    // colide com `.lina/events` do canvas). Sobrevive ao reboot em produção → o onboarding só aparece
    // na 1ª execução de verdade.
    let onboarding_dir = ws_root.join("onboarding");
    // ── SEED dos nós iniciais. PRODUÇÃO (default do aluno): NENHUM. O workspace nasce limpo e o aluno
    // cria o 1º agente com ⌘T (guiado pelo empty-state) — sem "Terminal A/B fantasma" poluindo o log.
    // DEMO (`LINA_DEMO=1`): semeia Terminal A + B (shells reais, idênticos) e arma o botão ⚡ A2A A→B.
    // Em QUALQUER caso, um spawn que falhe DEGRADA (loga e segue) — nunca panica (inv#6, antes `.expect`).
    let (focused, a2a): (NodeId, Option<Arc<A2aTrigger>>) = if demo {
        let _ = lock(&store).append(&DomainEvent::WorkspaceCreated {
            name: "walking-skeleton".into(),
            // W4-5: o walking-skeleton não passa pela galeria de Foco → preset não-setado (`""`).
            focus_preset: String::new(),
        });
        // DEMO DE PAPEL (`LINA_DEMO_ROLE`, F3-3 gate h): o painel "Como o [papel] pensa" precisa de
        // um papel VIVO que case as crenças semeadas (`mentality_demo_seed`). Quando o env nomeia um
        // papel, semeia UM terminal vivo COM ESSE PAPEL — sem o par A/B nem o ⚡ A2A (o teatro aqui é
        // a MENTALIDADE, não o A→B). Sem o env → demo padrão A→B (caminho inalterado, abaixo).
        if let Some(role) = demo_role() {
            // Nome leigo do papel na tela (nunca o código cru, inv#6) — "DEVELOPER" → "Desenvolvedor".
            let label = role_suggester::humanize(&role).0;
            let node = match nodes.admit_node(NodeAdmission::seeded_role_terminal(
                &label, &role, 30.0, 96.0,
            )) {
                Ok(node) => Some(node),
                Err(e) => {
                    eprintln!(
                        "lina-gpui: DEMO — terminal '{label}' ({role}) falhou ({e}); seguindo sem ele"
                    );
                    None
                }
            };
            (node.unwrap_or_default(), None)
        } else {
            // Porta seed/demo (tradutor fino — ADR 0022 §1): admissão pelo funil canônico, em
            // posição FIXA do roteiro. Um spawn que falhe DEGRADA (loga e segue) — nunca panica
            // (inv#6): o `Err` do funil vira aviso e o demo segue sem o nó.
            let seed_one = |name: &str, x: f64, y: f64| -> Option<NodeId> {
                match nodes.admit_node(NodeAdmission::seeded_terminal(name, x, y)) {
                    Ok(node) => Some(node),
                    Err(e) => {
                        eprintln!(
                            "lina-gpui: DEMO — spawn de '{name}' falhou ({e}); seguindo sem ele"
                        );
                        None
                    }
                }
            };
            let a = seed_one("Terminal A", 30.0, 96.0);
            let b = seed_one("Terminal B", 740.0, 96.0);
            // O ⚡ A2A A→B só faz sentido com AMBOS vivos; senão fica `None` (o botão some via
            // `ready()`). O grid de B vem do mapa vivo — o funil o registrou na admissão.
            let a2a = match (&a, &b) {
                (Some(na), Some(nb)) => lock(&grids).get(nb).cloned().map(|grid_b| {
                    Arc::new(A2aTrigger::new(
                        Arc::clone(&sup),
                        *na,
                        *nb,
                        grid_b,
                        // F1-0-2: o ⚡ demo usa o MESMO perfil real carregado no boot (sem demo hardcoded).
                        // A2A UNIVERSAL: este é o FALLBACK; o ⚡ resolve o profile do ALVO pelo registry.
                        injection_profile.clone(),
                        Arc::clone(&profile_registry),
                        Arc::clone(&store),
                        Arc::clone(&model),
                    ))
                }),
                _ => None,
            };
            // O foco arranca no 1º terminal vivo (A, ou B se A falhou); o render reaponta sozinho depois.
            let focused = a.or(b).unwrap_or_default();
            (focused, a2a)
        }
    } else {
        // PRODUÇÃO: sem nós. `focused` é a sentinela nil (`NodeId::default()`); o render reaponta ao 1º
        // card assim que o aluno cria um com ⌘T. `a2a = None` → o botão ⚡ nem aparece (gate `ready()`).
        (NodeId::default(), None)
    };

    // ── F1-5-1 · CENÁRIO DE CARGA REPRODUTÍVEL da sonda [PROF] (dev/medição; opt-in por env).
    // `LINA_LOAD=N` sobe N painéis pelo MESMO funil canônico (`admit_node`, ADR 0022), cada um
    // rodando o GERADOR `tools/loadgen.sh` (shell + script — SEM CLI de IA real): os primeiros
    // `LINA_LOAD_ACTIVE` (default: todos) alternam rajada/spinner ANSI; o resto fica em silêncio
    // (cenário-alvo do produto: 8-12 ativos + resto ocioso). Matriz da story: N∈{4,16,28}.
    // Como rodar: tasks/epico-f1/prof-baseline.md (rodar SEM LINA_DEMO — posições colidem).
    // Spawn que falha DEGRADA (loga e segue — inv#6), nunca panica. O gerador é `sh` (unix);
    // no Windows o spawn degrada com log — a sessão de medição da F1-5-1 é no Mac do fundador.
    let load_n: usize = std::env::var("LINA_LOAD")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    if load_n > 0 {
        let n = load_n.min(64); // backstop anti-dedo-gordo (64 shells já passa da matriz)
        if n < load_n {
            eprintln!("lina-gpui: [PROF] carga LINA_LOAD={load_n} clampada em {n}");
        }
        let active: usize = std::env::var("LINA_LOAD_ACTIVE")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(n)
            .min(n);
        // O gerador: override por env (`LINA_LOADGEN`) ou o do repo (caminho de COMPILE-TIME —
        // ferramenta de dev/medição, roda na máquina que compilou).
        let script = std::env::var("LINA_LOADGEN").unwrap_or_else(|_| {
            concat!(env!("CARGO_MANIFEST_DIR"), "/tools/loadgen.sh").to_string()
        });
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for i in 0..n {
            let mode = if i >= active {
                "silence"
            } else if i % 2 == 0 {
                "burst"
            } else {
                "spinner"
            };
            // Grade de 7 colunas; o NOME carrega o modo (legível no top-K do [PROF]).
            let x = 30.0 + (i % 7) as f64 * f64::from(CARD_W + 30.0);
            let y = 96.0 + (i / 7) as f64 * f64::from(CARD_H + 30.0);
            let admission = NodeAdmission {
                name: Some(format!("Carga {:02} ({mode})", i + 1)),
                role: "terminal".into(),
                engine: Some(AgentEngine {
                    program: "/bin/sh".into(),
                    args: vec![script.clone(), mode.into(), format!("{}", i + 1)],
                    profile_id: None,
                    label: "loadgen".into(),
                }),
                // Como o ⌘T (terminal puro): NENHUM kit/arquivo do Lina escrito (não é agente).
                cwd: CwdPolicy::UserHome { path: home.clone() },
                position: Some((x, y)),
                requested_by: None,
                autonomy: Autonomy::Assisted, // carga de profiling — autonomia default
                effort: lina_core::Effort::Medium, // F3-0-4: carga de profiling no default neutro
            };
            if let Err(e) = nodes.admit_node(admission) {
                eprintln!(
                    "lina-gpui: [PROF] carga — spawn do painel {} falhou ({e}); seguindo",
                    i + 1
                );
            }
        }
        eprintln!(
            "lina-gpui: [PROF] carga semeada: N={n} ativos={active} (burst/spinner alternados) ociosos={} · gerador {script}",
            n - active
        );
    }

    // M8 fatia (i): o snapshot fica AQUI (após demo/load-gen, como sempre) para o contador de
    // boot incluir os eventos do seed. Como as pumps já vivem (sobem no boot do runtime), a
    // leitura deixou de ser determinística — um tick adiantado pode somar eventos de fundo;
    // inofensivo (display do boot; o pump re-sincroniza o contador em ≤1 tick).
    let event_count = lock(&store).event_count().unwrap_or(0);
    lock(&model).event_count = event_count;

    // LINA_ATTENTION_DEMO (roteiro do fundador, idioma de LINA_DEMO/LINA_DASH): semeia
    // UM pedido REAL no log → o toast aparece sem esperar um Claude travado de verdade.
    // `1`|`yn` = permissão y/n (aprovável); `choice` = PERGUNTA de múltipla escolha
    // (R2b — toast "fez uma pergunta", ação primária IR ATÉ O TERMINAL, sem aprovar).
    // O hub/flag nasceram ANTES do pump (R2b); aqui só a SEMENTE (demo ⇒ live OFF) — o
    // boot do runtime já leu a MESMA env (helper único) p/ desligar a detecção viva.
    if let Some(kind) = runtime::attention_demo_kind().as_deref() {
        let (prompt_kind, detail) = match kind {
            "choice" => (
                lina_core::attention::PromptKind::Choice,
                "Qual opção você quer selecionar?".to_string(),
            ),
            _ => (
                lina_core::attention::PromptKind::Yn,
                "git push origin/master".to_string(),
            ),
        };
        let seeded = lock(&store).append(&lina_core::DomainEvent::PermissionAsked {
            node_id: "Terminal B".into(),
            tool: Some("Bash".into()),
            detail: Some(detail),
            evidence: lina_core::PermissionEvidence::Hook,
            stable_id: format!("demo-{}", lina_core::now_ms()),
            prompt_kind,
            vt_snapshot_hash: None,
        });
        match seeded {
            Ok(_) => eprintln!(
                "lina-gpui: [ATT] DEMO — PermissionAsked semeado (kind={prompt_kind:?}; toast em <2s)"
            ),
            Err(e) => eprintln!("lina-gpui: [ATT] DEMO — falha ao semear: {e}"),
        }
    }

    // F1-5-1: estado da sonda [PROF] no boot — o Maestro confirma por log que a rodada de
    // medição (LINA_PROF=1) e a de CONTROLE do overhead (LINA_PROF=0) são as que ele pediu.
    if prof::enabled_from_env() {
        eprintln!("lina-gpui: [PROF] sonda LIGADA (LINA_PROF=1) — janela ~120 frames; fases poll/assemble/chrome + layout_paint/present_vsync; overhead: compare o [FPS] com LINA_PROF=0");
    } else if std::env::var_os("LINA_PROF").is_some() {
        eprintln!("lina-gpui: [PROF] sonda DESLIGADA — rodada de CONTROLE do overhead (só [FPS])");
    }

    eprintln!(
        "lina-gpui: render de terminal · grid {cols}x{rows} · log {} · {event_count} eventos",
        dir.display()
    );

    // Auto-quit: SÓ quando `LINA_AUTOQUIT_MS` está setada (smoke/CI — dispara um A2A no meio, se houver,
    // e fecha). SEM a env, a janela fica aberta INDEFINIDAMENTE: um app de aluno NUNCA se fecha sozinho
    // (o antigo default de 30 min era um demo-ism crítico — removido).
    if let Some(ms) = std::env::var("LINA_AUTOQUIT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
    {
        let a2a_auto = a2a.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(ms / 2));
            if let Some(a) = &a2a_auto {
                a.fire("echo '📨 A2A automatico (smoke)'".to_string());
            }
            thread::sleep(Duration::from_millis(ms / 2));
            std::process::exit(0);
        });
    }

    // W4-4: handles compartilhados p/ o painel de persistência (lê event_count/recovering do model;
    // loga WorkspaceFocusSet no store; settings.json ao lado de `dir`). Clones ANTES do `move`.
    let panel_model = Arc::clone(&model);
    let panel_store = Arc::clone(&store);
    let panel_dir = dir.clone();
    // F1-4-4 fatia (ii): handle do epílogo (parada limpa de TODOS os drenos pós-app.run).
    let runtimes_epilogue = Arc::clone(&runtimes);
    let shared_for_view = shared.clone();
    application().run(move |cx: &mut App| {
        // F2-1-1b: fontes da identidade EMBARCADAS (zero dependência da máquina) + medição da
        // célula real. ORDEM REAL (desvio documentado do pedido de costura): `fit_dims`/spawn de
        // PTY rodaram ANTES deste closure (main(), ~4751) sobre as consts de fallback — que o
        // teste `cell_fallbacks_match_embedded_grid_font_metrics` PROVA idênticas ao TTF
        // embarcado; a medição aqui é confirmação em runtime (vence para o render via atomics;
        // divergência insana é recusada pelo guard de `set_cell_metrics`).
        if let Err(e) = cx.text_system().add_fonts(theme::embedded_fonts()) {
            eprintln!(
                "lina-gpui: fontes embarcadas não carregaram ({e}) — seguindo nas do sistema"
            );
        }
        {
            let t = theme::active().typography;
            let font_id = cx.text_system().resolve_font(&gpui::font(t.family.mono));
            let sz = px(f32::from(t.size.grid));
            match cx.text_system().advance(font_id, sz, 'M') {
                Ok(adv) => {
                    let h = cx.text_system().ascent(font_id, sz)
                        + cx.text_system().descent(font_id, sz);
                    bridge::set_cell_metrics(f32::from(adv.width), f32::from(h));
                }
                Err(e) => eprintln!(
                    "lina-gpui: não medi a célula do grid ({e}) — usando o fallback derivado do TTF"
                ),
            }
        }
        // inv#6: o CANVAS é a base e abre SEMPRE (nunca tela em branco). O onboarding entra como janela
        // SOBREPOSTA na 1ª execução (W4-1) e se fecha sozinho no fim ("Abrir meu Espaço →"), revelando o
        // canvas por baixo — sem precisar passar os handles do canvas pro onboarding.
        let bounds = Bounds::centered(None, size(px(1450.0), px(640.0)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lina Space — terminais reais (render de grid · sem fios)".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                // F2-1-4 (decisão F2-0-D nº2 — chrome SEGUE O SISTEMA): com a janela viva, o
                // appearance real existe. SEM captura (v2 anti-staleness): o observer só
                // carimba o estado do SO; quem decide re-aplicar é o estado VIVO da
                // preferência (`set_system_appearance` re-aplica só quando a preferência
                // corrente é Sistema — escuro/claro explícitos apenas guardam o carimbo,
                // para a troca futura nos Ajustes resolver certo). 1º fire corrige o chute
                // dark do boot para quem usa SO claro.
                let stamp_system_appearance = |window: &mut Window| {
                    let is_dark = matches!(
                        window.appearance(),
                        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                    );
                    theme::set_system_appearance(is_dark);
                };
                stamp_system_appearance(window);
                window
                    .observe_window_appearance(move |window, _cx| {
                        stamp_system_appearance(window);
                    })
                    .detach();
                cx.new(|cx| {
                    WorkspaceView::new(
                        nodes,
                        input,
                        a2a,
                        focused,
                        desk,
                        brake,
                        dash,
                        attention,
                        panel_dir.clone(), // F1-1-7: settings.json mora no dir do event log
                        runtimes,
                        shared_for_view,
                        window,
                        cx,
                    )
                })
            },
        );
        // no-panic: se a janela não abrir (gpui/SO catastrófico) encerra com mensagem, não com backtrace.
        if let Err(e) = opened {
            eprintln!("lina-gpui: não abri a janela do canvas ({e}); encerrando.");
            std::process::exit(1);
        }
        // W4-4: painel de persistência (env-gated LINA_PERSIST_PANEL; não perturba a demo do canvas).
        if persistence_ui::should_show() {
            persistence_ui::open_window(
                cx,
                Arc::clone(&panel_model),
                Arc::clone(&panel_store),
                panel_dir.clone(),
            );
        }
        // Onboarding turno-0 por cima (1ª execução / progresso < Done). Demo pula por padrão; dev força
        // com `LINA_ONBOARDING=1|0`. Aberto por ÚLTIMO → fica em foco sobre o canvas.
        if onboarding::should_show(&onboarding_dir, demo) {
            // `mailbox_dir` = `<ws_root>/.lina`: a etapa do segundo cérebro grava `vault.json` +
            // `vault-index/` aqui (integração com a doutrina via arquivos, sem handles do canvas).
            onboarding::open_window(cx, onboarding_dir.clone(), mailbox_dir.clone());
        }
        cx.activate(true);
    });

    // F1-4-4 fatia (ii): N runtimes vivos → o epílogo para TODOS os drenos (antes: pump único).
    runtime::stop_all(&runtimes_epilogue);
    drop(sup);
    drop(pty);
}

// ═══════════════════════ Rodada confiabilidade · tela honesta (#22/#21/#4) ═══════════════════════

/// Estado VISÍVEL do nó no cabeçalho do card, em pt-br do leigo — a MESMA voz que o leitor
/// de tela já fala (a a11y usa `canvas::aggregate_badge(...).label()`). Antes o card despejava
/// o `Debug` cru do enum (`Busy`/`Idle`/`Dead`/`Crashed`) — jargão em inglês na tela do fundador
/// (#22, "tela mente"). `needs_human=false`: o badge "precisa de você" (renderizado à parte) é o
/// dono daquele anúncio; aqui vai só o estado de vida. `unseen=0`: a contagem de linhas novas é
/// da periferia, não do título.
fn node_status_label(status: NodeStatus) -> String {
    canvas::aggregate_badge(status, false, 0).label()
}

/// Tipo do nó em pt-br do leigo — o card mostrava `WebBrowser`/`TaskPanel` crus (Debug do enum).
/// `#[non_exhaustive]`: um tipo FUTURO do core cai no fallback honesto "item" (vivo, sem fingir
/// um nome que não temos), nunca o identificador Rust.
fn node_kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Terminal => "Terminal",
        NodeKind::Note => "Nota",
        NodeKind::WebBrowser => "Navegador",
        NodeKind::Folder => "Pasta",
        NodeKind::Attachment => "Anexo",
        NodeKind::Trigger => "Gatilho",
        NodeKind::TaskPanel => "Painel",
        NodeKind::Feed => "Feed",
        _ => "item",
    }
}

/// #22 — texto do badge honesto "recebeu, não começou": um nó que recebeu um handoff
/// (`MessageDelivered`) e voltou a Idle SEM produzir trabalho não é um Ocioso comum — é um
/// despacho engolido. Alimentado pelo `AttentionKind::DeliveredNoProgress` que o W3
/// (`attention.rs`) publica na fila. Const isolada → o anti-vazamento testa a frase REAL que o
/// card renderiza (zero jargão de `DeliveredNoProgress`/`AttentionKind` na superfície).
const DELIVERY_STALLED_BADGE: &str = "recebeu, ainda não começou";

/// #21 — copy do disjuntor (circuit breaker): o estado MAIS crítico do produto aparecia como
/// âmbar "trabalhando" (o fundador concluiu "já terminou" e perdeu um worker). A frase carrega o
/// ESTADO ("pausado por segurança") + a SAÍDA de 1 clique ("liberar") no MESMO texto. Fonte única:
/// o card a renderiza no chip clicável; a `canvas::aggregate_badge` (a11y/header) lê ESTA const;
/// o gate `honest_state_tests` a assere (zero jargão, estado + saída). `pub(crate)` porque o
/// `canvas` a consome como a voz do leitor de tela.
pub(crate) const CIRCUIT_BREAKER_LABEL: &str = "pausado por segurança — clique para liberar";

#[cfg(test)]
mod ime_tests {
    use super::*;

    // W2-4 (dívida IME): o texto COMPOSTO que o IME entrega (dead-keys pt-br ´~^, cedilha, CJK) deve
    // virar exatamente os bytes UTF-8 que vão ao PTY do nó focado — é o análogo de `key_char`, mas a
    // partir do caractere FINAL composto. Determinístico e sem gpui (a UI não roda headless).
    #[test]
    fn composed_text_dead_keys_pt_br_to_utf8_bytes() {
        // ´ + a = á ; ~ + a = ã ; ^ + e = ê ; ~ + o = õ ; cedilha = ç (UTF-8 de cada).
        assert_eq!(composed_text_to_pty_bytes("á"), vec![0xC3, 0xA1]);
        assert_eq!(composed_text_to_pty_bytes("ã"), vec![0xC3, 0xA3]);
        assert_eq!(composed_text_to_pty_bytes("ê"), vec![0xC3, 0xAA]);
        assert_eq!(composed_text_to_pty_bytes("õ"), vec![0xC3, 0xB5]);
        assert_eq!(composed_text_to_pty_bytes("ç"), vec![0xC3, 0xA7]);
        // ASCII puro também flui por aqui (todo imprimível passou a ir pelo IME, não pelo keystroke).
        assert_eq!(composed_text_to_pty_bytes("a"), vec![b'a']);
        // Vazio → nada a enviar (o chamador NÃO submete WriteOp).
        assert!(composed_text_to_pty_bytes("").is_empty());
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    /// `path_looks_hydrated`: PATH mínimo do `launchd` (Finder) → falso (precisa hidratar); PATH com
    /// Homebrew/nvm/etc. (terminal) → verdadeiro (não mexe). Determinístico, sem env.
    #[test]
    fn detects_minimal_launchd_path_vs_rich() {
        assert!(!path_looks_hydrated("/usr/bin:/bin:/usr/sbin:/sbin"));
        assert!(!path_looks_hydrated(""));
        assert!(path_looks_hydrated("/opt/homebrew/bin:/usr/bin:/bin"));
        assert!(path_looks_hydrated("/usr/local/bin:/usr/bin"));
        assert!(path_looks_hydrated(
            "/Users/x/.nvm/versions/node/v22.0.0/bin:/usr/bin"
        ));
        assert!(path_looks_hydrated("/Users/x/.local/bin:/usr/bin"));
    }

    /// `known_cli_dir_candidates`: a contraparte de `path_looks_hydrated` — as MESMAS famílias de
    /// diretórios que o teste de riqueza reconhece, montadas a partir do home dado. Puro.
    #[test]
    fn known_cli_dir_candidates_covers_hydration_families() {
        let home = std::path::Path::new("/Users/x");
        let got = known_cli_dir_candidates(home);
        for want in [
            "/Users/x/.local/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/Users/x/.cargo/bin",
            "/Users/x/.bun/bin",
            "/Users/x/.volta/bin",
            "/Users/x/.deno/bin",
            "/Users/x/.opencode/bin",
        ] {
            assert!(
                got.iter().any(|p| p == std::path::Path::new(want)),
                "candidato ausente: {want}"
            );
        }
    }

    /// `existing_dirs`: só devolve candidatos que EXISTEM no disco — é o que torna o fallback
    /// determinístico sem depender do rc do usuário (bug 2026-06-11: timeout do `zsh -lic` sob
    /// carga deixava o app com o PATH mínimo do launchd e NENHUM motor spawnava no restore).
    #[test]
    fn existing_dirs_filters_by_disk() {
        let base = std::env::temp_dir().join(format!("lina-pathfb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join(".local/bin");
        std::fs::create_dir_all(&real).expect("cria dir real");
        let ghost = base.join(".volta/bin");
        let got = existing_dirs(&[real.clone(), ghost]);
        assert_eq!(got, vec![real.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ADR 0037 / Melhoria CLI-no-boot: `nvm_bin_dirs` enxerga os `bin` versionados do nvm que o
    /// fallback canônico ignora — é onde um `claude` instalado por `npm i -g` sob nvm vive.
    #[cfg(unix)]
    #[test]
    fn nvm_bin_dirs_finds_versioned_node_bins() {
        let home = std::env::temp_dir().join(format!("lina-nvm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let v20 = home.join(".nvm/versions/node/v20.1.0/bin");
        let v18 = home.join(".nvm/versions/node/v18.0.0/bin");
        std::fs::create_dir_all(&v20).expect("v20");
        std::fs::create_dir_all(&v18).expect("v18");
        let got = nvm_bin_dirs(&home);
        assert!(
            got.iter().any(|d| d.ends_with("v20.1.0/bin")),
            "viu v20: {got:?}"
        );
        assert!(
            got.iter().any(|d| d.ends_with("v18.0.0/bin")),
            "viu v18: {got:?}"
        );
        // sem nvm → vazio (degrada honestamente, nunca panica).
        let nada = std::env::temp_dir().join(format!("lina-sem-nvm-{}", std::process::id()));
        assert!(nvm_bin_dirs(&nada).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ADR 0037 / Melhoria CLI-no-boot: o cerne do fix — `resolve_cli_program_in` acha o `claude`
    /// num dir nvm do home QUANDO o `PATH` do boot (mínimo do Dock) NÃO o contém. É o que faz o
    /// restore re-lançar o CLI por caminho absoluto em vez de degradar para shell.
    #[cfg(unix)]
    #[test]
    fn resolve_cli_program_finds_binary_outside_minimal_path() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("lina-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let nvm_bin = home.join(".nvm/versions/node/v20.1.0/bin");
        std::fs::create_dir_all(&nvm_bin).expect("nvm bin");
        let claude = nvm_bin.join("claude");
        std::fs::write(&claude, "#!/bin/sh\n").expect("write claude");
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        // PATH mínimo do launchd (sem claude) → resolve pelo dir nvm do home.
        let got = resolve_cli_program_in("claude", "/usr/bin:/bin", Some(&home));
        assert_eq!(
            got.as_deref(),
            claude.to_str(),
            "acha o claude no dir nvm fora do PATH mínimo"
        );
        // já-caminho não é tocado (o exec resolve sozinho).
        assert_eq!(
            resolve_cli_program_in("/usr/local/bin/claude", "/usr/bin", Some(&home)),
            None
        );
        // binário inexistente → None: o restore degrada honestamente (não inventa caminho).
        assert_eq!(
            resolve_cli_program_in("binario-que-nao-existe-xyz", "/usr/bin", Some(&home)),
            None
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **Regressão do bug de tela 2026-06-20** (claude/opencode/agy somem do modal): `~/.local/bin`
    /// e `~/.opencode/bin` TÊM que entrar no PATH mesmo quando ele já "parece rico" (tem Homebrew)
    /// — "parecer hidratado" ≠ "ter o dir do claude". O `return` antigo na hidratação pulava isso.
    #[cfg(unix)]
    #[test]
    fn missing_cli_dirs_includes_local_bin_even_with_homebrew_present() {
        let home = std::env::temp_dir().join(format!("lina-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".local/bin")).expect(".local/bin");
        std::fs::create_dir_all(home.join(".opencode/bin")).expect(".opencode/bin");

        // PATH "rico" (Homebrew) mas SEM ~/.local/bin nem ~/.opencode/bin → ambos faltam.
        let missing = missing_cli_dirs("/opt/homebrew/bin:/usr/bin", &home);
        assert!(
            missing.iter().any(|d| d.ends_with("/.local/bin")),
            "faltou ~/.local/bin (onde mora o claude): {missing:?}"
        );
        assert!(
            missing.iter().any(|d| d.ends_with("/.opencode/bin")),
            "faltou ~/.opencode/bin: {missing:?}"
        );
        // já-presente não re-entra (idempotente).
        let local = home.join(".local/bin");
        let with_local = format!("{}:/opt/homebrew/bin", local.display());
        assert!(
            !missing_cli_dirs(&with_local, &home)
                .iter()
                .any(|d| d.ends_with("/.local/bin")),
            "~/.local/bin já estava no PATH — não deve re-entrar"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Fim-a-fim puro: PATH mínimo do launchd + fallback de dirs conhecidos existentes →
    /// o PATH resultante PASSA no `path_looks_hydrated` (o motor volta a ser achável).
    #[cfg(unix)]
    #[test]
    fn fallback_known_dirs_hydrates_minimal_path() {
        let minimal = "/usr/bin:/bin:/usr/sbin:/sbin";
        let extras = vec!["/Users/x/.local/bin".to_string()];
        let got = augmented_verify_path(minimal, &extras);
        assert!(path_looks_hydrated(&got), "PATH segue mínimo: {got}");
        assert!(got.split(':').any(|p| p == "/usr/bin"), "base preservado");
    }

    /// `parse_marked_path`: extrai o PATH carimbado pela sentinela, ignorando ruído de rc antes/depois,
    /// e descarta linha vazia. É o que blinda a hidratação contra `.zshrc` falante.
    #[test]
    fn parses_path_behind_sentinel_ignoring_rc_noise() {
        let marker = "__LINA_PATH__=";
        let out = "Welcome to zsh!\nsome rc banner\n__LINA_PATH__=/opt/homebrew/bin:/usr/bin\n";
        assert_eq!(
            parse_marked_path(out, marker),
            Some("/opt/homebrew/bin:/usr/bin")
        );
        // sem sentinela → None.
        assert_eq!(parse_marked_path("só ruído de rc\n", marker), None);
        // sentinela com PATH vazio → None (não sobrescreve com nada).
        assert_eq!(parse_marked_path("__LINA_PATH__=\n", marker), None);
    }

    /// `expand_tilde`: `~`/`~/…` → home (lê o HOME ambiente, sem mutá-lo); sem `~` → inalterado.
    #[test]
    fn expand_tilde_resolves_home() {
        if let Some(home) = std::env::var_os("HOME") {
            let home = home.to_string_lossy().into_owned();
            assert_eq!(
                expand_tilde("~/.opencode/bin"),
                format!("{home}/.opencode/bin")
            );
            assert_eq!(expand_tilde("~"), home);
        }
        assert_eq!(expand_tilde("/opt/homebrew/bin"), "/opt/homebrew/bin");
    }

    /// `augmented_verify_path`: extras vêm na FRENTE (binário recém-instalado tem prioridade), sem
    /// duplicatas, vazios ignorados, base preservado. (separador `:` no unix do teste)
    #[cfg(unix)]
    #[test]
    fn augmented_verify_path_prepends_and_dedups() {
        let base = "/usr/bin:/opt/homebrew/bin";
        let got = augmented_verify_path(base, &["/opt/homebrew/bin".into(), "/x/bin".into()]);
        let parts: Vec<&str> = got.split(':').collect();
        assert_eq!(parts.first(), Some(&"/opt/homebrew/bin"), "extra na frente");
        assert!(parts.contains(&"/x/bin"), "novo dir presente");
        assert!(parts.contains(&"/usr/bin"), "base preservado");
        assert_eq!(
            parts.iter().filter(|p| **p == "/opt/homebrew/bin").count(),
            1,
            "dedup do dir repetido"
        );
    }
}

#[cfg(test)]
mod keymap_tests {
    use super::*;
    use gpui::Modifiers;

    fn ks_mod(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    fn ks(key: &str, shift: bool) -> Keystroke {
        ks_mod(
            key,
            Modifiers {
                shift,
                ..Default::default()
            },
        )
    }

    /// **BUG 3 (causa raiz REAL) — MODELO B:** arrasto PURO seleciona LOCAL mesmo com o TUI em
    /// mouse-reporting; só ⌥ Option + TUI-aceitou encaminha. (Antes, mouse-reporting ON roubava o
    /// arrasto pro Claude → "seleção que desselecciona". A seleção estendendo ao vivo = PENDENTE TELA.)
    #[test]
    fn pointer_route_is_local_unless_option_and_tui_accept() {
        // Arrasto PURO (sem Option) → LOCAL, mesmo se o TUI quisesse o mouse.
        assert_eq!(pointer_route(false, false), PointerRoute::LocalSelect);
        assert_eq!(pointer_route(false, true), PointerRoute::LocalSelect);
        // ⌥ Option mas TUI NÃO quer mouse (shell puro) → LOCAL.
        assert_eq!(pointer_route(true, false), PointerRoute::LocalSelect);
        // ⌥ Option + TUI aceitou o reporting → encaminha ao agente.
        assert_eq!(pointer_route(true, true), PointerRoute::ForwardToTui);
    }

    /// **BUG A — Shift+Enter NÃO submete: emite ESC+CR (`0x1b 0x0D` = Meta/Alt+Enter)**, que o Claude
    /// Code interpreta como nova linha. Enter PURO continua sendo submit (`0x0D`). Determinístico, sem
    /// gpui. A confirmação da quebra de linha num Claude real = PENDENTE TELA DO FUNDADOR.
    #[test]
    fn shift_enter_is_newline_not_submit() {
        assert_eq!(
            keystroke_to_bytes(&ks("enter", true), false),
            vec![0x1b, 0x0D]
        );
        assert_eq!(
            keystroke_to_bytes(&ks("return", true), false),
            vec![0x1b, 0x0D]
        );
        // Enter / Return PUROS = submit (inalterado).
        assert_eq!(keystroke_to_bytes(&ks("enter", false), false), vec![0x0D]);
        assert_eq!(keystroke_to_bytes(&ks("return", false), false), vec![0x0D]);
        // Não-vacuoso: o ramo do shift é DISTINTO do submit puro.
        assert_ne!(keystroke_to_bytes(&ks("enter", true), false), vec![0x0D]);
    }

    /// **Cmd+Delete (mac) / Ctrl+Backspace (win) → LIMPAR a linha (`Ctrl+U` = `0x15`).** No Mac a
    /// tecla "delete" é o `"backspace"` do gpui; cobrimos `"backspace"|"delete"` sob `platform` e
    /// `"backspace"` sob `control`. Backspace/Delete PUROS inalterados. (Esvaziar na tela = PENDENTE
    /// TELA; o atalho de fechar terminal migrou p/ ⌘W para liberar esta tecla.)
    #[test]
    fn cmd_delete_and_ctrl_backspace_clear_line() {
        let cmd = Modifiers {
            platform: true,
            ..Default::default()
        };
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(
            keystroke_to_bytes(&ks_mod("backspace", cmd), false),
            vec![0x15]
        );
        assert_eq!(
            keystroke_to_bytes(&ks_mod("delete", cmd), false),
            vec![0x15]
        );
        assert_eq!(
            keystroke_to_bytes(&ks_mod("backspace", ctrl), false),
            vec![0x15]
        );
        // PUROS inalterados: backspace = 0x7f; delete = CSI 3~.
        assert_eq!(
            keystroke_to_bytes(&ks("backspace", false), false),
            vec![0x7f]
        );
        assert_eq!(
            keystroke_to_bytes(&ks("delete", false), false),
            vec![0x1b, b'[', b'3', b'~']
        );
        // Não-vacuoso: limpar != backspace puro.
        assert_ne!(
            keystroke_to_bytes(&ks_mod("backspace", cmd), false),
            vec![0x7f]
        );
    }
}

#[cfg(test)]
mod honest_state_tests {
    use super::*;

    /// Jargão de estado que NUNCA pode aparecer na superfície (identificadores crus do enum +
    /// vocabulário técnico do core). Extensão do anti-vazamento de `goal_card.rs` aos estados
    /// do card do canvas (#22) e aos textos dos estados novos (#21/#22).
    const STATE_JARGON: &[&str] = &[
        "busy",
        "idle",
        "blocked",
        "dead",
        "crashed",
        "starting",
        "running",
        "stalled",
        "deliverystalled",
        "attentionkind",
        "circuit",
        "breaker",
        "uuid",
        "debug",
    ];

    fn assert_no_jargon(text: &str, who: &str) {
        let low = text.to_lowercase();
        for j in STATE_JARGON {
            assert!(!low.contains(j), "{who} vazou jargão {j:?}: {text:?}");
        }
    }

    /// O cabeçalho do card fala o estado em pt-br do leigo — nunca o `Debug` cru do enum
    /// (`Busy`/`Idle`/`Dead`). É a MESMA palavra do leitor de tela (a a11y já usa
    /// `aggregate_badge(...).label()`): visual e a11y deixam de divergir.
    #[test]
    fn card_status_label_speaks_layman_without_jargon() {
        for status in [
            NodeStatus::Starting,
            NodeStatus::Idle,
            NodeStatus::Busy,
            NodeStatus::Running,
            // #21: o estado novo também fala pt-br sem jargão ("pausado por segurança…", nunca
            // "blocked"/"circuit"/"breaker" — proibidos no STATE_JARGON).
            NodeStatus::Blocked,
            NodeStatus::Crashed,
            NodeStatus::Dead,
        ] {
            let label = node_status_label(status);
            assert!(
                !label.trim().is_empty(),
                "todo estado tem palavra: {status:?}"
            );
            assert_no_jargon(&label, "status do card");
        }
        // Não-vacuoso: estados distintos produzem palavras distintas (não é constante).
        assert_ne!(
            node_status_label(NodeStatus::Busy),
            node_status_label(NodeStatus::Dead),
            "trabalhando ≠ encerrado"
        );
    }

    /// O tipo do nó também é pt-br — antes `WebBrowser`/`TaskPanel` saíam crus.
    #[test]
    fn card_kind_label_is_layman() {
        for kind in [
            NodeKind::Terminal,
            NodeKind::Note,
            NodeKind::WebBrowser,
            NodeKind::Folder,
            NodeKind::Attachment,
            NodeKind::Trigger,
            NodeKind::TaskPanel,
            NodeKind::Feed,
        ] {
            let label = node_kind_label(kind);
            assert!(!label.trim().is_empty(), "todo tipo tem nome: {kind:?}");
            // Os identificadores compostos em inglês são o sintoma do Debug cru.
            for raw in [
                "webbrowser",
                "taskpanel",
                "attachment",
                "trigger",
                "note",
                "folder",
            ] {
                assert!(
                    !label.to_lowercase().contains(raw),
                    "tipo vazou identificador cru {raw:?}: {label:?}"
                );
            }
        }
    }

    /// Os textos dos estados NOVOS (#22 "recebeu, não começou" · #21 disjuntor) são humanos,
    /// sem jargão, e carregam o caminho de recuperação onde se aplica.
    #[test]
    fn new_state_copies_are_human_and_actionable() {
        assert_no_jargon(DELIVERY_STALLED_BADGE, "badge recebeu-não-começou");
        assert_no_jargon(CIRCUIT_BREAKER_LABEL, "estado de disjuntor");
        // #22: a frase admite o recebimento E o não-início (é a diferença de um Ocioso comum).
        assert!(
            DELIVERY_STALLED_BADGE.contains("recebeu")
                && DELIVERY_STALLED_BADGE.contains("começou"),
            "o badge precisa dizer recebeu-mas-não-começou: {DELIVERY_STALLED_BADGE:?}"
        );
        // #21: estado humano ("segurança") + caminho de saída ("liberar") no MESMO texto.
        assert!(
            CIRCUIT_BREAKER_LABEL.contains("segurança")
                && CIRCUIT_BREAKER_LABEL.contains("liberar"),
            "o disjuntor precisa do estado + a saída: {CIRCUIT_BREAKER_LABEL:?}"
        );
        assert_ne!(
            DELIVERY_STALLED_BADGE, CIRCUIT_BREAKER_LABEL,
            "estados distintos, textos distintos"
        );
    }
}
