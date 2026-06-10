//! `a11y_live` — **SPIKE F1-2-7, caminho (a) do ADR 0028: live-region via custom `Element`**.
//!
//! O gpui do SHA pinado (`09165c1`) NUNCA chama `set_live` ao montar a árvore AccessKit
//! (13.15 achado 2: `Interactivity::write_a11y_info` seta 18 campos `aria_*`, nenhum `live`),
//! então `Role::Status` sozinho não vira live-region e o leitor de tela não auto-anuncia.
//! Este módulo prova o caminho (a) **sem patch no fork**: a trait `Element` do gpui expõe
//! `write_a11y_info` com default vazio (`gpui/src/element.rs:120`) e o pipeline a chama em
//! `Drawable::prepaint` (`element.rs:458-476`) sempre que o elemento tem `id()` + `a11y_role()`.
//! Um `Element` custom pode, portanto, escrever no nó o que o builder `div()` não expõe.
//!
//! **Contrato do adapter macOS** (`accesskit_macos-0.26.1/src/event.rs` — lido na fonte):
//! - o anúncio fala **`node.value()`**, não o label (`event.rs:35-44`:
//!   `Announcement { text: node.value().unwrap(), .. }`);
//! - dispara quando o nó live **nasce** na árvore já com value (`node_added`, `event.rs:236-239`:
//!   guard `value().is_some() && live() != Off`) — exatamente o caso do banner, que só entra
//!   na cena quando há mensagem — e quando o **value muda** num nó vivo (`event.rs:298-306`);
//! - um nó live só com `label` é **silenciosamente ignorado** (guard exige `value`).
//!
//! **Por que `id()` importa:** o gpui só inclui um elemento na árvore a11y se ele tem
//! `GlobalElementId` E `a11y_role() != None` — sem `id()`, `write_a11y_info` nem é chamado.
//!
//! **Honestidade (critério da story):** nada aqui afirma "conforme ARIA live-region" — o
//! auto-anúncio só vira fato após a validação NA TELA (VoiceOver falando sem foco; roteiro em
//! `tasks/epico-f1/spike-a11y-roteiro.md`). Até lá, o que está provado é a LÓGICA do nó
//! (testes headless abaixo) e a viabilidade do caminho (a) sem tocar o fork.

use gpui::accesskit::{Live, Node as A11yNode, Role as A11yRole};
use gpui::{
    div, text, App, Bounds, Div, DivFrameState, Element, ElementId, GlobalElementId, Hitbox,
    InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, Styled, Window,
};

use crate::theme;

/// Escreve no nó AccessKit as propriedades que fazem o auto-anúncio acontecer. Função PURA
/// e separada do [`Element`] de propósito (lição a11y do repo: testar a lógica do NÓ, não o
/// `TreeUpdate`): `live=Polite` (cortês: não interrompe a fala corrente), `value` (o TEXTO que
/// o adapter anuncia — `event.rs:37`) e `label` (identidade do nó ao FOCAR, comportamento que
/// o banner já tinha via `aria_label`).
pub fn write_live_node_props(node: &mut A11yNode, msg: &str) {
    node.set_live(Live::Polite);
    node.set_value(msg);
    node.set_label(msg);
}

/// O banner "resposta pronta" como **live-region de verdade**: visual idêntico ao do W4-6
/// (pílula `🔊 msg` verde/painel) + nó AccessKit com `live=Polite` que o `div()` builtin não
/// consegue produzir. O `Div` interno é PURAMENTE visual (sem id/role/aria_label) — o nó a11y
/// é do wrapper, evitando um nó `Status` duplicado na árvore.
pub struct LiveAnnounce {
    msg: String,
    div: Div,
}

/// Constrói o elemento de anúncio. `msg` cru vai para o nó a11y (value+label); o emoji 🔊 é
/// só visual (o leitor de tela não deve falar "alto-falante" antes de cada anúncio).
#[must_use]
pub fn live_announce(msg: &str) -> LiveAnnounce {
    let th = theme::active();
    let div = div()
        .px_3()
        .py_1()
        .rounded_md()
        .bg(gpui::rgb(th.surface.panel))
        .text_color(gpui::rgb(th.state.success))
        .child(text!(format!("🔊 {msg}")));
    LiveAnnounce {
        msg: msg.to_string(),
        div,
    }
}

impl Element for LiveAnnounce {
    // Delegação total de layout/pintura ao `Div` interno (padrão `WithRemSize` do zed:
    // `crates/ui/src/utils/with_rem_size.rs`) — os estados são os do `Div`.
    type RequestLayoutState = DivFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        // SEM id o gpui exclui o elemento da árvore a11y (`Drawable::prepaint` exige
        // `GlobalElementId` antes de chamar `write_a11y_info`). Reusa o id estável do banner.
        Some(ElementId::Name("a11y-live".into()))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<A11yRole> {
        // `Status` é o role honesto de um anúncio de estado (o mesmo do banner W4-6).
        Some(A11yRole::Status)
    }

    fn write_a11y_info(&self, node: &mut A11yNode) {
        // O coração do spike: o que o gpui não seta, o Element custom seta.
        write_live_node_props(node, &self.msg);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.div.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.div
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.div.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

impl IntoElement for LiveAnnounce {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O coração do spike: o nó produzido tem `live=Polite` — exatamente o campo que o gpui
    /// deste SHA nunca seta (13.15 achado 2). Se alguém remover o `set_live`, este teste falha.
    #[test]
    fn node_is_polite_live_region() {
        let mut node = A11yNode::new(A11yRole::Status);
        write_live_node_props(&mut node, "A: resposta pronta");
        assert_eq!(
            node.live(),
            Some(Live::Polite),
            "live=Polite é o que transforma Status em live-region auto-anunciável"
        );
    }

    /// Contrato do adapter macOS (`event.rs:35-44` + guard `value().is_some()`): o anúncio fala
    /// `node.value()` — um nó live só com `label` é ignorado em silêncio. value E label honestos.
    #[test]
    fn announcement_text_lives_in_value_and_label_is_honest() {
        let mut node = A11yNode::new(A11yRole::Status);
        write_live_node_props(&mut node, "A: resposta pronta");
        assert_eq!(
            node.value(),
            Some("A: resposta pronta"),
            "sem value o adapter NÃO anuncia (guard value().is_some())"
        );
        assert_eq!(
            node.label(),
            Some("A: resposta pronta"),
            "label preserva a identidade do nó ao FOCAR (comportamento do banner W4-6)"
        );
    }

    /// Elegibilidade na árvore a11y do gpui (`Drawable::prepaint`): `write_a11y_info` só é
    /// chamado com `id()` + `a11y_role()` presentes. Se alguém remover o id, o nó some da
    /// árvore SEM erro de compilação — este teste é a regressão.
    #[test]
    fn element_is_eligible_for_the_a11y_tree() {
        let el = live_announce("resposta pronta");
        assert!(
            el.id().is_some(),
            "sem ElementId o gpui exclui o nó da árvore a11y"
        );
        assert_eq!(
            el.a11y_role(),
            Some(A11yRole::Status),
            "role honesto de anúncio de estado"
        );
    }

    /// Ponta-a-ponta da LÓGICA (sem janela): o `Element::write_a11y_info` do wrapper produz o
    /// nó completo — live + value + label — a partir da mensagem crua (sem o emoji visual).
    #[test]
    fn element_write_a11y_info_produces_complete_live_node() {
        let el = live_announce("Revisor: resposta pronta");
        let mut node = A11yNode::new(A11yRole::Status);
        el.write_a11y_info(&mut node);
        assert_eq!(node.live(), Some(Live::Polite));
        assert_eq!(node.value(), Some("Revisor: resposta pronta"));
        assert_eq!(node.label(), Some("Revisor: resposta pronta"));
        assert!(
            !node.value().unwrap_or_default().contains('🔊'),
            "o emoji é visual; o leitor de tela não deve falar 'alto-falante'"
        );
    }
}
