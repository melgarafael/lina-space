//! `a11y_live` — **live-region de PRODUÇÃO (ADR 0028), via custom `Element`** (promovido do spike
//! F1-2-7 na F2-2-2; agora `mod a11y_live;` no crate root, consumido por toast/badge/banner).
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
    div, text, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ParentElement, Pixels, Styled, Window,
};

use crate::theme;

/// **Cortesia do anúncio** (F2-2-2 / ADR 0028). `Polite` não interrompe a fala corrente — o
/// default de quase tudo (toast informativo, badge que mudou de cor). `Assertive` interrompe — RESERVADO
/// a "precisa de você AGORA" (gate humano, erro que trava o fluxo); usar à toa treina o usuário a
/// ignorar o leitor de tela. Mapeia para o `Live` do AccessKit e para o `Role` honesto do nó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Politeness {
    /// Não interrompe — enfileira atrás da fala atual (`Live::Polite`, `Role::Status`).
    Polite,
    /// Interrompe — só para o que exige ação imediata (`Live::Assertive`, `Role::Alert`).
    Assertive,
}

impl Politeness {
    #[must_use]
    fn live(self) -> Live {
        match self {
            Politeness::Polite => Live::Polite,
            Politeness::Assertive => Live::Assertive,
        }
    }

    /// O `Role` honesto: `Status` (polite) vs `Alert` (assertive) — o adapter trata os dois como
    /// live-region, mas o role correto preserva a semântica para outras tecnologias assistivas.
    #[must_use]
    fn role(self) -> A11yRole {
        match self {
            Politeness::Polite => A11yRole::Status,
            Politeness::Assertive => A11yRole::Alert,
        }
    }
}

/// Escreve no nó AccessKit as propriedades que fazem o auto-anúncio acontecer. Função PURA
/// e separada do [`Element`] de propósito (lição a11y do repo: testar a lógica do NÓ, não o
/// `TreeUpdate`): `live` (cortesia — ver [`Politeness`]), `value` (o TEXTO que o adapter anuncia —
/// `event.rs:37`) e `label` (identidade do nó ao FOCAR, comportamento que o banner já tinha via
/// `aria_label`).
pub fn write_live_node_props(node: &mut A11yNode, msg: &str, politeness: Politeness) {
    node.set_live(politeness.live());
    node.set_value(msg);
    node.set_label(msg);
}

/// **Live-region de PRODUÇÃO (F2-2-2 / ADR 0028).** A promoção do spike: antes o `Element` custom
/// envolvia SÓ a pílula do banner; agora envolve QUALQUER elemento (`child: El`), de modo que toast,
/// badge e banner **nascem compondo** o nó auto-anunciável (retrofit proibido). O `child` é
/// PURAMENTE visual (sem id/role/aria próprio do nó vivo); a identidade a11y é DESTE wrapper, com um
/// `id` ÚNICO por instância — banner + N toasts + badges coexistem sem colidir na árvore.
pub struct LiveRegion<El> {
    id: ElementId,
    msg: String,
    politeness: Politeness,
    child: El,
}

/// Envolve `child` num live-region. `id` ÚNICO por instância viva (colisão de id colapsa nós na
/// árvore a11y). `msg` é o texto cru que o adapter anuncia (`value`); `politeness` escolhe se
/// interrompe. Use isto para tornar QUALQUER componente acessível por construção.
#[must_use]
pub fn live_region<El: IntoElement>(
    id: impl Into<ElementId>,
    msg: impl Into<String>,
    politeness: Politeness,
    child: El,
) -> LiveRegion<El::Element> {
    LiveRegion {
        id: id.into(),
        msg: msg.into(),
        politeness,
        child: child.into_element(),
    }
}

/// **Anúncio standalone** (a API limpa do despacho: `announce(texto, cortesia)`) — a pílula `🔊 msg`
/// do banner W4-6 como live-region de verdade. `msg` cru vai ao nó (value+label); o emoji 🔊 é só
/// visual (o leitor não deve falar "alto-falante" antes de cada anúncio). Id estável singleton.
#[must_use]
pub fn announce(msg: &str, politeness: Politeness) -> LiveRegion<Div> {
    let th = theme::active();
    let pill = div()
        .px_3()
        .py_1()
        .rounded_md()
        .bg(gpui::rgb(th.surface.panel))
        .text_color(gpui::rgb(th.state.success))
        .child(text!(format!("🔊 {msg}")));
    live_region(ElementId::Name("a11y-live".into()), msg, politeness, pill)
}

/// Back-compat (W4-6 / `a11y.rs`): anúncio cortês do banner. Açúcar para [`announce`] com [`Politeness::Polite`].
#[must_use]
pub fn live_announce(msg: &str) -> LiveRegion<Div> {
    announce(msg, Politeness::Polite)
}

impl<El: Element> Element for LiveRegion<El> {
    // Delegação total de layout/pintura ao `child` (padrão `WithRemSize` do zed:
    // `crates/ui/src/utils/with_rem_size.rs`) — os estados são os do elemento envolvido.
    type RequestLayoutState = El::RequestLayoutState;
    type PrepaintState = El::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        // SEM id o gpui exclui o elemento da árvore a11y (`Drawable::prepaint` exige
        // `GlobalElementId` antes de chamar `write_a11y_info`).
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<A11yRole> {
        // Role honesto conforme a cortesia: `Status` (polite) vs `Alert` (assertive).
        Some(self.politeness.role())
    }

    fn write_a11y_info(&self, node: &mut A11yNode) {
        // O coração do spike: o que o gpui não seta, o Element custom seta.
        write_live_node_props(node, &self.msg, self.politeness);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.child.request_layout(id, inspector_id, window, cx)
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
        self.child
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
        self.child.paint(
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

impl<El: Element> IntoElement for LiveRegion<El> {
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
        write_live_node_props(&mut node, "A: resposta pronta", Politeness::Polite);
        assert_eq!(
            node.live(),
            Some(Live::Polite),
            "live=Polite é o que transforma Status em live-region auto-anunciável"
        );
    }

    /// F2-2-2: `Assertive` mapeia para `Live::Assertive` (interrompe) + `Role::Alert` — reservado a
    /// "precisa de você". A cortesia errada treina o usuário a ignorar o leitor; o teste trava o mapa.
    #[test]
    fn assertive_interrupts_and_is_alert_role() {
        let mut node = A11yNode::new(A11yRole::Alert);
        write_live_node_props(&mut node, "Aprovação necessária", Politeness::Assertive);
        assert_eq!(node.live(), Some(Live::Assertive), "assertive interrompe");
        assert_eq!(Politeness::Assertive.role(), A11yRole::Alert);
        assert_eq!(Politeness::Polite.role(), A11yRole::Status);
    }

    /// O wrapper genérico torna QUALQUER elemento acessível por construção (ADR 0028): id próprio,
    /// role conforme a cortesia, e o nó carrega o value/label da mensagem.
    #[test]
    fn live_region_wraps_any_child_with_its_own_node() {
        let el = live_region(
            ElementId::Name("toast-1".into()),
            "Espaço arquivado",
            Politeness::Polite,
            div().child(text!("conteúdo visual qualquer")),
        );
        assert!(el.id().is_some(), "id próprio por instância (sem colisão)");
        assert_eq!(el.a11y_role(), Some(A11yRole::Status));
        let mut node = A11yNode::new(A11yRole::Status);
        el.write_a11y_info(&mut node);
        assert_eq!(node.value(), Some("Espaço arquivado"));
        assert_eq!(node.live(), Some(Live::Polite));
    }

    /// Contrato do adapter macOS (`event.rs:35-44` + guard `value().is_some()`): o anúncio fala
    /// `node.value()` — um nó live só com `label` é ignorado em silêncio. value E label honestos.
    #[test]
    fn announcement_text_lives_in_value_and_label_is_honest() {
        let mut node = A11yNode::new(A11yRole::Status);
        write_live_node_props(&mut node, "A: resposta pronta", Politeness::Polite);
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
