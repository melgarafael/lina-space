//! `canvas_focus` — **F1-2-6: focus manager do canvas (widget composto + roving tabindex)**.
//!
//! Fonte (13.15 achado 6): o padrão WAI-ARIA APG de roving tabindex é o que o Figma usa em
//! canvas — o canvas é **UM** tab stop (widget composto); dentro dele, **exatamente 1 nó é
//! focável por vez** e as setas movem esse foco em ordem espacial estável. As ids únicas por
//! terminal (NodeId UUID) já existem; o que faltava era ESTE focus manager.
//!
//! **O que vive aqui (gpui-free, headless-testável):**
//! - [`FocusLevel`] + [`CanvasFocus`]: a máquina de NÍVEL — `Terminal` (teclas → PTY do nó
//!   focado; é o comportamento histórico do app e o default) ↔ `Canvas` (setas navegam entre
//!   nós; Enter desce ao terminal). O app NUNCA teve essa separação: hoje toda tecla não-atalho
//!   cai no PTY (`main.rs` `handle_key`, fallback `keystroke_to_bytes`).
//! - [`CanvasFocus::on_key`]: decide a INTENÇÃO ([`KeyIntent`]) da tecla no nível corrente e
//!   aplica a transição de nível. Quem executa o efeito (focar nó, mover câmera, mandar byte ao
//!   PTY) é a fiação fina no shell — o manager decide, não toca janela.
//! - [`reading_order`] / [`next_in_dir`] / [`next_busy`]: navegação espacial **determinística**
//!   (ordem de leitura `(y, x, id)`; o empate SEMPRE resolve por id — mesma entrada, mesma saída).
//! - [`roving_marks`]: o contrato do roving — dado o roster e o corrente, marca **exatamente um**
//!   nó como focável para a árvore AccessKit (o render aplica `.focusable()` ao marcado).
//! - [`zoom_to_focus`]: o alvo de câmera ao navegar — CENTRALIZA o nó focado na área útil
//!   (abaixo da topbar), **mantendo o zoom do usuário** (só reduz se o card não couber; nunca
//!   aproxima sozinho) — e `animate = !reduce_motion` (consome [`crate::a11y::reduce_motion_effective`],
//!   a fonte ÚNICA de W4-6: com reduce-motion, salto direto, sem animação — critério 3 da story).
//!
//! **⚠️ Política do Esc (registro honesto):** a story manda "Esc volta ao nível canvas" e é o
//! que [`CanvasFocus::on_key`] decide no nível `Terminal`. PORÉM o Esc é tecla VIVA nos CLIs
//! (no Claude Code, Esc interrompe o turno; no vim, sai do insert) — hoje ele vai ao PTY
//! (`keystroke_to_bytes` → `0x1b`). A máquina expõe a transição como comando nomeado
//! ([`KeyIntent::ExitToCanvas`]); QUAL tecla a dispara é decisão de 1 linha na fiação — se a
//! tela do fundador mostrar que perder o Esc-no-CLI dói, troca-se o gatilho (ex.: `Shift+Esc`)
//! sem tocar a máquina. O trade-off está no roteiro de tela e na entrega (arbitragem do Maestro).
//!
//! Âncoras: porta **`UiHost`** (o focus manager vive no shell; o core não conhece foco);
//! invariante #6 (a11y é P0). O nó corrente continua sendo `self.focused` do shell — fonte
//! única; este módulo NÃO o duplica (recebe o roster e o corrente por parâmetro).

// O consumo nasce na COSTURA em `main.rs` (dono externo nesta rodada — pedido registrado na
// `.entrega-f1-2-6.md`); num crate-binário, `pub` sem consumidor é dead_code. Mesmo idioma do
// `a11y::wcag`/`MAX_FOCUS` (reservado, com teste vivo). REMOVER este allow ao aplicar a fiação.
#![allow(dead_code)]

use std::collections::BTreeSet;

use lina_host::NodeId;

use crate::bridge::{Camera, ZOOM_MIN};

// ═══════════════════════════ máquina de nível (widget composto APG) ═══════════════════════════

/// Nível de foco do teclado. `Terminal` é o default — o comportamento histórico do app
/// (toda tecla dirige o PTY do nó focado) fica intacto até o usuário SUBIR ao canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusLevel {
    /// Teclas dirigem o TERMINAL focado (→ PTY). Esc sobe ao nível canvas (política da fonte —
    /// ver o doc do módulo sobre o trade-off com Esc-no-CLI).
    #[default]
    Terminal,
    /// Teclas dirigem o CANVAS: setas navegam entre nós (roving), Enter desce ao terminal,
    /// `n` abre Novo Agente, `o` alterna ao próximo ocupado. Digitação solta é IGNORADA
    /// (nunca vaza ao PTY de um terminal em que o usuário não "está").
    Canvas,
}

/// Direção de navegação espacial (setas no nível canvas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// A INTENÇÃO decidida para uma tecla — o manager decide, a fiação executa. Manter efeito fora
/// daqui é o que torna a máquina headless-testável (mesmo split lógica/render de `canvas.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyIntent {
    /// Nível canvas: mover o roving focus na direção (a fiação chama [`next_in_dir`] +
    /// [`zoom_to_focus`] e `self.focus(next)`).
    Navigate(Dir),
    /// Nível canvas → terminal (Enter): a digitação volta a fluir ao PTY do nó focado.
    EnterTerminal,
    /// Nível terminal → canvas (Esc, política da fonte): as setas passam a navegar.
    ExitToCanvas,
    /// Nível canvas, `o`: alternar ao próximo terminal OCUPADO (precisa de você / rodando) —
    /// atalho de produtividade da story (a fiação chama [`next_busy`]).
    NextBusy,
    /// Nível canvas, `n`: Novo Agente (mesmo fluxo M6 do ⌘N — a fiação reusa
    /// `open_agent_modal_create`).
    NewAgent,
    /// Nível terminal: a tecla segue o fluxo histórico (`keystroke_to_bytes` → PTY).
    PassToTerminal,
    /// Nível canvas: tecla sem mapeamento — CONSUMIDA (não vaza ao PTY, não navega).
    Ignore,
}

/// A máquina de nível do foco. Estado mínimo de propósito: o NÓ corrente é `self.focused`
/// do shell (fonte única — não duplicar); aqui vive só o que não existia: o nível.
#[derive(Debug, Default)]
pub struct CanvasFocus {
    level: FocusLevel,
}

impl CanvasFocus {
    /// Nível corrente (o render usa para diferenciar o anel de foco canvas × terminal).
    #[must_use]
    pub fn level(&self) -> FocusLevel {
        self.level
    }

    /// Decide a intenção da tecla no nível corrente e APLICA a transição de nível (Enter/Esc).
    /// `key` é a `Keystroke::key` do gpui ("up", "enter", "escape", "n"…); `chorded` = ⌘/Ctrl
    /// pressionado (chords nunca navegam: ou são atalhos globais — tratados ANTES na fiação —
    /// ou são bytes de controle do PTY, ex.: Ctrl+C).
    pub fn on_key(&mut self, key: &str, chorded: bool) -> KeyIntent {
        let intent = match self.level {
            FocusLevel::Terminal => {
                if !chorded && key == "escape" {
                    KeyIntent::ExitToCanvas
                } else {
                    KeyIntent::PassToTerminal
                }
            }
            FocusLevel::Canvas => {
                if chorded {
                    // Chord no nível canvas: NÃO vaza ao PTY (o usuário não "está" no terminal).
                    KeyIntent::Ignore
                } else {
                    match key {
                        "left" => KeyIntent::Navigate(Dir::Left),
                        "right" => KeyIntent::Navigate(Dir::Right),
                        "up" => KeyIntent::Navigate(Dir::Up),
                        "down" => KeyIntent::Navigate(Dir::Down),
                        "enter" | "return" => KeyIntent::EnterTerminal,
                        "n" => KeyIntent::NewAgent,
                        "o" => KeyIntent::NextBusy,
                        // Tab: o canvas é UM tab stop (widget composto) — sair dele é papel do
                        // tab-system da janela, não do manager. Esc: já está no topo (canvas).
                        _ => KeyIntent::Ignore,
                    }
                }
            }
        };
        match intent {
            KeyIntent::EnterTerminal => self.level = FocusLevel::Terminal,
            KeyIntent::ExitToCanvas => self.level = FocusLevel::Canvas,
            _ => {}
        }
        intent
    }
}

// ═══════════════════════════ navegação espacial determinística ═══════════════════════════

/// Ordena o roster em **ordem de leitura** `(y, x, id)` — estável e determinística (o id UUID
/// desempata posições idênticas; `total_cmp` evita o buraco de NaN). É a ordem que Left/Right
/// percorrem e a base do teste headless do critério 2.
#[must_use]
pub fn reading_order(nodes: &[(NodeId, f32, f32)]) -> Vec<NodeId> {
    let mut v: Vec<(NodeId, f32, f32)> = nodes.to_vec();
    v.sort_by(|a, b| {
        a.2.total_cmp(&b.2)
            .then(a.1.total_cmp(&b.1))
            .then(a.0.cmp(&b.0))
    });
    v.into_iter().map(|(id, _, _)| id).collect()
}

/// O próximo nó a partir de `current` na direção `dir`. Determinístico:
/// - `Right`/`Left`: próximo/anterior na [`reading_order`], **com wrap** — qualquer nó é
///   alcançável segurando uma única tecla (não-técnico-first: sem beco sem saída).
/// - `Up`/`Down`: o vizinho com `dy` estrito na direção, menor `(|dy|, |dx|, id)` — o mais
///   próximo "na vertical"; na borda devolve `None` (fica onde está).
///
/// `None` também quando `current` não está no roster (defensivo: o render já reaponta o
/// focado quando um nó morre — `main.rs`; aqui nunca se inventa um destino).
#[must_use]
pub fn next_in_dir(nodes: &[(NodeId, f32, f32)], current: NodeId, dir: Dir) -> Option<NodeId> {
    let (cx, cy) = nodes
        .iter()
        .find(|(id, _, _)| *id == current)
        .map(|(_, x, y)| (*x, *y))?;
    match dir {
        Dir::Right | Dir::Left => {
            let order = reading_order(nodes);
            let idx = order.iter().position(|id| *id == current)?;
            if order.len() < 2 {
                return None;
            }
            let next = match dir {
                Dir::Right => (idx + 1) % order.len(),
                _ => (idx + order.len() - 1) % order.len(),
            };
            Some(order[next])
        }
        Dir::Up | Dir::Down => nodes
            .iter()
            .filter(|(id, _, y)| {
                *id != current
                    && match dir {
                        Dir::Down => *y > cy,
                        _ => *y < cy,
                    }
            })
            .min_by(|(ida, xa, ya), (idb, xb, yb)| {
                (ya - cy)
                    .abs()
                    .total_cmp(&(yb - cy).abs())
                    .then((xa - cx).abs().total_cmp(&(xb - cx).abs()))
                    .then(ida.cmp(idb))
            })
            .map(|(id, _, _)| *id),
    }
}

/// O próximo terminal **ocupado** a partir de `current`, na ordem de leitura com wrap — o
/// atalho `o` da story ("alternar para próximo terminal ocupado"). `busy` é o conjunto que a
/// fiação computa do estado real (precisa-de-você / rodando — mesmos sinais do badge W4-2).
/// `None` quando nenhum OUTRO nó está ocupado (não navega para si próprio).
#[must_use]
pub fn next_busy(
    nodes: &[(NodeId, f32, f32)],
    busy: &BTreeSet<NodeId>,
    current: NodeId,
) -> Option<NodeId> {
    let order = reading_order(nodes);
    let idx = order.iter().position(|id| *id == current)?;
    (1..order.len())
        .map(|k| order[(idx + k) % order.len()])
        .find(|id| busy.contains(id))
}

// ═══════════════════════════ roving tabindex (contrato AccessKit) ═══════════════════════════

/// O contrato do roving para a árvore AccessKit: **exatamente um** nó do canvas é focável por
/// vez — o corrente. O render aplica `.focusable()` (gpui: `Interactivity.focusable` →
/// `node.add_action(Action::Focus)`) SÓ ao marcado `true`. Corrente fora do roster → todos
/// `false` (o render reaponta o focado no frame seguinte; nunca dois focáveis).
#[must_use]
pub fn roving_marks(nodes: &[(NodeId, f32, f32)], current: NodeId) -> Vec<(NodeId, bool)> {
    nodes
        .iter()
        .map(|(id, _, _)| (*id, *id == current))
        .collect()
}

// ═══════════════════════════ zoom-to-focus (critério 3: reduce-motion) ═══════════════════════════

/// Margens da área útil — MESMOS valores do `Camera::reveal` (bridge.rs): manter em sincronia.
const MARGIN: f32 = 16.0;
const TOPBAR: f32 = 44.0;

/// O movimento de câmera decidido por [`zoom_to_focus`]: o alvo + se deve ANIMAR. A fiação
/// aplica `camera` direto quando `animate == false` (salto — reduce-motion, critério 3) e
/// interpola até o alvo quando `true`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraMove {
    pub camera: Camera,
    pub animate: bool,
}

/// O alvo de câmera ao navegar o foco: CENTRALIZA o card na área útil (abaixo da topbar),
/// **mantendo o zoom do usuário** — só reduz se o card não couber (nunca aproxima sozinho:
/// navegação com setas não deve "bombar" o zoom de quem afastou a vista). `reduce_motion` vem
/// de [`crate::a11y::reduce_motion_effective`] (fonte única) e decide salto × animação.
#[must_use]
pub fn zoom_to_focus(
    card_world: (f32, f32),
    card_size: (f32, f32),
    viewport: (f32, f32),
    current: Camera,
    reduce_motion: bool,
) -> CameraMove {
    let avail_w = (viewport.0 - 2.0 * MARGIN).max(1.0);
    let avail_h = (viewport.1 - TOPBAR - 2.0 * MARGIN).max(1.0);
    let fit = (avail_w / card_size.0.max(1.0)).min(avail_h / card_size.1.max(1.0));
    let zoom = current
        .zoom
        .min(fit)
        .clamp(ZOOM_MIN, current.zoom.max(ZOOM_MIN));
    let center = (MARGIN + avail_w / 2.0, TOPBAR + MARGIN + avail_h / 2.0);
    let card_center = (
        card_world.0 + card_size.0 / 2.0,
        card_world.1 + card_size.1 / 2.0,
    );
    let pan = (
        center.0 - card_center.0 * zoom,
        center.1 - card_center.1 * zoom,
    );
    CameraMove {
        camera: Camera { pan, zoom },
        animate: !reduce_motion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids(n: usize) -> Vec<NodeId> {
        // now_v7 é monotônico — ids[0] < ids[1] < … (tiebreak determinístico nos testes).
        (0..n).map(|_| Uuid::now_v7()).collect()
    }

    /// Critério 2 (ordem de navegação): a ordem de leitura é (y, x, id) — estável e
    /// determinística, inclusive com nó "solto" fora do grid.
    #[test]
    fn reading_order_is_y_then_x_then_id() {
        let v = ids(5);
        let nodes = vec![
            (v[3], 100.0, 0.0), // linha 0, direita
            (v[0], 0.0, 0.0),   // linha 0, esquerda
            (v[4], 50.0, 30.0), // "solto" entre as linhas
            (v[2], 100.0, 100.0),
            (v[1], 0.0, 100.0),
        ];
        assert_eq!(reading_order(&nodes), vec![v[0], v[3], v[4], v[1], v[2]]);
        // Determinismo: mesma entrada, mesma saída (duas chamadas idênticas).
        assert_eq!(reading_order(&nodes), reading_order(&nodes));
    }

    /// Posições IDÊNTICAS desempatam por id — nunca depende de ordem de inserção.
    #[test]
    fn identical_positions_tiebreak_by_id() {
        let v = ids(2);
        let nodes = vec![(v[1], 10.0, 10.0), (v[0], 10.0, 10.0)];
        assert_eq!(reading_order(&nodes), vec![v[0], v[1]]);
    }

    /// Right/Left percorrem a ordem de leitura COM wrap — todo nó alcançável com uma tecla.
    #[test]
    fn right_left_walk_reading_order_with_wrap() {
        let v = ids(4);
        // grid 2×2: a(0,0) b(100,0) / c(0,100) d(100,100)
        let nodes = vec![
            (v[0], 0.0, 0.0),
            (v[1], 100.0, 0.0),
            (v[2], 0.0, 100.0),
            (v[3], 100.0, 100.0),
        ];
        assert_eq!(next_in_dir(&nodes, v[0], Dir::Right), Some(v[1]));
        assert_eq!(
            next_in_dir(&nodes, v[1], Dir::Right),
            Some(v[2]),
            "fim da linha desce"
        );
        assert_eq!(
            next_in_dir(&nodes, v[3], Dir::Right),
            Some(v[0]),
            "wrap total"
        );
        assert_eq!(
            next_in_dir(&nodes, v[0], Dir::Left),
            Some(v[3]),
            "wrap reverso"
        );
    }

    /// Up/Down vão ao vizinho VERTICAL mais próximo (|dy| depois |dx|); na borda, ficam.
    #[test]
    fn up_down_pick_nearest_vertical_neighbor_and_stop_at_edge() {
        let v = ids(4);
        let nodes = vec![
            (v[0], 0.0, 0.0),
            (v[1], 100.0, 0.0),
            (v[2], 0.0, 100.0),
            (v[3], 100.0, 100.0),
        ];
        assert_eq!(
            next_in_dir(&nodes, v[0], Dir::Down),
            Some(v[2]),
            "|dx|=0 vence |dx|=100"
        );
        assert_eq!(next_in_dir(&nodes, v[1], Dir::Down), Some(v[3]));
        assert_eq!(next_in_dir(&nodes, v[2], Dir::Up), Some(v[0]));
        assert_eq!(
            next_in_dir(&nodes, v[0], Dir::Up),
            None,
            "borda: fica onde está"
        );
    }

    /// Corrente fora do roster (nó morreu entre frames) → None; roster de 1 → None.
    #[test]
    fn vanished_current_or_single_node_navigates_nowhere() {
        let v = ids(3);
        let nodes = vec![(v[0], 0.0, 0.0), (v[1], 100.0, 0.0)];
        assert_eq!(next_in_dir(&nodes, v[2], Dir::Right), None);
        let single = vec![(v[0], 0.0, 0.0)];
        assert_eq!(next_in_dir(&single, v[0], Dir::Right), None);
    }

    /// `o`: próximo OCUPADO na ordem de leitura com wrap; sem outro ocupado → None.
    #[test]
    fn next_busy_cycles_reading_order_and_skips_idle() {
        let v = ids(4);
        let nodes = vec![
            (v[0], 0.0, 0.0),
            (v[1], 100.0, 0.0),
            (v[2], 0.0, 100.0),
            (v[3], 100.0, 100.0),
        ];
        let busy: BTreeSet<NodeId> = [v[2]].into_iter().collect();
        assert_eq!(
            next_busy(&nodes, &busy, v[0]),
            Some(v[2]),
            "pula o v[1] ocioso"
        );
        assert_eq!(next_busy(&nodes, &busy, v[3]), Some(v[2]), "wrap");
        let none: BTreeSet<NodeId> = BTreeSet::new();
        assert_eq!(next_busy(&nodes, &none, v[0]), None);
        let only_self: BTreeSet<NodeId> = [v[0]].into_iter().collect();
        assert_eq!(
            next_busy(&nodes, &only_self, v[0]),
            None,
            "não navega para si"
        );
    }

    /// Critério 2 (roving): EXATAMENTE 1 nó focável por vez — o corrente; corrente sumido → 0.
    #[test]
    fn roving_marks_exactly_one_focusable() {
        let v = ids(3);
        let nodes = vec![(v[0], 0.0, 0.0), (v[1], 100.0, 0.0), (v[2], 0.0, 100.0)];
        let marks = roving_marks(&nodes, v[1]);
        assert_eq!(
            marks.iter().filter(|(_, f)| *f).count(),
            1,
            "roving = 1 focável"
        );
        assert!(
            marks.iter().any(|(id, f)| *id == v[1] && *f),
            "o focável é o corrente"
        );
        let gone = roving_marks(&nodes, ids(1)[0]);
        assert_eq!(
            gone.iter().filter(|(_, f)| *f).count(),
            0,
            "corrente sumido: nenhum (render reaponta)"
        );
    }

    /// A máquina de nível: default = Terminal (não-regressão); Esc sobe; Enter desce; setas
    /// navegam SÓ no canvas; digitação no canvas NÃO vaza ao PTY.
    #[test]
    fn level_machine_routes_keys_per_level() {
        let mut f = CanvasFocus::default();
        assert_eq!(
            f.level(),
            FocusLevel::Terminal,
            "default preserva o comportamento atual"
        );
        assert_eq!(
            f.on_key("up", false),
            KeyIntent::PassToTerminal,
            "seta no terminal → PTY"
        );
        assert_eq!(
            f.on_key("c", true),
            KeyIntent::PassToTerminal,
            "Ctrl+C → PTY (intacto)"
        );
        assert_eq!(f.on_key("escape", false), KeyIntent::ExitToCanvas);
        assert_eq!(f.level(), FocusLevel::Canvas, "Esc subiu ao canvas");
        assert_eq!(f.on_key("up", false), KeyIntent::Navigate(Dir::Up));
        assert_eq!(f.on_key("n", false), KeyIntent::NewAgent);
        assert_eq!(f.on_key("o", false), KeyIntent::NextBusy);
        assert_eq!(
            f.on_key("x", false),
            KeyIntent::Ignore,
            "digitação NÃO vaza ao PTY"
        );
        assert_eq!(
            f.on_key("c", true),
            KeyIntent::Ignore,
            "chord no canvas não vaza"
        );
        assert_eq!(f.on_key("enter", false), KeyIntent::EnterTerminal);
        assert_eq!(f.level(), FocusLevel::Terminal, "Enter desceu ao terminal");
    }

    /// Critério 3: reduce-motion → salto (animate=false); sem reduce-motion → anima. A MESMA
    /// fonte única de W4-6 (`a11y::reduce_motion_effective`) alimenta o parâmetro na fiação.
    #[test]
    fn zoom_to_focus_respects_reduce_motion() {
        let cam = Camera::default();
        let mv = zoom_to_focus((0.0, 0.0), (560.0, 360.0), (1200.0, 800.0), cam, true);
        assert!(!mv.animate, "reduce-motion: salto direto, sem animação");
        let mv = zoom_to_focus((0.0, 0.0), (560.0, 360.0), (1200.0, 800.0), cam, false);
        assert!(mv.animate, "sem reduce-motion: anima até o alvo");
    }

    /// O alvo CENTRALIZA o card na área útil mantendo o zoom (conferido pela transformação
    /// REAL da câmera — mesma fonte `world_to_screen` do render/culling).
    #[test]
    fn zoom_to_focus_centers_card_in_usable_area_keeping_zoom() {
        let cam = Camera {
            pan: (999.0, -999.0),
            zoom: 1.0,
        };
        let (vw, vh) = (1200.0, 800.0);
        let card = (560.0, 360.0);
        let mv = zoom_to_focus((2000.0, 3000.0), card, (vw, vh), cam, true);
        assert_eq!(
            mv.camera.zoom, 1.0,
            "zoom do usuário mantido quando o card cabe"
        );
        let (sx, sy) = mv
            .camera
            .world_to_screen((2000.0 + card.0 / 2.0, 3000.0 + card.1 / 2.0));
        let expected = (
            MARGIN + (vw - 2.0 * MARGIN) / 2.0,
            TOPBAR + MARGIN + (vh - TOPBAR - 2.0 * MARGIN) / 2.0,
        );
        assert!(
            (sx - expected.0).abs() < 0.01 && (sy - expected.1).abs() < 0.01,
            "centro do card ({sx},{sy}) ≠ centro da área útil ({},{})",
            expected.0,
            expected.1
        );
    }

    /// Card que NÃO cabe no zoom atual → zoom reduz até caber (nunca abaixo de ZOOM_MIN);
    /// zoom nunca AUMENTA sozinho.
    #[test]
    fn zoom_to_focus_only_shrinks_to_fit_never_zooms_in() {
        let cam = Camera {
            pan: (0.0, 0.0),
            zoom: 2.0,
        };
        let mv = zoom_to_focus((0.0, 0.0), (560.0, 360.0), (800.0, 600.0), cam, true);
        assert!(mv.camera.zoom < 2.0, "card não cabia em 2.0× → reduz");
        assert!(mv.camera.zoom >= ZOOM_MIN, "nunca abaixo do mínimo");
        // card minúsculo + zoom baixo: NÃO aproxima sozinho.
        let cam = Camera {
            pan: (0.0, 0.0),
            zoom: 0.5,
        };
        let mv = zoom_to_focus((0.0, 0.0), (560.0, 360.0), (1920.0, 1080.0), cam, true);
        assert_eq!(
            mv.camera.zoom, 0.5,
            "navegar não 'bomba' o zoom de quem afastou a vista"
        );
    }
}
