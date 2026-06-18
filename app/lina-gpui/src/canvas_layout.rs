//! `canvas_layout` — **F2-3-4: "Arrumar" como verbo (grade limpa / lado a lado), lógica PURA.**
//!
//! Norte (onda F2-3): o fundador clica "Arrumar" (e acha na paleta ⌘K) e a bagunça de cards vira
//! uma GRADE limpa, ou "lado a lado" — previsível, sem nada redimensionar sozinho e sem os cards
//! embaralharem.
//!
//! **O que vive aqui (gpui-free, headless-testável, ZERO deps de crate → compila em isolamento):**
//! - [`arrange_grid`]: posições de uma grade de **4 colunas** (espelha a RÉGUA de `bridge::grid_slot`
//!   — mesma origem, passo `card+gap` e 4 colunas row-major). Com `origin=(30,96)` e cards em ordem
//!   de criação contígua, coincide com a grade que o usuário viu ao abrir o app — o único modelo
//!   mental de "arrumado" que o leigo tem.
//! - [`arrange_side_by_side`]: preset "lado a lado" — UMA fileira horizontal reta (mesmo `y`, `x`
//!   crescente), **nunca enrola**. É o gesto de COMPARAÇÃO (ver terminais alinhados na mesma altura).
//!
//! **Funções PURAS e INDEXADAS:** produzem posições para os índices `0..n`; QUAL card vai em cada
//! índice é decisão da COSTURA (call-site em `main.rs`). A ordem recomendada — e exigida pelos
//! invariantes — é **ordem de CRIAÇÃO** (estável, imune a foco). Ver "Invariantes" e a entrega.
//!
//! **Invariantes "niri" (duros):**
//! - **Determinístico:** mesma entrada ⇒ mesma saída, sempre (sem `HashMap` iter, sem float instável).
//! - **Nada se redimensiona sozinho:** a saída é `Vec<(f32,f32)>` — SEM tamanho. É impossível, por
//!   tipo, esta função mexer em `w/h`. Só reposiciona.
//! - **Cards não embaralham:** a estabilidade vem da ordem de criação no call-site. z-order MUDA no
//!   foco (`bridge.rs`: `BTreeMap<NodeId,u64>` re-pondera ao focar) → usá-lo faria o card pular de
//!   slot só por ser clicado antes do "Arrumar". Ordem-de-leitura(y,x) é auto-referente (1px de drag
//!   inverte vizinhos). Criação é a única ordem que o app já declara estável — e segue a mesma
//!   régua `k=0,1,2…` do `grid_slot` (o init usa `next_free_slot`, que preenche buracos; com cards
//!   contíguos os dois coincidem).
//!
//! **Costura (dono: Maestro, em `main.rs`):** itera os nós em ordem de criação, chama `arrange_*`, e
//! faz **diff-and-emit** — emite `DomainEvent::NodeMoved { node, x: x as f64, y: y as f64 }` SÓ para
//! os índices cuja posição calculada difere da atual (reusa o seam do mover, F2-3-1). Como a fórmula
//! é determinística e idêntica, comparar `==` por bits é seguro → o 2º "Arrumar" vira no-op (log
//! limpo, sem animação à toa). Após aplicar, pode chamar `camera.reveal/fit` para N grande.
//!
//! **PEDIDO DE COSTURA (fronteira):** [`GRID_COLS`] aqui DEVE espelhar `bridge::grid_slot` (COLS=4).
//! O ideal é uma fonte única `pub const GRID_COLS` em `bridge.rs` que ambos leiam — mas `bridge.rs`
//! é do Maestro (fora da minha fronteira). Mantenho a const local + este aviso; a unificação está
//! registrada na `.entrega-f2-3-4.md`. Estilo `debug_assert!` de pré-condição igual ao `canvas_drag.rs`.

// Sem chamador no binário até a costura de `main.rs` (dono: Maestro) entrar; num crate-binário,
// `pub` sem consumidor é dead-code que o `clippy -D` reprova. Cada função é exercitada pelos testes
// abaixo (não-vacuosos). REMOVER este allow ao aplicar a fiação. (Mesmo idioma de canvas_drag/zoom.)
#![allow(dead_code)]

/// Colunas da grade "arrumada". **ESPELHA `bridge::grid_slot` (COLS=4)** — a grade que o usuário vê
/// ao abrir o app. Manter os dois em sincronia (pedido de unificação numa fonte única em `bridge.rs`
/// registrado na entrega; `bridge.rs` é do Maestro). Trocar para grade quadrada-ish é 1 linha:
/// ver `cols_square` documentada nos testes (graft do painel de design — NÃO ativa por default).
pub const GRID_COLS: usize = 4;

/// Posições (canto superior-esquerdo, em coords de MUNDO) de uma **grade limpa de 4 colunas**,
/// row-major, a partir de `origin`, com passo `card+gap`. `i=0` cai exatamente em `origin`.
/// `n=0` ⇒ vazio. Para `n<4` a função emite só os `n` cards numa única fileira (sem slot vazio à
/// direita): "coluna fantasma" é impossível por construção — geramos posições para `0..n`, nunca
/// preenchemos uma grade fixa. (Por isso NÃO há `min(GRID_COLS, n)`: seria um no-op.)
#[must_use]
pub fn arrange_grid(
    n: usize,
    card_w: f32,
    card_h: f32,
    gap: f32,
    origin: (f32, f32),
) -> Vec<(f32, f32)> {
    debug_assert!(gap >= 0.0, "arrange_grid: gap não-negativo (CARD_GAP=30)");
    if n == 0 {
        return Vec::new();
    }
    let cols = GRID_COLS;
    let (step_x, step_y) = (card_w + gap, card_h + gap);
    (0..n)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            (
                origin.0 + col as f32 * step_x,
                origin.1 + row as f32 * step_y,
            )
        })
        .collect()
}

/// Posições de uma fileira "lado a lado": UMA linha horizontal reta (mesmo `y = origin.1`, `x`
/// estritamente crescente), **nunca enrola**. `n=0` ⇒ vazio; `n=1` ⇒ `[origin]`. `card_h` entra na
/// assinatura por simetria de contrato (não é usado: `y` é constante — invariante "não redimensiona").
#[must_use]
pub fn arrange_side_by_side(
    n: usize,
    card_w: f32,
    card_h: f32,
    gap: f32,
    origin: (f32, f32),
) -> Vec<(f32, f32)> {
    debug_assert!(gap >= 0.0, "arrange_side_by_side: gap não-negativo");
    let _ = card_h; // simetria de contrato; y é constante (sem resize, sem segunda fileira).
    let step_x = card_w + gap;
    (0..n)
        .map(|i| (origin.0 + i as f32 * step_x, origin.1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures = constantes reais do app (bridge.rs): CARD_W/H/GAP e a origem do grid_slot.
    const W: f32 = 680.0;
    const H: f32 = 500.0;
    const G: f32 = 30.0;
    const ORIGIN: (f32, f32) = (30.0, 96.0); // MARGIN_X / MARGIN_Y do grid_slot

    /// Réplica da fórmula de `bridge::grid_slot(k)` (4 colunas) — não posso importar `bridge` no
    /// harness de isolamento, então comparo contra a fórmula DOCUMENTADA, valor a valor.
    fn grid_slot_replica(k: usize) -> (f32, f32) {
        let (cols, step_x, step_y) = (4usize, W + G, H + G);
        (
            ORIGIN.0 + (k % cols) as f32 * step_x,
            ORIGIN.1 + (k / cols) as f32 * step_y,
        )
    }

    /// Predicado de não-sobreposição: dois cards (cantos sup-esq) não se sobrepõem se estão
    /// separados em x por >= card_w OU em y por >= card_h (gap=0 ⇒ encostam, sem overlap de interior).
    fn no_overlap(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() >= W || (a.1 - b.1).abs() >= H
    }

    #[test]
    fn grid_n0_vazio() {
        assert!(arrange_grid(0, W, H, G, ORIGIN).is_empty());
    }

    #[test]
    fn grid_n1_eq_origin() {
        assert_eq!(arrange_grid(1, W, H, G, ORIGIN), vec![ORIGIN]);
    }

    #[test]
    fn grid_origin_respeitada() {
        let o = (500.0, 300.0);
        assert_eq!(
            arrange_grid(7, W, H, G, o)[0],
            o,
            "pos[0] deve ser exatamente origin"
        );
        assert_eq!(arrange_side_by_side(7, W, H, G, o)[0], o);
    }

    #[test]
    fn grid_no_overlap_pairwise() {
        for &n in &[2usize, 3, 4, 5, 6, 7, 8, 12, 40] {
            let p = arrange_grid(n, W, H, G, ORIGIN);
            assert_eq!(p.len(), n, "n={n} ⇒ n posições");
            for i in 0..p.len() {
                for j in (i + 1)..p.len() {
                    assert!(no_overlap(p[i], p[j]), "overlap n={n} entre {i} e {j}");
                }
            }
        }
    }

    #[test]
    fn grid_fixed4_cols_trava_estrategia() {
        // n=5: row-major 4 colunas ⇒ EXATAMENTE 4 valores de x distintos (sqrt daria 3). Trap anti-sqrt.
        let p = arrange_grid(5, W, H, G, ORIGIN);
        let mut xs: Vec<i64> = p.iter().map(|q| (q.0 * 100.0) as i64).collect();
        xs.sort_unstable();
        xs.dedup();
        assert_eq!(
            xs.len(),
            4,
            "fixed-4: n=5 tem 4 colunas distintas (sqrt teria 3)"
        );
    }

    #[test]
    fn grid_reproduz_grid_slot_pixel_a_pixel() {
        // Com a origem do init, arrange_grid == grid_slot(k) para todo k — "Arrumar = voltar à abertura".
        let p = arrange_grid(7, W, H, G, ORIGIN);
        for (k, &pos) in p.iter().enumerate() {
            assert_eq!(pos, grid_slot_replica(k), "divergiu do grid_slot em k={k}");
        }
    }

    #[test]
    fn grid_ultima_fileira_parcial_alinha_esquerda() {
        // n=6, cols=4: índices 4,5 caem na fileira 1, colunas 0 e 1 (sem centralizar, sem buraco).
        let p = arrange_grid(6, W, H, G, ORIGIN);
        assert_eq!(p[4], (ORIGIN.0, ORIGIN.1 + (H + G)));
        assert_eq!(p[5], (ORIGIN.0 + (W + G), ORIGIN.1 + (H + G)));
    }

    #[test]
    fn grid_small_n_single_row() {
        // n=3 (< 4 colunas): UMA fileira de 3 (todos em y=origin.1), 3 colunas distintas. Discrimina
        // o número de colunas: com cols=2 (ou ceil(sqrt(3))=2) o índice 2 cairia na 2ª fileira ⇒ falha.
        let p = arrange_grid(3, W, H, G, ORIGIN);
        assert!(
            p.iter().all(|q| (q.1 - ORIGIN.1).abs() < 1e-4),
            "n=3 deve ser 1 fileira (sem wrap prematuro)"
        );
        let mut xs: Vec<i64> = p.iter().map(|q| (q.0 * 100.0) as i64).collect();
        xs.sort_unstable();
        xs.dedup();
        assert_eq!(xs.len(), 3, "n=3 ⇒ 3 colunas distintas");
    }

    #[test]
    fn grid_value_snapshot_e_deterministico() {
        // Trava VALORES exatos (não só repetibilidade f(x)==f(x)): pega qualquer mutação da fórmula
        // de passo/origem/row-major. n=6 cobre 1ª fileira cheia + início da 2ª.
        let p = arrange_grid(6, W, H, G, ORIGIN);
        let esperado = vec![
            (ORIGIN.0, ORIGIN.1),
            (ORIGIN.0 + (W + G), ORIGIN.1),
            (ORIGIN.0 + 2.0 * (W + G), ORIGIN.1),
            (ORIGIN.0 + 3.0 * (W + G), ORIGIN.1),
            (ORIGIN.0, ORIGIN.1 + (H + G)),
            (ORIGIN.0 + (W + G), ORIGIN.1 + (H + G)),
        ];
        assert_eq!(p, esperado, "snapshot de posições da grade 4-col");
        assert_eq!(p, arrange_grid(6, W, H, G, ORIGIN), "determinístico");
    }

    #[test]
    fn sbs_n0_vazio_e_n1_origin() {
        assert!(arrange_side_by_side(0, W, H, G, ORIGIN).is_empty());
        assert_eq!(arrange_side_by_side(1, W, H, G, ORIGIN), vec![ORIGIN]);
    }

    #[test]
    fn sbs_uma_fileira_mesmo_y() {
        // Trava o invariante "lado a lado = 1 linha": todos com o MESMO y, nunca enrola.
        let p = arrange_side_by_side(20, W, H, G, ORIGIN);
        assert!(
            p.iter().all(|q| (q.1 - ORIGIN.1).abs() < 1e-4),
            "y deve ser constante"
        );
    }

    #[test]
    fn sbs_x_estritamente_crescente() {
        let p = arrange_side_by_side(20, W, H, G, ORIGIN);
        assert!(
            p.windows(2).all(|w| w[1].0 > w[0].0),
            "x sempre cresce (sem wrap)"
        );
    }

    #[test]
    fn sbs_no_overlap() {
        let p = arrange_side_by_side(15, W, H, G, ORIGIN);
        for i in 0..p.len() {
            for j in (i + 1)..p.len() {
                assert!(no_overlap(p[i], p[j]), "overlap side_by_side {i}/{j}");
            }
        }
    }

    #[test]
    fn invariante_sem_resize_pos0_independe_de_wh() {
        // pos[0] == origin para QUALQUER w/h ⇒ o tamanho não vaza para a posição (só reposiciona).
        assert_eq!(arrange_grid(3, 1.0, 1.0, G, ORIGIN)[0], ORIGIN);
        assert_eq!(arrange_grid(3, 9999.0, 9999.0, G, ORIGIN)[0], ORIGIN);
    }

    #[test]
    fn gap_zero_encosta_sem_overlap() {
        // gap=0: cards encostam (passo = card_w/card_h); bordas coincidem, sem overlap de interior.
        let p = arrange_grid(8, W, H, 0.0, ORIGIN);
        for i in 0..p.len() {
            for j in (i + 1)..p.len() {
                assert!(no_overlap(p[i], p[j]), "gap=0 não deve sobrepor {i}/{j}");
            }
        }
    }
}
