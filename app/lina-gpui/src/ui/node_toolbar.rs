//! **F2-2-5 · Node toolbar** — a SEGUNDA superfície do registry de ações (a primeira é a paleta). Uma
//! barra flutuante de 3–4 ações ROTULADAS sobre o card focado (modelo Canva — nunca ribbon, nunca
//! icon-only; D4-A3). builder + `RenderOnce`, **token-puro**, **view-agnóstico** (o `on_invoke` vem do
//! call-site). Consome [`crate::palette::node_commands`] — **um registry, duas portas** (mouse aqui,
//! teclado na paleta): a regra anti-Zed é verdadeira por construção.
//!
//! O que é PURO e testável sem gpui: [`toolbar_anchor`] (posição + flip + `None` fora de foco) e
//! [`variant_for`] (ação→variante de botão). O render é casca fina sobre elas e sobre o `ui::Button`.
//!
//! **Câmera (ADR 0029):** a barra só LÊ a posição do card para se ancorar; NUNCA move a câmera. A
//! única ação que mexe na câmera é Centralizar, e só por clique humano.

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, ElementId, IntoElement, Role, SharedString, Window,
};

use crate::palette::{node_commands, NodeCtx, PaletteAction};
use crate::theme;

use super::{Button, ButtonSize, ButtonVariant};

// Strings — fonte única do módulo (padrão F1-4). PLACEHOLDERS da r6: o @Redator entrega a copy
// congelada (R9 — zero jargão). Reconciliar com COPY_TB_* dele no fechamento da rodada.
// TODO-copy(@Redator): confirmar verbos finais.
/// Glifo + rótulo leigo: NUNCA icon-only (D4-A3 / WCAG 1.4.1 — o glifo é ornamento, não o sinal).
pub const COPY_TB_ATENDER: &str = "⚑ Atender";
pub const COPY_TB_EDITAR: &str = "✎ Editar";
pub const COPY_TB_CENTRALIZAR: &str = "⤢ Centralizar";
pub const COPY_TB_ENCERRAR: &str = "✕ Encerrar";

/// Retângulo do card em ESPAÇO DE TELA (já passado por `world_to_screen`). Só `x`/`y` (canto superior
/// esquerdo) entram na ancoragem — a barra alinha pela borda esquerda do card (spec §1.1); `w`/`h`
/// ficam no contrato para o flip-into futuro e o `PortalEngine`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// **Ancoragem PURA da toolbar (spec §1.1).** `None` se o nó não está focado (periferia/suspenso — a
/// barra existe só para o nó em `Zone::Focus`). Senão: posição ACIMA do card (`top = y − h − gap`); se
/// isso invadir a área segura sob a topbar (`top < safe_top`), **flip** para DENTRO da borda superior
/// do card. Tudo em espaço de tela (a barra é chrome, não escala com o zoom). `safe_top`/`gap`/`h` são
/// PARÂMETROS (o shell passa os valores vivos) — zero medida mágica aqui.
#[must_use]
pub fn toolbar_anchor(
    card: CardRect,
    toolbar_h: f32,
    gap: f32,
    safe_top: f32,
    in_focus: bool,
) -> Option<(f32, f32)> {
    if !in_focus {
        return None; // R3: a barra é a camada de composição do nó SELECIONADO — periferia não é alvo.
    }
    let above = card.y - toolbar_h - gap;
    let top = if above < safe_top {
        card.y + gap // flip: logo abaixo da borda superior do card, nunca sob a topbar
    } else {
        above
    };
    Some((card.x, top))
}

/// **Ação → variante do botão (spec §2.2).** Cor = significado (OP-1): Atender é a primária
/// (`Confirm`), Encerrar é destrutiva (`Destructive`), o resto é superfície neutra (`Secondary`) —
/// cor é reservada a significado, não decoração.
#[must_use]
pub fn variant_for(action: &PaletteAction) -> ButtonVariant {
    match action {
        PaletteAction::GotoAtencao(_) => ButtonVariant::Confirm,
        PaletteAction::CloseNode(_) => ButtonVariant::Destructive,
        _ => ButtonVariant::Secondary,
    }
}

/// Handler de invocação: recebe a AÇÃO clicada. O call-site (costura `main.rs`) o produz capturando a
/// view; ele EXECUTA a ação e apenda `PaletteCommandInvoked{key}` — o MESMO caminho da paleta.
pub type InvokeHandler = Rc<dyn Fn(&PaletteAction, &mut Window, &mut App)>;

/// **A toolbar contextual** (catálogo `ui/`). builder + `RenderOnce`. Consome `node_commands(ctx)` e
/// renderiza um [`Button`] rotulado por `Command`, com a variante de [`variant_for`]. O posicionamento
/// (via [`toolbar_anchor`]) e a execução (via `on_invoke`) são da costura `main.rs`.
#[derive(IntoElement)]
pub struct NodeToolbar {
    ctx: NodeCtx,
    /// Nome do nó — para o `aria_label` do container ("Ações de <nome>").
    node_label: SharedString,
    on_invoke: Option<InvokeHandler>,
}

impl NodeToolbar {
    pub fn new(ctx: NodeCtx, node_label: impl Into<SharedString>) -> Self {
        Self {
            ctx,
            node_label: node_label.into(),
            on_invoke: None,
        }
    }

    /// Liga a execução: `handler(&action, window, app)` é chamado no clique de cada botão. O call-site
    /// (main.rs) executa a ação E apenda `PaletteCommandInvoked` — paridade com a paleta.
    #[must_use]
    pub fn on_invoke(
        mut self,
        handler: impl Fn(&PaletteAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_invoke = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for NodeToolbar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let t = theme::active();
        let gap = f32::from(t.spacing.sm);
        let pad = f32::from(t.spacing.sm);
        let cmds = node_commands(&self.ctx);
        let on_invoke = self.on_invoke;

        // Container: superfície chrome + borda 1px + radius.md, FLAT (sem sombra). `id` próprio para
        // o nó `Toolbar` entrar na árvore a11y; `aria_label` com o nome do nó (leitor de tela).
        let mut bar = div()
            .id(ElementId::Name(
                format!("node-toolbar-{}", self.ctx.id).into(),
            ))
            .flex()
            .items_center()
            .gap(px(gap))
            .p(px(pad))
            .bg(rgb(t.surface.chrome))
            .border_1()
            .border_color(rgb(t.surface.border))
            .rounded_md()
            .role(Role::Toolbar)
            .aria_label(format!("Ações de {}", self.node_label));

        for cmd in cmds {
            let variant = variant_for(&cmd.action);
            let mut btn = Button::new(ElementId::Name(cmd.key().into()), cmd.label.clone())
                .variant(variant)
                .size(ButtonSize::Sm);
            if let Some(handler) = on_invoke.clone() {
                let action = cmd.action.clone();
                btn = btn
                    .on_click(move |_ev: &ClickEvent, window, app| handler(&action, window, app));
            }
            bar = bar.child(btn);
        }
        bar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_host::{NodeKind, NodeStatus};
    use uuid::Uuid;

    fn ctx(needs_human: bool, roster_len: usize) -> NodeCtx {
        NodeCtx {
            id: Uuid::now_v7(),
            kind: NodeKind::Terminal,
            status: NodeStatus::Idle,
            needs_human,
            roster_len,
        }
    }

    /// Ação→variante (spec §2.2): Atender=primária (Confirm), Encerrar=Destructive, resto=Secondary.
    #[test]
    fn variant_maps_meaning_to_color() {
        let id = Uuid::now_v7();
        assert_eq!(
            variant_for(&PaletteAction::GotoAtencao(id)),
            ButtonVariant::Confirm
        );
        assert_eq!(
            variant_for(&PaletteAction::CloseNode(id)),
            ButtonVariant::Destructive
        );
        assert_eq!(
            variant_for(&PaletteAction::EditAgent(id)),
            ButtonVariant::Secondary
        );
        assert_eq!(
            variant_for(&PaletteAction::FocusNode(id)),
            ButtonVariant::Secondary
        );
    }

    /// O registry e a variante casam por estado: nó que pede você → 4 ações, Encerrar=Destructive,
    /// Atender=Confirm, nenhuma icon-only.
    #[test]
    fn toolbar_actions_track_state_and_have_labels() {
        let cmds = node_commands(&ctx(true, 3));
        assert_eq!(cmds.len(), 4);
        for c in &cmds {
            assert!(!c.label.trim().is_empty(), "nunca icon-only (D4-A3)");
        }
        assert_eq!(
            variant_for(&cmds[0].action),
            ButtonVariant::Confirm,
            "Atender primária"
        );
        assert_eq!(
            variant_for(&cmds[cmds.len() - 1].action),
            ButtonVariant::Destructive,
            "Encerrar destrutiva por último"
        );
    }

    /// Ancoragem (spec §1.1): `None` fora de foco; posição acima quando cabe; FLIP quando invadiria a
    /// topbar. Sempre alinhada pela esquerda do card; tudo em espaço de tela.
    #[test]
    fn anchor_none_off_focus_and_flips_under_topbar() {
        let card = CardRect {
            x: 200.0,
            y: 300.0,
            w: 680.0,
            h: 500.0,
        };
        // Fora de foco → sem barra.
        assert_eq!(toolbar_anchor(card, 40.0, 8.0, 44.0, false), None);
        // Cabe acima (300 − 40 − 8 = 252 ≥ 44): top = 252, left = x.
        assert_eq!(
            toolbar_anchor(card, 40.0, 8.0, 44.0, true),
            Some((200.0, 252.0))
        );
        // Card colado no topo (y=50): acima = 50−40−8 = 2 < 44 → FLIP para dentro (y + gap = 58).
        let high = CardRect { y: 50.0, ..card };
        assert_eq!(
            toolbar_anchor(high, 40.0, 8.0, 44.0, true),
            Some((200.0, 58.0))
        );
    }
}
