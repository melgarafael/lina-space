//! `lina-vt` — emulação de terminal (VT100/xterm) atrás da trait `VtBackend` (story **W0-2**).
//!
//! ## Âncora de continuidade (ver `CLAUDE.md`)
//! `VtBackend` abstrai o motor VT para que `alacritty_terminal` possa ser trocado
//! por `libghostty-vt` no futuro **sem tocar no resto**. O grid parseado aqui é
//! o que alimenta o render (W2) e a detecção de idle do A2A (W0-9/W0-10).

use std::collections::BTreeSet;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
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
/// + negrito. É o que o shell desenha (texto colorido), sem reparsing.
#[derive(Debug, Clone, Copy)]
pub struct VtCell {
    pub c: char,
    pub fg: VtRgb,
    pub bg: VtRgb,
    pub bold: bool,
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
/// scrollback cresce internamente até `Config::scrolling_history`).
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

/// Backend VT sobre `alacritty_terminal` 0.26 (parser `vte` 0.15 + grid + damage).
pub struct AlacrittyBackend {
    term: Term<NullProxy>,
    parser: Processor,
    size: GridSize,
    /// Linhas (viewport) alteradas desde o último `reset_damage`. Acumulamos aqui
    /// porque `Term::damage()` exige `&mut self`, enquanto a trait expõe
    /// `damaged_rows(&self)`.
    damage: BTreeSet<usize>,
}

impl AlacrittyBackend {
    /// Cria um backend com um grid de `cols x rows` (mínimo 1x1).
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = GridSize {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
        };
        let term = Term::new(Config::default(), &size, NullProxy);
        Self {
            term,
            parser: Processor::new(),
            size,
            damage: BTreeSet::new(),
        }
    }

    /// Acesso somente-leitura ao grid parseado — útil para consumidores internos
    /// (render, testes célula-a-célula).
    fn grid(&self) -> &alacritty_terminal::Grid<alacritty_terminal::term::cell::Cell> {
        self.term.grid()
    }

    /// Marca todas as linhas do viewport como danificadas (usado em resize).
    fn damage_all(&mut self) {
        self.damage.clear();
        self.damage.extend(0..self.size.rows);
    }
}

impl VtBackend for AlacrittyBackend {
    fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);

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
        let row = &self.grid()[Line(viewport_line as i32)];
        (0..self.size.cols)
            .map(|col| &row[Column(col)])
            .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|cell| cell.c)
            .collect()
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
            cells[line * cols + col] = VtCell { c, fg, bg, bold };
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
}
