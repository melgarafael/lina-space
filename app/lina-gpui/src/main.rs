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

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, rgb, size, text, App, Bounds, ClickEvent, ClipboardItem, Context,
    FocusHandle, FontWeight, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Render, Rgba, Role, ScrollDelta, ScrollWheelEvent,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

use lina_core::{
    build_paste, DomainEvent, EventStore, NodeStatus as CoreStatus, PtyManager, Supervisor, VtRgb,
    VtScreen,
};
use lina_host::{InputSink, NodeId, NodeKind, NodeStatus, WriteOp};

use lina_bootstrap::Autonomy;

use bridge::{
    card_visible, cell_in_selection, demo_profile, encode_pointer, hit_test, lock, normalize_sel,
    screen_to_cell, scrub_pty_secret_env, shell_cmd, spawn_pump, wire_terminal, A2aTrigger,
    BootstrapWriter, BrokerPump, Camera, CmdFactory, CoreInput, CustodyDesk, Desk, GpuiBridgeHost,
    Grid, MailboxPump, Model, NodeManager, NodeView, PtrAction, SharedModel, CARD_H, CARD_W,
    CELL_H, CELL_W,
};
use lina_core::Mailbox;
// W3-6c (ADR 0004): cofre de segredos (demo: backend em memória `MockStore`).
use lina_secrets::{MockStore, SecretVault};

/// Tamanho da fonte do grid (Menlo). A célula (`CELL_W`/`CELL_H`) e o layout vivem no `bridge`.
const FONT_PX: f32 = 13.0;
/// Cor de fundo das células SELECIONADAS (W2-4) — azul fosco, mantém o texto legível.
const SELECTION_BG: VtRgb = VtRgb {
    r: 0x33,
    g: 0x45,
    b: 0x73,
};

/// Quantas cols×rows cabem na área de grid de um card (>= 80×24, "tamanho útil").
fn fit_dims() -> (u16, u16) {
    let pad = 8.0;
    let title = 34.0;
    let cols = (((CARD_W - 2.0 * pad) / CELL_W).floor() as u16).max(80);
    let rows = (((CARD_H - title - 2.0 * pad) / CELL_H).floor() as u16).max(24);
    (cols, rows)
}

/// Traduz uma `Keystroke` em bytes para o PTY. Imprimíveis via `key_char`; setas como
/// CSI (ou SS3 em `app_cursor`/DECCKM); Home/End/PageUp·Down/Insert/Delete/F1-F12; Ctrl+letra.
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
        "space" => return vec![b' '],
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
    if let Some(kc) = &ks.key_char {
        if !kc.is_empty() {
            return kc.clone().into_bytes();
        }
    }
    if key.chars().count() == 1 {
        return key.as_bytes().to_vec();
    }
    Vec::new()
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
    let mut runs: Vec<Run> = Vec::new();
    for (col, cell) in cells.iter().enumerate() {
        let selected =
            sel.is_some_and(|(sc, sr, ec, er)| cell_in_selection(col, row, sc, sr, ec, er));
        let (fg, bg, bold) = if cursor_col == Some(col) {
            // Bloco do cursor: bg accent, fg escuro.
            (
                VtRgb {
                    r: 0x0a,
                    g: 0x0e,
                    b: 0x27,
                },
                VtRgb {
                    r: 0x7a,
                    g: 0xa2,
                    b: 0xf7,
                },
                false,
            )
        } else if selected {
            // Célula SELECIONADA: bg de destaque, mantém fg/negrito (texto legível).
            (cell.fg, SELECTION_BG, cell.bold)
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

/// O canvas gpui — a ÚNICA parte que conhece o toolkit. O estado dos nós (add/remove, model,
/// grids) vive no [`NodeManager`] gpui-free; a view só renderiza, roteia input e foca.
struct WorkspaceView {
    nodes: NodeManager,
    input: Arc<dyn InputSink>,
    a2a: Arc<A2aTrigger>,
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
    /// W4-2 · M2 "Novo Agente": `Some(buffer)` enquanto o humano DIGITA o nome do agente novo (modo
    /// nomeação). Enter cria (papel derivado do nome), Esc cancela. `None` = fora do modo nomeação.
    naming: Option<String>,
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
    /// W4-2 · M3/M4: `Some((kind, buffer))` enquanto o humano DIGITA o título da nota/pasta a criar
    /// (disparado pela paleta). Enter cria + foca; Esc cancela. `None` = fora do modo criação.
    creating: Option<(creators::CreatorKind, String)>,
}

impl WorkspaceView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        nodes: NodeManager,
        input: Arc<dyn InputSink>,
        a2a: Arc<A2aTrigger>,
        focused: NodeId,
        desk: Desk,
        brake: wiring::Brake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
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
            naming: None,
            brake,
            // W4-6 gap3: este campo é o OVERRIDE do usuário (rodapé); o SO/env entra via
            // `a11y::reduce_motion_effective` (fonte única) no pulso/rótulo. Começa sem override.
            reduce_motion: false,
            a11y_live: a11y::LiveRegion::default(),
            palette: palette::PaletteState::default(),
            creating: None,
        }
    }

    /// W4-2 · M1: a lista de comandos da paleta (rebuild a cada Cmd-K — o roster muda). Comandos
    /// estáticos + "Focar: <nó>" por nó vivo.
    fn palette_commands(&self) -> Vec<palette::Command> {
        use palette::{Command, PaletteAction as A};
        let mut cmds = vec![
            Command::new("✦ Novo agente", A::NewAgent),
            Command::new("⏸ Pausar / retomar orquestração (freio)", A::ToggleBrake),
            Command::new("📝 Nova nota", A::NewNote),
            Command::new("📁 Nova pasta", A::NewFolder),
        ];
        for (id, nv) in self.nodes.cards() {
            cmds.push(Command::new(
                format!("🎯 Focar: {}", nv.name),
                A::FocusNode(id),
            ));
        }
        cmds
    }

    /// W4-2 · M1: executa a ação escolhida na paleta (toca o estado do canvas — `naming`/`focus`/`brake`
    /// ou abre o modo CRIAÇÃO de nota/pasta — M3/M4).
    fn run_palette_action(&mut self, action: palette::PaletteAction, window: &Window) {
        use palette::PaletteAction as A;
        match action {
            A::NewAgent => self.naming = Some(String::new()), // reusa o modo nomeação do M2
            A::FocusNode(node) => {
                self.focus(node);
                self.reveal(node, window);
            }
            A::ToggleBrake => lock(&self.brake).toggle_requested = true,
            // M3/M4: abre o modo CRIAÇÃO (digita o título → Enter cria + foca; ver `handle_key`).
            A::NewNote => self.creating = Some((creators::CreatorKind::Note, String::new())),
            A::NewFolder => self.creating = Some((creators::CreatorKind::Folder, String::new())),
        }
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
        // W4-2 · M2 — MODO NOMEAÇÃO: enquanto o humano digita o nome do agente novo, as teclas montam
        // o buffer (NÃO vão para o PTY). Enter cria (papel derivado do nome); Esc cancela; Backspace
        // apaga. Captura ANTES de tudo (é um modo modal leve).
        if self.naming.is_some() {
            match ks.key.as_str() {
                "escape" => {
                    self.naming = None;
                }
                "enter" | "return" => {
                    let name = self.naming.take().unwrap_or_default();
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        match self.nodes.create_agent(&name) {
                            Ok(node) => {
                                self.focus(node);
                                self.reveal(node, window);
                            }
                            Err(e) => eprintln!("lina-gpui: M2 criar agente '{name}': {e}"),
                        }
                    }
                }
                "backspace" => {
                    if let Some(buf) = self.naming.as_mut() {
                        buf.pop();
                    }
                }
                _ => {
                    // Caractere imprimível → entra no nome (ignora chords com ⌘/Ctrl e teclas-controle).
                    if !ks.modifiers.platform && !ks.modifiers.control {
                        if let Some(kc) = ks
                            .key_char
                            .as_ref()
                            .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                        {
                            if let Some(buf) = self.naming.as_mut() {
                                buf.push_str(kc);
                            }
                        }
                    }
                }
            }
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
                self.run_palette_action(action, window);
            }
            cx.notify();
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
        if ks.modifiers.platform && ks.key == "t" {
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

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

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
            rgb(0x7aa2f7)
        } else {
            rgb(0x222a44)
        };
        let mut root = div()
            .id("canvas")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(rgb(0x0a0e27))
            .border_2()
            .border_dashed()
            .border_color(aura_color)
            .text_color(rgb(0xc8d3f5))
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
                // ZOOM ao rolar no FUNDO; sobre um card VISÍVEL, o handler do card cuida.
                let cursor = (f32::from(ev.position.x), f32::from(ev.position.y));
                let v = window.viewport_size();
                let vp = (f32::from(v.width), f32::from(v.height));
                if hit_test(
                    &view.camera,
                    cursor,
                    &view.cards_z_asc(),
                    (CARD_W, CARD_H),
                    vp,
                )
                .is_none()
                {
                    view.camera.zoom_by(cursor, scroll_zoom_factor(ev.delta));
                }
            }))
            .on_key_down(cx.listener(|view, ev: &KeyDownEvent, window, cx| {
                view.handle_key(ev, window, cx);
            }));

        for (idx, (id, nv)) in cards.iter().enumerate() {
            let node_id = *id;
            // ID de elemento ESTÁVEL por nó (não o idx do loop, que muda com cull/z-order).
            let card_eid = eid.get(&node_id).copied().unwrap_or(idx);
            // W4-2: a ZONA do nó (foco/periferia/suspenso) decide COMO desenhar. Fonte do `in_viewport`
            // é o MESMO culling (W2-3). Suspenso (fora da viewport) → 0 draw-calls (continue).
            let in_viewport = card_visible(&cam, (nv.x, nv.y), (CARD_W, CARD_H), viewport);
            let zone = canvas::classify_zone(node_id == focused, in_viewport);
            if zone == canvas::Zone::Suspended {
                continue;
            }
            let (sx, sy) = cam.world_to_screen((nv.x, nv.y));
            let z = cam.zoom;
            let card_border = if node_id == focused {
                rgb(0x7aa2f7)
            } else {
                rgb(0x2a3152)
            };
            let status_dot = match nv.status {
                NodeStatus::Idle => rgb(0x9ece6a),
                NodeStatus::Busy | NodeStatus::Running => rgb(0xe0af68),
                NodeStatus::Crashed | NodeStatus::Dead => rgb(0xf7768e),
                _ => rgb(0x7aa2f7),
            };

            let mut title = div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(0x141a36))
                .text_size(px(13.0 * z))
                .text_color(rgb(0x7aa2f7))
                .child(div().size(px(9.0 * z)).rounded_full().bg(status_dot))
                .child(text!(nv.name.clone()))
                .child(
                    div()
                        .text_color(rgb(0x5b658f))
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
                        .bg(rgb(0x33202c))
                        .text_color(rgb(0xf7768e))
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
                // W4-6 a11y: nome + estado ANUNCIÁVEL (badge do W4-2: "💤 dormindo"/"precisa de você"/…).
                .aria_label(a11y::node_label(&nv.name, nv.status, needs_human))
                .flex_1()
                .overflow_hidden()
                .p_1()
                .bg(rgb(0x0d1228))
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
                                .text_color(rgb(0xc8d3f5))
                                .text_size(px(15.0 * z))
                                .child(text!(nv.name.clone())),
                        )
                        .child(
                            div()
                                .text_color(rgb(0x5b658f))
                                .text_size(px(11.0 * z))
                                .child(text!(what)),
                        ),
                );
            } else {
                // W4-2 · P0 (degradação de ociosos, red-team doc 12): SÓ o nó-TERMINAL em FOCO desenha o
                // grid ao vivo (grande, 120fps). Na PERIFERIA, cartão COMPACTO — BADGE honesto, SEM grid
                // (custo de CPU de ocioso ~0). Clicar foca → vira grande.
                match zone {
                    canvas::Zone::Focus => {
                        let grid = lock(&self.nodes.grids).get(&node_id).cloned();
                        if let Some(g) = grid {
                            let screen = lock(&g).screen();
                            body = body.child(render_grid(&screen, z, card_sel));
                        }
                    }
                    canvas::Zone::Periphery => {
                        // "precisa de você" se há um gate humano pendente para este nó (round 5).
                        let needs_human = lock(&self.desk)
                            .queue
                            .iter()
                            .any(|p| p.requester() == nv.name);
                        let badge = canvas::aggregate_badge(nv.status, needs_human, 0);
                        body = body.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .size_full()
                                .child(
                                    div()
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(rgb(badge.bg()))
                                        .text_color(rgb(0x11111b))
                                        .text_size(px(14.0 * z))
                                        .child(text!(badge.label())),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(0x5b658f))
                                        .text_size(px(11.0 * z))
                                        .child(text!("clique para focar")),
                                ),
                        );
                    }
                    // Suspenso já deu `continue` acima (culling).
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
                .bg(rgb(0x0d1228))
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
                .on_scroll_wheel(cx.listener(move |view, ev: &ScrollWheelEvent, _w, _cx| {
                    // ⌘+scroll sobre o card = zoom do canvas; scroll normal = rolar o terminal.
                    if ev.modifiers.platform {
                        let cursor = (f32::from(ev.position.x), f32::from(ev.position.y));
                        view.camera.zoom_by(cursor, scroll_zoom_factor(ev.delta));
                        return;
                    }
                    let dy: f32 = match ev.delta {
                        ScrollDelta::Lines(p) => p.y,
                        ScrollDelta::Pixels(p) => p.y / px(CELL_H),
                    };
                    // Mouse reporting: tenta reportar a roda (atômico); se o TUI consumiu, NÃO rola
                    // o scrollback. Sem pré-check separado de reporting (evita a corrida).
                    if dy != 0.0 {
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        let mods = (ev.modifiers.shift, ev.modifiers.alt, ev.modifiers.control);
                        if let Some(cell) = view.screen_cell(node_id, pos) {
                            let action = if dy > 0.0 {
                                PtrAction::WheelUp
                            } else {
                                PtrAction::WheelDown
                            };
                            if view.send_pointer(node_id, action, cell, mods) {
                                return;
                            }
                        }
                    }
                    let delta = dy.round() as i32;
                    if delta != 0 {
                        let grid = lock(&view.nodes.grids).get(&node_id).cloned();
                        if let Some(g) = grid {
                            lock(&g).scroll(delta);
                        }
                    }
                }))
                .child(title)
                .child(body);

            root = root.child(card);
        }

        // PULSO efêmero A→B (a metáfora "sem fios"). W4-3: respeita REDUCE-MOTION — com ele ligado, a
        // animação é suprimida (a entrega A2A ocorre igual; o pulso é decorativo).
        // W4-6 gap3: reduce-motion lido da FONTE ÚNICA `a11y::reduce_motion_effective` (SO/env sempre
        // respeitado + override do rodapé) — o W4-3 só inverte o bool.
        if let Some(p) = pulse
            .filter(|_| wiring::animate_pulse(a11y::reduce_motion_effective(self.reduce_motion)))
        {
            if let Some(t) = p.progress() {
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
                            .bg(rgb(0x7aa2f7))
                            .opacity(alpha * 0.35),
                    );
                    root = root.child(
                        div()
                            .absolute()
                            .left(dot_x - px(6.0))
                            .top(dot_y - px(6.0))
                            .size(px(12.0))
                            .rounded_full()
                            .bg(rgb(0xbb9af7))
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
            .bg(rgb(0x0c1130))
            .text_color(rgb(0x7aa2f7))
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
                    .child(div().size(px(8.0)).rounded_full().bg(rgb(0x9ece6a)))
                    .child(
                        div()
                            .text_color(rgb(0x9ece6a))
                            .child(text!("Time conectado · todos se falam")),
                    ),
            );
        }

        topbar = topbar.child(
            div()
                .text_color(rgb(0x5b658f))
                .child(text!(format!("log: {event_count} eventos"))),
        );

        // O ⚡ A2A (A→B) só aparece com AMBOS os alvos vivos: se o fundador fechar A ou B,
        // escondemos o botão em vez de deixá-lo falhar em silêncio.
        if self.a2a.ready() {
            topbar = topbar.child(
                div()
                    .id("a2a-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x3d59c9))
                    .text_color(rgb(0xeef1ff))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                        // O B é um shell real: injeta um COMANDO válido (roda limpo nele).
                        view.a2a.fire(
                            "echo '📨 A2A recebido de Terminal A · cooperacao sem fios'"
                                .to_string(),
                        );
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
                .bg(rgb(0x2c7a4b))
                .text_color(rgb(0xeef1ff))
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

        // W4-2 · M2 "Novo Agente": entra no modo nomeação (o humano digita o nome → papel derivado).
        // Quando ativo, mostra o buffer com cursor; senão, o botão.
        match &self.naming {
            Some(buf) => {
                topbar = topbar.child(
                    div()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(0x3a3f5a))
                        .text_color(rgb(0xeef1ff))
                        .child(text!(format!(
                            "✦ Novo agente: {buf}▌  (Enter cria · Esc cancela)"
                        ))),
                );
            }
            None => {
                topbar = topbar.child(
                    div()
                        .id("new-agent-btn")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(0x6c4ad1))
                        .text_color(rgb(0xeef1ff))
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                            // Abre o modo nomeação; o `handle_key` monta o buffer e cria no Enter.
                            view.naming = Some(String::new());
                        }))
                        .child(text!("✦ Novo Agente")),
                );
            }
        }

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
                    .bg(rgb(0x3a3f5a))
                    .text_color(rgb(0xeef1ff))
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
                .bg(rgb(0x2a3152))
                .text_color(rgb(0xc8d3f5))
                .cursor_pointer()
                .on_click(cx.listener(|view, _ev: &ClickEvent, _w, _cx| {
                    view.camera.reset();
                }))
                .child(text!("🏠 Centralizar")),
        );

        if recovering {
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0xf7768e))
                    .text_color(rgb(0x11111b))
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
                    .bg(rgb(0xf7768e))
                    .text_color(rgb(0x11111b))
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
            let bg = if custody_pending { 0xe0af68 } else { 0x2c7a4b };
            topbar = topbar.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(bg))
                    .text_color(rgb(0x11111b))
                    .child(text!(banner)),
            );
        }

        // W4-3 · RODAPÉ — o FREIO da auto-orquestração (sempre acessível) + toggle de reduce-motion.
        let paused = lock(&self.brake).paused;
        // W4-6 gap3: o rótulo do rodapé reflete o EFETIVO (fonte única) — não o override cru.
        let reduce_motion = a11y::reduce_motion_effective(self.reduce_motion);
        // BUG 5: rótulo em linguagem de leigo (sem "orquestração" cru) + legenda explicando o que faz.
        let (freio_bg, freio_txt) = if paused {
            (0x2c7a4b, "▶ Retomar cooperação dos agentes")
        } else {
            (0xe0af68, "⏸ Pausar cooperação dos agentes")
        };
        // BUG 5: o toggle deixa EXPLÍCITO o que liga/desliga (ANIMAÇÕES) + cor de estado óbvia (âmbar
        // quando desligadas = "redução ativa"). `reduce_motion` já é o EFETIVO (fonte única, W4-6).
        let (anim_bg, anim_fg, anim_txt) = if reduce_motion {
            (0xe0af68, 0x11111b, "🎞 Animações: DESLIGADAS")
        } else {
            (0x2a3152, 0xc8d3f5, "🎞 Animações: ligadas")
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
            .bg(rgb(0x0c1130))
            .child(
                div()
                    .id("freio-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(freio_bg))
                    .text_color(rgb(0x11111b))
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
            .child(div().text_color(rgb(0x5b658f)).child(text!(
                "ℹ Cooperação = os agentes se delegam tarefas sozinhos · Pausar segura isso (nada se perde, retoma quando quiser)"
            )));
        if paused {
            footer = footer.child(div().text_color(rgb(0xe0af68)).child(text!(
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
        // W4-2 · M1: a PALETA, quando aberta, é o overlay mais ao TOPO (modal sobre o canvas/chrome).
        if self.palette.is_open() {
            root.child(self.palette.render())
        } else {
            root
        }
    }
}

fn main() {
    let (cols, rows) = fit_dims();

    let dir = std::env::temp_dir().join("lina-space-ws2");
    let store = Arc::new(Mutex::new(
        EventStore::open(&dir).expect("abrir EventStore"),
    ));

    let pty = Arc::new(Mutex::new(PtyManager::new()));
    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    // Fábrica de nós: todo terminal é um SHELL INTERATIVO REAL e COMPLETO — idêntico em
    // capacidade (o B aceita teclado e roda claude/vim igual ao A). Add/remove em runtime
    // reusa EXATAMENTE esta fábrica via NodeManager.
    let cmd_factory: CmdFactory = Arc::new(shell_cmd);

    // W3-2 · BOOTSTRAP turno-0: o app escreve `<cwd>/CLAUDE.md` (8 blocos) por terminal e o
    // reescreve a cada mudança de roster. ws_root por terminal; vault/autonomia/bin `lina`
    // configuráveis por env (LINA_VAULT/LINA_BIN). O bootstrap é best-effort (não trava o app).
    let ws_root = std::env::temp_dir().join("lina-space-ws3");
    // W3-4: a mailbox compartilhada do workspace (`<ws_root>/.lina`). `LINA_HOME` aponta os
    // terminais (que herdam o env) para cá → `lina ask`/`handshake` depositam no MESMO outbox que
    // o supervisor (o `MailboxPump`) observa. (Vale para A/B e para os nós adicionados em runtime.)
    let mailbox_dir = ws_root.join(".lina");
    if let Err(e) = std::fs::create_dir_all(&mailbox_dir) {
        // Não fatal (o resto do app — canvas, terminais — funciona sem A2A), mas VISÍVEL: sem a
        // mailbox, `lina ask` não terá para onde escrever.
        eprintln!(
            "lina-gpui: não criou a mailbox {} (A2A indisponível): {e}",
            mailbox_dir.display()
        );
    }
    std::env::set_var("LINA_HOME", &mailbox_dir);
    let vault_path =
        std::env::var("LINA_VAULT").unwrap_or_else(|_| ws_root.join("vault").display().to_string());
    let lina_bin = std::env::var("LINA_BIN").unwrap_or_else(|_| "lina".to_string());
    let bootstrap = BootstrapWriter::new(ws_root.clone(), vault_path, Autonomy::Assisted, lina_bin)
        .map_err(|e| eprintln!("lina-gpui: bootstrap desativado: {e}"))
        .ok();

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

    // Cada terminal spawna no SEU cwd (<ws_root>/<key>), onde fica o CLAUDE.md dele.
    let (mut cmd_a, mut cmd_b) = ((*cmd_factory)("Terminal A"), (*cmd_factory)("Terminal B"));
    if let Some(bw) = &bootstrap {
        let da = bw.dir_for("A");
        let _ = std::fs::create_dir_all(&da);
        cmd_a = cmd_a.cwd(da);
        let db = bw.dir_for("B");
        let _ = std::fs::create_dir_all(&db);
        cmd_b = cmd_b.cwd(db);
    }

    // Os 2 nós iniciais (A, B), pela MESMA fiação dos nós adicionados em runtime.
    let (node_a, grid_a) = {
        let mut p = lock(&pty);
        wire_terminal(
            &mut p,
            &sup,
            &delta_tx,
            "A",
            "Terminal A",
            "terminal",
            cmd_a,
            cols,
            rows,
        )
    }
    .expect("wire A");
    let (node_b, grid_b) = {
        let mut p = lock(&pty);
        wire_terminal(
            &mut p,
            &sup,
            &delta_tx,
            "B",
            "Terminal B",
            "terminal",
            cmd_b,
            cols,
            rows,
        )
    }
    .expect("wire B");
    let _ = sup.set_status(node_a, CoreStatus::Running);
    let _ = sup.set_status(node_b, CoreStatus::Running);

    {
        let mut s = lock(&store);
        let _ = s.append(&DomainEvent::WorkspaceCreated {
            name: "walking-skeleton".into(),
            // W4-5: o walking-skeleton não passa pela galeria de Foco → preset não-setado (`""`).
            focus_preset: String::new(),
        });
        for (node, name, x, y) in [
            (node_a, "Terminal A", 30.0_f64, 96.0_f64),
            (node_b, "Terminal B", 740.0, 96.0),
        ] {
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
    }
    let event_count = lock(&store).event_count().unwrap_or(0);

    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    {
        let mut m = lock(&model);
        m.seed_node(
            node_a,
            NodeView::new("Terminal A", NodeKind::Terminal, 30.0, 96.0),
        );
        m.seed_node(
            node_b,
            NodeView::new("Terminal B", NodeKind::Terminal, 740.0, 96.0),
        );
        m.event_count = event_count;
    }

    let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));
    {
        let mut g = lock(&grids);
        g.insert(node_a, Arc::clone(&grid_a));
        g.insert(node_b, Arc::clone(&grid_b));
    }
    let mut keys: BTreeMap<NodeId, String> = BTreeMap::new();
    keys.insert(node_a, "A".to_string());
    keys.insert(node_b, "B".to_string());

    // W3-7c · TETO DE CUSTO REAL. `LINA_TOKEN_BUDGET_DAY` (tokens/dia ESTIMADOS ≈ bytes/4 de output;
    // ver cost.rs) ARMA o teto; 0/ausente = desligado (default, sem regressão). A bomba mede o output
    // dos PTYs e apenda TokenUsageReported no MESMO store (ws2) que o CostLedger soma.
    let token_budget_day: u64 = std::env::var("LINA_TOKEN_BUDGET_DAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let idle_ms = demo_profile().idle_ms.unwrap_or(200);
    let bridge = GpuiBridgeHost::new(Arc::clone(&model));
    let mut pump = spawn_pump(bridge, delta_rx, bus_rx, Arc::clone(&store), idle_ms)
        .expect("subir a ponte core→UI");

    let input: Arc<dyn InputSink> = Arc::new(CoreInput::new(Arc::clone(&sup)));
    let a2a = Arc::new(A2aTrigger::new(
        Arc::clone(&sup),
        node_a,
        node_b,
        Arc::clone(&grid_b),
        demo_profile(),
        Arc::clone(&store),
        Arc::clone(&model),
    ));

    // O dono dos nós em runtime (add/remove): reusa pty/sup/store/model/grids. O `seq` inicia
    // em 2 porque A=0 e B=1 já consumiram os rótulos A/B.
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
        2,
        cmd_factory,
        bootstrap,
        mailbox_dir.clone(), // W4-2 M3/M4: `.lina/` onde notas/pastas persistem
    );
    // W3-2: escreve o CLAUDE.md/estado inicial de A e B (roster = {A, B}).
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

    // DEMO: timer de auto-quit da janela. Antes ele só existia quando a env
    // LINA_AUTOQUIT_MS estava setada (caminho de smoke/CI: dispara um A2A no meio e
    // fecha); sem a env, a janela ficava aberta indefinidamente. Para o fundador
    // (não-técnico) validar o Terminal B COM CALMA, agora há um auto-quit PADRÃO de
    // 5 min quando a env não é dada — sem A2A surpresa.
    // DEMO: reverter para sem-auto-quit-padrão (timer só via env) após validação.
    const DEMO_AUTOQUIT_MS: u64 = 1_800_000; // 30 min (demo do fundador — reverter p/ 300_000 ou só-env após validar)
    match std::env::var("LINA_AUTOQUIT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
    {
        // Smoke/CI: a env (curta) dispara um A2A no meio e fecha — comportamento de teste.
        Some(ms) => {
            let a2a_auto = Arc::clone(&a2a);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms / 2));
                a2a_auto.fire("echo '📨 A2A automatico (smoke)'".to_string());
                thread::sleep(Duration::from_millis(ms / 2));
                std::process::exit(0);
            });
        }
        // DEMO: sem env → auto-quit calmo de 5 min (sem A2A surpresa) p/ validação visual.
        None => {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(DEMO_AUTOQUIT_MS));
                std::process::exit(0);
            });
        }
    }

    // W4-4: handles compartilhados p/ o painel de persistência (lê event_count/recovering do model;
    // loga WorkspaceFocusSet no store; settings.json ao lado de `dir`). Clones ANTES do `move`.
    let panel_model = Arc::clone(&model);
    let panel_store = Arc::clone(&store);
    let panel_dir = dir.clone();
    application().run(move |cx: &mut App| {
        // W4-1: onboarding turno-0 (T0→T3). Env-gated (LINA_ONBOARDING) p/ não perturbar a demo do
        // canvas durante o dev paralelo; fundador valida com `LINA_ONBOARDING=1 cargo run`.
        if onboarding::should_show() {
            onboarding::open_window(cx);
            cx.activate(true);
            return;
        }
        let bounds = Bounds::centered(None, size(px(1450.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lina Space — terminais reais (render de grid · sem fios)".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| WorkspaceView::new(nodes, input, a2a, node_a, desk, brake, window, cx))
            },
        )
        .expect("abrir a janela gpui");
        // W4-4: painel de persistência (env-gated LINA_PERSIST_PANEL; não perturba a demo do canvas).
        if persistence_ui::should_show() {
            persistence_ui::open_window(
                cx,
                Arc::clone(&panel_model),
                Arc::clone(&panel_store),
                panel_dir.clone(),
            );
        }
        cx.activate(true);
    });

    pump.stop();
    let _ = grid_a;
    drop(sup);
    drop(pty);
}
