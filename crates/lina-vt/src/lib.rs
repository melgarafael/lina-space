//! `lina-vt` — emulação de terminal (VT100/xterm) atrás da trait `VtBackend` (story **W0-2**).
//!
//! ## Âncora de continuidade (ver `CLAUDE.md`)
//! `VtBackend` abstrai o motor VT para que `alacritty_terminal` possa ser trocado
//! por `libghostty-vt` no futuro **sem tocar no resto**. O grid parseado aqui é
//! o que alimenta o render (W2) e a detecção de idle do A2A (W0-9/W0-10).

use std::collections::BTreeSet;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode as AlacTermMode;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

/// Modos do terminal relevantes ao A2A e ao input (bracketed-paste; cursor-keys DECCKM).
#[derive(Debug, Default, Clone, Copy)]
pub struct TermMode {
    pub bracketed_paste: bool,
    /// DECCKM (application cursor keys): setas viram `ESC O A/B/C/D` em vez de `ESC [ …`.
    pub app_cursor: bool,
}

/// Cor RGB resolvida (sem vazar tipos do alacritty no contrato `VtBackend`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VtRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Uma célula renderizável do grid: caractere + cores **já resolvidas** (inverse aplicado)
/// + negrito + **largura do cluster**. É o que o shell desenha (texto colorido), sem reparsing.
#[derive(Debug, Clone, Copy)]
pub struct VtCell {
    pub c: char,
    pub fg: VtRgb,
    pub bg: VtRgb,
    pub bold: bool,
    /// Largura da célula em **colunas de terminal**, lida dos flags do alacritty (W2-4):
    /// - `1` = normal (1 coluna);
    /// - `2` = **cabeça** de cluster wide (CJK/emoji): ocupa esta coluna **e a seguinte**;
    /// - `0` = **continuação/spacer** (a coluna à direita de um wide): o render NÃO desenha,
    ///   é coberta pela cabeça. Mantê-la no grid preserva o alinhamento coluna-a-coluna.
    ///
    /// Acentos combinantes pt-br (ex.: `a` + U+0301) NÃO criam célula: o alacritty os
    /// anexa como *zerowidth* à base, então a base permanece `width == 1`.
    pub width: u8,
}

/// Cursor visível: posição (linha/coluna do viewport) + se está visível.
#[derive(Debug, Clone, Copy, Default)]
pub struct VtCursor {
    pub line: usize,
    pub col: usize,
    pub visible: bool,
}

/// **Snapshot do GRID VISÍVEL** (cols×rows de células + cursor + offset de scroll). É o
/// produto central do render de terminal de verdade: TUIs redesenham a tela inteira, então
/// o shell pinta este snapshot a cada frame — não deltas de linha empilhados.
#[derive(Debug, Clone)]
pub struct VtScreen {
    pub cols: usize,
    pub rows: usize,
    /// `rows * cols` células, row-major (linha 0 = topo do viewport).
    pub cells: Vec<VtCell>,
    pub cursor: VtCursor,
    /// Quantas linhas o viewport está rolado para o passado (0 = no fundo, ao vivo).
    pub display_offset: usize,
}

impl VtScreen {
    /// As `cols` células da linha `line` do viewport.
    #[must_use]
    pub fn row(&self, line: usize) -> &[VtCell] {
        let start = line * self.cols;
        let end = (start + self.cols).min(self.cells.len());
        self.cells.get(start..end).unwrap_or(&[])
    }
}

// ─────────────────────── Seleção & Mouse reporting (aditivo à W0-2) ───────────────────────

/// Protocolo de **mouse reporting** que o TUI ativou via DECSET. O `lina-vt`
/// fala exclusivamente SGR (modo 1006); este enum diz **quais eventos** o TUI
/// quer receber. Os três modos são mutuamente exclusivos no emulador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseProtocol {
    /// `?1000h` — só press/release de botões (e roda). Sem movimento.
    ReportClick,
    /// `?1002h` — press/release + movimento **enquanto um botão está pressionado** (drag).
    ButtonMotion,
    /// `?1003h` — press/release + **todo** movimento, mesmo sem botão (hover).
    AnyMotion,
}

/// Botão de um evento de mouse. A roda (`WheelUp`/`WheelDown`) é reportada como
/// um "press" em todos os protocolos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

/// Natureza do evento de mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    Press,
    Release,
    /// Movimento do ponteiro (drag se houver botão; hover se `button == None`).
    Motion,
}

/// Modificadores de teclado ativos no momento do evento (somados ao código SGR).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MouseModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// Um evento de mouse em coordenadas de **viewport** (0-based). `button == None`
/// representa movimento sem botão pressionado (só relevante em `AnyMotion`).
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub button: Option<MouseButton>,
    pub kind: MouseEventKind,
    /// Coluna do viewport (0-based; vira 1-based na sequência SGR).
    pub col: usize,
    /// Linha do viewport (0-based; vira 1-based na sequência SGR).
    pub row: usize,
    pub mods: MouseModifiers,
}

/// Contrato do motor de emulação VT. `alacritty_terminal` é a impl da Onda 0.
pub trait VtBackend: Send {
    /// Aplica bytes vindos do PTY ao grid.
    fn advance(&mut self, bytes: &[u8]);
    /// Linhas alteradas desde o último `reset_damage` (para render incremental).
    fn damaged_rows(&self) -> Vec<usize>;
    fn reset_damage(&mut self);
    /// Modos atuais (bracketed-paste etc.) — usado pela entrega A2A faseada.
    fn mode(&self) -> TermMode;
    /// Última linha não-vazia do grid já parseado (sem ANSI) — para detectar
    /// prompt-pronto no A2A, **lendo o grid, não OCR de pixel**.
    fn last_nonempty_line(&self) -> String;
    /// Texto puro (sem ANSI) de uma linha do viewport — base do render incremental
    /// do shell de UI (W2): ao receber `HostEvent::GridDelta { dirty_rows }`, a UI
    /// re-lê **só** essas linhas do grid (lê o grid parseado, não OCR de pixel).
    /// Índice fora do viewport devolve `String` vazia.
    fn row_text(&self, viewport_line: usize) -> String;
    /// **Snapshot do grid VISÍVEL atual** (cols×rows + cores resolvidas + cursor) — a base
    /// do render de terminal de verdade (W2): o shell pinta a tela inteira a cada frame.
    fn screen(&self) -> VtScreen;
    /// Rola o histórico (scrollback): `delta > 0` sobe para o passado, `< 0` desce; o
    /// próximo `screen()` reflete o novo `display_offset`.
    fn scroll(&mut self, delta_lines: i32);
    /// Dimensões atuais do grid `(cols, rows)`.
    fn dims(&self) -> (usize, usize);
    fn resize(&mut self, cols: u16, rows: u16);

    // ── Seleção de texto (headless; respeita wrap de linha) ──

    /// Define a seleção **inclusiva** entre duas células do viewport (0-based),
    /// modo livre (não-retangular). A ordem dos pontos não importa — o backend
    /// normaliza (topo-esquerda → baixo-direita). Coordenadas fora do grid são
    /// saturadas para a borda. Sobrescreve qualquer seleção anterior.
    fn set_selection(&mut self, start_col: usize, start_row: usize, end_col: usize, end_row: usize);
    /// Texto da seleção atual (sem ANSI), **respeitando wrap**: linhas que
    /// continuam na seguinte (`WRAPLINE`) concatenam sem `\n`. `None` se não há
    /// seleção ativa.
    fn selection_text(&self) -> Option<String>;
    /// Limpa a seleção atual (após isto, `selection_text()` devolve `None`).
    fn clear_selection(&mut self);

    // ── Mouse reporting (SGR 1006) ──

    /// Protocolo de mouse que o TUI pediu (`None` = não pediu nada).
    fn mouse_protocol(&self) -> Option<MouseProtocol>;
    /// `true` se o TUI pediu qualquer mouse reporting.
    fn is_mouse_reporting(&self) -> bool {
        self.mouse_protocol().is_some()
    }
    /// Codifica `event` na sequência **SGR 1006** (`CSI < Cb ; Cx ; Cy M` para
    /// press/motion, `… m` para release) a enviar ao PTY, ou `None` quando
    /// nenhum byte deve ser gerado: o TUI não pediu mouse reporting, não negociou
    /// a codificação SGR (`?1006h`), ou o evento não pertence ao protocolo ativo
    /// (ex.: movimento em `ReportClick`). TUIs que não pedem mouse **nunca**
    /// recebem bytes.
    fn encode_mouse(&self, event: MouseEvent) -> Option<Vec<u8>>;

    // ── Scrollback `append-on-scroll` (W5-2 FIAÇÃO) ──

    /// Drena as linhas de scrollback **colhidas** desde a última chamada — as que rolaram para
    /// fora do viewport (cabo `append-on-scroll`), em ordem cronológica e byte-idênticas ao grid
    /// parseado. O pty-host as persiste no `lina_core::scrollback` (`ScrollbackStore`), fechando o
    /// anti-leak: o ring fica no `cap`, o histórico completo vai ao disco. Backends sem captura
    /// devolvem vazio. **Chame após cada `advance`** para manter o buffer interno limitado.
    fn take_scrollback(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Profundidade ATUAL do scrollback mantido em RAM pelo motor VT (linhas no ring) — para
    /// asseverar o teto de RAM (== `cap` em regime estável). `0` quando não aplicável.
    fn scrollback_len(&self) -> usize {
        0
    }
}

// ───────────────────────── AlacrittyBackend (impl da Onda 0) ─────────────────────────

/// Listener de eventos exigido por `alacritty_terminal`. Na Onda 0 ele é um
/// no-op: os eventos do terminal (ex.: title set, clipboard) são consumidos no
/// pty-host (W0-3) e na camada de UI (ondas futuras), não aqui.
#[derive(Clone, Copy, Default)]
struct NullProxy;

impl EventListener for NullProxy {
    fn send_event(&self, _event: Event) {}
}

/// Dimensões do grid passadas a `Term::new`/`Term::resize`. Para o argumento de
/// tamanho do alacritty, `total_lines == screen_lines` (o histórico de
/// scrollback cresce internamente até `Config::scrolling_history`, que o
/// `AlacrittyBackend` fixa pelo **cap de scrollback** configurável — W5-2).
#[derive(Clone, Copy)]
struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// W5-2: cap **default provisório** de linhas de scrollback mantidas em RAM por painel.
///
/// É o teto do ring-buffer interno do alacritty (`Config::scrolling_history`): ao excedê-lo,
/// as linhas mais ANTIGAS saem da RAM. O histórico além do cap NÃO se perde — é paginado em
/// disco pelo `lina_core::scrollback` (invariante #6, "nada se perde num crash").
///
/// **Por que capar:** `(#painéis × scrollback)` sem teto é a classe de leak que estourou a RAM
/// de Ghostty/Warp (~41 GB) sob output torrencial de CLI de IA. O cap fecha essa porta.
///
/// O valor (10_000) é **provisório**: o benchmark W5-1 calibra o número final. O entregável é o
/// MECANISMO (cap configurável + paginação), não a constante.
pub const DEFAULT_SCROLLBACK_CAP: usize = 10_000;

/// W5-2 FIAÇÃO — tamanho do **sub-chunk** com que o `advance` capturador alimenta o parser.
///
/// O parser `vte` é uma máquina de estado por byte: fatiar a entrada é semanticamente
/// transparente (mesmo resultado de um `advance` único). Fatiamos para **colher as linhas que
/// rolaram para o histórico ANTES que a próxima rolagem as eviccione** — o cabo `append-on-scroll`
/// que [`AlacrittyBackend::take_scrollback`] entrega ao `lina_core::scrollback`.
pub const SCROLLBACK_HARVEST_STEP: usize = 512;

/// W5-2 FIAÇÃO — **headroom** (linhas) que o ring interno do alacritty ganha além do `cap` durante
/// a captura. Mantido `2× STEP`: como uma rolagem por `LF` move ≤1 linha/byte, um sub-chunk de
/// `STEP` bytes empurra ≤ `STEP` linhas — `slack = 2×STEP` garante que o histórico **nunca satura**
/// (logo nunca evicciona) dentro de um sub-chunk de output normal, e o crescimento medido
/// (`history_size` após − antes) é o nº EXATO de linhas a colher. Após colher, o ring é **trimado
/// de volta ao `cap`** (estado estável = `cap`; pico transitório = `cap + slack`).
///
/// Bound conhecido: `CSI Ps S` (scroll-up explícito) move até `rows` linhas com poucos bytes, então
/// um firehose de `SU` no *primary screen* poderia exceder o slack — caso **adversarial e irreal**
/// (TUIs que usam scroll-region vivem no *alt-screen*, onde `scrolling_history=0` e nada é
/// capturado). O cabo é à prova de perda para output via `LF` (o caso real de scrollback).
pub const SCROLLBACK_HARVEST_SLACK: usize = 2 * SCROLLBACK_HARVEST_STEP;

/// Backend VT sobre `alacritty_terminal` 0.26 (parser `vte` 0.15 + grid + damage).
pub struct AlacrittyBackend {
    term: Term<NullProxy>,
    parser: Processor,
    size: GridSize,
    /// Linhas (viewport) alteradas desde o último `reset_damage`. Acumulamos aqui
    /// porque `Term::damage()` exige `&mut self`, enquanto a trait expõe
    /// `damaged_rows(&self)`.
    damage: BTreeSet<usize>,
    /// W5-2: cap de linhas de scrollback mantidas em RAM (teto do ring-buffer do grid).
    scrollback_cap: usize,
    /// W5-2 FIAÇÃO: se `true`, o `advance` colhe as linhas que rolam para o histórico (cabo
    /// `append-on-scroll`) em [`Self::captured`]. `false` = caminho legado, sem custo.
    capture: bool,
    /// W5-2 FIAÇÃO: headroom do ring durante a captura (0 quando `capture` desligada). Com captura,
    /// `Config::scrolling_history == scrollback_cap + slack`; o ring é trimado de volta ao cap a
    /// cada sub-chunk (ver [`SCROLLBACK_HARVEST_SLACK`]).
    slack: usize,
    /// W5-2 FIAÇÃO: linhas colhidas (saídas do viewport) ainda não drenadas por
    /// [`Self::take_scrollback`]. Ordem cronológica (mais antiga primeiro).
    captured: Vec<String>,
}

impl AlacrittyBackend {
    /// Cria um backend com um grid de `cols x rows` (mínimo 1x1) e o cap de scrollback
    /// **default** ([`DEFAULT_SCROLLBACK_CAP`]). Sem captura de scrollback (caminho legado).
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_scrollback_cap(cols, rows, DEFAULT_SCROLLBACK_CAP)
    }

    /// Como [`AlacrittyBackend::new`], mas com o **cap de scrollback** explícito (linhas/painel
    /// mantidas em RAM — W5-2). `scrollback_cap` vira o `Config::scrolling_history` do alacritty:
    /// o ring-buffer do grid descarta da RAM as linhas mais antigas além do cap (o excedente é
    /// paginado em disco pelo core). Permite ao app/benchmark (W5-1) calibrar o teto por painel
    /// sem recompilar o emulador. Sem captura de scrollback (caminho legado).
    pub fn with_scrollback_cap(cols: u16, rows: u16, scrollback_cap: usize) -> Self {
        Self::build(cols, rows, scrollback_cap, false)
    }

    /// W5-2 FIAÇÃO: backend com **captura `append-on-scroll`** ligada. Toda linha que rola para
    /// fora do viewport é colhida (byte-idêntica, em ordem) e fica disponível em
    /// [`Self::take_scrollback`] — o pty-host as persiste no `lina_core::scrollback` (o cabo que
    /// fecha o anti-leak da W5-1: o ring fica no `cap`, o histórico completo vai ao disco).
    ///
    /// O ring interno ganha [`SCROLLBACK_HARVEST_SLACK`] de headroom durante a captura, mas é
    /// **trimado de volta ao `cap`** a cada sub-chunk — o estado estável (`scrollback_len`) é `cap`.
    pub fn with_scrollback_capture(cols: u16, rows: u16, scrollback_cap: usize) -> Self {
        Self::build(cols, rows, scrollback_cap, true)
    }

    /// Builder compartilhado dos construtores. Com `capture`, o ring do alacritty é provisionado
    /// em `cap + slack` (headroom de colheita); sem captura, exatamente `cap` (legado).
    fn build(cols: u16, rows: u16, scrollback_cap: usize, capture: bool) -> Self {
        let size = GridSize {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
        };
        let slack = if capture { SCROLLBACK_HARVEST_SLACK } else { 0 };
        let config = Config {
            scrolling_history: scrollback_cap.saturating_add(slack),
            ..Config::default()
        };
        let term = Term::new(config, &size, NullProxy);
        Self {
            term,
            parser: Processor::new(),
            size,
            damage: BTreeSet::new(),
            scrollback_cap,
            capture,
            slack,
            captured: Vec::new(),
        }
    }

    /// W5-2: cap de scrollback (linhas/painel mantidas em RAM) deste backend. Com captura ligada,
    /// é o teto ESTÁVEL do ring (o headroom transitório de colheita não conta).
    #[must_use]
    pub fn scrollback_cap(&self) -> usize {
        self.scrollback_cap
    }

    /// Acesso somente-leitura ao grid parseado — útil para consumidores internos
    /// (render, testes célula-a-célula).
    fn grid(&self) -> &alacritty_terminal::Grid<alacritty_terminal::term::cell::Cell> {
        self.term.grid()
    }

    /// Lineariza a linha de índice ABSOLUTO `line` do grid (negativo = histórico/scrollback) para
    /// texto puro (sem ANSI), pulando os espaçadores de caractere largo — mesma regra de
    /// [`VtBackend::row_text`], mas sobre o histórico também. Base da **colheita de scrollback**
    /// (W5-2): ler a linha que rolou para o histórico, byte-fiel ao grid parseado.
    fn line_text_at(&self, line: i32) -> String {
        let row = &self.grid()[Line(line)];
        (0..self.size.cols)
            .map(|col| &row[Column(col)])
            .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|cell| cell.c)
            .collect()
    }

    /// W5-2 FIAÇÃO — `advance` com **captura `append-on-scroll`**. Alimenta o parser em sub-chunks
    /// de [`SCROLLBACK_HARVEST_STEP`] bytes; após cada um, colhe as linhas que entraram no histórico
    /// (ordem cronológica, byte-idêntica) ANTES de qualquer eviccão e trima o ring de volta ao cap.
    ///
    /// **Invariante (zero perda p/ output via `LF`):** o histórico nunca satura dentro de um
    /// sub-chunk (headroom `slack ≥ 2×STEP` > crescimento máximo `STEP`), então
    /// `s = history_size_depois − history_size_antes` é o nº EXATO de linhas que rolaram, e as `s`
    /// mais novas (Line `-s..=-1`) são exatamente elas — nenhuma se perde entre rolar e colher.
    fn advance_capturing(&mut self, bytes: &[u8]) {
        let cap = self.scrollback_cap;
        let ceiling = cap.saturating_add(self.slack); // == Config::scrolling_history do ring
        for step in bytes.chunks(SCROLLBACK_HARVEST_STEP.max(1)) {
            let before = self.grid().history_size();
            self.parser.advance(&mut self.term, step);
            let after = self.grid().history_size();
            if after > before {
                // Colhe as `s` linhas mais NOVAS (as que acabaram de entrar no histórico), em
                // ordem cronológica: a mais antiga das novas está em Line(-s); a mais nova em Line(-1).
                let s = after - before;
                for k in (1..=s).rev() {
                    let text = self.line_text_at(-(k as i32));
                    self.captured.push(text.trim_end().to_string());
                }
            }
            // Trima o ring de volta ao cap (remove os mais antigos — JÁ colhidos) e restaura o
            // headroom de captura para o próximo sub-chunk. O 2º `update_history` devolve o
            // `max_scroll_limit` ao teto sem reintroduzir linhas (só encolhe quando atual > alvo).
            if after > cap {
                let g = self.term.grid_mut();
                g.update_history(cap);
                g.update_history(ceiling);
            }
        }
    }

    /// Marca todas as linhas do viewport como danificadas (usado em resize).
    fn damage_all(&mut self) {
        self.damage.clear();
        self.damage.extend(0..self.size.rows);
    }

    /// Converte uma célula do viewport (`col`, `row` 0-based) num `Point`
    /// absoluto do grid, saturando às bordas e descontando o scroll atual
    /// (`display_offset`) — inverso da conversão feita em `screen()`.
    fn viewport_point(&self, col: usize, row: usize) -> Point {
        let col = col.min(self.size.cols.saturating_sub(1));
        let row = row.min(self.size.rows.saturating_sub(1));
        let abs_line = row as i32 - self.term.grid().display_offset() as i32;
        Point::new(Line(abs_line), Column(col))
    }
}

impl VtBackend for AlacrittyBackend {
    fn advance(&mut self, bytes: &[u8]) {
        if self.capture {
            // Captura `append-on-scroll`: sub-chunk + colheita + trim (W5-2 FIAÇÃO).
            self.advance_capturing(bytes);
        } else {
            // Caminho legado: um único passo, sem colheita (custo zero).
            self.parser.advance(&mut self.term, bytes);
        }

        // Acumula as linhas danificadas por este `advance`. `Term::damage()`
        // devolve o dano acumulado desde o último `Term::reset_damage`; só
        // chamamos `reset_damage` no nosso próprio `reset_damage`, então o
        // acumulador reflete tudo que mudou desde então.
        let rows: Vec<usize> = match self.term.damage() {
            TermDamage::Full => (0..self.size.rows).collect(),
            TermDamage::Partial(iter) => iter.map(|line| line.line).collect(),
        };
        self.damage.extend(rows);
    }

    fn damaged_rows(&self) -> Vec<usize> {
        self.damage.iter().copied().collect()
    }

    fn reset_damage(&mut self) {
        self.damage.clear();
        self.term.reset_damage();
    }

    fn mode(&self) -> TermMode {
        let m = self.term.mode();
        TermMode {
            bracketed_paste: m.contains(AlacTermMode::BRACKETED_PASTE),
            app_cursor: m.contains(AlacTermMode::APP_CURSOR),
        }
    }

    fn last_nonempty_line(&self) -> String {
        (0..self.size.rows)
            .rev()
            .map(|line| self.row_text(line))
            .map(|text| text.trim_end().to_string())
            .find(|text| !text.is_empty())
            .unwrap_or_default()
    }

    /// Lineariza uma linha do viewport para texto puro (sem ANSI), pulando os
    /// espaçadores de caractere largo. Tabs já chegam expandidos em espaços pelo
    /// próprio emulador, então a saída é texto cru pronto para consumo.
    fn row_text(&self, viewport_line: usize) -> String {
        if viewport_line >= self.size.rows {
            return String::new();
        }
        self.line_text_at(viewport_line as i32)
    }

    fn screen(&self) -> VtScreen {
        let cols = self.size.cols;
        let rows = self.size.rows;
        // Defaults casados com o card do shell (fg claro / bg do terminal).
        let default_fg = VtRgb {
            r: 0xc8,
            g: 0xd3,
            b: 0xf5,
        };
        let default_bg = VtRgb {
            r: 0x0d,
            g: 0x12,
            b: 0x28,
        };
        let blank = VtCell {
            c: ' ',
            fg: default_fg,
            bg: default_bg,
            bold: false,
            width: 1,
        };
        let mut cells = vec![blank; rows.saturating_mul(cols)];

        // `renderable_content` dá o viewport ATUAL (respeitando o display_offset do scroll).
        let content = self.term.renderable_content();
        let display_offset = content.display_offset;
        let cur = content.cursor;
        for indexed in content.display_iter {
            // `point` é terminal-absoluto (linhas negativas = scrollback). Converte p/ viewport.
            let vline = indexed.point.line.0 + display_offset as i32;
            if vline < 0 {
                continue;
            }
            let (line, col) = (vline as usize, indexed.point.column.0);
            if line >= rows || col >= cols {
                continue;
            }
            let cell = indexed.cell;
            let bold = cell.flags.contains(Flags::BOLD);
            let mut fg = resolve_color(cell.fg, default_fg, default_bg);
            let mut bg = resolve_color(cell.bg, default_fg, default_bg);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let c = if cell.c == '\0' { ' ' } else { cell.c };
            // Largura do cluster, direto dos flags do alacritty: cabeça wide = 2 colunas;
            // spacer (à direita do wide, ou o de fim-de-linha quando o wide não cabe) = 0;
            // qualquer outra = 1. Combinantes pt-br ficam embutidos na base (não chegam aqui).
            let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                0
            } else {
                1
            };
            cells[line * cols + col] = VtCell {
                c,
                fg,
                bg,
                bold,
                width,
            };
        }

        let cur_vline = cur.point.line.0 + display_offset as i32;
        let visible = cur.shape != alacritty_terminal::vte::ansi::CursorShape::Hidden
            && cur_vline >= 0
            && (cur_vline as usize) < rows;
        let cursor = VtCursor {
            line: cur_vline.max(0) as usize,
            col: cur.point.column.0.min(cols.saturating_sub(1)),
            visible,
        };

        VtScreen {
            cols,
            rows,
            cells,
            cursor,
            display_offset,
        }
    }

    fn scroll(&mut self, delta_lines: i32) {
        self.term.scroll_display(Scroll::Delta(delta_lines));
        // Rolar muda a tela inteira: marca tudo danificado p/ o próximo snapshot.
        self.damage_all();
    }

    fn dims(&self) -> (usize, usize) {
        (self.size.cols, self.size.rows)
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.size = GridSize {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
        };
        self.term.resize(self.size);
        // Um resize redesenha tudo: marca o viewport inteiro como danificado.
        self.damage_all();
    }

    fn set_selection(
        &mut self,
        start_col: usize,
        start_row: usize,
        end_col: usize,
        end_row: usize,
    ) {
        // Normaliza para ordem de leitura (linha, depois coluna): a ponta
        // "anterior" recebe Side::Left e a "posterior" Side::Right, garantindo
        // que AMBAS as células de borda entrem na seleção, venha o arraste da
        // direção que vier.
        let (a, b) = ((start_row, start_col), (end_row, end_col));
        let (first, last) = if a <= b { (a, b) } else { (b, a) };
        let start = self.viewport_point(first.1, first.0);
        let end = self.viewport_point(last.1, last.0);
        let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
        sel.update(end, Side::Right);
        self.term.selection = Some(sel);
    }

    fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    fn mouse_protocol(&self) -> Option<MouseProtocol> {
        let m = self.term.mode();
        // Os modos são mutuamente exclusivos; vamos do mais abrangente ao menos.
        if m.contains(AlacTermMode::MOUSE_MOTION) {
            Some(MouseProtocol::AnyMotion)
        } else if m.contains(AlacTermMode::MOUSE_DRAG) {
            Some(MouseProtocol::ButtonMotion)
        } else if m.contains(AlacTermMode::MOUSE_REPORT_CLICK) {
            Some(MouseProtocol::ReportClick)
        } else {
            None
        }
    }

    fn encode_mouse(&self, event: MouseEvent) -> Option<Vec<u8>> {
        let protocol = self.mouse_protocol()?;
        // Falamos exclusivamente SGR 1006: sem `?1006h` negociado, não emitimos
        // (codificações legacy X10/UTF-8 estão fora de escopo nesta fase).
        if !self.term.mode().contains(AlacTermMode::SGR_MOUSE) {
            return None;
        }
        encode_sgr_mouse(event, protocol)
    }

    fn take_scrollback(&mut self) -> Vec<String> {
        std::mem::take(&mut self.captured)
    }

    fn scrollback_len(&self) -> usize {
        self.grid().history_size()
    }
}

/// Tabela xterm 256 cores (auto-contida — o `alacritty_terminal` não embarca palette).
fn xterm256(idx: u8) -> VtRgb {
    const ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x00, 0x00),
        (0x00, 0xcd, 0x00),
        (0xcd, 0xcd, 0x00),
        (0x00, 0x00, 0xee),
        (0xcd, 0x00, 0xcd),
        (0x00, 0xcd, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x7f, 0x7f, 0x7f),
        (0xff, 0x00, 0x00),
        (0x00, 0xff, 0x00),
        (0xff, 0xff, 0x00),
        (0x5c, 0x5c, 0xff),
        (0xff, 0x00, 0xff),
        (0x00, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];
    if idx < 16 {
        let (r, g, b) = ANSI[idx as usize];
        VtRgb { r, g, b }
    } else if idx < 232 {
        let i = idx - 16;
        let to = |v: u8| -> u8 {
            if v == 0 {
                0
            } else {
                55 + v * 40
            }
        };
        VtRgb {
            r: to(i / 36),
            g: to((i % 36) / 6),
            b: to(i % 6),
        }
    } else {
        let v = 8u8.saturating_add((idx - 232).saturating_mul(10));
        VtRgb { r: v, g: v, b: v }
    }
}

/// Resolve uma `Color` do alacritty (`Named`/`Indexed`/`Spec`) numa `VtRgb` concreta.
fn resolve_color(color: Color, default_fg: VtRgb, default_bg: VtRgb) -> VtRgb {
    match color {
        Color::Spec(rgb) => VtRgb {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        },
        Color::Indexed(i) => xterm256(i),
        Color::Named(NamedColor::Foreground) => default_fg,
        Color::Named(NamedColor::Background) => default_bg,
        Color::Named(NamedColor::Cursor) => default_fg,
        Color::Named(named) => {
            let i = named as usize;
            if i < 16 {
                xterm256(i as u8)
            } else {
                default_fg
            }
        }
    }
}

/// Codifica um evento de mouse na sequência **SGR 1006**, filtrando pelo
/// `protocol` ativo. Devolve `None` quando o evento não deve ser reportado nesse
/// protocolo. Convenção de bytes (xterm): `CSI < Cb ; Cx ; Cy M` para
/// press/motion/roda, `… m` para release.
///
/// `Cb` = código do botão (Left 0, Middle 1, Right 2, sem-botão 3, roda 64/65),
/// somado a modificadores (Shift 4, Alt 8, Ctrl 16) e ao bit de movimento (32,
/// exceto roda). `Cx`/`Cy` são 1-based (SGR não tem o teto de 223 do X10).
fn encode_sgr_mouse(ev: MouseEvent, protocol: MouseProtocol) -> Option<Vec<u8>> {
    let is_wheel = matches!(
        ev.button,
        Some(MouseButton::WheelUp | MouseButton::WheelDown)
    );
    let is_motion = ev.kind == MouseEventKind::Motion;

    // "Sem botão" só é coerente em movimento (hover); descarta press/release sem botão.
    if ev.button.is_none() && !is_motion {
        return None;
    }

    // Filtragem por protocolo (a roda é sempre reportada).
    if is_motion && !is_wheel {
        match ev.button {
            // Movimento sem botão: só no modo "todo movimento".
            None if protocol != MouseProtocol::AnyMotion => return None,
            // Drag (movimento com botão): exige ButtonMotion ou AnyMotion.
            Some(_) if protocol == MouseProtocol::ReportClick => return None,
            _ => {}
        }
    }

    // Código do botão (Cb). A roda já embute o bit 64; "sem botão" = 3.
    let mut cb: u32 = match ev.button {
        Some(MouseButton::Left) => 0,
        Some(MouseButton::Middle) => 1,
        Some(MouseButton::Right) => 2,
        Some(MouseButton::WheelUp) => 64,
        Some(MouseButton::WheelDown) => 65,
        None => 3,
    };
    if ev.mods.shift {
        cb += 4;
    }
    if ev.mods.alt {
        cb += 8;
    }
    if ev.mods.control {
        cb += 16;
    }
    if is_motion && !is_wheel {
        cb += 32;
    }

    let cx = ev.col + 1;
    let cy = ev.row + 1;
    // Release usa o sufixo minúsculo 'm'; press/motion/roda usam 'M'.
    let suffix = if ev.kind == MouseEventKind::Release && !is_wheel {
        'm'
    } else {
        'M'
    };
    Some(format!("\x1b[<{cb};{cx};{cy}{suffix}").into_bytes())
}

// ───────────────────────── GhosttyBackend (porta de futuro) ─────────────────────────
//
// Âncora de continuidade do CLAUDE.md: quando `libghostty-vt` (C-ABI) for adotado,
// `GhosttyBackend` implementa a MESMA trait `VtBackend`, e nada mais muda. Fica atrás
// da feature `ghostty` (DESLIGADA por padrão) — é só a assinatura, não há build real
// de libghostty na Onda 0.
//
// A FFI esperada (esboço, quando ligada):
//   extern "C" {
//       fn ghostty_vt_new(cols: u16, rows: u16) -> *mut GhosttyVt;
//       fn ghostty_vt_advance(vt: *mut GhosttyVt, bytes: *const u8, len: usize);
//       fn ghostty_vt_free(vt: *mut GhosttyVt);
//   }

// Tipo-âncora unitário: o handle FFI (`*mut GhosttyVt`) entra ao adotar
// libghostty-vt, embrulhado num wrapper `Send` (como alacritty/portable-pty
// fazem). Mantê-lo unitário agora preserva a invariante `VtBackend: Send`.
#[cfg(feature = "ghostty")]
pub struct GhosttyBackend;

#[cfg(feature = "ghostty")]
impl VtBackend for GhosttyBackend {
    fn advance(&mut self, _bytes: &[u8]) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn damaged_rows(&self) -> Vec<usize> {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn reset_damage(&mut self) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn mode(&self) -> TermMode {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn last_nonempty_line(&self) -> String {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn row_text(&self, _viewport_line: usize) -> String {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn screen(&self) -> VtScreen {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn scroll(&mut self, _delta_lines: i32) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn dims(&self) -> (usize, usize) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn resize(&mut self, _cols: u16, _rows: u16) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn set_selection(
        &mut self,
        _start_col: usize,
        _start_row: usize,
        _end_col: usize,
        _end_row: usize,
    ) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn selection_text(&self) -> Option<String> {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn clear_selection(&mut self) {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn mouse_protocol(&self) -> Option<MouseProtocol> {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
    fn encode_mouse(&self, _event: MouseEvent) -> Option<Vec<u8>> {
        unimplemented!("GhosttyBackend: habilitar ao adotar libghostty-vt (ver CLAUDE.md)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: lê a célula `(linha_viewport, coluna)` do grid parseado.
    fn cell_char(b: &AlacrittyBackend, line: usize, col: usize) -> char {
        b.grid()[Line(line as i32)][Column(col)].c
    }

    /// Critério de aceite (a): sequência ANSI conhecida (cores + move de cursor +
    /// `ESC[2J` + texto) → assert do grid célula-a-célula.
    #[test]
    fn ansi_sequence_lands_in_grid_cell_by_cell() {
        let mut b = AlacrittyBackend::new(20, 6);

        // Limpa a tela (ESC[2J), vai para casa (ESC[H), escreve "AB" em verde,
        // muda para coluna 6 (CSI 6 G), escreve "Z", e desce uma linha para "go".
        //   \x1b[2J         -> erase display
        //   \x1b[H          -> cursor home (linha 0, col 0)
        //   \x1b[32m        -> foreground verde
        //   AB              -> escreve A em (0,0), B em (0,1)
        //   \x1b[6G         -> cursor para coluna 6 (col index 5)
        //   Z               -> escreve Z em (0,5)
        //   \x1b[2;1H       -> cursor para linha 2 (index 1), col 1 (index 0)
        //   go              -> escreve g em (1,0), o em (1,1)
        b.advance(b"\x1b[2J\x1b[H\x1b[32mAB\x1b[6GZ\x1b[2;1Hgo");

        // Linha 0
        assert_eq!(cell_char(&b, 0, 0), 'A');
        assert_eq!(cell_char(&b, 0, 1), 'B');
        assert_eq!(cell_char(&b, 0, 2), ' '); // intervalo limpo entre B e Z
        assert_eq!(cell_char(&b, 0, 5), 'Z');
        // Linha 1
        assert_eq!(cell_char(&b, 1, 0), 'g');
        assert_eq!(cell_char(&b, 1, 1), 'o');

        // Linearização sem ANSI: a última linha não-vazia é a 1 ("go").
        assert_eq!(b.last_nonempty_line(), "go");
        // `row_text` preserva a largura da linha (posições de coluna); quem apara
        // os espaços à direita é `last_nonempty_line`. "AB" + 3 espaços + "Z".
        assert_eq!(b.row_text(0).trim_end(), "AB   Z");
    }

    /// Critério de aceite (b): habilitar bracketed-paste (`CSI ? 2004 h`) →
    /// `mode().bracketed_paste == true`; e desabilitar (`l`) volta a `false`.
    #[test]
    fn bracketed_paste_mode_toggles() {
        let mut b = AlacrittyBackend::new(10, 3);
        assert!(!b.mode().bracketed_paste, "começa desligado");

        b.advance(b"\x1b[?2004h");
        assert!(
            b.mode().bracketed_paste,
            "CSI ? 2004 h liga o bracketed-paste"
        );

        b.advance(b"\x1b[?2004l");
        assert!(!b.mode().bracketed_paste, "CSI ? 2004 l desliga");
    }

    /// Critério de aceite (c): `damaged_rows()` reporta exatamente as linhas
    /// alteradas entre dois `advance` (incluindo as linhas de cursor antigo/novo,
    /// que precisam ser repintadas — modelo de damage do alacritty).
    #[test]
    fn damage_reports_exact_changed_rows() {
        let mut b = AlacrittyBackend::new(20, 6);

        // Normaliza o estado de damage antes de medir deltas.
        b.advance(b"\x1b[2J\x1b[H");
        b.reset_damage();
        assert_eq!(
            b.damaged_rows(),
            Vec::<usize>::new(),
            "após reset, nada danificado"
        );

        // advance 1: escreve na linha 0; cursor termina na linha 0.
        b.advance(b"line0");
        assert_eq!(b.damaged_rows(), vec![0], "só a linha 0 mudou");

        b.reset_damage();

        // advance 2: CR+LF leva o cursor à linha 1 e escreve. Mudam a linha 1
        // (texto novo) e a linha 0 (o cursor saiu dela e precisa ser repintada).
        b.advance(b"\r\nline1");
        assert_eq!(
            b.damaged_rows(),
            vec![0, 1],
            "linhas 0 (cursor saiu) e 1 (texto)"
        );
    }

    /// `resize` marca o viewport inteiro como danificado e o grid passa a ter as
    /// novas dimensões (sem panic).
    #[test]
    fn resize_marks_full_damage_and_changes_dims() {
        let mut b = AlacrittyBackend::new(20, 6);
        b.reset_damage();
        b.resize(40, 10);
        assert_eq!(b.damaged_rows(), (0..10).collect::<Vec<_>>());
        // Escrever depois do resize cabe nas novas colunas sem panic.
        b.advance(b"\x1b[H0123456789012345678901234567890123456789");
        assert_eq!(cell_char(&b, 0, 39), '9');
    }

    /// `last_nonempty_line` ignora linhas em branco abaixo do conteúdo.
    #[test]
    fn last_nonempty_line_skips_trailing_blanks() {
        let mut b = AlacrittyBackend::new(12, 5);
        b.advance(b"\x1b[2J\x1b[H\x1b[1;1Hhello");
        assert_eq!(b.last_nonempty_line(), "hello");
    }

    /// O backend é `Send` (exigência da trait `VtBackend: Send`).
    #[test]
    fn backend_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AlacrittyBackend>();
    }

    /// W5-2 (camada VT): o ring-buffer de scrollback do painel respeita o **cap
    /// configurável** — despeja MUITO mais linhas que o cap e o histórico em RAM do
    /// grid estabiliza no teto (não cresce monotonicamente). É a metade-VT do mecanismo
    /// anti-leak: aqui o cap LIMITA a RAM; o excedente é paginado em disco pelo core.
    #[test]
    fn scrollback_ring_respects_configurable_cap() {
        let cap = 100usize;
        let mut b = AlacrittyBackend::with_scrollback_cap(20, 5, cap);
        assert_eq!(
            b.scrollback_cap(),
            cap,
            "cap exposto deve casar com o configurado"
        );

        // Despeja 5_000 linhas (50× o cap) — bem além do que cabe na janela viva.
        for i in 0..5_000u32 {
            b.advance(format!("linha {i}\r\n").as_bytes());
        }

        // O histórico de scrollback mantido em RAM pelo grid NUNCA passa do cap: o
        // ring-buffer descarta as linhas mais antigas (teto de RAM respeitado).
        let hist = b.grid().history_size();
        assert!(
            hist <= cap,
            "history_size={hist} > cap={cap}: scrollback NÃO está capado em RAM"
        );
        // E está de fato CHEIO no cap (mantém uma janela real, não degenera p/ zero).
        assert_eq!(
            hist, cap,
            "esperava o ring de scrollback cheio exatamente no cap"
        );
    }

    /// W5-2: o construtor `new` usa o cap default documentado (provisório, calibrável
    /// pelo benchmark W5-1) — o mecanismo é o entregável, não o número.
    #[test]
    fn new_uses_documented_default_scrollback_cap() {
        let b = AlacrittyBackend::new(20, 5);
        assert_eq!(b.scrollback_cap(), DEFAULT_SCROLLBACK_CAP);
    }

    // ───────── W5-2 FIAÇÃO: captura append-on-scroll (cabo p/ o ScrollbackStore) ─────────

    /// As linhas capturadas devem ser o PREFIXO exato da sequência emitida: ordem
    /// cronológica + byte-idêntico + ZERO gap. (`make(i)` = a i-ésima linha emitida.)
    fn assert_prefix(captured: &[String], make: impl Fn(usize) -> String) {
        let expected: Vec<String> = (0..captured.len()).map(make).collect();
        assert_eq!(
            captured,
            expected.as_slice(),
            "linhas capturadas não são o prefixo exato (gap/ordem/conteúdo divergiu)"
        );
    }

    /// FIAÇÃO W5-2: cada linha que rola para fora do viewport é capturada (append-on-scroll),
    /// em ordem e byte-idêntica — base do cabo que o pty-host liga ao `ScrollbackStore`.
    #[test]
    fn scrollback_capture_harvests_each_scrolled_line_in_order() {
        let cap = 8usize;
        let rows = 4u16;
        let mut b = AlacrittyBackend::with_scrollback_capture(20, rows, cap);

        const N: usize = 50;
        for i in 0..N {
            b.advance(format!("L{i}\r\n").as_bytes());
        }
        let captured = b.take_scrollback();

        // Prefixo exato L0..L{len-1}: zero gap, ordem cronológica, byte-idêntico.
        assert_prefix(&captured, |i| format!("L{i}"));
        // No máximo `rows` linhas ficam visíveis → pelo menos N - rows saíram do viewport.
        assert!(
            captured.len() >= N - rows as usize,
            "esperava ≥ {} capturadas, veio {}",
            N - rows as usize,
            captured.len()
        );
        assert!(captured.len() <= N);
        // O ring em RAM estabiliza NO cap (linhas em RAM == cap).
        assert_eq!(
            b.scrollback_len(),
            cap,
            "ring de scrollback deve ficar exatamente no cap"
        );
        // Drenou: segunda chamada vem vazia.
        assert!(
            b.take_scrollback().is_empty(),
            "take_scrollback deve drenar o buffer"
        );
    }

    /// FIAÇÃO W5-2 (CRUX inv#4): output TORRENCIAL num ÚNICO `advance` (>> cap linhas) — o caso
    /// que a leitura-do-histórico-pós-advance perderia (eviccão dentro do mesmo advance). O
    /// sub-chunk + harvest captura TODAS sem gap. É a diferença entre "nada se perde" e perder
    /// a janela inteira sob firehose.
    #[test]
    fn scrollback_capture_survives_torrential_single_advance() {
        let cap = 8usize;
        let rows = 4u16;
        let mut b = AlacrittyBackend::with_scrollback_capture(20, rows, cap);

        const N: usize = 5_000; // 625× o cap, num único advance
        let mut blast = String::with_capacity(N * 8);
        for i in 0..N {
            blast.push_str(&format!("L{i}\r\n"));
        }
        b.advance(blast.as_bytes());
        let captured = b.take_scrollback();

        // ZERO PERDA: prefixo exato, sem buracos, apesar de >> cap linhas num advance só.
        assert_prefix(&captured, |i| format!("L{i}"));
        assert!(
            captured.len() >= N - rows as usize,
            "perdeu linhas sob firehose: {} de {}",
            captured.len(),
            N
        );
        assert_eq!(b.scrollback_len(), cap, "ring trimado ao cap pós-firehose");
    }

    /// FIAÇÃO W5-2: sem captura (construtores `new`/`with_scrollback_cap`), `take_scrollback`
    /// é inerte (vazio) — o caminho legado não paga custo nem muda de comportamento.
    #[test]
    fn no_capture_means_empty_scrollback() {
        let mut b = AlacrittyBackend::with_scrollback_cap(20, 4, 8);
        for i in 0..50u32 {
            b.advance(format!("L{i}\r\n").as_bytes());
        }
        assert!(
            b.take_scrollback().is_empty(),
            "backend sem captura não deve acumular scrollback"
        );
        // E o ring continua respeitando o cap (comportamento legado intacto).
        assert_eq!(b.grid().history_size(), 8);
    }

    // ───────────────────────────── Seleção de texto ─────────────────────────────

    /// Seleção de uma única linha extrai exatamente as células inclusas.
    #[test]
    fn selection_extracts_single_line_text() {
        let mut b = AlacrittyBackend::new(20, 4);
        b.advance(b"\x1b[2J\x1b[H\x1b[1;1Hhello world");
        // "hello" = colunas 0..=4 da linha 0.
        b.set_selection(0, 0, 4, 0);
        assert_eq!(b.selection_text().as_deref(), Some("hello"));
    }

    /// Seleção em múltiplas linhas SEM wrap junta com `\n`.
    #[test]
    fn selection_spans_multiple_lines_with_newline() {
        let mut b = AlacrittyBackend::new(20, 4);
        // Duas linhas independentes: "foo" (linha 0) e "bar" (linha 1).
        b.advance(b"\x1b[2J\x1b[H\x1b[1;1Hfoo\x1b[2;1Hbar");
        b.set_selection(0, 0, 2, 1);
        assert_eq!(b.selection_text().as_deref(), Some("foo\nbar"));
    }

    /// Seleção atravessando uma linha com `WRAPLINE` concatena SEM `\n`
    /// (a linha continua na seguinte — é uma só linha lógica).
    #[test]
    fn selection_respects_wrapline_no_newline() {
        // Grid de 5 colunas: "abcdefgh" transborda e faz wrap (DECAWM default).
        let mut b = AlacrittyBackend::new(5, 4);
        b.advance(b"\x1b[2J\x1b[Habcdefgh");
        // linha 0 = "abcde" (marcada WRAPLINE), linha 1 = "fgh".
        b.set_selection(0, 0, 2, 1);
        assert_eq!(b.selection_text().as_deref(), Some("abcdefgh"));
    }

    /// A ordem dos pontos não importa (fim → início dá o mesmo texto).
    #[test]
    fn selection_is_direction_agnostic() {
        let mut b = AlacrittyBackend::new(20, 4);
        b.advance(b"\x1b[2J\x1b[H\x1b[1;1Hhello");
        b.set_selection(4, 0, 0, 0);
        assert_eq!(b.selection_text().as_deref(), Some("hello"));
    }

    /// `clear_selection` zera a seleção; `selection_text` volta a `None`.
    #[test]
    fn clear_selection_yields_none() {
        let mut b = AlacrittyBackend::new(20, 4);
        b.advance(b"\x1b[2J\x1b[Hhello");
        b.set_selection(0, 0, 4, 0);
        assert!(b.selection_text().is_some());
        b.clear_selection();
        assert_eq!(b.selection_text(), None);
    }

    /// Sem seleção definida, `selection_text` é `None`.
    #[test]
    fn no_selection_returns_none() {
        let b = AlacrittyBackend::new(10, 3);
        assert_eq!(b.selection_text(), None);
    }

    // ─────────────────────────── Mouse reporting (SGR 1006) ───────────────────────────

    /// Liga um protocolo de mouse + a codificação SGR (`?1006h`).
    fn enable_mouse(b: &mut AlacrittyBackend, decset: &[u8]) {
        b.advance(decset);
        b.advance(b"\x1b[?1006h");
    }

    /// Critério (c): TUI que NÃO pede mouse não gera byte algum.
    #[test]
    fn no_mouse_bytes_when_mode_not_requested() {
        let b = AlacrittyBackend::new(80, 24);
        assert!(!b.is_mouse_reporting());
        assert_eq!(b.mouse_protocol(), None);
        let ev = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Press,
            col: 0,
            row: 0,
            mods: MouseModifiers::default(),
        };
        assert_eq!(b.encode_mouse(ev), None, "sem reporting → nenhum byte");
    }

    /// Press e release do botão esquerdo batem byte a byte (sufixo M vs m).
    #[test]
    fn sgr_press_and_release_left_button() {
        let mut b = AlacrittyBackend::new(80, 24);
        enable_mouse(&mut b, b"\x1b[?1000h");
        assert_eq!(b.mouse_protocol(), Some(MouseProtocol::ReportClick));

        // Press na coluna 0, linha 0 → 1;1. Cb=0, sufixo 'M'.
        let press = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Press,
            col: 0,
            row: 0,
            mods: MouseModifiers::default(),
        };
        assert_eq!(b.encode_mouse(press).as_deref(), Some(&b"\x1b[<0;1;1M"[..]));

        // Release na coluna 9, linha 4 → 10;5. Cb=0, sufixo 'm'.
        let release = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Release,
            col: 9,
            row: 4,
            mods: MouseModifiers::default(),
        };
        assert_eq!(
            b.encode_mouse(release).as_deref(),
            Some(&b"\x1b[<0;10;5m"[..])
        );
    }

    /// `ReportClick` (1000) não reporta movimento.
    #[test]
    fn report_click_suppresses_motion() {
        let mut b = AlacrittyBackend::new(80, 24);
        enable_mouse(&mut b, b"\x1b[?1000h");
        let drag = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Motion,
            col: 3,
            row: 2,
            mods: MouseModifiers::default(),
        };
        assert_eq!(b.encode_mouse(drag), None, "ReportClick não reporta motion");
    }

    /// `ButtonMotion` (1002) reporta drag (bit 32) mas não hover sem botão.
    #[test]
    fn button_motion_reports_drag_with_motion_bit() {
        let mut b = AlacrittyBackend::new(80, 24);
        enable_mouse(&mut b, b"\x1b[?1002h");
        assert_eq!(b.mouse_protocol(), Some(MouseProtocol::ButtonMotion));

        // Drag com esquerdo: Cb = 0 + 32 = 32. col 5,row 3 → 6;4.
        let drag = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Motion,
            col: 5,
            row: 3,
            mods: MouseModifiers::default(),
        };
        assert_eq!(b.encode_mouse(drag).as_deref(), Some(&b"\x1b[<32;6;4M"[..]));

        // Movimento SEM botão não é reportado em ButtonMotion.
        let hover = MouseEvent {
            button: None,
            kind: MouseEventKind::Motion,
            col: 5,
            row: 3,
            mods: MouseModifiers::default(),
        };
        assert_eq!(b.encode_mouse(hover), None);
    }

    /// `AnyMotion` (1003) reporta hover sem botão: Cb = 3 + 32 = 35.
    #[test]
    fn any_motion_reports_buttonless_hover() {
        let mut b = AlacrittyBackend::new(80, 24);
        enable_mouse(&mut b, b"\x1b[?1003h");
        assert_eq!(b.mouse_protocol(), Some(MouseProtocol::AnyMotion));
        let hover = MouseEvent {
            button: None,
            kind: MouseEventKind::Motion,
            col: 0,
            row: 0,
            mods: MouseModifiers::default(),
        };
        assert_eq!(
            b.encode_mouse(hover).as_deref(),
            Some(&b"\x1b[<35;1;1M"[..])
        );
    }

    /// Roda (Cb=64) e modificadores (Shift 4 + Ctrl 16) somam corretamente.
    #[test]
    fn wheel_and_modifiers_encode() {
        let mut b = AlacrittyBackend::new(80, 24);
        enable_mouse(&mut b, b"\x1b[?1000h");

        // Roda para cima: Cb=64. col 0,row 0 → 1;1.
        let wheel = MouseEvent {
            button: Some(MouseButton::WheelUp),
            kind: MouseEventKind::Press,
            col: 0,
            row: 0,
            mods: MouseModifiers::default(),
        };
        assert_eq!(
            b.encode_mouse(wheel).as_deref(),
            Some(&b"\x1b[<64;1;1M"[..])
        );

        // Esquerdo + Shift(4) + Ctrl(16) = Cb 20. col 2,row 1 → 3;2.
        let mods = MouseModifiers {
            shift: true,
            alt: false,
            control: true,
        };
        let click = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Press,
            col: 2,
            row: 1,
            mods,
        };
        assert_eq!(
            b.encode_mouse(click).as_deref(),
            Some(&b"\x1b[<20;3;2M"[..])
        );
    }

    /// Mouse reporting ativo MAS sem SGR negociado → nenhum byte (só falamos SGR).
    #[test]
    fn no_sgr_means_no_bytes_even_with_reporting() {
        let mut b = AlacrittyBackend::new(80, 24);
        b.advance(b"\x1b[?1000h"); // pede mouse, mas NÃO `?1006h`
        assert!(b.is_mouse_reporting(), "o protocolo está ativo…");
        let ev = MouseEvent {
            button: Some(MouseButton::Left),
            kind: MouseEventKind::Press,
            col: 0,
            row: 0,
            mods: MouseModifiers::default(),
        };
        assert_eq!(b.encode_mouse(ev), None, "…mas sem SGR não emitimos bytes");
    }

    // ──────────────── W2-4: largura de cluster (wide/grapheme) — corpus golden ────────────────

    /// `(char, width)` das células da linha 0 ATÉ o primeiro vazio — base dos goldens.
    fn row_prefix(b: &AlacrittyBackend) -> Vec<(char, u8)> {
        b.screen()
            .row(0)
            .iter()
            .map(|c| (c.c, c.width))
            .take_while(|(c, w)| !(*c == ' ' && *w == 1))
            .collect()
    }

    /// CORPUS GOLDEN — a estrutura `(char, width)` DETERMINÍSTICA que o alacritty 0.26
    /// produz para cada fixture. Snapshot exato (observado via `--nocapture`, não presumido).
    ///
    /// Critérios do épico W2-4 cobertos:
    /// - CJK ("中") e emoji ("😀") são **cabeça wide (2) + spacer (0)** → ocupam 2 células;
    /// - o acento combinante pt-br (`a`+U+0301) NÃO cria célula (zerowidth na base) → o `b`
    ///   seguinte fica na col 1 → **acento em 1 célula**;
    /// - a **emoji-família ZWJ** começa wide. NB FIEL: o alacritty mede por *code point*
    ///   (East Asian Width) e **não** colapsa o cluster ZWJ — cada sub-emoji é wide, então
    ///   a família alinha em **8 colunas** (4×`[2,0]`), não em 2. O golden registra a verdade
    ///   do motor (uma camada de grapheme-clustering ZWJ seria render-side, fora da W2-4).
    #[test]
    fn width_corpus_golden() {
        // (nome, entrada, prefixo esperado de `(char, width)`).
        type Golden = (&'static str, &'static str, &'static [(char, u8)]);
        let corpus: &[Golden] = &[
            ("ascii", "Hi", &[('H', 1), ('i', 1)]),
            ("cjk", "中", &[('中', 2), (' ', 0)]),
            ("emoji", "😀", &[('😀', 2), (' ', 0)]),
            ("combining-ptbr", "a\u{0301}b", &[('a', 1), ('b', 1)]),
            (
                "zwj-family",
                "👨\u{200d}👩\u{200d}👧\u{200d}👦",
                &[
                    ('👨', 2),
                    (' ', 0),
                    ('👩', 2),
                    (' ', 0),
                    ('👧', 2),
                    (' ', 0),
                    ('👦', 2),
                    (' ', 0),
                ],
            ),
        ];
        for (name, input, expected) in corpus {
            let mut b = AlacrittyBackend::new(20, 2);
            b.advance(input.as_bytes());
            assert_eq!(
                &row_prefix(&b),
                expected,
                "corpus golden divergiu em [{name}]"
            );
        }
    }

    /// ALINHAMENTO dinâmico: o glifo escrito APÓS um wide cai na coluna correta (col 2),
    /// não na 1 — o spacer preserva a grade. Complementa o golden estático.
    #[test]
    fn width_wide_char_aligns_following_glyph() {
        let mut b = AlacrittyBackend::new(10, 2);
        b.advance("中".as_bytes());
        b.advance(b"x");
        let cells: Vec<(char, u8)> = b.screen().row(0).iter().map(|c| (c.c, c.width)).collect();
        assert_eq!(cells[0], ('中', 2));
        assert_eq!(cells[1].1, 0, "spacer ocupa a col 1");
        assert_eq!(cells[2], ('x', 1), "o 'x' alinha na col 2 (após o wide)");
    }
}
