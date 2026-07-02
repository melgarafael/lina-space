//! F4-1-2-UI · **modal "Conectar seu WhatsApp"** — o leigo cola o endereço do servidor + a chave de
//! acesso, escaneia um QR Code (como no WhatsApp Web) e a conta fica conectada. Zero jargão (o leigo vê
//! "conectar seu WhatsApp", não "sessão Waha"); invariante #6 não-técnico-first + #2 exposição opt-in.
//!
//! Split do shell (padrão `credential_modal`/`webhook_modal`): [`WhatsAppModal`] é **gpui-free e
//! testável** (estado dos campos, foco, validação, fase); [`render`] é a casca fina gpui sobre
//! [`crate::ui::Modal`]. Duas fases na MESMA janela: (1) FORMULÁRIO (endereço + chave) e, após
//! "Conectar", (2) PAREAMENTO (mostra o QR + status). A chamada de rede (connect/QR/poll) roda
//! **off-critical-path** numa thread de fundo (ADR 0046 — nunca trava o canvas); ela deposita o
//! progresso em [`ChannelConnectState`], que esta tela só LÊ. A chave secreta sai UMA vez no plano de
//! commit (rumo ao cofre) e nunca persiste na struct (doutrina ADR 0004, espelha `credential_modal`).

use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, ClickEvent, Context, Image, ImageFormat, Pixels,
    SharedString, Size,
};

use crate::theme;
use crate::ui::{clamp_frame, ButtonVariant, Modal, ModalAction, RadiusExt};
use crate::WorkspaceView;

// ═══════════════════════ copy (auditável — zero jargão; fala humano) ═══════════════════════

pub const COPY_TITLE: &str = "Conectar seu WhatsApp";
pub const COPY_INTRO: &str =
    "Conecte sua conta lendo um QR Code, como no WhatsApp Web. A chave fica guardada com segurança \
     no seu computador e some da tela — nem a IA vê o valor depois.";
pub const COPY_ADDRESS_LABEL: &str = "Endereço do seu servidor de WhatsApp";
pub const COPY_ADDRESS_HELP: &str =
    "Onde seu WhatsApp está rodando. Deixe como está se for no seu próprio computador.";
pub const COPY_KEY_LABEL: &str = "Chave de acesso";
pub const COPY_KEY_PLACEHOLDER: &str = "Cole aqui — fica oculta";
pub const COPY_CONNECT: &str = "Conectar";
pub const COPY_CANCEL: &str = "Cancelar";
pub const COPY_ERR_INCOMPLETE: &str = "Preencha o endereço e a chave de acesso para conectar.";
pub const COPY_DISCARD_HINT: &str = "Aperte Esc de novo para descartar o que você digitou.";

// Fase de pareamento (após "Conectar").
pub const COPY_CONNECTING: &str = "Conectando ao seu WhatsApp…";
pub const COPY_SCAN_TITLE: &str = "Escaneie com o celular";
pub const COPY_SCAN_STEPS: &str =
    "No celular: WhatsApp › Aparelhos conectados › Conectar um aparelho — e aponte a câmera para o código.";
pub const COPY_SCAN_WAITING: &str = "Aguardando você escanear…";
pub const COPY_CONNECTED: &str = "Tudo certo! Seu WhatsApp está conectado.";
pub const COPY_DONE: &str = "Pronto";
pub const COPY_RETRY: &str = "Tentar de novo";

/// Endereço default — o mesmo loopback local-first do transporte (ADR 0050 §3). Duplicado como string
/// para manter o modal puro (sem acoplar a tela ao core); o servidor remoto (VPS) é colado pelo leigo.
// ponytail: espelha lina_core::channel_waha::DEFAULT_BASE_URL — string trivial, não vale acoplar a UI ao core.
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:3000";

/// O campo focado — Tab/↓ cicla na ordem de leitura (endereço → chave).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaField {
    Address,
    Key,
}

/// **Estado de PAREAMENTO** depositado pela thread de conexão (off-critical-path) e LIDO pela tela.
/// A thread escreve; o `render` desenha. Nenhum campo é segredo: o QR é público (some ao parear) e a
/// mensagem de erro é humana, sem token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelConnectState {
    /// Pedido de conexão em voo (POST da sessão) — antes do QR chegar.
    Connecting,
    /// O QR chegou; aguardando o leigo escanear. `qr_png` são os bytes PNG (públicos).
    AwaitingScan { qr_png: Vec<u8> },
    /// A sessão entrou em `WORKING` — conta pareada.
    Connected,
    /// Falhou (Waha fora do ar, chave errada, tempo esgotado) — mensagem já humanizada.
    Failed(String),
}

/// O plano de COMMIT do "Conectar" — leva a chave UMA vez rumo ao cofre + o endereço para a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectPlan {
    /// Endereço efetivo do servidor Waha (default local ou o remoto colado pelo leigo).
    pub base_url: String,
    /// A chave de acesso (`X-Api-Key`) — consumida no commit, NUNCA logada/exibida após conectar.
    pub api_key: String,
}

/// O modal — estado PURO (nenhum gpui; nenhum efeito até "Conectar").
pub struct WhatsAppModal {
    base_url: String,
    api_key: String,
    focus: WaField,
    error: Option<String>,
    /// `true` após "Conectar": a tela passa da FASE formulário para a FASE pareamento (QR/status).
    submitted: bool,
    /// Esc com conteúdo arma a confirmação de descarte (2º Esc fecha) — só na fase formulário.
    discard_armed: bool,
}

impl Default for WhatsAppModal {
    fn default() -> Self {
        Self::new()
    }
}

impl WhatsAppModal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Nunca tela em branco (invariante #6): nasce com o endereço local; o leigo edita se for remoto.
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            focus: WaField::Key,
            error: None,
            submitted: false,
            discard_armed: false,
        }
    }

    // ── getters (a tela lê; a chave em claro NUNCA sai daqui senão no plano de commit) ──
    #[must_use]
    pub fn focus_field(&self) -> WaField {
        self.focus
    }
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    /// Comprimento da chave digitada — a tela mascara com `•` × este número. Nunca o valor.
    #[must_use]
    pub fn key_len(&self) -> usize {
        self.api_key.chars().count()
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[must_use]
    pub fn submitted(&self) -> bool {
        self.submitted
    }
    #[must_use]
    pub fn discard_armed(&self) -> bool {
        self.discard_armed
    }

    // ── mutadores ──
    /// Digita no campo focado. Limpa o erro e desarma o descarte.
    pub fn type_char(&mut self, s: &str) {
        self.error = None;
        self.discard_armed = false;
        self.focused_buf_mut().push_str(s);
    }

    /// Apaga o último caractere do campo focado.
    pub fn backspace(&mut self) {
        self.error = None;
        self.discard_armed = false;
        self.focused_buf_mut().pop();
    }

    /// Move o foco para o campo dado (clique na tela).
    pub fn set_focus(&mut self, f: WaField) {
        self.focus = f;
    }

    /// Cicla o foco na ordem de leitura: `dir > 0` avança (Tab), `dir < 0` volta; dá a volta nas pontas.
    pub fn cycle_focus(&mut self, dir: i32) {
        const ORDER: [WaField; 2] = [WaField::Address, WaField::Key];
        let idx = ORDER.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = ORDER.len() as i32;
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        let next = (((idx as i32 + dir) % n) + n) % n;
        self.focus = ORDER[next as usize];
    }

    /// Esc: na fase de pareamento sempre fecha (`true`). No formulário com conteúdo, o 1º arma o descarte
    /// (`false`, segue aberto) e o 2º fecha; vazio fecha de imediato — não perde o digitado por acidente.
    pub fn escape(&mut self) -> bool {
        if self.submitted {
            return true;
        }
        let has_content = !self.api_key.is_empty() || self.base_url != DEFAULT_BASE_URL;
        if !has_content || self.discard_armed {
            return true;
        }
        self.discard_armed = true;
        false
    }

    /// `true` quando endereço e chave têm conteúdo (após trim).
    #[must_use]
    pub fn can_commit(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }

    /// Constrói o plano (CONSUMINDO a chave da struct), marca `submitted` e passa para a fase de
    /// pareamento. Incompleto → registra o erro leve e devolve `None` (segue no formulário).
    pub fn commit(&mut self) -> Option<ConnectPlan> {
        if !self.can_commit() {
            self.error = Some(COPY_ERR_INCOMPLETE.to_string());
            return None;
        }
        self.submitted = true;
        Some(ConnectPlan {
            base_url: self.base_url.trim().to_string(),
            // `mem::take` esvazia a chave da struct: o segredo não fica residente após o commit.
            api_key: std::mem::take(&mut self.api_key),
        })
    }

    /// "Tentar de novo" após falha: volta ao formulário (a chave foi consumida no commit, então o leigo
    /// a recola). Mantém o endereço para não fazê-lo digitar de novo o servidor.
    pub fn reset_to_form(&mut self) {
        self.submitted = false;
        self.error = None;
        self.discard_armed = false;
        self.focus = WaField::Key;
    }

    /// Buffer do campo focado (para `type_char`/`backspace`).
    fn focused_buf_mut(&mut self) -> &mut String {
        match self.focus {
            WaField::Address => &mut self.base_url,
            WaField::Key => &mut self.api_key,
        }
    }
}

// Dimensões do modal (consts nomeadas — padrão da casa, fora da catraca de tokens, espelha
// `credential_modal::MODAL_W`; `px(CONST)` não é literal cru).
const MODAL_W: f32 = 480.0;
const MODAL_W_MIN: f32 = 320.0;
const MODAL_H_FLOOR: f32 = 260.0;
/// Lado do QR renderizado (px). Const nomeada — não conta na catraca (não é literal cru dentro de `px(`).
const QR_SIZE: f32 = 240.0;

// ───────────────────────────────── render (casca fina gpui) ─────────────────────────────────

/// Pinta o modal sobre [`Modal`] (role=Dialog + aria + oclusão). Duas fases: FORMULÁRIO (antes de
/// "Conectar") e PAREAMENTO (lê `connect` — QR/status depositado pela thread off-critical-path).
pub fn render(
    modal: &WhatsAppModal,
    connect: Option<&ChannelConnectState>,
    viewport: Size<Pixels>,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = theme::active();
    let frame = clamp_frame(
        f32::from(viewport.width),
        f32::from(viewport.height),
        MODAL_W,
        MODAL_W_MIN,
        f32::from(t.spacing.lg),
        MODAL_H_FLOOR,
    );

    let mut m = Modal::new("whatsapp-modal", frame)
        .title(COPY_TITLE)
        .aria(COPY_TITLE)
        .dim(true)
        .close(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_cancel(cx)))
        .dismiss_on_backdrop(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_cancel(cx)));

    if !modal.submitted() {
        // ── FASE 1 · formulário (endereço + chave) ──
        m = m
            .body(form_body(modal, cx))
            .action(ModalAction::new(
                "wa-cancel",
                COPY_CANCEL,
                ButtonVariant::Secondary,
                cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_cancel(cx)),
            ))
            .action(ModalAction::new(
                "wa-connect",
                COPY_CONNECT,
                ButtonVariant::Confirm,
                cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_commit(cx)),
            ));
    } else {
        // ── FASE 2 · pareamento (QR/status da thread off-critical-path) ──
        m = m.body(pairing_body(connect));
        match connect {
            Some(ChannelConnectState::Connected) => {
                m = m.action(ModalAction::new(
                    "wa-done",
                    COPY_DONE,
                    ButtonVariant::Confirm,
                    cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_cancel(cx)),
                ));
            }
            Some(ChannelConnectState::Failed(_)) => {
                m = m.action(ModalAction::new(
                    "wa-retry",
                    COPY_RETRY,
                    ButtonVariant::Confirm,
                    cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_retry(cx)),
                ));
            }
            // Conectando / aguardando scan: só "Cancelar" (desistir do pareamento).
            _ => {
                m = m.action(ModalAction::new(
                    "wa-cancel-pair",
                    COPY_CANCEL,
                    ButtonVariant::Secondary,
                    cx.listener(|v, _ev: &ClickEvent, _w, cx| v.whatsapp_cancel(cx)),
                ));
            }
        }
    }

    m.into_any_element()
}

/// Corpo da fase formulário: intro de segurança + endereço + chave (mascarada) + erro/aviso de descarte.
fn form_body(modal: &WhatsAppModal, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let t = theme::active();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.md)))
        .child(muted_text(COPY_INTRO))
        .child(field_row(
            cx,
            "wa-address",
            COPY_ADDRESS_LABEL,
            modal.base_url(),
            Some(COPY_ADDRESS_HELP),
            modal.focus_field() == WaField::Address,
            WaField::Address,
        ))
        .child(field_row(
            cx,
            "wa-key",
            COPY_KEY_LABEL,
            &"•".repeat(modal.key_len()),
            None,
            modal.focus_field() == WaField::Key,
            WaField::Key,
        ));

    if let Some(err) = modal.error() {
        body = body.child(
            div()
                .text_size(px(f32::from(t.typography.size.small)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.state.danger))
                .child(SharedString::from(err.to_string())),
        );
    } else if modal.discard_armed() {
        body = body.child(
            div()
                .text_size(px(f32::from(t.typography.size.small)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.state.warning))
                .child(COPY_DISCARD_HINT),
        );
    }
    body.into_any_element()
}

/// Corpo da fase pareamento: desenha o estado depositado pela thread (spinner/QR/sucesso/erro).
fn pairing_body(connect: Option<&ChannelConnectState>) -> AnyElement {
    let t = theme::active();
    let col = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(f32::from(t.spacing.md)));
    match connect {
        Some(ChannelConnectState::AwaitingScan { qr_png }) => col
            .child(strong_text(COPY_SCAN_TITLE))
            .child(muted_text(COPY_SCAN_STEPS))
            .child(
                // QR num cartão branco (o leitor do celular precisa de fundo claro + contraste).
                div()
                    .p(px(f32::from(t.spacing.md)))
                    .rounded_content()
                    .bg(rgb(t.text.bright))
                    .child(
                        img(Arc::new(Image::from_bytes(
                            ImageFormat::Png,
                            qr_png.clone(),
                        )))
                        .w(px(QR_SIZE))
                        .h(px(QR_SIZE)),
                    ),
            )
            .child(muted_text(COPY_SCAN_WAITING))
            .into_any_element(),
        Some(ChannelConnectState::Connected) => {
            col.child(strong_text(COPY_CONNECTED)).into_any_element()
        }
        Some(ChannelConnectState::Failed(msg)) => col
            .child(
                div()
                    .text_size(px(f32::from(t.typography.size.body)))
                    .font_family(t.typography.family.ui)
                    .text_color(rgb(t.state.danger))
                    .child(SharedString::from(msg.clone())),
            )
            .into_any_element(),
        // Connecting / ainda sem estado: spinner textual honesto.
        _ => col.child(muted_text(COPY_CONNECTING)).into_any_element(),
    }
}

/// Texto secundário (cor `muted`, tamanho body) — reuso interno das duas fases.
fn muted_text(s: &'static str) -> impl IntoElement {
    let t = theme::active();
    div()
        .text_size(px(f32::from(t.typography.size.body)))
        .font_family(t.typography.family.ui)
        .text_color(rgb(t.text.muted))
        .child(s)
}

/// Texto de destaque (cor `bright`, peso semibold) — títulos das fases.
fn strong_text(s: &'static str) -> impl IntoElement {
    let t = theme::active();
    div()
        .text_size(px(f32::from(t.typography.size.body)))
        .font_family(t.typography.family.ui)
        .font_weight(gpui::FontWeight(f32::from(t.typography.weight.semibold)))
        .text_color(rgb(t.text.bright))
        .child(s)
}

/// Uma linha rótulo + (ajuda opcional) + caixa de texto. `shown` já vem mascarado para a chave.
fn field_row(
    cx: &mut Context<WorkspaceView>,
    id: &'static str,
    label: &'static str,
    shown: &str,
    help: Option<&'static str>,
    focused: bool,
    field: WaField,
) -> impl IntoElement {
    let t = theme::active();
    let (text_color, content): (u32, SharedString) = if shown.is_empty() {
        (t.text.muted, SharedString::from(COPY_KEY_PLACEHOLDER))
    } else {
        (t.text.bright, SharedString::from(shown.to_string()))
    };
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.xs)))
        .child(
            div()
                .text_size(px(f32::from(t.typography.size.small)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.text.primary))
                .child(label),
        );
    if let Some(help) = help {
        col = col.child(
            div()
                .text_size(px(f32::from(t.typography.size.small)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.text.muted))
                .child(help),
        );
    }
    col.child(
        div()
            .id(id)
            .w_full()
            .min_w(px(0.))
            .overflow_hidden()
            .px(px(f32::from(t.spacing.md)))
            .py(px(f32::from(t.spacing.sm)))
            .rounded_content()
            .bg(rgb(t.surface.chrome))
            .border_1()
            .border_color(rgb(if focused {
                t.focus.ring
            } else {
                t.surface.border
            }))
            .text_size(px(f32::from(t.typography.size.body)))
            .font_family(t.typography.family.ui)
            .text_color(rgb(text_color))
            .cursor_pointer()
            .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                v.whatsapp_focus(field, cx);
            }))
            .child(content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_modal_starts_on_key_with_default_address() {
        let m = WhatsAppModal::new();
        assert_eq!(m.focus_field(), WaField::Key);
        assert_eq!(
            m.base_url(),
            DEFAULT_BASE_URL,
            "nasce com o endereço local (nunca em branco)"
        );
        assert_eq!(m.key_len(), 0);
        assert!(!m.submitted());
        assert!(!m.can_commit(), "falta a chave → não conecta");
    }

    #[test]
    fn typing_key_then_commit_consumes_secret_and_advances_phase() {
        let mut m = WhatsAppModal::new();
        m.set_focus(WaField::Key);
        for c in "minha-chave-secreta".chars() {
            m.type_char(&c.to_string());
        }
        assert!(m.can_commit());
        let plan = m.commit().expect("commit completo devolve plano");
        assert_eq!(
            plan.api_key, "minha-chave-secreta",
            "a chave sai UMA vez no plano"
        );
        assert_eq!(plan.base_url, DEFAULT_BASE_URL);
        assert!(m.submitted(), "passou para a fase de pareamento");
        assert_eq!(
            m.key_len(),
            0,
            "a chave não fica residente na struct após o commit"
        );
    }

    #[test]
    fn cannot_commit_without_key_registers_soft_error() {
        let mut m = WhatsAppModal::new();
        assert!(!m.can_commit(), "só o endereço default, sem chave");
        assert_eq!(m.commit(), None);
        assert_eq!(m.error(), Some(COPY_ERR_INCOMPLETE));
        assert!(!m.submitted(), "incompleto não avança de fase");
    }

    #[test]
    fn editing_address_to_remote_is_preserved() {
        let mut m = WhatsAppModal::new();
        m.set_focus(WaField::Address);
        for _ in 0..DEFAULT_BASE_URL.chars().count() {
            m.backspace();
        }
        for c in "http://100.64.0.1:3000".chars() {
            m.type_char(&c.to_string());
        }
        m.set_focus(WaField::Key);
        m.type_char("k");
        let plan = m.commit().expect("commit");
        assert_eq!(
            plan.base_url, "http://100.64.0.1:3000",
            "o endereço remoto colado é usado"
        );
    }

    #[test]
    fn tab_cycles_between_the_two_fields() {
        let mut m = WhatsAppModal::new();
        m.set_focus(WaField::Address);
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), WaField::Key);
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), WaField::Address, "dá a volta");
    }

    #[test]
    fn retry_after_failure_returns_to_form() {
        let mut m = WhatsAppModal::new();
        m.set_focus(WaField::Key);
        m.type_char("k");
        m.commit();
        assert!(m.submitted());
        m.reset_to_form();
        assert!(!m.submitted(), "tentar de novo volta ao formulário");
        assert_eq!(m.focus_field(), WaField::Key);
    }

    #[test]
    fn escape_in_form_with_content_arms_then_closes() {
        let mut m = WhatsAppModal::new();
        m.type_char("x"); // foco inicial é a chave
        assert!(!m.escape(), "1º Esc com conteúdo ARMA");
        assert!(m.discard_armed());
        assert!(m.escape(), "2º Esc fecha");
    }

    #[test]
    fn escape_in_pairing_phase_closes_immediately() {
        let mut m = WhatsAppModal::new();
        m.type_char("k");
        m.commit();
        assert!(m.escape(), "na fase de pareamento, Esc fecha direto");
    }

    #[test]
    fn no_plaintext_key_getter_exists() {
        // Catraca de DESIGN: a única saída da chave em claro é o `api_key` do plano no commit.
        // A struct só expõe `key_len` (comprimento p/ máscara).
        let mut m = WhatsAppModal::new();
        m.set_focus(WaField::Key);
        m.type_char("s3cr3t");
        assert_eq!(m.key_len(), 6);
    }
}
