//! `lina-vt` — emulação de terminal (VT100/xterm) atrás da trait `VtBackend` (story **W0-2**).
//!
//! ## Âncora de continuidade (ver `CLAUDE.md`)
//! `VtBackend` abstrai o motor VT para que `alacritty_terminal` possa ser trocado
//! por `libghostty-vt` no futuro **sem tocar no resto**. O grid parseado aqui é
//! o que alimenta o render (W2) e a detecção de idle do A2A (W0-9/W0-10).

#![allow(dead_code)]

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
    fn resize(&mut self, cols: u16, rows: u16);
}

// TODO(W0-2): impl AlacrittyBackend (sobre alacritty_terminal 0.26).
// TODO(W0-2): stub GhosttyBackend (#[cfg(feature)] desligada) — porta de futuro.
