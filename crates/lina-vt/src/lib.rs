//! `lina-vt` — emulação de terminal (VT100/xterm) atrás da trait `VtBackend` (story **W0-2**).
//!
//! ## Âncora de continuidade (ver `CLAUDE.md`)
//! `VtBackend` abstrai o motor VT para que `alacritty_terminal` possa ser trocado
//! por `libghostty-vt` no futuro **sem tocar no resto**. O grid parseado aqui é
//! o que alimenta o render (W2) e a detecção de idle do A2A (W0-9/W0-10).

use std::collections::BTreeSet;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode as AlacTermMode;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::Processor;

/// Modos do terminal relevantes ao A2A (ex.: bracketed-paste).
#[derive(Debug, Default, Clone, Copy)]
pub struct TermMode {
    pub bracketed_paste: bool,
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
        TermMode {
            bracketed_paste: self.term.mode().contains(AlacTermMode::BRACKETED_PASTE),
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
