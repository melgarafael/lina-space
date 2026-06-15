//! `canvas_snap` — **F2-3-3: encaixe passivo (snap) ao arrastar um card (lógica PURA, gpui-free).**
//!
//! Ao arrastar um terminal para perto de outro, ele "imanta" num alinhamento limpo — bordas
//! coincidentes ou o respiro canônico (`CARD_GAP`) entre cards — sem NUNCA pular sozinho. É o que
//! faz a organização parecer de Figma. Complementa o mover (F2-3-1, `canvas_drag.rs`): o `CardDrag`
//! produz o preview cru; este módulo ajusta esse preview ao soltar/durante o move.
//!
//! **Modelo (snap por EIXO independente — o segredo do "imã" do Figma):** X e Y são resolvidos em
//! separado. Para cada eixo, cada outro card gera linhas de encaixe candidatas; a função escolhe a
//! mais próxima da coordenada arrastada DENTRO do `threshold` e ajusta SÓ aquela coordenada (a outra
//! fica livre). Tamanho de card é uniforme hoje (`CARD_W`/`CARD_H` fixos), então alinhar
//! borda-esquerda/borda-direita/centro colapsa numa única candidata (`base`); as outras duas são o
//! gap canônico de cada lado (`base ± (span + gap)`) — encostar o card à direita ou à esquerda do
//! vizinho com o respiro padrão.
//!
//! **Threshold em TELA, escalado por zoom:** o "perto o bastante" é **8px de TELA**. A costura passa
//! `threshold_world = 8.0 / zoom` (8px de tela = menos mundo quando ampliado) — a função recebe o
//! valor de MUNDO e fica pura, agnóstica de câmera.
//!
//! **Invariantes (passivo):** a função é total, determinística e SEM efeito colateral — só
//! *sugere* uma posição. Quem aplica é a costura, e SÓ durante um gesto ativo (`CardDrag`): nada se
//! move sozinho. **viewport-only:** a costura passa em `others` apenas os cards relevantes (visíveis,
//! exceto o próprio arrastado); a função não filtra — é total sobre o que recebe.

#![allow(dead_code)] // consumidor é a costura de `mouse_move`/`up` em main.rs (dono: Maestro); exercitado por teste.

/// **Encaixa a posição arrastada nos alinhamentos dos outros cards.** `dragged_xy` é o canto
/// superior-esquerdo (espaço de MUNDO) do card sob arrasto; `others` são os cantos sup-esq dos
/// cards de referência (a costura já filtrou: visíveis, sem o próprio). `card_w`/`card_h` = tamanho
/// (uniforme); `gap` = respiro canônico (`CARD_GAP`); `threshold_world` = raio de imã em mundo
/// (`8.0 / zoom`). Retorna a posição AJUSTADA, ou a original em qualquer eixo sem candidato perto.
/// Determinística e total — nenhum `others` ⇒ devolve `dragged_xy` intacto (no-op).
#[must_use]
pub fn snap_position(
    dragged_xy: (f32, f32),
    others: &[(f32, f32)],
    card_w: f32,
    card_h: f32,
    gap: f32,
    threshold_world: f32,
) -> (f32, f32) {
    let (dx, dy) = dragged_xy;
    let snapped_x = best_snap(
        dx,
        others
            .iter()
            .flat_map(|o| axis_candidates(o.0, card_w, gap)),
        threshold_world,
    )
    .unwrap_or(dx);
    let snapped_y = best_snap(
        dy,
        others
            .iter()
            .flat_map(|o| axis_candidates(o.1, card_h, gap)),
        threshold_world,
    )
    .unwrap_or(dy);
    (snapped_x, snapped_y)
}

/// As 3 linhas de encaixe que um card de referência oferece num eixo: alinhar bordas/centro
/// (`base` — colapsam com tamanho uniforme) e encostar com o respiro canônico de cada lado
/// (`base ± (span + gap)`). `span` = `card_w` no eixo X, `card_h` no eixo Y.
fn axis_candidates(base: f32, span: f32, gap: f32) -> [f32; 3] {
    [base, base + span + gap, base - span - gap]
}

/// A candidata mais próxima de `coord` dentro de `threshold`, ou `None` se nenhuma encaixa.
/// Desempate determinístico: menor distância primeiro, depois menor valor de candidato. Usa
/// `f32::total_cmp` (ordem total, sem `unwrap`/`Option` por NaN); `threshold < 0` nunca casa.
fn best_snap(coord: f32, candidates: impl Iterator<Item = f32>, threshold: f32) -> Option<f32> {
    candidates
        .filter(|c| (c - coord).abs() <= threshold)
        .min_by(|a, b| {
            (a - coord)
                .abs()
                .total_cmp(&(b - coord).abs())
                .then(a.total_cmp(b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Valores canônicos do canvas (bridge.rs ~140) — para os testes baterem com a realidade.
    const CARD_W: f32 = 680.0;
    const CARD_H: f32 = 500.0;
    const GAP: f32 = 30.0;
    const THRESH: f32 = 8.0; // 8px de tela @ zoom 1.0

    /// Sem outros cards = no-op: devolve a posição arrastada intacta (não inventa encaixe do nada).
    #[test]
    fn no_others_is_noop() {
        let p = (123.0, 456.0);
        assert_eq!(snap_position(p, &[], CARD_W, CARD_H, GAP, THRESH), p);
    }

    /// Dentro do threshold, a borda imanta EXATAMENTE na coluna/linha do vizinho (alinhamento limpo).
    #[test]
    fn snaps_edges_within_threshold() {
        let other = (1000.0, 2000.0);
        // arrastado 5px fora do alinhamento de bordas em ambos os eixos (5 < 8)
        let dragged = (1005.0, 1995.0);
        let snapped = snap_position(dragged, &[other], CARD_W, CARD_H, GAP, THRESH);
        assert_eq!(
            snapped,
            (1000.0, 2000.0),
            "imanta exato na borda do vizinho"
        );
    }

    /// Imanta no RESPIRO canônico: encostar à direita do vizinho fica a `CARD_W + GAP`, gap exato.
    #[test]
    fn snaps_to_canonical_gap() {
        let other = (1000.0, 2000.0);
        let target_x = other.0 + CARD_W + GAP; // 1710 — lado a lado, respiro padrão
        let dragged = (target_x + 6.0, 2003.0); // 6px fora no x, 3px fora no y (ambos < 8)
        let snapped = snap_position(dragged, &[other], CARD_W, CARD_H, GAP, THRESH);
        assert_eq!(
            snapped,
            (target_x, 2000.0),
            "gap = CARD_GAP exato; y alinha topo"
        );
        assert_eq!(
            snapped.0 - other.0,
            CARD_W + GAP,
            "respiro é exatamente CARD_GAP"
        );
    }

    /// Fora do threshold NÃO encaixa — o card fica onde o usuário largou (imã não puxa de longe).
    #[test]
    fn no_snap_outside_threshold() {
        let other = (1000.0, 2000.0);
        let dragged = (1020.0, 2025.0); // 20/25px fora — além de 8
        assert_eq!(
            snap_position(dragged, &[other], CARD_W, CARD_H, GAP, THRESH),
            dragged,
            "longe demais: nada se move"
        );
    }

    /// O threshold escala com o zoom (8/zoom): o MESMO offset encaixa ampliado (threshold maior em
    /// mundo) e NÃO encaixa reduzido — prova o `threshold_world` (não-vacuoso: fixar o threshold quebra).
    #[test]
    fn threshold_scales_with_zoom() {
        let other = (1000.0, 2000.0);
        let dragged = (1012.0, 2000.0); // 12px de mundo fora do alinhamento de bordas

        // zoom 0.5 → threshold_world = 16 > 12 → encaixa
        let zoomed_out = snap_position(dragged, &[other], CARD_W, CARD_H, GAP, 8.0 / 0.5);
        assert_eq!(zoomed_out.0, 1000.0, "8/0.5=16 > 12 → imanta");

        // zoom 2.0 → threshold_world = 4 < 12 → NÃO encaixa
        let zoomed_in = snap_position(dragged, &[other], CARD_W, CARD_H, GAP, 8.0 / 2.0);
        assert_eq!(zoomed_in.0, 1012.0, "8/2=4 < 12 → fica onde largou");
    }

    /// Eixos independentes: x pode imantar enquanto y permanece livre (e vice-versa).
    #[test]
    fn axes_are_independent() {
        let other = (1000.0, 2000.0);
        let dragged = (1004.0, 2050.0); // x 4px (encaixa), y 50px (não)
        let snapped = snap_position(dragged, &[other], CARD_W, CARD_H, GAP, THRESH);
        assert_eq!(snapped, (1000.0, 2050.0), "x imanta, y fica onde estava");
    }

    /// Entre duas candidatas dentro do threshold, escolhe a MAIS PRÓXIMA (determinístico).
    #[test]
    fn picks_nearest_candidate() {
        // dois vizinhos cujas colunas (x) ficam a 3px e a 6px do arrastado — pega a de 3px.
        let near = (1003.0, 5000.0); // |1003-1000| = 3
        let far = (994.0, 9000.0); // |994-1000| = 6
        let dragged = (1000.0, 0.0);
        let snapped = snap_position(dragged, &[far, near], CARD_W, CARD_H, GAP, THRESH);
        assert_eq!(snapped.0, 1003.0, "imanta na coluna mais próxima");
    }

    /// Passivo de verdade: se já está exatamente alinhado, o resultado é idêntico (sem jitter).
    #[test]
    fn already_aligned_is_stable() {
        let other = (1000.0, 2000.0);
        let dragged = (1000.0, 2000.0);
        assert_eq!(
            snap_position(dragged, &[other], CARD_W, CARD_H, GAP, THRESH),
            dragged
        );
    }

    /// `axis_candidates` gera as 3 linhas certas: a coluna do vizinho e o respiro de cada lado.
    #[test]
    fn axis_candidates_are_base_and_canonical_gaps() {
        assert_eq!(
            axis_candidates(1000.0, CARD_W, GAP),
            [1000.0, 1710.0, 290.0]
        );
    }
}
