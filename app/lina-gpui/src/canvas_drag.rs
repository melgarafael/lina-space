//! `canvas_drag` — **F2-3-1: arrastar-e-mover um card no canvas (lógica PURA, gpui-free).**
//!
//! Hoje o drag do canvas só faz PAN da câmera (`main.rs`); cards não se movem. Este módulo é o
//! TRONCO testável do gesto "arrastar o terminal pra onde soltou": converte o delta de tela em
//! delta de MUNDO (ciente do zoom), decide mover-card vs pan-fundo, modela o gesto inteiro como
//! uma máquina de estado e fornece a ordem-z por **fractional index** (ADR 0029 §2: "trazer à
//! frente" = 1 chave nova sem reindexar vizinhos). O `main.rs` (costura do Maestro) consome isto:
//! no FIM do gesto emite `DomainEvent::NodeMoved { node, x, y }` com a posição FINAL.
//!
//! **Invariantes (duros), provados pelos testes deste módulo:**
//! - **Nada se move sem gesto:** sem `CardDrag::begin` não há `CommitMove`; um clique sem arrasto
//!   (begin→finish sem deslocamento) devolve `None`.
//! - **Esc = ZERO `NodeMoved`:** `cancel()` marca o gesto; `finish()` então devolve `None`.
//! - **Evento no FIM, posição FINAL:** `finish()` lê o ÚLTIMO `update` (nunca acumula por frame).
//!
//! **Espaços/tipos:** posições de card vivem em `NodeView { x: f32, y: f32 }` (espaço de MUNDO);
//! a câmera é `screen = world * zoom + pan`. Trabalhamos em `f32` (coerente com `Camera`/`NodeView`);
//! a costura converte para o `f64` do evento `NodeMoved` ao emitir (`x as f64`). Zero `px(...)`/gpui
//! aqui — geometria pura (a catraca de tokens não casa este arquivo).
//!
//! **Dead-code consciente:** as funções públicas ainda NÃO têm chamador no binário — o consumidor
//! é a costura de mouse em `main.rs` (dono: Maestro, entregue como diff nesta fatia). Até a costura
//! entrar, o `#![allow(dead_code)]` abaixo evita que o `clippy -D warnings` do bin reprove o módulo
//! (mesmo padrão dos itens reservados de `canvas.rs`); cada função é exercitada pelos testes daqui.
#![allow(dead_code)]

use lina_host::NodeId;

// ───────────────────────────── 1. delta de tela → delta de mundo ─────────────────────────────

/// **Delta de tela → delta de MUNDO**, ciente do zoom. Como `screen = world*zoom + pan`, a
/// subtração de dois pontos de tela cancela o `pan`; o deslocamento de mundo é puramente
/// `Δtela / zoom`. Sob `zoom = 2.0`, arrastar 100px na tela move 50 no mundo (o card "anda menos"
/// porque está ampliado) — sem isto, o card escaparia do cursor em qualquer zoom ≠ 1.
#[must_use]
pub fn drag_delta_world(start_screen: (f32, f32), now_screen: (f32, f32), zoom: f32) -> (f32, f32) {
    debug_assert!(
        zoom > 0.0 && zoom.is_finite(),
        "zoom deve ser positivo e finito (Camera clampa em [ZOOM_MIN, ZOOM_MAX])"
    );
    (
        (now_screen.0 - start_screen.0) / zoom,
        (now_screen.1 - start_screen.1) / zoom,
    )
}

/// **Aplica um delta de mundo à posição original** do card → nova posição de mundo. O canvas é
/// ilimitado por design (a câmera trata o enquadramento via `reveal`/auto-pan; ADR 0029: câmera
/// FORA do event log) — não há bounds de mundo para clampar, então a operação é soma pura. Se um
/// dia existirem bounds, o clamp entra AQUI (uma só fonte da verdade do "onde o card pode parar").
#[must_use]
pub fn apply_drag(orig_xy: (f32, f32), delta: (f32, f32)) -> (f32, f32) {
    (orig_xy.0 + delta.0, orig_xy.1 + delta.1)
}

// ───────────────────────────── 2. decisão mover-card vs pan-fundo ─────────────────────────────

/// O que um `mouse_down` no canvas inicia, decidido pelo `hit_test` do ponto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    /// O clique caiu sobre um card → arrasta ESSE card (move o nó).
    MoveCard(NodeId),
    /// O clique caiu no fundo (vazio) → arrasta a CÂMERA (pan), comportamento histórico.
    PanCamera,
}

/// **Mover-card vs pan-fundo.** Dado o resultado do `hit_test` (bridge.rs) no ponto do
/// `mouse_down`, decide o modo do gesto: card sob o cursor → move o card; fundo → pan da câmera.
/// Puro e total — a costura só pluga `hit_test(...)` na entrada.
#[must_use]
pub fn drag_mode(hit: Option<NodeId>) -> DragMode {
    match hit {
        Some(node) => DragMode::MoveCard(node),
        None => DragMode::PanCamera,
    }
}

// ───────────────────────────── 3. máquina de gesto (move-card) ─────────────────────────────

/// A intenção emitida no FIM de um arrasto de card — a costura a traduz em
/// `DomainEvent::NodeMoved { node, x: x as f64, y: y as f64 }`. Carrega a posição FINAL de mundo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommitMove {
    pub node: NodeId,
    pub x: f32,
    pub y: f32,
}

/// **Gesto de arrasto de um card em progresso** — a máquina que torna os invariantes da onda
/// código provável. Criada em `mouse_down` sobre um card (`begin`), atualizada a cada
/// `mouse_move` (`update` — só recalcula o PREVIEW, nunca emite evento), e encerrada em
/// `mouse_up` (`finish` → `Some(CommitMove)` se moveu) OU em `Esc` (`cancel` → `finish` vira
/// `None`). O render lê `preview()` por frame; o event log só recebe o resultado de `finish`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardDrag {
    node: NodeId,
    orig_world: (f32, f32),
    start_screen: (f32, f32),
    current_world: (f32, f32),
    cancelled: bool,
}

impl CardDrag {
    /// Inicia o gesto: o card `node` está em `orig_world` e o cursor pegou-o em `start_screen`
    /// (tela). O preview começa exatamente na origem (nada se moveu ainda).
    #[must_use]
    pub fn begin(node: NodeId, orig_world: (f32, f32), start_screen: (f32, f32)) -> Self {
        Self {
            node,
            orig_world,
            start_screen,
            current_world: orig_world,
            cancelled: false,
        }
    }

    /// `mouse_move`: recalcula a posição de PREVIEW a partir do delta de tela acumulado desde o
    /// início (ciente do zoom atual). NÃO emite evento — o render pinta o preview; o log nada vê.
    pub fn update(&mut self, now_screen: (f32, f32), zoom: f32) {
        let delta = drag_delta_world(self.start_screen, now_screen, zoom);
        self.current_world = apply_drag(self.orig_world, delta);
    }

    /// A posição de mundo corrente (preview) — o que o render desenha durante o arrasto.
    #[must_use]
    pub fn preview(&self) -> (f32, f32) {
        self.current_world
    }

    /// O nó sob arrasto (a costura usa p/ `focus(node)` — bombar ao topo da ordem-z no `mouse_down`).
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Posição de MUNDO onde o card estava no início do gesto. A costura usa p/ REVERTER o preview
    /// ao cancelar com Esc (o preview move o `NodeView` em memória; cancelar volta à origem).
    #[must_use]
    pub fn origin(&self) -> (f32, f32) {
        self.orig_world
    }

    /// `Esc`: cancela o gesto. Idempotente. Após isto, `finish` devolve `None` — invariante
    /// "gesto cancelado = ZERO `NodeMoved`": o card volta à origem e o log não registra nada.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// `mouse_up`: encerra o gesto e devolve a intenção a emitir, ou `None`.
    /// `None` quando: foi **cancelado** (Esc) OU não houve deslocamento líquido (clique sem
    /// arrasto — `current == orig`). `Some` carrega a posição FINAL (último `update`).
    #[must_use]
    pub fn finish(self) -> Option<CommitMove> {
        if self.cancelled || self.current_world == self.orig_world {
            return None;
        }
        Some(CommitMove {
            node: self.node,
            x: self.current_world.0,
            y: self.current_world.1,
        })
    }
}

// ───────────────────────────── 4. ordem-z por fractional index ─────────────────────────────

/// Alfabeto base-62 em ordem ASCII crescente (`'0'<'9'<'A'<'Z'<'a'<'z'`) → a ordem lexicográfica
/// das chaves COINCIDE com a ordem numérica das frações que elas representam. É o que deixa um
/// `BTreeMap<String, _>` (ou `ORDER BY` no SQLite) ordenar a pilha-z sem epsilon nem reindexação.
const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

/// Índice (0..BASE) de um dígito do alfabeto. `DIGITS` é contíguo por faixa ASCII.
fn digit_val(c: u8) -> usize {
    match c {
        b'0'..=b'9' => (c - b'0') as usize,
        b'A'..=b'Z' => (c - b'A') as usize + 10,
        b'a'..=b'z' => (c - b'a') as usize + 36,
        _ => unreachable!("chave de ordem-z só contém dígitos base-62 emitidos por este módulo"),
    }
}

/// **Chave fracionária estritamente entre `a` e `b`** (ordem-z). `None` = borda aberta: `a=None`
/// é "antes de tudo" (-∞), `b=None` é "depois de tudo" (+∞). "Trazer um card à frente" =
/// `fractional_between(Some(topo), None)` → 1 chave nova, vizinhos intocados.
///
/// Escolha (lexicográfica, padrão Excalidraw/Figma — NÃO f64): um `f64` por midpoint esgota a
/// mantissa após ~50 inserções no MESMO gap (não há mais valor representável entre dois
/// adjacentes); a string base-62 cresce em comprimento mas NUNCA esgota — fundação de 5 anos, não
/// protótipo. Pré-condição (quando ambos `Some`): `a < b` lexicograficamente (o chamador garante;
/// `debug_assert` documenta). O resultado é sempre não-vazio e estritamente entre as bordas.
#[must_use]
pub fn fractional_between(a: Option<&str>, b: Option<&str>) -> String {
    if let (Some(a), Some(b)) = (a, b) {
        debug_assert!(
            a < b,
            "fractional_between exige a < b (recebi {a:?} >= {b:?})"
        );
    }
    let lower = a.unwrap_or("");
    midpoint(lower, b)
}

/// Ponto médio entre a fração `lower` (string base-62, "" = 0.0) e `upper` (`Some` = string;
/// `None` = 1.0, borda aberta). Devolve `m` com `lower < m < upper`. Algoritmo canônico de
/// fractional indexing: copia o prefixo comum; no 1º dígito divergente pega o dígito médio; se os
/// dígitos forem ADJACENTES (sem médio), fixa o de baixo e desce um nível com a borda superior
/// aberta (BASE) — garante uma chave estritamente maior que o resto de `lower`.
fn midpoint(lower: &str, upper: Option<&str>) -> String {
    let lo_bytes = lower.as_bytes();
    let up_bytes = upper.map(str::as_bytes);
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0usize;
    // `upper_open`: uma vez que descemos abaixo do dígito de `upper`, todo o resto de `upper`
    // passa a valer BASE (ilimitado) — é o "desce um nível" do caso adjacente.
    let mut upper_open = upper.is_none();
    loop {
        let lo = lo_bytes.get(i).map(|&c| digit_val(c)).unwrap_or(0);
        let hi = if upper_open {
            BASE
        } else {
            // upper é Some aqui (upper_open só fica true por None ou após descer)
            up_bytes
                .and_then(|u| u.get(i))
                .map(|&c| digit_val(c))
                .unwrap_or(BASE)
        };
        debug_assert!(
            lo <= hi,
            "midpoint pressupõe lower <= upper dígito a dígito"
        );
        if lo == hi {
            out.push(DIGITS[lo]);
            i += 1;
            continue;
        }
        let mid = (lo + hi) / 2;
        if mid != lo {
            out.push(DIGITS[mid]);
            break;
        }
        // Adjacentes (hi == lo + 1): fixa `lo`, abre a borda superior e desce um nível —
        // a próxima iteração busca um dígito > resto de `lower` (com hi = BASE).
        out.push(DIGITS[lo]);
        upper_open = true;
        i += 1;
    }
    // `out` é sempre não-vazio (todo caminho de saída faz ao menos um push) e ASCII-válido.
    String::from_utf8(out).expect("DIGITS é ASCII por construção")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_host::NodeId;

    /// `NodeId` é `uuid::Uuid` (lina_host) — `uuid` é dev-dependency; fabricamos um id determinístico
    /// (o que o Supervisor faria em produção). O valor só precisa ser estável dentro do teste.
    fn node() -> NodeId {
        uuid::Uuid::from_u128(1)
    }

    // ─── delta de tela → mundo (sob zoom) ───

    /// Sob zoom = 1 o delta de mundo é o delta de tela cru.
    #[test]
    fn delta_identity_at_zoom_one() {
        assert_eq!(
            drag_delta_world((10.0, 10.0), (60.0, 40.0), 1.0),
            (50.0, 30.0)
        );
    }

    /// Sob zoom ≠ 1 o delta DIVIDE pelo zoom (não-vacuoso: remover o `/zoom` quebra ambos).
    #[test]
    fn delta_scales_inverse_to_zoom() {
        assert_eq!(
            drag_delta_world((0.0, 0.0), (100.0, 50.0), 2.0),
            (50.0, 25.0)
        );
        assert_eq!(
            drag_delta_world((0.0, 0.0), (100.0, 50.0), 0.5),
            (200.0, 100.0)
        );
    }

    /// `apply_drag` soma o delta à origem.
    #[test]
    fn apply_drag_translates() {
        assert_eq!(apply_drag((10.0, 20.0), (5.0, -8.0)), (15.0, 12.0));
    }

    // ─── mover-card vs pan ───

    #[test]
    fn mode_card_when_hit_pan_when_empty() {
        let n = node();
        assert_eq!(drag_mode(Some(n)), DragMode::MoveCard(n));
        assert_eq!(drag_mode(None), DragMode::PanCamera);
    }

    // ─── máquina de gesto: invariantes ───

    /// Invariante "evento no FIM, posição FINAL": vários `update` → `finish` reflete só o último,
    /// já convertido para mundo sob o zoom corrente.
    #[test]
    fn finish_emits_final_position_not_per_frame() {
        let n = node();
        let mut g = CardDrag::begin(n, (100.0, 100.0), (0.0, 0.0));
        g.update((20.0, 20.0), 1.0); // frame intermediário — não deve "vencer"
        g.update((40.0, 10.0), 2.0); // final: delta mundo (20,5) → (120,105)
        assert_eq!(g.preview(), (120.0, 105.0));
        let mv = g.finish().expect("houve deslocamento → emite");
        assert_eq!((mv.node, mv.x, mv.y), (n, 120.0, 105.0));
    }

    /// Invariante "nada se move sem gesto": clique sem arrasto (begin→finish, zero update) = `None`.
    #[test]
    fn click_without_drag_emits_nothing() {
        let g = CardDrag::begin(node(), (50.0, 50.0), (5.0, 5.0));
        assert_eq!(g.finish(), None);
    }

    /// Idem para um `update` que volta exatamente à origem (deslocamento líquido nulo).
    #[test]
    fn net_zero_drag_emits_nothing() {
        let mut g = CardDrag::begin(node(), (50.0, 50.0), (5.0, 5.0));
        g.update((5.0, 5.0), 1.0); // now == start → delta 0 → current == orig
        assert_eq!(g.finish(), None);
    }

    /// Invariante "Esc = ZERO `NodeMoved`": após `cancel`, mesmo com deslocamento, `finish` = `None`
    /// (não-vacuoso: remover o check de `cancelled` faria `finish` devolver `Some`).
    #[test]
    fn cancel_suppresses_emission_despite_movement() {
        let mut g = CardDrag::begin(node(), (0.0, 0.0), (0.0, 0.0));
        g.update((300.0, 300.0), 1.0); // moveu bastante…
        assert_eq!(g.preview(), (300.0, 300.0));
        g.cancel(); // …mas Esc
        assert_eq!(g.finish(), None, "Esc não pode emitir NodeMoved");
    }

    // ─── fractional index ───

    /// A chave gerada é SEMPRE estritamente entre as bordas — em todas as combinações de bordas.
    #[test]
    fn fractional_strictly_between() {
        let mid = fractional_between(None, None);
        assert!(!mid.is_empty());

        let after = fractional_between(Some(&mid), None);
        assert!(after > mid, "{after} deve ser > {mid}");

        let before = fractional_between(None, Some(&mid));
        assert!(before < mid, "{before} deve ser < {mid}");

        let between = fractional_between(Some(&before), Some(&mid));
        assert!(
            before < between && between < mid,
            "{before} < {between} < {mid}"
        );
    }

    /// "Trazer à frente" repetido (sempre `between(topo, None)`) é monotônico e nunca esgota —
    /// 200 inserções mantêm a ordenação estrita (f64 teria estourado a mantissa num único gap).
    #[test]
    fn bring_to_front_is_monotonic_and_unbounded() {
        let mut top = fractional_between(None, None);
        for _ in 0..200 {
            let next = fractional_between(Some(&top), None);
            assert!(next > top, "trazer à frente deve crescer: {next} > {top}");
            top = next;
        }
    }

    /// Inserir repetidamente no MESMO gap (entre dois adjacentes) nunca colapsa — o caso que
    /// mata o f64. 100 inserções, cada chave estritamente entre o par corrente.
    #[test]
    fn repeated_insert_same_gap_never_collapses() {
        let mut lo = fractional_between(None, None);
        let hi = fractional_between(Some(&lo), None);
        let hi = fractional_between(Some(&hi), None); // garante um gap largo p/ começar
        for _ in 0..100 {
            let mid = fractional_between(Some(&lo), Some(&hi));
            assert!(lo < mid && mid < hi, "colapsou o gap: {lo} < {mid} < {hi}");
            lo = mid; // fecha o gap pela esquerda a cada passo
        }
    }

    /// Bordas adjacentes no 1º dígito ("A" / "B") exigem descer um nível — a chave começa em "A".
    #[test]
    fn adjacent_first_digit_descends() {
        let k = fractional_between(Some("A"), Some("B"));
        assert!("A" < k.as_str() && k.as_str() < "B", "A < {k} < B");
        assert!(
            k.starts_with('A'),
            "deve fixar o dígito de baixo e descer: {k}"
        );
    }
}
