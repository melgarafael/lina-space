//! `sizing` — geometria PURA do redimensionamento do terminal vivo (story **F2-3-2**).
//!
//! Duas peças, ambas DETERMINÍSTICAS e testáveis sem GPU/PTY:
//! - [`px_to_grid`]: converte a área de terminal (px, já descontado o chrome do card)
//!   na grade do PTY (cols×rows), com **pisos** (um terminal nunca vira 0×0) e
//!   **arredondamento por piso** (coluna/linha parcial não cabe → é descartada).
//! - [`ResizeDebouncer`]: coalesce o fluxo contínuo de tamanhos durante o gesto de
//!   arrasto em no máximo **1** `PtyResize` por janela (~100ms), com commit final ao
//!   soltar. O tempo é INJETADO (`now: Instant`) → o coalescing é testável de forma
//!   determinística, sem depender do relógio real.
//!
//! Por que aqui (lina-vt) e não no `PtyHost` (lina-core): é matemática de grade pura,
//! sem estado de A2A nem ciclo de vida de terminal; e um debouncer com tempo injetado
//! é muito mais testável que estado de relógio enfiado no host. O app consome ambos na
//! costura do canvas (main.rs), que também emite o evento durável `TerminalRedimensionado`.

use std::time::{Duration, Instant};

/// Converte a área de terminal disponível (em px, JÁ descontado o chrome do card —
/// barra de título / padding) na grade do PTY (`cols`×`rows`).
///
/// - **Piso de arredondamento:** uma coluna/linha que não cabe inteira é descartada
///   (senão o conteúdo seria cortado na borda).
/// - **Pisos** `min_cols`/`min_rows`: um terminal nunca encolhe a 0×0.
/// - **Entrada degenerada** (px ≤ 0, cell ≤ 0, `NaN`/`∞`) → devolve o piso; nunca entra
///   em pânico nem devolve lixo.
#[must_use]
pub fn px_to_grid(
    w_px: f32,
    h_px: f32,
    cell_w: f32,
    cell_h: f32,
    min_cols: u16,
    min_rows: u16,
) -> (u16, u16) {
    (
        grid_dim(w_px, cell_w, min_cols),
        grid_dim(h_px, cell_h, min_rows),
    )
}

/// Uma dimensão da grade: `floor(px / cell)`, com piso e blindagem contra entrada
/// degenerada (NaN/∞ falham todas as comparações `is_finite`/`>`).
fn grid_dim(px: f32, cell: f32, floor: u16) -> u16 {
    if !(px.is_finite() && cell.is_finite()) || px <= 0.0 || cell <= 0.0 {
        return floor;
    }
    // `n` é finito ≥ 0; clamp explicita a intenção antes do cast saturante f32→u16.
    let n = (px / cell).floor().clamp(0.0, f32::from(u16::MAX)) as u16;
    n.max(floor)
}

/// Coalesce o fluxo de tamanhos de um gesto de resize em poucos comandos `PtyResize`.
///
/// Contrato (tempo INJETADO para testabilidade determinística):
/// - [`offer`](Self::offer): durante o arrasto, no máximo **1** emissão por janela
///   (borda de ataque — o primeiro tamanho de cada janela passa; os seguintes, dentro
///   da janela, são coalescidos e o mais recente fica pendente).
/// - [`flush`](Self::flush): ao **soltar**, devolve o tamanho pendente coalescido (o
///   commit final), ou `None` se o último já foi emitido. Idempotente.
/// - [`cancel`](Self::cancel): gesto abortado (Esc) ⇒ `flush` passa a devolver `None`
///   (ZERO evento durável — doutrina ADR 0029 §1, espelha `CardDrag::cancel`).
/// - Tamanho igual ao último emitido nunca re-emite (resize é idempotente).
#[derive(Debug, Clone)]
pub struct ResizeDebouncer {
    window: Duration,
    last_emit: Option<Instant>,
    last_grid: Option<(u16, u16)>,
    pending: Option<(u16, u16)>,
    cancelled: bool,
}

impl ResizeDebouncer {
    /// Cria um debouncer com a janela de coalescing dada (ex.: `~100ms`).
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            last_emit: None,
            last_grid: None,
            pending: None,
            cancelled: false,
        }
    }

    /// Aborta o gesto (Esc): o commit final ([`flush`](Self::flush)) passa a devolver
    /// `None`. Operacionaliza "gesto cancelado = ZERO evento" (ADR 0029 §1) no módulo,
    /// não só na costura.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Oferece um tamanho candidato no instante `now` durante o gesto.
    /// `Some(grid)` ⇒ dispare o `PtyResize` agora; `None` ⇒ coalescido.
    pub fn offer(&mut self, now: Instant, cols: u16, rows: u16) -> Option<(u16, u16)> {
        let grid = (cols, rows);
        if self.last_grid == Some(grid) {
            // Já aplicado; descarta qualquer pendência redundante.
            self.pending = None;
            return None;
        }
        match self.last_emit {
            // Dentro da janela: coalesce, guardando só o mais recente.
            Some(t) if now.saturating_duration_since(t) < self.window => {
                self.pending = Some(grid);
                None
            }
            // Primeira emissão, ou janela expirada: dispara já.
            _ => {
                self.last_emit = Some(now);
                self.last_grid = Some(grid);
                self.pending = None;
                Some(grid)
            }
        }
    }

    /// Commit final ao soltar: o tamanho pendente coalescido, ou `None`.
    /// Gesto cancelado ([`cancel`](Self::cancel)) ⇒ sempre `None`.
    pub fn flush(&mut self) -> Option<(u16, u16)> {
        if self.cancelled {
            return None;
        }
        let grid = self.pending.take()?;
        if self.last_grid == Some(grid) {
            return None;
        }
        self.last_grid = Some(grid);
        Some(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ───────────────────────────── px_to_grid ─────────────────────────────

    #[test]
    fn px_to_grid_divide_em_grade_exata() {
        // 800/8 = 100 cols exatas; 400/16 = 25 rows exatas.
        assert_eq!(px_to_grid(800.0, 400.0, 8.0, 16.0, 80, 24), (100, 25));
    }

    #[test]
    fn px_to_grid_descarta_coluna_parcial() {
        // 807/8 = 100.875 → 100 (a coluna parcial não cabe; NUNCA arredonda p/ 101).
        let (cols, rows) = px_to_grid(807.0, 415.0, 8.0, 16.0, 1, 1);
        assert_eq!(cols, 100, "coluna parcial descartada");
        // 415/16 = 25.9375 → 25 (linha parcial descartada).
        assert_eq!(rows, 25, "linha parcial descartada");
    }

    #[test]
    fn px_to_grid_respeita_pisos() {
        // Área minúscula (12×2 cabe), mas os pisos 80×24 vencem: nunca 0×0.
        assert_eq!(px_to_grid(100.0, 32.0, 8.0, 16.0, 80, 24), (80, 24));
    }

    #[test]
    fn px_to_grid_piso_unitario_nunca_zero() {
        // Com piso 1×1 e área menor que uma célula (1/8=0.125→0, 1/16→0), o piso garante
        // 1×1 — um terminal jamais colapsa a 0×0, mesmo no limite.
        assert_eq!(px_to_grid(1.0, 1.0, 8.0, 16.0, 1, 1), (1, 1));
    }

    #[test]
    fn px_to_grid_entrada_degenerada_volta_ao_piso() {
        // cell 0 ⇒ divisão inválida ⇒ piso de cols (rows válidas seguem: 400/16=25).
        assert_eq!(px_to_grid(800.0, 400.0, 0.0, 16.0, 80, 24), (80, 25));
        // px negativo ⇒ piso de cols.
        assert_eq!(px_to_grid(-5.0, 400.0, 8.0, 16.0, 80, 24), (80, 25));
        // NaN/∞ ⇒ piso em ambos.
        assert_eq!(
            px_to_grid(f32::NAN, f32::INFINITY, 8.0, 16.0, 80, 24),
            (80, 24)
        );
    }

    // ──────────────────────────── ResizeDebouncer ───────────────────────────

    #[test]
    fn debouncer_coalesce_n_chamadas_em_uma_por_janela() {
        let mut d = ResizeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        // 1ª da janela passa (borda de ataque).
        assert_eq!(d.offer(t0, 80, 24), Some((80, 24)));
        // as seguintes, dentro de 100ms, são coalescidas (engolidas).
        assert_eq!(d.offer(t0 + Duration::from_millis(20), 82, 24), None);
        assert_eq!(d.offer(t0 + Duration::from_millis(50), 84, 25), None);
        assert_eq!(d.offer(t0 + Duration::from_millis(90), 86, 26), None);
        // 4 chamadas → 1 emissão na janela. O commit final (soltar) leva o último visto.
        assert_eq!(d.flush(), Some((86, 26)));
        assert_eq!(d.flush(), None, "flush é idempotente");
    }

    #[test]
    fn debouncer_nova_janela_emite_de_novo() {
        let mut d = ResizeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert_eq!(d.offer(t0, 80, 24), Some((80, 24)));
        // coalesce dentro da janela.
        assert_eq!(d.offer(t0 + Duration::from_millis(40), 90, 24), None);
        // passou a janela (>100ms desde a emissão em t0) → emite de novo.
        assert_eq!(
            d.offer(t0 + Duration::from_millis(120), 95, 24),
            Some((95, 24))
        );
    }

    #[test]
    fn debouncer_ignora_tamanho_repetido() {
        let mut d = ResizeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert_eq!(d.offer(t0, 80, 24), Some((80, 24)));
        // mesmo tamanho, mesmo em janela nova: nada a fazer (resize é idempotente).
        assert_eq!(d.offer(t0 + Duration::from_millis(200), 80, 24), None);
        assert_eq!(d.flush(), None);
    }

    #[test]
    fn debouncer_cancela_zera_commit() {
        // Gesto cancelado (Esc) ⇒ flush devolve None mesmo havendo pendente coalescido
        // (ADR 0029 §1: gesto cancelado = ZERO evento durável). Não-vacuoso: sem o check
        // de `cancelled` em flush(), o pending (82,25) vazaria como commit.
        let mut d = ResizeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert_eq!(d.offer(t0, 80, 24), Some((80, 24)));
        assert_eq!(d.offer(t0 + Duration::from_millis(30), 82, 25), None); // coalesce → pending
        d.cancel();
        assert_eq!(d.flush(), None, "gesto cancelado não commita");
        assert_eq!(d.flush(), None, "idempotente após cancelar");
    }

    #[test]
    fn debouncer_novo_gesto_apos_flush_emite() {
        // Ciclo isolado: após o commit final de um gesto, um tamanho NOVO em janela nova
        // volta a emitir (o debouncer não fica "preso" pós-flush).
        let mut d = ResizeDebouncer::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert_eq!(d.offer(t0, 80, 24), Some((80, 24)));
        assert_eq!(d.offer(t0 + Duration::from_millis(30), 90, 30), None);
        assert_eq!(d.flush(), Some((90, 30)), "commit final do 1º gesto");
        // novo tamanho, nova janela (>100ms desde a última emissão) → emite de novo.
        assert_eq!(
            d.offer(t0 + Duration::from_millis(200), 100, 36),
            Some((100, 36))
        );
    }
}
