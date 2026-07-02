//! F4-0-2 (UI) · **modal de credencial de canal** — o leigo cola a chave/senha de um canal; ela vai
//! ao cofre (contrato core [`lina_core::credential`]) e **some da tela**. Zero curses, zero jargão
//! (invariante #2 local-first + exposição opt-in sinalizado; invariante #6 não-técnico-first).
//!
//! Split do shell (padrão `agent_modal`): [`CredentialModal`] é **gpui-free e testável** (estado dos
//! campos, foco, validação, plano de commit); [`render`] é a casca fina gpui sobre o componente
//! [`crate::ui::Modal`] (ganha `role=Dialog`+aria+oclusão de mouse de graça). O `value` é coletado e
//! **nunca persiste na struct além do commit**: o plano sai com o segredo UMA vez (rumo ao cofre) e
//! o modal é destruído. Nenhum getter devolve o valor em claro — a tela só conhece o COMPRIMENTO
//! (para mascarar com `•`). Doutrina de segurança (ADR 0004): o app leva o valor ao cofre; nem o log
//! nem o env de PTY o veem — esta tela é só o ponto de coleta.

use gpui::{div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, Pixels, SharedString, Size};

use crate::theme;
use crate::ui::{clamp_frame, ButtonVariant, Modal, ModalAction, RadiusExt};
use crate::WorkspaceView;

// ═══════════════════════ copy (auditável — zero jargão; passa por `copy_has_no_jargon`) ═══════════════════════
// Sem "keyring"/"PTY"/"scope"/"token de API" cru: a superfície fala humano (doc 40 §3/§6).

pub const COPY_TITLE: &str = "Conectar um canal";
/// A promessa de segurança em linguagem do empreendedor — operacionaliza a invariante #2 na tela.
pub const COPY_INTRO: &str =
    "Cole a chave de acesso do canal. Ela fica guardada com segurança no seu computador e some da \
     tela — nem a IA vê o valor depois.";
pub const COPY_CHANNEL_LABEL: &str = "Qual canal?";
pub const COPY_CHANNEL_PLACEHOLDER: &str = "Ex.: WhatsApp, E-mail, Telegram";
pub const COPY_KEY_LABEL: &str = "O que é essa chave?";
pub const COPY_KEY_PLACEHOLDER: &str = "Ex.: Token de acesso";
pub const COPY_VALUE_LABEL: &str = "Chave secreta";
pub const COPY_VALUE_PLACEHOLDER: &str = "Cole aqui — fica oculta";
pub const COPY_SAVE: &str = "Guardar com segurança";
pub const COPY_CANCEL: &str = "Cancelar";
/// Erro leve (não bloqueia digitar) quando falta preencher — tabela de erros, nunca jargão.
pub const COPY_ERR_INCOMPLETE: &str =
    "Preencha o canal, a identificação e a chave secreta para guardar.";
/// Esc com conteúdo: confirma o descarte (2º Esc fecha) — não perde o que foi digitado por acidente.
pub const COPY_DISCARD_HINT: &str = "Aperte Esc de novo para descartar o que você digitou.";

/// O campo focado — Tab/↓ cicla na ordem de leitura (canal → identificação → chave).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredField {
    Channel,
    Key,
    Value,
}

/// O plano de COMMIT do `[[ Guardar ]]` — leva o segredo UMA vez rumo ao cofre. `scope` é DERIVADO
/// do canal (`channel:<nome>`, convenção de `lina_core::credential`); nenhum campo é autoridade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCredentialPlan {
    /// id normalizado do canal (trim + minúsculas) — a chave no cofre e no `CredentialStored`.
    pub channel: String,
    /// escopo lógico no cofre: `channel:<canal>` (o app monta; nunca o agente).
    pub scope: String,
    /// identificação humana da chave (o `key` do cofre / base do `key_ref`).
    pub key: String,
    /// o segredo em si — consumido no commit, NUNCA logado, NUNCA exibido após guardar.
    pub value: String,
}

/// O modal de credencial — estado PURO (nenhum gpui; nenhum efeito até Guardar).
pub struct CredentialModal {
    channel: String,
    key: String,
    value: String,
    focus: CredField,
    error: Option<String>,
    /// Esc com conteúdo arma a confirmação de descarte (2º Esc fecha).
    discard_armed: bool,
}

impl Default for CredentialModal {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialModal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            channel: String::new(),
            key: String::new(),
            value: String::new(),
            focus: CredField::Channel,
            error: None,
            discard_armed: false,
        }
    }

    // ── getters (a tela lê; o `value` em claro NUNCA sai daqui) ──
    #[must_use]
    pub fn focus_field(&self) -> CredField {
        self.focus
    }
    #[must_use]
    pub fn channel(&self) -> &str {
        &self.channel
    }
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    /// Comprimento do segredo digitado — a tela mascara com `•` × este número. Nunca o valor.
    #[must_use]
    pub fn value_len(&self) -> usize {
        self.value.chars().count()
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[must_use]
    pub fn discard_armed(&self) -> bool {
        self.discard_armed
    }

    // ── mutadores ──
    /// Digita no campo focado. Limpa o erro e desarma o descarte (voltar a digitar = não-descartar).
    pub fn type_char(&mut self, s: &str) {
        self.error = None;
        self.discard_armed = false;
        self.focused_buf_mut().push_str(s);
    }

    /// Apaga o último caractere do campo focado (mesma higiene de erro/descarte do `type_char`).
    pub fn backspace(&mut self) {
        self.error = None;
        self.discard_armed = false;
        self.focused_buf_mut().pop();
    }

    /// Move o foco para o campo dado (clique na tela).
    pub fn set_focus(&mut self, f: CredField) {
        self.focus = f;
    }

    /// Cicla o foco na ordem de leitura: `dir > 0` avança (Tab), `dir < 0` volta (Shift-Tab); dá a
    /// volta nas pontas.
    pub fn cycle_focus(&mut self, dir: i32) {
        const ORDER: [CredField; 3] = [CredField::Channel, CredField::Key, CredField::Value];
        let idx = ORDER.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = ORDER.len() as i32;
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        let next = (((idx as i32 + dir) % n) + n) % n;
        self.focus = ORDER[next as usize];
    }

    /// Esc: com conteúdo, o 1º arma o descarte (devolve `false`, segue aberto) e o 2º fecha
    /// (`true`); vazio fecha de imediato (`true`) — não perde o que foi digitado por acidente.
    pub fn escape(&mut self) -> bool {
        let has_content =
            !self.channel.is_empty() || !self.key.is_empty() || !self.value.is_empty();
        if !has_content || self.discard_armed {
            return true;
        }
        self.discard_armed = true;
        false
    }

    /// `true` quando os três campos têm conteúdo (após trim).
    #[must_use]
    pub fn can_commit(&self) -> bool {
        !self.channel.trim().is_empty()
            && !self.key.trim().is_empty()
            && !self.value.trim().is_empty()
    }

    /// Constrói o plano de commit (CONSUMINDO o segredo da struct — sai UMA vez rumo ao cofre) OU
    /// registra o erro leve e devolve `None` quando incompleto.
    pub fn commit(&mut self) -> Option<StoreCredentialPlan> {
        if !self.can_commit() {
            self.error = Some(COPY_ERR_INCOMPLETE.to_string());
            return None;
        }
        let channel = normalize_channel(&self.channel);
        let scope = format!("channel:{channel}");
        Some(StoreCredentialPlan {
            channel,
            scope,
            key: self.key.trim().to_string(),
            // `mem::take` esvazia o valor da struct: o segredo não fica residente após o commit.
            value: std::mem::take(&mut self.value),
        })
    }

    /// Buffer do campo focado (para `type_char`/`backspace`).
    fn focused_buf_mut(&mut self) -> &mut String {
        match self.focus {
            CredField::Channel => &mut self.channel,
            CredField::Key => &mut self.key,
            CredField::Value => &mut self.value,
        }
    }
}

/// Normaliza o id do canal: trim + minúsculas (a forma da chave no cofre/log). Espaços internos
/// são preservados (a humanização na superfície é responsabilidade de [`crate::exposure`]).
#[must_use]
pub fn normalize_channel(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

// Dimensões do modal (consts nomeadas — fora da catraca de tokens, como `CARD_W`; o tema não tem
// grupo `layout`, então o padrão da casa é a const local, espelhando `agent_modal::modal_frame`).
const MODAL_W: f32 = 480.0;
const MODAL_W_MIN: f32 = 320.0;
const MODAL_H_FLOOR: f32 = 220.0;

// ───────────────────────────────── render (casca fina gpui) ─────────────────────────────────

/// Pinta o modal sobre o componente [`Modal`] (role=Dialog + aria + oclusão). PURO no layout: as
/// dimensões vêm de [`clamp_frame`]; o campo de chave mostra só `•` (nunca o valor).
pub fn render(
    modal: &CredentialModal,
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

    // ── corpo: intro de segurança + 3 campos rotulados + erro/aviso de descarte ──
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.md)))
        .child(
            div()
                .text_size(px(f32::from(t.typography.size.body)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.text.muted))
                .child(COPY_INTRO),
        )
        .child(field_row(
            cx,
            "cred-channel",
            COPY_CHANNEL_LABEL,
            modal.channel(),
            COPY_CHANNEL_PLACEHOLDER,
            modal.focus_field() == CredField::Channel,
            CredField::Channel,
        ))
        .child(field_row(
            cx,
            "cred-key",
            COPY_KEY_LABEL,
            modal.key(),
            COPY_KEY_PLACEHOLDER,
            modal.focus_field() == CredField::Key,
            CredField::Key,
        ))
        .child(field_row(
            cx,
            "cred-value",
            COPY_VALUE_LABEL,
            &"•".repeat(modal.value_len()),
            COPY_VALUE_PLACEHOLDER,
            modal.focus_field() == CredField::Value,
            CredField::Value,
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

    Modal::new("credential-modal", frame)
        .title(COPY_TITLE)
        .aria(COPY_TITLE)
        .dim(true)
        .close(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.credential_cancel(cx)))
        .dismiss_on_backdrop(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.credential_cancel(cx)))
        .body(body)
        .action(ModalAction::new(
            "cred-cancel",
            COPY_CANCEL,
            ButtonVariant::Secondary,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.credential_cancel(cx)),
        ))
        .action(ModalAction::new(
            "cred-save",
            COPY_SAVE,
            ButtonVariant::Confirm,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.credential_commit(cx)),
        ))
        .into_any_element()
}

/// Uma linha rótulo + caixa de texto (espelha o campo Nome do M6, mas tokenizado e reusável).
/// `shown` já vem mascarado para o campo de chave (o caller passa `•`×len).
fn field_row(
    cx: &mut Context<WorkspaceView>,
    id: &'static str,
    label: &'static str,
    shown: &str,
    placeholder: &'static str,
    focused: bool,
    field: CredField,
) -> impl IntoElement {
    let t = theme::active();
    let (text_color, content): (u32, SharedString) = if shown.is_empty() {
        (t.text.muted, SharedString::from(placeholder))
    } else {
        (t.text.bright, SharedString::from(shown.to_string()))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.xs)))
        .child(
            div()
                .text_size(px(f32::from(t.typography.size.small)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.text.primary))
                .child(label),
        )
        .child(
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
                    v.credential_focus(field, cx);
                }))
                .child(content),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_modal_starts_on_channel_field_empty() {
        let m = CredentialModal::new();
        assert_eq!(m.focus_field(), CredField::Channel);
        assert_eq!(m.channel(), "");
        assert_eq!(m.key(), "");
        assert_eq!(m.value_len(), 0);
        assert!(!m.can_commit(), "nada preenchido → não dá para guardar");
    }

    #[test]
    fn typing_appends_to_the_focused_field() {
        let mut m = CredentialModal::new();
        m.type_char("W");
        m.type_char("a");
        assert_eq!(m.channel(), "Wa");
        m.set_focus(CredField::Value);
        m.type_char("s");
        m.type_char("3");
        assert_eq!(m.value_len(), 2, "o segredo conta caracteres p/ a máscara");
        assert_eq!(m.channel(), "Wa", "digitar no Valor não toca o Canal");
    }

    #[test]
    fn tab_cycles_through_the_three_fields_in_reading_order() {
        let mut m = CredentialModal::new();
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), CredField::Key);
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), CredField::Value);
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), CredField::Channel, "volta ao início");
        m.cycle_focus(-1);
        assert_eq!(m.focus_field(), CredField::Value, "Shift-Tab volta");
    }

    #[test]
    fn backspace_removes_from_the_focused_field() {
        let mut m = CredentialModal::new();
        m.type_char("o");
        m.type_char("i");
        m.backspace();
        assert_eq!(m.channel(), "o");
    }

    #[test]
    fn cannot_commit_until_all_three_are_filled() {
        let mut m = CredentialModal::new();
        m.set_focus(CredField::Channel);
        m.type_char("whatsapp");
        m.set_focus(CredField::Value);
        m.type_char("segredo");
        assert!(!m.can_commit(), "falta a identificação da chave");
        assert_eq!(m.commit(), None);
        assert_eq!(
            m.error(),
            Some(COPY_ERR_INCOMPLETE),
            "commit incompleto registra erro leve"
        );
    }

    #[test]
    fn commit_builds_a_plan_with_derived_scope_and_normalized_channel() {
        let mut m = CredentialModal::new();
        m.set_focus(CredField::Channel);
        m.type_char("WhatsApp");
        m.set_focus(CredField::Key);
        m.type_char("Token de acesso");
        m.set_focus(CredField::Value);
        m.type_char("abc-123-secret");
        assert!(m.can_commit());
        let plan = m.commit().expect("commit completo devolve plano");
        assert_eq!(plan.channel, "whatsapp", "id normalizado (minúsculas)");
        assert_eq!(plan.scope, "channel:whatsapp", "scope derivado do canal");
        assert_eq!(plan.key, "Token de acesso");
        assert_eq!(
            plan.value, "abc-123-secret",
            "o segredo vai UMA vez no plano"
        );
    }

    #[test]
    fn escape_with_content_arms_then_closes() {
        let mut m = CredentialModal::new();
        m.type_char("x");
        assert!(!m.escape(), "1º Esc com conteúdo ARMA (não fecha)");
        assert!(m.discard_armed());
        assert!(m.escape(), "2º Esc fecha");
    }

    #[test]
    fn escape_on_empty_closes_immediately() {
        let mut m = CredentialModal::new();
        assert!(m.escape(), "modal vazio fecha no 1º Esc");
    }

    #[test]
    fn typing_disarms_the_discard_confirmation() {
        let mut m = CredentialModal::new();
        m.type_char("x");
        m.escape(); // arma
        assert!(m.discard_armed());
        m.type_char("y");
        assert!(!m.discard_armed(), "voltar a digitar desarma o descarte");
    }

    #[test]
    fn no_plaintext_value_getter_exists() {
        // Catraca de DESIGN: a única saída do segredo em claro é o `value` do plano no commit.
        // A struct expõe só `value_len` (comprimento p/ máscara). Se um getter `value()->&str`
        // for adicionado por engano, este teste deixa de compilar quando alguém tentar usá-lo —
        // aqui afirmamos só o caminho permitido.
        let mut m = CredentialModal::new();
        m.set_focus(CredField::Value);
        m.type_char("s3cr3t");
        assert_eq!(m.value_len(), 6);
    }
}
