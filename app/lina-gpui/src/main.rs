//! `lina-gpui` — o **walking skeleton com render de terminal de verdade** (Onda 2). Canvas
//! gpui com 2 terminais reais do core; cada card pinta o **snapshot do grid VISÍVEL**
//! (`VtBackend::screen()`) a cada frame — cols×rows com **cores por célula + cursor** —
//! então TUIs ricas (Claude Code, vim) aparecem corretas. **Scroll** (scrollback),
//! **teclas especiais** (setas/Home/End/PageUp·Down/F-keys → CSI), **resize** (PTY+grid),
//! e o diferencial **A2A com pulso "sem fios"**. Tradução core↔UI em [`bridge`] (gpui-free).

mod bridge;
// W4-2: modelo de cena do canvas T4 (zonas foco/periferia/suspenso + badge) — gpui-free, testável.
mod canvas;
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
// W4-5: galeria de Focos (T3) — presets que montam o Espaço com time + papéis; gpui-free, testável.
mod gallery;
// W4-6: acessibilidade (AccessibleBuffer sem ANSI, live-region "resposta pronta", WCAG, reduce-motion).
mod a11y;
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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gpui::{
    div, point, prelude::*, px, rgb, size, text, App, Bounds, ClickEvent, ClipboardItem, Context,
    Entity, FocusHandle, FontWeight, InputHandler, KeyDownEvent, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point, Render, Rgba,
    Role, ScrollDelta, ScrollWheelEvent, TitlebarOptions, UTF16Selection, Window, WindowBounds,
    WindowOptions,
};
use gpui_platform::application;

use lina_core::{
    build_paste, DomainEvent, EventStore, NodeStatus as CoreStatus, PtyManager, Supervisor, VtRgb,
    VtScreen,
};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use lina_bootstrap::Autonomy;

use bridge::{
    card_visible, cell_in_selection, encode_pointer, hit_test, load_injection_profile, lock,
    normalize_sel, screen_to_cell, scrub_pty_secret_env, shell_cmd, spawn_pump, wire_terminal,
    A2aTrigger, BootstrapWriter, BrokerPump, Camera, CmdFactory, CoreInput, CustodyDesk, Desk,
    GpuiBridgeHost, Grid, MailboxPump, Model, NodeManager, NodeView, PtrAction, SharedModel,
    CARD_H, CARD_W, CELL_H, CELL_W,
};
use lina_core::Mailbox;
// W3-6c (ADR 0004): cofre de segredos (demo: backend em memória `MockStore`).
use lina_secrets::{MockStore, SecretVault};

/// Tamanho da fonte do grid (Menlo). A célula (`CELL_W`/`CELL_H`) e o layout vivem no `bridge`.
const FONT_PX: f32 = 13.0;

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
    let cols = (((CARD_W - 2.0 * pad) / CELL_W).floor() as u16).max(80);
    let rows = (((CARD_H - title - 2.0 * pad) / CELL_H).floor() as u16).max(24);
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
fn render_line(
    cells: &[lina_core::VtCell],
    cursor_col: Option<usize>,
    row: usize,
    sel: Option<(usize, usize, usize, usize)>,
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
) -> impl IntoElement {
    let cur = screen.cursor;
    div()
        .flex()
        .flex_col()
        .font_family("Menlo")
        .text_size(px(FONT_PX * scale))
        // BUG A: line-height EXPLÍCITA = CELL_H. A natural do Menlo 13px (~18-19px) é MAIOR que
        // CELL_H=17, então as 26 rows (`fit_dims`) ocupavam mais que o body do card e a ÚLTIMA linha (a
        // barra de input "❯ …" da TUI) era CLIPADA pelo `overflow_hidden`. Travando em CELL_H, as 26
        // rows cabem exatamente (== o que `fit_dims` calcula) → o input APARECE.
        .line_height(px(CELL_H * scale))
        .children((0..screen.rows).map(move |r| {
            let cursor_col = (cur.visible && cur.line == r).then_some(cur.col);
            render_line(screen.row(r), cursor_col, r, sel)
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

/// Percentil (nearest-rank) de um slice **já ordenado** de ms. `p` em `[0.0, 100.0]`; slice vazio →
/// `0.0`. Barato e sem alocar: índice = `round(p/100 · (n-1))`, saturado em `[0, n-1]` (sem panic).
fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// O canvas gpui — a ÚNICA parte que conhece o toolkit. O estado dos nós (add/remove, model,
/// grids) vive no [`NodeManager`] gpui-free; a view só renderiza, roteia input e foca.
struct WorkspaceView {
    nodes: NodeManager,
    input: Arc<dyn InputSink>,
    // `None` em produção (sem A/B semeados) → o botão ⚡ A2A nem renderiza. `Some` no demo (A→B).
    a2a: Option<Arc<A2aTrigger>>,
    focused: NodeId,
    focus: FocusHandle,
    /// Câmera 2D do canvas (pan + zoom). Transform world↔screen, culling e hit-test usam ela.
    camera: Camera,
    /// Arrasto do FUNDO (pan): `(mouse_inicial_em_tela, pan_inicial)` em px de tela.
    drag: Option<((f32, f32), (f32, f32))>,
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
    dash: dashboard::DashWiring,
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
}

impl WorkspaceView {
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)] // construtor único do root; os fios entram aqui.
    fn new(
        nodes: NodeManager,
        input: Arc<dyn InputSink>,
        a2a: Option<Arc<A2aTrigger>>,
        focused: NodeId,
        desk: Desk,
        brake: wiring::Brake,
        dash: dashboard::DashWiring,
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
        Self {
            nodes,
            input,
            a2a,
            focused,
            focus,
            camera: Camera::default(),
            drag: None,
            z_order: BTreeMap::new(),
            z_next: 0,
            sel: None,
            dragging_sel: false,
            report_node: None,
            desk,
            agent_modal: None,
            modal_scan: None,
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
        }
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
        }
    }

    /// F1-1-5 (wiring): abre/fecha o painel "Atividade e custos" (P6/fluxo c). Enquanto
    /// aberto, um loop de tick (~500ms) marca dirty — timeline/sessões mudam em threads
    /// gpui-free e NÃO acordam o event loop (lição: animação precisa de dirty).
    fn toggle_dashboard(&mut self, cx: &mut Context<Self>) {
        self.dashboard_open = !self.dashboard_open;
        cx.notify(); // o heartbeat do painel (new) cuida do refresh enquanto aberto
    }

    /// F1-1-5 (P6/fluxo c — wiring): o painel "Atividade e custos". Hierarquia do
    /// wireframe: 1) estado (palavra+cor) → 2) custo (dinheiro, `~estimado` obrigatório)
    /// → 3) atividade. Tokens 100% do `theme` (zero cor hardcoded); `ElementId` único
    /// por linha (lição AccessKit); sonda `[DASH]` loga a latência evento→card 1× por
    /// evento — o Maestro valida por DADOS, o fundador pelo olho.
    fn render_dashboard(&mut self, th: &theme::Theme) -> impl IntoElement {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let today = dashboard::utc_day_prefix(now_ms);
        let sessions = lock(&self.dash.sessions).clone();
        let total = dashboard::workspace_cost_today(&sessions, &self.dash.ws_root, &today);
        let engine = dashboard::EngineBadge::from_cli(Some(&self.dash.cli_id));

        // Linhas por terminal: nome + estado canônico + atividade da timeline.
        struct Row {
            name: String,
            state: dashboard::UiState,
            activity: String,
        }
        let mut rows: Vec<Row> = Vec::new();
        {
            let tl = lock(&self.dash.timeline);
            let model = lock(&self.nodes.model);
            for view in model.nodes.values() {
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
                    NodeStatus::Starting | NodeStatus::Running => "Running",
                    _ => "Running", // variantes futuras: vivo sem distinção (fallback honesto)
                };
                let state = dashboard::UiState::from_projection(
                    Some(status_str),
                    view.status == NodeStatus::Crashed,
                );
                let activity = match tl.activity(&view.name, now_ms) {
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
                    name: view.name.clone(),
                    state,
                    activity,
                });
            }
        }

        let state_color = |s: dashboard::UiState| -> u32 {
            match s.color_token() {
                "verde" => th.state.success,
                "ambar" => th.state.warning,
                "vermelho-suave" => th.state.danger,
                "azul-escuro" => th.accent.primary,
                _ => th.text.muted, // cinza / cinza-escuro
            }
        };

        let mut panel = div()
            .id("dash-panel")
            .absolute()
            .top(px(44.))
            .right_0()
            .bottom(px(28.))
            .w(px(340.))
            .bg(rgb(th.surface.panel))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(th.text.primary))
                    .child("Atividade e custos do time"),
            )
            .child(
                // Custo do Espaço HOJE — dinheiro primeiro, com o marcador honesto.
                div()
                    .id("dash-total")
                    .text_size(px(15.))
                    .text_color(rgb(th.text.primary))
                    .child(format!("Hoje: {}", total.text)),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(th.text.muted))
                    .child(format!(
                        "motor: {} · {}",
                        engine.label,
                        dashboard::COST_TOOLTIP
                    )),
            );
        for (i, row) in rows.into_iter().enumerate() {
            let color = state_color(row.state);
            panel = panel.child(
                div()
                    .id(("dash-row", i as u64))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .bg(rgb(th.surface.card))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(th.text.primary))
                            .child(format!("{}  ● {}", row.name, row.state.label())),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(color))
                            .child(row.activity),
                    ),
            );
        }
        panel
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
        let modal = agent_modal::AgentModal::new_create(
            Arc::new(role_suggester::W31Suggester),
            self.live_names(),
        );
        self.open_agent_modal(modal, cx);
    }

    /// Abre o M6-E em modo EDITAR (pré-preenchido com nome + papel correntes do nó).
    fn open_agent_modal_edit(&mut self, node: NodeId, cx: &mut Context<Self>) {
        let Some(name) = lock(&self.nodes.model)
            .nodes
            .get(&node)
            .map(|v| v.name.clone())
        else {
            return; // nó sumiu entre o build da paleta e o clique — nada a editar.
        };
        let role = self.nodes.node_role(node);
        let modal = agent_modal::AgentModal::new_edit(
            Arc::new(role_suggester::W31Suggester),
            node,
            &name,
            role.as_deref(),
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
                .create_agent_with(
                    &p.name,
                    Some(&p.engine),
                    p.cwd.as_deref(),
                    p.role.as_deref(),
                )
                .map(Some),
            Plan::Save(p) => self
                .nodes
                .rename_node(p.node, &p.name)
                .and_then(|()| match &p.role {
                    Some(role) => self.nodes.assign_role(p.node, role),
                    None => Ok(()),
                })
                .map(|()| None),
        };
        match result {
            Ok(created) => {
                if let Plan::Create(p) = &plan {
                    if let Some(note) = &p.dup_note {
                        eprintln!("lina-gpui: M6 — {note}");
                    }
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
                    let leiga = if e.starts_with("não consigo trabalhar nessa pasta") {
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
    fn extend_selection(&mut self, screen_pt: (f32, f32)) {
        let Some((node, anchor, head)) = self.sel.as_ref().map(|s| (s.node, s.anchor, s.head))
        else {
            return;
        };
        let Some(cell) = self.screen_cell(node, screen_pt) else {
            return;
        };
        if cell == head {
            return;
        }
        if let Some(sel) = self.sel.as_mut() {
            sel.head = cell;
        }
        if let Some(g) = self.grid_of(node) {
            lock(&g).set_selection(anchor.0, anchor.1, cell.0, cell.1);
        }
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

    /// Mouse-down sobre o grid de um nó: reporta ao TUI (se pediu mouse) OU inicia seleção.
    fn pointer_down(&mut self, node: NodeId, ev: &MouseDownEvent) {
        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
        let Some(cell) = self.screen_cell(node, pos) else {
            return;
        };
        let mods = (ev.modifiers.shift, ev.modifiers.alt, ev.modifiers.control);
        self.focus(node);
        // Toda nova interação no grid zera a seleção anterior (de qualquer card).
        self.clear_selection_state();
        // `send_pointer` é atômico: se o TUI pediu mouse, reporta o press e devolve `true` (NÃO
        // seleciona); senão, inicia a seleção neste card.
        if self.send_pointer(node, PtrAction::Press, cell, mods) {
            self.report_node = Some(node);
        } else {
            self.sel = Some(Sel {
                node,
                anchor: cell,
                head: cell,
            });
            self.dragging_sel = true;
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

    /// Fecha o nó focado, reposiciona o foco num nó vivo (nunca tela em branco) e o revela.
    fn close_focused(&mut self, window: &Window) {
        let node = self.focused;
        if let Err(e) = self.nodes.remove_node(node) {
            eprintln!("lina-gpui: fechar nó focado: {e}");
            return;
        }
        self.z_order.remove(&node);
        // `first` num let SEPARADO: solta o lock do model ANTES do reveal (que re-locka o
        // model) — em edition 2021 o guard temporário de um `if let` viveria por todo o bloco,
        // causando re-lock na mesma thread (deadlock).
        let first = lock(&self.nodes.model).order.first().copied();
        if let Some(first) = first {
            self.focus(first);
            self.reveal(first, window);
        }
    }

    /// ⌘R: renomeia o terminal focado para o PRÓXIMO nome do ciclo (nomes que o role-discovery
    /// resolve em papéis distintos). Persiste `NodeRenamed` e reescreve os `CLAUDE.md`.
    fn rename_focused_cycle(&mut self) {
        const NAMES: [&str; 5] = ["@Arquiteto", "@Dev Backend", "@QA", "@Frontend", "@Maestro"];
        let cur = lock(&self.nodes.model)
            .nodes
            .get(&self.focused)
            .map(|v| v.name.clone());
        let Some(cur) = cur else { return };
        let idx = NAMES
            .iter()
            .position(|n| *n == cur)
            .map_or(0, |i| (i + 1) % NAMES.len());
        if let Err(e) = self.nodes.rename_node(self.focused, NAMES[idx]) {
            eprintln!("lina-gpui: renomear: {e}");
        }
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
                self.run_palette_action(action, window, cx);
            }
            // Modal: consome a tecla de filtro p/ o IME não vazar a query ao PTY (ver naming acima).
            cx.stop_propagation();
            cx.notify();
            return;
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
        // ⌘K abre a paleta (rebuild dos comandos com o roster do momento).
        if ks.modifiers.platform && ks.key == "k" {
            let cmds = self.palette_commands();
            self.palette.open(cmds);
            cx.notify();
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
                    None => eprintln!("lina-gpui: ⌘⏎ sem pedido pendente (nada a confirmar)"),
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
        // ⌘R renomeia o terminal focado (W3-2): cicla nomes role-suggestivos → o papel é
        // re-resolvido (W3-1) e o `CLAUDE.md` de TODOS é reescrito, SEM reiniciar o app.
        if ks.modifiers.platform && ks.key == "r" {
            self.rename_focused_cycle();
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
        if ks.modifiers.platform && ks.key == "backspace" {
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
                cx.listener(|view, ev: &MouseDownEvent, window, _cx| {
                    // PAN só ao arrastar o FUNDO (não sobre um card VISÍVEL → sem zona morta).
                    let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                    let v = window.viewport_size();
                    let vp = (f32::from(v.width), f32::from(v.height));
                    if hit_test(&view.camera, pos, &view.cards_z_asc(), (CARD_W, CARD_H), vp)
                        .is_none()
                    {
                        view.drag = Some((pos, view.camera.pan));
                    }
                }),
            )
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _w, _cx| {
                let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                if let Some((mouse0, pan0)) = view.drag {
                    if ev.dragging() {
                        view.camera.pan = (pan0.0 + pos.0 - mouse0.0, pan0.1 + pos.1 - mouse0.1);
                    }
                } else if view.dragging_sel {
                    // Estende a seleção até a célula sob o cursor (mesmo fora do card).
                    view.extend_selection(pos);
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
                cx.listener(|view, ev: &MouseUpEvent, _w, _cx| {
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
                    ScrollDelta::Pixels(p) => p.y / px(CELL_H),
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
        for (idx, (id, nv)) in cards.iter().enumerate() {
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
            // Foco = token `focus.ring` (≥3:1 nos 2 temas — gate F1-2-1; F1-2-6 reusa no teclado).
            let card_border = if node_id == focused {
                rgb(th.focus.ring)
            } else {
                rgb(th.surface.border)
            };
            let status_dot = match nv.status {
                NodeStatus::Idle => rgb(th.state.success),
                NodeStatus::Busy | NodeStatus::Running => rgb(th.state.warning),
                NodeStatus::Crashed | NodeStatus::Dead => rgb(th.state.danger),
                _ => rgb(th.accent.primary),
            };

            let mut title = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(th.surface.panel))
                .text_size(px(13.0 * z))
                .text_color(rgb(th.accent.primary))
                .child(div().size(px(9.0 * z)).rounded_full().bg(status_dot))
                .child(text!(nv.name.clone()))
                .child(
                    div()
                        .text_color(rgb(th.text.muted))
                        .child(text!(format!("· {:?} · {:?}", nv.kind, nv.status))),
                );
            // O ✕ só aparece quando há >1 card: o ÚLTIMO nó nunca pode ser fechado (o canvas
            // jamais fica em branco), então não mostramos um botão que recusaria a ação.
            if cards.len() > 1 {
                title = title.child(div().flex_1()).child(
                    div()
                        .id(("close", card_eid))
                        .px_2()
                        .rounded_md()
                        .bg(rgb(th.surface.danger_muted))
                        .text_color(rgb(th.state.danger))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _ev: &ClickEvent, window, _cx| {
                            if let Err(e) = view.nodes.remove_node(node_id) {
                                eprintln!("lina-gpui: fechar terminal: {e}");
                                return;
                            }
                            view.z_order.remove(&node_id);
                            // Reposiciona o foco num nó vivo (nunca tela em branco) e o revela.
                            // `first` em let separado: solta o lock do model antes do reveal.
                            if view.focused == node_id {
                                let first = lock(&view.nodes.model).order.first().copied();
                                if let Some(first) = first {
                                    view.focus(first);
                                    view.reveal(first, window);
                                }
                            }
                        }))
                        .child(text!("✕")),
                );
            }

            // Range de seleção (normalizado) para ESTE card, se a seleção pertence a ele.
            let card_sel = self
                .sel
                .filter(|s| s.node == node_id)
                .map(|s| normalize_sel(s.anchor, s.head));
            // W4-6 a11y: "precisa de você" anunciável — há um gate de custódia pendente p/ este nó?
            let needs_human = lock(&self.desk)
                .queue
                .iter()
                .any(|p| p.requester() == nv.name);
            // O conteúdo do terminal: o SNAPSHOT do grid lido AGORA (não deltas).
            let mut body = div()
                .id(("grid", card_eid))
                .role(Role::Terminal)
                // W4-6 a11y: nome + estado ANUNCIÁVEL (badge do W4-2: "aguardando"/"precisa de você"/…).
                .aria_label(a11y::node_label(&nv.name, nv.status, needs_human))
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
                    cx.listener(move |view, ev: &MouseDownEvent, _w, _cx| {
                        // Sobre o grid: mouse reporting (se o TUI pediu) OU inicia seleção.
                        view.pointer_down(node_id, ev);
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
                        .child(div().text_size(px(40.0 * z)).child(text!(icon)))
                        .child(
                            div()
                                .text_color(rgb(th.text.primary))
                                .text_size(px(15.0 * z))
                                .child(text!(nv.name.clone())),
                        )
                        .child(
                            div()
                                .text_color(rgb(th.text.muted))
                                .text_size(px(11.0 * z))
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
                            body = body.child(render_grid(&screen, z, card_sel));
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
                .child(title)
                .child(body);

            root = root.child(card);
        }

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
                let p50 = percentile_sorted(&sorted, 50.0);
                let p95 = percentile_sorted(&sorted, 95.0);
                let p99 = percentile_sorted(&sorted, 99.0);
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
                .on_click(cx.listener(|view, _ev: &ClickEvent, window, _cx| {
                    // Cria um shell real novo, idêntico aos demais, foca e o TRAZ p/ a viewport.
                    match view.nodes.add_node() {
                        Ok(node) => {
                            view.focus(node);
                            view.reveal(node, window);
                        }
                        Err(e) => eprintln!("lina-gpui: add_node: {e}"),
                    }
                }))
                .child(text!("➕ Terminal")),
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
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                    view.camera.reset();
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
        // F1-1-5 (P6/fluxo c): painel "Atividade e custos" — zona lateral direita, sob os modais.
        let root = if self.dashboard_open {
            let panel = self.render_dashboard(&th);
            root.child(panel)
        } else {
            root
        };
        // F1-2-2 · M6/M6-E: o modal de Agente é o overlay central (evoluiu o overlay do M2 — BUG 4
        // resolvido por um modal de verdade). A paleta, quando aberta, continua mais ao topo.
        let root = match &self.agent_modal {
            Some(m) => root.child(agent_modal::render(m, cx)),
            None => root,
        };
        // W4-2 · M1: a PALETA, quando aberta, é o overlay mais ao TOPO (modal sobre o canvas/chrome).
        if self.palette.is_open() {
            root.child(self.palette.render())
        } else {
            root
        }
    }
}

/// PRODUÇÃO vs DEMO. `LINA_DEMO` (1/true/on/yes) liga o modo demo do fundador: semeia Terminal A/B,
/// arma o botão ⚡ A2A A→B e grava o workspace em `temp`. Sem a env → PRODUÇÃO (default do aluno).
fn lina_demo_enabled() -> bool {
    matches!(
        std::env::var("LINA_DEMO").ok().as_deref().map(str::trim),
        Some("1") | Some("true") | Some("on") | Some("yes")
    )
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
/// passo (⌘T). Some sozinho assim que o 1º card existe.
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
        .child(div().text_size(px(44.0)).child(text!("✨")))
        .child(
            div()
                .text_size(px(20.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(th.text.primary))
                .child(text!("Seu Espaço está pronto")),
        )
        .child(
            div()
                .text_size(px(15.0))
                .text_color(rgb(th.text.secondary))
                .child(text!("Pressione ⌘T para criar seu primeiro agente")),
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
    if path_looks_hydrated(&current) {
        return; // já rico (terminal ou já hidratado) → não mexe.
    }
    if let Some(resolved) = resolve_login_shell_path() {
        if resolved != current {
            // edition 2021: `set_var` é seguro (e rodamos antes de qualquer thread/janela subir).
            std::env::set_var("PATH", resolved.as_str());
            eprintln!(
                "lina-gpui: PATH hidratado do shell de login ({} entradas) — CLIs/npm agora visíveis",
                resolved.split(':').count()
            );
        }
    }
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
    let (cols, rows) = fit_dims();
    // BUG A (instrumentação de boot): o PTY é spawnado com EXATAMENTE estas `rows`, e o render trava a
    // line-height em CELL_H → as `rows` cabem inteiras no card (a ÚLTIMA = barra de input da TUI). O
    // Maestro confere: pty_rows (nos logs por-card) == estas `rows`.
    eprintln!(
        "lina-gpui: layout · card {CARD_W}x{CARD_H} · cell {CELL_W}x{CELL_H} · font {FONT_PX}px (line-height travada={CELL_H}) · PTY {cols}x{rows} → {rows} linhas cabem (a ultima = input)"
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
    let mailbox_dir = ws_root.join(".lina");
    let dir = mailbox_dir.join("events");
    // F1-2-1: aplica o tema PERSISTIDO (T7 `settings.json`, ao lado do event log) ANTES de abrir
    // qualquer janela — a escolha dark/light + acento sobrevive ao restart, 100% local (inv #2).
    {
        let s = persistence_ui::load_settings(&dir);
        theme::apply(s.theme_mode(), &s.accent);
    }
    // F1-0-2 critério 5: o perfil de injeção REAL (claude-code.toml — ready_timeout/busy_markers
    // calibrados) substitui o demo hardcoded; fallback-com-warning no loader (nunca panic). O
    // caminho carregado + os campos do fix saem AQUI no log de boot, p/ inspeção.
    let injection_profile = load_injection_profile(Some(&mailbox_dir));
    // Estado PERSISTENTE do onboarding (progresso + log próprio), co-locado no workspace (subdir, não
    // colide com `.lina/events` do canvas). Sobrevive ao reboot em produção → o onboarding só aparece
    // na 1ª execução de verdade.
    let onboarding_dir = ws_root.join("onboarding");
    eprintln!(
        "lina-gpui: workspace em {} ({})",
        ws_root.display(),
        if demo {
            "DEMO/temp"
        } else {
            "produção/persistente"
        }
    );
    // inv#6 (app NUNCA quebrar): um `.db` corrompido NÃO panica o boot. Tenta o store persistente; se
    // falhar, loga ALTO sem destruir o arquivo (recuperação in-UI é W0-6, fora de escopo) e cai num
    // store temporário só nesta sessão — garante que o app abre. Instalação nova abre limpo, sem cair
    // aqui. (`.expect` antigo virava backtrace na cara do aluno.)
    let store = Arc::new(Mutex::new(match EventStore::open(&dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "lina-gpui: não abri seus dados em {} ({e}). Eles estão preservados; abrindo um \
                 espaço temporário só nesta sessão.",
                dir.display()
            );
            let tmp = std::env::temp_dir()
                .join("lina-space-fallback")
                .join(".lina")
                .join("events");
            match EventStore::open(&tmp) {
                Ok(s) => s,
                Err(e2) => {
                    eprintln!(
                        "lina-gpui: o store de fallback também falhou em {} ({e2}) — encerrando.",
                        tmp.display()
                    );
                    std::process::exit(1);
                }
            }
        }
    }));

    // F1-0-8 (costura coordenada — mecanismo do Dev 02): fecha a GERAÇÃO ANTERIOR no log logo
    // após abrir o store e ANTES de registrar os nós da sessão nova — sem esta linha o mecanismo
    // existe mas não roda no caminho real (mesmo padrão da fiação W5-2). Nós da sessão passada
    // sem morte registrada viram `dead` póstumo (replay = roster vivo, 0 fantasmas — gate F1-0
    // (e)). Best-effort: falha loga ALTO e o boot segue (inv#6 — o app nunca morre no boot).
    match lina_core::lifecycle::close_previous_generation(&mut lock(&store)) {
        Ok(ghosts) => eprintln!(
            "lina-gpui: F1-0-8 — geração anterior fechada no boot: {} fantasma(s)",
            ghosts.len()
        ),
        Err(e) => eprintln!(
            "lina-gpui: F1-0-8 — não fechei a geração anterior ({e}); seguindo (replay pode mostrar fantasmas)"
        ),
    }

    let pty = Arc::new(Mutex::new(PtyManager::new()));
    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    // Fábrica de nós: todo terminal é um SHELL INTERATIVO REAL e COMPLETO — idêntico em
    // capacidade (o B aceita teclado e roda claude/vim igual ao A). Add/remove em runtime
    // reusa EXATAMENTE esta fábrica via NodeManager.
    let cmd_factory: CmdFactory = Arc::new(shell_cmd);

    // W3-2 · BOOTSTRAP turno-0: o app escreve `<cwd>/CLAUDE.md` (8 blocos) por terminal e o
    // reescreve a cada mudança de roster. `ws_root`/`mailbox_dir` já foram definidos acima (o event
    // store do app mora em `<ws_root>/.lina/events`, o MESMO log do bin); vault/autonomia/bin `lina`
    // configuráveis por env (LINA_VAULT/LINA_BIN). O bootstrap é best-effort (não trava o app).
    // W3-4: a mailbox compartilhada do workspace (`<ws_root>/.lina`). `LINA_HOME` aponta os
    // terminais (que herdam o env) para cá → `lina ask`/`handshake` depositam no MESMO outbox que
    // o supervisor (o `MailboxPump`) observa, e o bin `lina do/guard` apenda no MESMO event log.
    if let Err(e) = std::fs::create_dir_all(&mailbox_dir) {
        // Não fatal (o resto do app — canvas, terminais — funciona sem A2A), mas VISÍVEL: sem a
        // mailbox, `lina ask` não terá para onde escrever.
        eprintln!(
            "lina-gpui: não criou a mailbox {} (A2A indisponível): {e}",
            mailbox_dir.display()
        );
    }
    std::env::set_var("LINA_HOME", &mailbox_dir);
    // O segundo cérebro (onboarding) grava o vault escolhido em `.lina/vault.json`. Se houver um
    // `primary`, ele tem prioridade — reflete a escolha REAL do usuário e faz `{{vault_writable_paths}}`/
    // `{{vault_tino_path}}` apontarem pro vault linkado na próxima reescrita do bootstrap. Senão, cai no
    // override de dev `LINA_VAULT` e, por fim, no default `<ws_root>/vault`.
    let vault_path = obsidian::read_primary_vault(&mailbox_dir)
        .or_else(|| std::env::var("LINA_VAULT").ok())
        .unwrap_or_else(|| ws_root.join("vault").display().to_string());
    let lina_bin = std::env::var("LINA_BIN").unwrap_or_else(|_| "lina".to_string());
    let mut bootstrap =
        BootstrapWriter::new(ws_root.clone(), vault_path, Autonomy::Assisted, lina_bin)
            .map_err(|e| eprintln!("lina-gpui: bootstrap desativado: {e}"))
            .ok();

    // ── F1-1-3/F1-1-5 (wiring) · HOOKS + TIMELINE + SESSÕES — antes de qualquer write/spawn. ──
    // Listener 1x no boot; cada terminal ganha token no settings SE o perfil declarar a
    // capability (TOML — inv#3). Degradação limpa: sem capability/bind → app como antes.
    let dash =
        dashboard::DashWiring::new(injection_profile.id.clone(), ws_root.display().to_string());
    if injection_profile.capabilities.has("hooks") {
        if let Some(hooks) = bridge::HooksShared::start() {
            if let Some(bw) = bootstrap.as_mut() {
                bw.set_hooks(Arc::clone(&hooks));
            }
            let tl = Arc::clone(&dash.timeline);
            hooks.spawn_drain(move |ev| {
                // Sonda [DASH]: latência de INGESTÃO (chegada no listener → timeline).
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                eprintln!(
                    "lina-gpui: [DASH] hook node={} kind={:?} tool={:?} lat_ingest_ms={}",
                    ev.node_id,
                    ev.kind,
                    ev.tool_name,
                    now.saturating_sub(ev.ts)
                );
                if let Ok(mut t) = tl.lock() {
                    t.note(ev);
                }
            });
        }
    } else {
        eprintln!(
            "lina-gpui: [DASH] perfil '{}' sem capability hooks — settings sem hooks (como antes)",
            injection_profile.id
        );
    }
    // Sessões (camada 3 — custo~): poll do SessionWatch em thread própria (gpui-free);
    // o pattern vem do TOML do perfil (F1-1-1); HOME real do usuário (não é teste).
    if let Some(pattern) = injection_profile.session_dir_pattern.clone() {
        if let Some(home) = std::env::var_os("HOME") {
            let out = Arc::clone(&dash.sessions);
            let cli_id = injection_profile.id.clone();
            thread::spawn(move || {
                let mut watch = lina_session_watch::SessionWatch::with_home(PathBuf::from(home));
                watch.add_source(cli_id, pattern);
                let mut known: std::collections::BTreeSet<(String, String)> =
                    std::collections::BTreeSet::new();
                loop {
                    if let Ok(o) = watch.poll_once() {
                        if !o.sessions_updated.is_empty() {
                            known.extend(o.sessions_updated);
                            let snapshot: Vec<_> = known
                                .iter()
                                .filter_map(|(c, s)| watch.scanner().session(c, s))
                                .collect();
                            if let Ok(mut dst) = out.lock() {
                                *dst = snapshot;
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            });
        }
    }

    // ── W3-6c (ADR 0004) · COFRE DE SEGREDO + ENV LIMPO (fios 1 e 3) — ANTES de spawnar qualquer PTY.
    // Cofre do workspace. DEMO: backend em memória (`MockStore`) p/ não tocar o keyring do SO no
    // teatro do fundador; PRODUÇÃO trocaria por `SecretVault::new("lina-space/<ws>")` (KeyringStore).
    // Semente OPCIONAL via `LINA_DEPLOY_TOKEN`: com ela, a custódia EXECUTA (segredo do cofre); sem
    // ela, o cofre fica vazio e a custódia BLOQUEIA (DeniedNoSecret) — ambos provam a blindagem.
    let custody_vault = SecretVault::with_store("lina-space/walking-skeleton", MockStore::new());
    if let Ok(token) = std::env::var("LINA_DEPLOY_TOKEN") {
        match custody_vault.set("deploy", "prod", &token) {
            Ok(()) => {
                eprintln!("lina-gpui: cofre semeado (deploy/prod) a partir de LINA_DEPLOY_TOKEN")
            }
            Err(e) => eprintln!("lina-gpui: nao semeou o cofre (deploy/prod): {e}"),
        }
    }
    // Fio 3: os PTYs filhos herdam o env do app no spawn → remover as vars de segredo do PRÓPRIO env
    // AQUI (antes de qualquer `wire_terminal`) garante que nenhum terminal do agente nasça com o token.
    let scrubbed = scrub_pty_secret_env();
    if !scrubbed.is_empty() {
        eprintln!("lina-gpui: env limpo — vars de segredo removidas do env do app (PTYs nao as herdam): {scrubbed:?}");
    }
    // Mesa de custódia compartilhada: a UI confirma (⌘⏎); a BrokerPump executa COM o segredo do cofre.
    let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
    // W4-3: FREIO compartilhado — a UI pede pausa/retoma; a MailboxPump aplica no Router e espelha.
    let brake: wiring::Brake = wiring::new_brake();

    // ── SEED dos nós iniciais. PRODUÇÃO (default do aluno): NENHUM. O workspace nasce limpo e o aluno
    // cria o 1º agente com ⌘T (guiado pelo empty-state) — sem "Terminal A/B fantasma" poluindo o log.
    // DEMO (`LINA_DEMO=1`): semeia Terminal A + B (shells reais, idênticos) e arma o botão ⚡ A2A A→B.
    // Em QUALQUER caso, um spawn que falhe DEGRADA (loga e segue) — nunca panica (inv#6, antes `.expect`).
    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));

    #[allow(clippy::type_complexity)]
    let (focused, a2a, seq_start, keys): (
        NodeId,
        Option<Arc<A2aTrigger>>,
        u32,
        BTreeMap<NodeId, String>,
    ) = if demo {
        let _ = lock(&store).append(&DomainEvent::WorkspaceCreated {
            name: "walking-skeleton".into(),
            // W4-5: o walking-skeleton não passa pela galeria de Foco → preset não-setado (`""`).
            focus_preset: String::new(),
        });
        // Spawna+fia um terminal demo; em SUCESSO persiste NodeAdded+TerminalSpawned, projeta no model e
        // registra o grid. Devolve `(NodeId, Grid)` ou `None` (spawn falhou → degrada, sem panic).
        let seed_one = |key: &str, name: &str, x: f64, y: f64| -> Option<(NodeId, Grid)> {
            let mut cmd = (*cmd_factory)(name);
            if let Some(bw) = &bootstrap {
                let d = bw.dir_for(key);
                let _ = std::fs::create_dir_all(&d);
                cmd = cmd.cwd(d);
            }
            let wired = {
                let mut p = lock(&pty);
                wire_terminal(
                    &mut p, &sup, &delta_tx, key, name, "terminal", cmd, cols, rows,
                )
            };
            match wired {
                Ok((node, grid)) => {
                    let _ = sup.set_status(node, CoreStatus::Running);
                    {
                        let mut s = lock(&store);
                        let _ = s.append(&DomainEvent::NodeAdded {
                            node,
                            kind: "Terminal".into(),
                            x,
                            y,
                        });
                        let _ = s.append(&DomainEvent::TerminalSpawned {
                            node,
                            cli: name.into(),
                        });
                    }
                    lock(&model).seed_node(
                        node,
                        NodeView::new(name, NodeKind::Terminal, x as f32, y as f32),
                    );
                    lock(&grids).insert(node, Arc::clone(&grid));
                    Some((node, grid))
                }
                Err(e) => {
                    eprintln!("lina-gpui: DEMO — spawn de '{name}' falhou ({e}); seguindo sem ele");
                    None
                }
            }
        };
        let a = seed_one("A", "Terminal A", 30.0, 96.0);
        let b = seed_one("B", "Terminal B", 740.0, 96.0);
        let mut keys: BTreeMap<NodeId, String> = BTreeMap::new();
        if let Some((na, _)) = &a {
            keys.insert(*na, "A".to_string());
        }
        if let Some((nb, _)) = &b {
            keys.insert(*nb, "B".to_string());
        }
        let seq = keys.len() as u32;
        // O ⚡ A2A A→B só faz sentido com AMBOS vivos; senão fica `None` (o botão some via `ready()`).
        let a2a = match (&a, &b) {
            (Some((na, _)), Some((nb, grid_b))) => Some(Arc::new(A2aTrigger::new(
                Arc::clone(&sup),
                *na,
                *nb,
                Arc::clone(grid_b),
                // F1-0-2: o ⚡ demo usa o MESMO perfil real carregado no boot (sem demo hardcoded).
                injection_profile.clone(),
                Arc::clone(&store),
                Arc::clone(&model),
            ))),
            _ => None,
        };
        // O foco arranca no 1º terminal vivo (A, ou B se A falhou); o render reaponta sozinho depois.
        let focused = a
            .as_ref()
            .or(b.as_ref())
            .map(|(n, _)| *n)
            .unwrap_or_default();
        (focused, a2a, seq, keys)
    } else {
        // PRODUÇÃO: sem nós. `focused` é a sentinela nil (`NodeId::default()`); o render reaponta ao 1º
        // card assim que o aluno cria um com ⌘T. `a2a = None` → o botão ⚡ nem aparece (gate `ready()`).
        (NodeId::default(), None, 0u32, BTreeMap::new())
    };

    let event_count = lock(&store).event_count().unwrap_or(0);
    lock(&model).event_count = event_count;

    // W3-7c · TETO DE CUSTO REAL. `LINA_TOKEN_BUDGET_DAY` (tokens/dia ESTIMADOS ≈ bytes/4 de output;
    // ver cost.rs) ARMA o teto; 0/ausente = desligado (default, sem regressão). A bomba mede o output
    // dos PTYs e apenda TokenUsageReported no MESMO store (`<ws_root>/.lina/events`, o log
    // compartilhado com o bin `lina`) que o CostLedger soma.
    let token_budget_day: u64 = std::env::var("LINA_TOKEN_BUDGET_DAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let idle_ms = injection_profile.idle_ms.unwrap_or(200);
    let bridge = GpuiBridgeHost::new(Arc::clone(&model));
    // inv#6/no-panic: sem a ponte core→UI o app não renderiza terminais — encerra com mensagem clara
    // em vez de backtrace. (Falha só em condição catastrófica de sistema; instalação nova não cai aqui.)
    // FIX DE GATE F1-0: o pump precisa do Supervisor p/ devolver o nó a Idle no fim-de-resposta
    // (transição event-sourced — destrava a retenção F1-0-4 na 2ª mensagem ao mesmo alvo).
    let mut pump = match spawn_pump(
        bridge,
        delta_rx,
        bus_rx,
        Arc::clone(&store),
        idle_ms,
        Arc::clone(&sup),
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lina-gpui: não subi a ponte core→UI ({e}); o app não consegue renderizar — encerrando.");
            std::process::exit(1);
        }
    };

    let input: Arc<dyn InputSink> = Arc::new(CoreInput::new(Arc::clone(&sup)));

    // O dono dos nós em runtime (add/remove): reusa pty/sup/store/model/grids. `seq_start` é o nº de nós
    // semeados (2 no demo A/B; 0 em produção) → o 1º ⌘T do aluno em produção nasce como "Terminal A".
    let nodes = NodeManager::new(
        Arc::clone(&pty),
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&model),
        Arc::clone(&grids),
        keys,
        delta_tx,
        cols,
        rows,
        seq_start,
        cmd_factory,
        bootstrap,
        mailbox_dir.clone(), // W4-2 M3/M4: `.lina/` onde notas/pastas persistem
    );
    // W3-2: escreve o CLAUDE.md/estado inicial do roster semeado (vazio em produção — best-effort).
    nodes.rewrite_bootstrap();

    // W3-4: sobe o SUPERVISOR que observa a mailbox (`<ws_root>/.lina/outbox/`). A partir daqui,
    // um `lina ask @B "oi"` de qualquer terminal trafega: outbox → guardrails → deliver_a2a → B.
    // Thread DAEMON (observador de fundo): o app sai via `process::exit`, que encerra a thread —
    // o handle é mantido só para deixar o ciclo de vida explícito.
    let _mailbox_pump = MailboxPump::new(
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&grids),
        Mailbox::new(&mailbox_dir),
        Arc::clone(&model),
        Arc::clone(&brake), // W4-3: freio (pausa/retoma a auto-orquestração)
        token_budget_day,   // W3-7c: arma o teto de custo no Router (0 = desligado)
        injection_profile,  // F1-0-2: o claude-code.toml real (fallback-com-warning no loader)
    )
    .spawn();

    // W3-6c (ADR 0004): a BROKER PUMP observa a FILA DE BROKER (`<ws_root>/.lina/broker/`, irmã do
    // outbox A2A), aplica o gate humano (⌘⏎ na janela) e chama `run_custody` — que obtém o segredo do
    // cofre e executa. O agente NUNCA tem o token. Thread daemon (encerra com o `process::exit`).
    let _broker_pump = BrokerPump::new(
        Mailbox::new(mailbox_dir.join("broker")),
        Arc::clone(&store),
        custody_vault,
        Arc::clone(&desk),
        Arc::clone(&model),
        Arc::clone(&sup), // roster vivo: valida que a origem do pedido é um nó REAL (hole 3)
    )
    .spawn();

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
    application().run(move |cx: &mut App| {
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
                cx.new(|cx| {
                    WorkspaceView::new(nodes, input, a2a, focused, desk, brake, dash, window, cx)
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

    pump.stop();
    drop(sup);
    drop(pty);
}

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
