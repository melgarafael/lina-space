//! `canvas_zoom` — **F2-3-5: zoom inteligente do canvas (fit / selection / LOD com histerese)**.
//!
//! Norte (onda F2-3): o fundador aperta uma tecla e vê TODOS os terminais enquadrados (nunca
//! perdido no vazio), ou foca a seleção; e ao afastar o zoom os cards SIMPLIFICAM sem sumir.
//!
//! **O que vive aqui (gpui-free, headless-testável):**
//! - [`Rect`] + [`bounds_of`]: o bounding box (em coords de MUNDO) da union dos cards.
//! - [`zoom_to_fit`] / [`zoom_to_selection`]: dado um `Rect` + a viewport, calculam a [`Camera`]
//!   (pan+zoom) que enquadra aquele retângulo com respiro, na área útil abaixo da topbar,
//!   clampada em `[zoom_min, zoom_max]`. Selection é o MESMO enquadramento aplicado à seleção —
//!   a diferença está em QUEM monta o `Rect` no call-site, não na matemática.
//! - [`Lod`] + [`lod_for`]: o nível de detalhe (2 degraus) derivado do zoom, com HISTERESE
//!   (limiar de subida ≠ de descida) para não piscar perto da fronteira.
//!
//! **Invariante da onda:** a câmera nunca anda sem gesto. Estas funções só CALCULAM; quem aplica
//! em `self.camera` é o handler de tecla (costura do Maestro em `main.rs`). `zoom_to_fit` é gesto
//! pontual (a costura PODE animar suave); o LOD é estado DERIVADO do zoom (sem animação por frame).
//!
//! **LOD degrada fidelidade, NUNCA remove a camada de composição.** [`Lod::Compact`] é um pedido
//! ao render para simplificar o conteúdo do card (menos texto/cromo) — jamais para deixar de
//! desenhar o card ou seu slot portal (critério R3 do épico). Quem honra isso é o render.
//!
//! Fonte única: [`Camera`], [`ZOOM_MIN`], [`ZOOM_MAX`] vêm de [`crate::bridge`] — o mesmo tipo que
//! o render, o culling e o hit-test usam. A constante [`TOPBAR`] espelha a de `Camera::reveal` /
//! `canvas_focus::zoom_to_focus` (manter em sincronia: é a faixa superior que a barra cobre).

// Fiação (costura do Maestro em `main.rs`): ⌘1 `zoom_to_fit` / ⌘2 `zoom_to_selection` JÁ aplicados
// (`handle_key`). O LOD (`Lod`/`lod_for`) fica para a PASSADA DE TELA (F2-3-5b): "compacto" é uma
// decisão visual do fundador (o que o card simplifica ao afastar), não se inventa headless. Por isso
// o allow permanece até o LOD ser fiado no render — então removê-lo expõe qualquer dead-code real.
#![allow(dead_code)]

use crate::bridge::Camera;

/// Faixa superior coberta pela barra do app — área útil começa abaixo dela. MESMO valor de
/// `Camera::reveal` (bridge.rs) e `canvas_focus::zoom_to_focus`: manter os três em sincronia.
const TOPBAR: f32 = 44.0;

/// Retângulo em coords de MUNDO: canto superior-esquerdo (`x`, `y`) + tamanho (`w`, `h`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Centro do retângulo (em mundo).
    #[must_use]
    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Bounding box (union) dos cards: cada card é o retângulo `(x, y, card_w, card_h)`. `None` quando
/// não há cards — o call-site trata como no-op (não há o que enquadrar; a câmera fica como está).
#[must_use]
pub fn bounds_of(cards: &[(f32, f32)], card_w: f32, card_h: f32) -> Option<Rect> {
    if cards.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &(x, y) in cards {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + card_w);
        max_y = max_y.max(y + card_h);
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    })
}

/// Câmera (pan+zoom) que ENQUADRA `bounds` na área útil da `viewport` (abaixo da topbar), com
/// `padding` px de respiro em cada borda, zoom clampado em `[zoom_min, zoom_max]` e o centro do
/// retângulo no centro da área útil. Se o conteúdo não couber nem no `zoom_min` (maior que a
/// janela), fica centrado — overflow simétrico, nunca perdido no vazio.
#[must_use]
pub fn zoom_to_fit(
    bounds: Rect,
    viewport: (f32, f32),
    padding: f32,
    zoom_min: f32,
    zoom_max: f32,
) -> Camera {
    debug_assert!(
        zoom_min > 0.0 && zoom_min <= zoom_max,
        "zoom_to_fit: faixa de zoom inválida (min={zoom_min}, max={zoom_max})"
    );
    let avail_w = (viewport.0 - 2.0 * padding).max(1.0);
    let avail_h = (viewport.1 - TOPBAR - 2.0 * padding).max(1.0);
    let (bw, bh) = (bounds.w.max(1.0), bounds.h.max(1.0));
    let zoom = (avail_w / bw).min(avail_h / bh).clamp(zoom_min, zoom_max);
    let area_center = (padding + avail_w / 2.0, TOPBAR + padding + avail_h / 2.0);
    let (bcx, bcy) = bounds.center();
    Camera {
        pan: (area_center.0 - bcx * zoom, area_center.1 - bcy * zoom),
        zoom,
    }
}

/// Foca a SELEÇÃO: o mesmo enquadramento de [`zoom_to_fit`] aplicado ao bounding box da seleção
/// (1+ cards). Função própria por semântica no call-site — a matemática é idêntica de propósito
/// (enquadrar um retângulo é enquadrar um retângulo).
#[must_use]
#[inline]
pub fn zoom_to_selection(
    sel_bounds: Rect,
    viewport: (f32, f32),
    padding: f32,
    zoom_min: f32,
    zoom_max: f32,
) -> Camera {
    zoom_to_fit(sel_bounds, viewport, padding, zoom_min, zoom_max)
}

/// Nível de detalhe do card. Degrada fidelidade do CONTEÚDO, nunca remove a camada de composição.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lod {
    /// Conteúdo cheio (zoom de leitura). É o default — o canvas nasce em zoom 1.0.
    #[default]
    Full,
    /// Conteúdo simplificado (zoom afastado): menos texto/cromo, mas o card e seu slot continuam.
    Compact,
}

/// Limiar de SUBIDA a [`Lod::Full`]: só vira Full quando o zoom passa disto.
const LOD_UP: f32 = 0.75;
/// Limiar de DESCIDA a [`Lod::Compact`]: só vira Compact quando o zoom cai abaixo disto.
/// `LOD_DOWN < LOD_UP` cria a faixa morta `[LOD_DOWN, LOD_UP]` que mantém o nível anterior.
const LOD_DOWN: f32 = 0.6;

/// Nível de detalhe para `zoom`, com HISTERESE: acima de [`LOD_UP`] → `Full`; abaixo de
/// [`LOD_DOWN`] → `Compact`; ENTRE eles mantém `last` (faixa morta — não pisca na fronteira).
#[must_use]
pub fn lod_for(zoom: f32, last: Lod) -> Lod {
    if zoom >= LOD_UP {
        Lod::Full
    } else if zoom <= LOD_DOWN {
        Lod::Compact
    } else {
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{ZOOM_MAX, ZOOM_MIN};

    const PAD: f32 = 40.0;

    /// Aplica a câmera a um ponto de mundo (mesma transform do render: `screen = world*zoom + pan`).
    fn to_screen(cam: &Camera, w: (f32, f32)) -> (f32, f32) {
        (w.0 * cam.zoom + cam.pan.0, w.1 * cam.zoom + cam.pan.1)
    }

    #[test]
    fn bounds_of_unions_all_cards() {
        let cards = [(0.0, 0.0), (1000.0, 600.0), (-200.0, 300.0)];
        let b = bounds_of(&cards, 680.0, 500.0).expect("3 cards têm bounds");
        // min canto = (-200, 0); max canto = (1000+680, 600+500) = (1680, 1100).
        assert!((b.x - (-200.0)).abs() < 1e-3, "x={}", b.x);
        assert!((b.y - 0.0).abs() < 1e-3, "y={}", b.y);
        assert!((b.w - (1680.0 - -200.0)).abs() < 1e-3, "w={}", b.w);
        assert!((b.h - 1100.0).abs() < 1e-3, "h={}", b.h);
    }

    #[test]
    fn bounds_of_empty_is_none() {
        // No-op de 0 cards: não há o que enquadrar (a câmera fica como está no call-site).
        assert_eq!(bounds_of(&[], 680.0, 500.0), None);
    }

    #[test]
    fn fit_encloses_every_card_inside_usable_area() {
        // Conteúdo que cabe sem bater no piso de zoom → enquadramento real, verificável.
        let cards = [(0.0, 0.0), (1000.0, 600.0)];
        let (cw, ch) = (680.0, 500.0);
        let viewport = (1400.0, 1000.0);
        let b = bounds_of(&cards, cw, ch).expect("2 cards");
        let cam = zoom_to_fit(b, viewport, PAD, ZOOM_MIN, ZOOM_MAX);
        // O zoom de fit caiu DENTRO da faixa (não saturou) — senão o teste seria vácuo.
        assert!(
            cam.zoom > ZOOM_MIN && cam.zoom < ZOOM_MAX,
            "fit deveria caber sem clamp, zoom={}",
            cam.zoom
        );
        // Todo canto de todo card cai dentro da viewport e abaixo da topbar.
        for &(x, y) in &cards {
            for corner in [(x, y), (x + cw, y + ch)] {
                let (sx, sy) = to_screen(&cam, corner);
                assert!(
                    sx >= 0.0 && sx <= viewport.0,
                    "canto x fora: {sx} (vp {})",
                    viewport.0
                );
                assert!(
                    sy >= TOPBAR && sy <= viewport.1,
                    "canto y fora/atrás da topbar: {sy}"
                );
            }
        }
    }

    #[test]
    fn fit_saturates_at_zoom_max_for_tiny_content() {
        // Um card pequeno numa viewport enorme: não aproxima além de ZOOM_MAX.
        let b = bounds_of(&[(0.0, 0.0)], 680.0, 500.0).expect("1 card");
        let cam = zoom_to_fit(b, (4000.0, 3000.0), PAD, ZOOM_MIN, ZOOM_MAX);
        assert!(
            (cam.zoom - ZOOM_MAX).abs() < 1e-4,
            "zoom satura em ZOOM_MAX, got {}",
            cam.zoom
        );
    }

    #[test]
    fn fit_saturates_at_zoom_min_for_huge_content() {
        // Conteúdo gigante: afasta só até ZOOM_MIN (não some por completo).
        let b = Rect {
            x: 0.0,
            y: 0.0,
            w: 40000.0,
            h: 30000.0,
        };
        let cam = zoom_to_fit(b, (1200.0, 800.0), PAD, ZOOM_MIN, ZOOM_MAX);
        assert!(
            (cam.zoom - ZOOM_MIN).abs() < 1e-4,
            "zoom satura em ZOOM_MIN, got {}",
            cam.zoom
        );
    }

    #[test]
    fn fit_centers_bounds_in_usable_area() {
        let b = Rect {
            x: 100.0,
            y: 100.0,
            w: 600.0,
            h: 400.0,
        };
        let viewport = (1400.0, 1000.0);
        let cam = zoom_to_fit(b, viewport, PAD, ZOOM_MIN, ZOOM_MAX);
        let (scx, scy) = to_screen(&cam, b.center());
        let avail_w = viewport.0 - 2.0 * PAD;
        let avail_h = viewport.1 - TOPBAR - 2.0 * PAD;
        let (ecx, ecy) = (PAD + avail_w / 2.0, TOPBAR + PAD + avail_h / 2.0);
        assert!((scx - ecx).abs() < 1e-2, "centro x: {scx} vs {ecx}");
        assert!((scy - ecy).abs() < 1e-2, "centro y: {scy} vs {ecy}");
    }

    #[test]
    fn selection_matches_fit_for_same_rect() {
        let b = Rect {
            x: 10.0,
            y: 20.0,
            w: 680.0,
            h: 500.0,
        };
        let vp = (1280.0, 800.0);
        assert_eq!(
            zoom_to_selection(b, vp, PAD, ZOOM_MIN, ZOOM_MAX),
            zoom_to_fit(b, vp, PAD, ZOOM_MIN, ZOOM_MAX)
        );
    }

    #[test]
    fn lod_full_above_up_compact_below_down() {
        assert_eq!(lod_for(LOD_UP + 0.01, Lod::Compact), Lod::Full);
        assert_eq!(lod_for(2.0, Lod::Compact), Lod::Full);
        assert_eq!(lod_for(LOD_DOWN - 0.01, Lod::Full), Lod::Compact);
        assert_eq!(lod_for(0.4, Lod::Full), Lod::Compact);
    }

    #[test]
    fn lod_hysteresis_keeps_last_in_dead_zone() {
        // No meio da faixa morta o nível NÃO troca — depende só do estado anterior.
        let mid = (LOD_UP + LOD_DOWN) / 2.0;
        assert!(
            mid > LOD_DOWN && mid < LOD_UP,
            "fixture: mid na faixa morta"
        );
        assert_eq!(lod_for(mid, Lod::Full), Lod::Full);
        assert_eq!(lod_for(mid, Lod::Compact), Lod::Compact);
    }

    #[test]
    fn lod_does_not_flip_when_creeping_up_through_dead_zone() {
        // Subindo de Compact: continua Compact em toda a faixa morta; só vira Full ao cruzar UP.
        let mut lod = Lod::Compact;
        for step in 0..=20 {
            let z = LOD_DOWN + (LOD_UP - LOD_DOWN) * (step as f32 / 20.0) - 1e-4;
            lod = lod_for(z, lod);
            assert_eq!(
                lod,
                Lod::Compact,
                "não deve subir antes de cruzar LOD_UP (z={z})"
            );
        }
        assert_eq!(
            lod_for(LOD_UP, lod),
            Lod::Full,
            "ao cruzar LOD_UP vira Full"
        );
    }
}
