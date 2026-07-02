//! F4-WA-2c (UI) · **modal "Receber um aviso de fora"** — o leigo liga um aviso externo (webhook) a
//! um terminal vivo: escolhe QUAL terminal recebe, escreve O QUE FAZER quando o aviso chegar
//! (instrução = AUTORIDADE, obrigatória) e se a IA pode agir sozinha. O gesto vira o evento congelado
//! [`lina_core::events::DomainEvent::WebhookConfigured`]`{target_node,instruction,autonomy_level}`
//! (contrato da largada F4-WA, commit `ca471ee`). Zero jargão de webhook na fala: o leigo vê
//! "endereço" e "senha de segurança" (invariante #6 não-técnico-first).
//!
//! Doutrina de segurança (spec 55 §Segurança): a INSTRUÇÃO do dono é a única autoridade da ação — por
//! isso a UI **recusa salvar sem instrução** (sem ela não há o que decidir). Os DADOS do payload
//! externo são input não-confiável; a separação é reforçada na fala do toggle ("coisas sérias sempre
//! pedem confirmação" — o gate `gated-hard` é inquebrável, ADR 0004).
//!
//! Split do shell (padrão `credential_modal`/`agent_modal`): [`WebhookModal`] é **gpui-free e
//! testável** (estado dos campos, foco, validação, plano de commit, estágio); [`render`] é a casca
//! fina gpui sobre o componente [`crate::ui::Modal`] (ganha `role=Dialog`+aria+oclusão de mouse de
//! graça). O `secret` é GERADO server-side (engine `lina-webhooks` — fora desta tela); a view o
//! recebe por [`WebhookModal::present_outcome`] e o exibe UMA vez (vive em RAM no modal; o modal é
//! destruído ao fechar). O event log NUNCA carrega o secret (invariante #2 / spec 55 §5).

use gpui::{div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, Pixels, SharedString, Size};

use crate::theme;
use crate::ui::{clamp_frame, ButtonVariant, Modal, ModalAction, RadiusExt};
use crate::WorkspaceView;

// ═══════════════════════ copy (auditável — zero jargão; passa por `copy_has_no_jargon`) ═══════════════════════
// Sem "webhook"/"endpoint"/"HMAC"/"payload" cru: a superfície fala humano (spec 55 §Superfície).

pub const COPY_TITLE: &str = "Receber um aviso de fora";
/// Abre explicando, em uma frase, o que esta tela liga — sem citar "webhook".
pub const COPY_INTRO: &str =
    "Conecte um aviso de fora (um pedido, um pagamento, uma mensagem de outro app) a um dos seus \
     terminais. Quando o aviso chegar, a IA faz o que você pedir aqui.";

pub const COPY_NODE_LABEL: &str = "Qual terminal recebe o aviso?";
/// Quando o Espaço não tem nenhum terminal vivo — caminho do leigo nunca tem beco (inv #6).
pub const COPY_NODE_EMPTY: &str =
    "Você ainda não tem nenhum terminal aberto. Crie um agente primeiro e volte aqui.";

pub const COPY_INSTRUCTION_LABEL: &str = "O que fazer quando o aviso chegar?";
pub const COPY_INSTRUCTION_PLACEHOLDER: &str =
    "Ex.: quando um pedido novo chegar, registre na planilha e me mande um resumo.";

pub const COPY_AUTONOMY_LABEL: &str = "A IA pode agir sozinha?";
pub const COPY_AUTONOMY_NOTIFY: &str = "Me avisa antes";
pub const COPY_AUTONOMY_AUTO: &str = "Pode agir";
/// O gate inquebrável traduzido: mesmo em "Pode agir", o irreversível pede confirmação (ADR 0004).
pub const COPY_AUTONOMY_HINT: &str =
    "Mesmo com \u{201c}Pode agir\u{201d}, coisas sérias (mandar dinheiro, publicar, apagar) sempre \
     pedem sua confirmação.";

pub const COPY_SAVE: &str = "Criar o aviso";
pub const COPY_CANCEL: &str = "Cancelar";
pub const COPY_DONE: &str = "Pronto";
pub const COPY_COPY_URL: &str = "Copiar endereço";
pub const COPY_COPY_SECRET: &str = "Copiar senha";

/// Erro leve (não bloqueia digitar) quando falta o essencial — a instrução é a autoridade (spec 55).
pub const COPY_ERR_NO_INSTRUCTION: &str =
    "Escreva o que a IA deve fazer quando o aviso chegar — sem isso, não há o que ela faça.";
pub const COPY_ERR_NO_NODE: &str = "Escolha qual terminal vai receber o aviso.";

/// Esc com conteúdo: confirma o descarte (2º Esc fecha) — não perde o que foi escrito por acidente.
pub const COPY_DISCARD_HINT: &str = "Aperte Esc de novo para descartar o que você escreveu.";
/// Entre enviar e o servidor devolver o endereço/senha — estado honesto, sem número (inv #6).
pub const COPY_PENDING: &str = "Criando o aviso\u{2026}";

// ── estágio de resultado (após criar) ──
/// Título da etapa que mostra o endereço + a senha de segurança UMA vez.
pub const COPY_OUTCOME_TITLE: &str = "Pronto! Use este endereço e esta senha";
pub const COPY_OUTCOME_URL_LABEL: &str = "Endereço para receber o aviso";
pub const COPY_OUTCOME_SECRET_LABEL: &str = "Senha de segurança (aparece só agora — guarde)";
pub const COPY_OUTCOME_SECRET_WARN: &str =
    "Esta senha aparece uma única vez. Copie e guarde em lugar seguro — depois ela some daqui.";
/// Aviso honesto de exposição (invariante #2 local-first; túnel é opt-in sinalizado).
pub const COPY_OUTCOME_TUNNEL_WARN: &str =
    "Este endereço só funciona no seu computador. Se você ligar o acesso pela internet, ele fica \
     acessível de fora — ligue só se souber o que está fazendo.";

/// O campo focado pelo teclado — Tab/↓ cicla na ordem de leitura (terminal → instrução).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookField {
    /// A lista de terminais (↑/↓ escolhem quando focada; clique também escolhe).
    Terminal,
    /// O campo de instrução (o único que recebe texto digitado).
    Instruction,
}

/// Nível de ação do aviso (spec 55 §3). String canônica no evento congelado; aqui é enum p/ a UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebhookAutonomy {
    /// CONSERVADOR (default): a IA propõe e espera o "sim". Casa com `default_notify_before` do evento.
    #[default]
    NotifyBefore,
    /// A IA age sozinha — exceto no irreversível, que SEMPRE passa pelo gate humano (ADR 0004).
    Autonomous,
}

impl WebhookAutonomy {
    /// A string canônica gravada em `WebhookConfigured.autonomy_level` (spec 55 §3 — string, não enum).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WebhookAutonomy::NotifyBefore => "notify_before",
            WebhookAutonomy::Autonomous => "autonomous",
        }
    }
}

/// Qual etapa o modal está mostrando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookStage {
    /// Preenchendo a config (terminal + instrução + nível).
    Form,
    /// Config enviada; aguardando o servidor gerar endereço/senha (entre o commit e o `present_outcome`).
    Pending,
    /// Endereço + senha gerados — exibidos UMA vez.
    Outcome,
}

/// O endereço + a senha gerados pelo servidor (engine `lina-webhooks`), entregues à view por
/// [`WebhookModal::present_outcome`]. A `secret` é exibida UMA vez e vive só em RAM no modal.
///
/// **`Debug` MANUAL que redige o secret** (não `derive`): um `{:?}`/`tracing` futuro NUNCA vaza a
/// senha em claro — defesa anti Debug-leak (este codebase já teve um na F3). A url fica visível
/// (não é segredo). Os testes comparam campos (`o.url`/`o.secret`), nunca `{:?}` do struct.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookOutcome {
    /// `http://127.0.0.1:<porta>/hook/<hook_id>` — montado pelo servidor (conhece porta + hook_id).
    pub url: String,
    /// A senha de segurança (HMAC) — mostrada 1×; o event log NUNCA a contém (spec 55 §5).
    pub secret: String,
}

impl std::fmt::Debug for WebhookOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookOutcome")
            .field("url", &self.url)
            .field("secret", &"<redigido>")
            .finish()
    }
}

/// O plano de COMMIT do `[[ Criar o aviso ]]` — vira o gesto `webhook.configure` rumo ao core, que
/// emite `WebhookConfigured{target_node,instruction,autonomy_level}`. Nenhum campo é autoridade: o
/// `target_node` é o NOME do terminal escolhido (resolução A2A no core, não aqui).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureWebhookPlan {
    /// terminal dono do aviso (por NOME — ADR 0026).
    pub target_node: String,
    /// a INSTRUÇÃO do usuário = AUTORIDADE. Garantida não-vazia (a UI recusa salvar sem ela).
    pub instruction: String,
    /// `"notify_before"` | `"autonomous"` — a string canônica do evento.
    pub autonomy_level: String,
}

/// O modal de webhook — estado PURO (nenhum gpui; nenhum efeito até Criar).
pub struct WebhookModal {
    /// Nomes dos terminais vivos (as opções de quem recebe). Vazio = Espaço sem terminal.
    live_terminals: Vec<String>,
    /// Índice do terminal escolhido em `live_terminals` (`None` = nenhum escolhido ainda).
    selected: Option<usize>,
    instruction: String,
    autonomy: WebhookAutonomy,
    focus: WebhookField,
    error: Option<String>,
    /// Esc com conteúdo arma a confirmação de descarte (2º Esc fecha).
    discard_armed: bool,
    /// `None` no formulário; `Some` após o servidor devolver endereço/senha (estágio resultado).
    outcome: Option<WebhookOutcome>,
    /// `true` entre o commit e a chegada do `outcome` (config enviada, aguardando o servidor).
    pending: bool,
}

impl WebhookModal {
    /// Abre o modal com a lista de terminais vivos. Se houver exatamente UM, já o pré-seleciona (o
    /// leigo não precisa escolher onde só há uma opção).
    #[must_use]
    pub fn new(live_terminals: Vec<String>) -> Self {
        let selected = if live_terminals.len() == 1 {
            Some(0)
        } else {
            None
        };
        Self {
            live_terminals,
            selected,
            instruction: String::new(),
            autonomy: WebhookAutonomy::NotifyBefore,
            focus: WebhookField::Terminal,
            error: None,
            discard_armed: false,
            outcome: None,
            pending: false,
        }
    }

    // ── getters (a tela lê) ──
    #[must_use]
    pub fn focus_field(&self) -> WebhookField {
        self.focus
    }
    #[must_use]
    pub fn live_terminals(&self) -> &[String] {
        &self.live_terminals
    }
    /// Nome do terminal escolhido (`None` enquanto nada foi escolhido).
    #[must_use]
    pub fn target_node(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.live_terminals.get(i))
            .map(String::as_str)
    }
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }
    #[must_use]
    pub fn instruction(&self) -> &str {
        &self.instruction
    }
    #[must_use]
    pub fn autonomy(&self) -> WebhookAutonomy {
        self.autonomy
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[must_use]
    pub fn discard_armed(&self) -> bool {
        self.discard_armed
    }
    #[must_use]
    pub fn outcome(&self) -> Option<&WebhookOutcome> {
        self.outcome.as_ref()
    }
    /// Qual etapa mostrar: resultado (tem outcome) > aguardando (pending) > formulário.
    #[must_use]
    pub fn stage(&self) -> WebhookStage {
        if self.outcome.is_some() {
            WebhookStage::Outcome
        } else if self.pending {
            WebhookStage::Pending
        } else {
            WebhookStage::Form
        }
    }

    // ── mutadores (só valem no formulário) ──
    /// Digita no campo de instrução (o único que recebe texto). Limpa o erro e desarma o descarte.
    pub fn type_char(&mut self, s: &str) {
        if self.stage() != WebhookStage::Form {
            return;
        }
        self.error = None;
        self.discard_armed = false;
        if self.focus == WebhookField::Instruction {
            self.instruction.push_str(s);
        }
    }

    /// Apaga o último caractere da instrução (mesma higiene de erro/descarte do `type_char`).
    pub fn backspace(&mut self) {
        if self.stage() != WebhookStage::Form || self.focus != WebhookField::Instruction {
            return;
        }
        self.error = None;
        self.discard_armed = false;
        self.instruction.pop();
    }

    /// Move o foco para o campo dado (clique na tela).
    pub fn set_focus(&mut self, f: WebhookField) {
        self.focus = f;
    }

    /// Cicla o foco terminal ↔ instrução: `dir > 0` avança (Tab), `dir < 0` volta (Shift-Tab).
    pub fn cycle_focus(&mut self, dir: i32) {
        const ORDER: [WebhookField; 2] = [WebhookField::Terminal, WebhookField::Instruction];
        let idx = ORDER.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = ORDER.len() as i32;
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        let next = (((idx as i32 + dir) % n) + n) % n;
        self.focus = ORDER[next as usize];
    }

    /// Escolhe o terminal pelo índice (clique numa linha da lista). Fora do alcance = no-op.
    pub fn select_node(&mut self, idx: usize) {
        if idx < self.live_terminals.len() {
            self.selected = Some(idx);
            self.error = None;
        }
    }

    /// Move a escolha de terminal por teclado (↑/↓ quando a lista está focada). Sem escolha ainda, a 1ª
    /// tecla pega a ponta (↓ = primeiro, ↑ = último); depois dá a volta nas pontas. No-op se não há
    /// terminal.
    pub fn cycle_node(&mut self, dir: i32) {
        let n = self.live_terminals.len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            None => {
                if dir < 0 {
                    n - 1
                } else {
                    0
                }
            }
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            Some(cur) => ((((cur as i32) + dir) % n as i32 + n as i32) % n as i32) as usize,
        };
        self.selected = Some(next);
        self.error = None;
    }

    /// Fixa o nível (clique num dos dois segmentos: "Me avisa antes" / "Pode agir").
    pub fn set_autonomy(&mut self, level: WebhookAutonomy) {
        self.autonomy = level;
    }

    /// Esc: no resultado/aguardando, fecha de imediato (`true`). No formulário com conteúdo, o 1º arma
    /// o descarte (`false`, segue aberto) e o 2º fecha (`true`); vazio fecha de imediato.
    pub fn escape(&mut self) -> bool {
        if self.stage() != WebhookStage::Form {
            return true;
        }
        let has_content = !self.instruction.is_empty() || self.selected.is_some();
        if !has_content || self.discard_armed {
            return true;
        }
        self.discard_armed = true;
        false
    }

    /// `true` quando há terminal escolhido E instrução não-vazia (após trim) — a instrução é a
    /// autoridade; sem ela não há ação (spec 55 §Segurança).
    #[must_use]
    pub fn can_commit(&self) -> bool {
        self.selected.is_some() && !self.instruction.trim().is_empty()
    }

    /// Constrói o plano de commit e entra em "aguardando" (o servidor gera endereço/senha), OU
    /// registra o erro leve e devolve `None` quando incompleto. A instrução é trimada (autoridade
    /// limpa); o nível vira a string canônica do evento.
    pub fn commit(&mut self) -> Option<ConfigureWebhookPlan> {
        // Só o formulário cria; já-aguardando/já-pronto não re-emite o gesto (anti duplo-envio).
        if self.stage() != WebhookStage::Form {
            return None;
        }
        if !self.can_commit() {
            // O mesmo predicado da tela, com a mensagem específica do que falta (a instrução é a
            // autoridade — sem terminal OU sem ela, a UI recusa salvar; spec 55 §Segurança).
            self.error = Some(
                if self.target_node().is_none() {
                    COPY_ERR_NO_NODE
                } else {
                    COPY_ERR_NO_INSTRUCTION
                }
                .to_string(),
            );
            return None;
        }
        let node = self.target_node().map(str::to_string).unwrap_or_default();
        let instruction = self.instruction.trim().to_string();
        self.pending = true;
        self.error = None;
        Some(ConfigureWebhookPlan {
            target_node: node,
            instruction,
            autonomy_level: self.autonomy.as_str().to_string(),
        })
    }

    /// O servidor devolveu endereço + senha → vai para o estágio de resultado (exibe 1×). Idempotente
    /// no sentido de que sobrescreve um outcome anterior (o último vence).
    ///
    /// Dirigida pela costura de retorno do servidor (F4-WA-1): a engine `lina-webhooks` gera
    /// `hook_id`+secret e o `take_webhook_outcome` do `NodeManager` entrega à view, que chama isto pelo
    /// loop de poll `drain_webhook_outcome`.
    pub fn present_outcome(&mut self, outcome: WebhookOutcome) {
        self.pending = false;
        self.error = None;
        self.outcome = Some(outcome);
    }
}

// Dimensões do modal (consts nomeadas — fora da catraca de tokens; padrão de `credential_modal`:
// o tema não tem grupo `layout`, então a const local é a casa, e `px(MODAL_W)` não conta literal cru).
const MODAL_W: f32 = 520.0;
const MODAL_W_MIN: f32 = 340.0;
const MODAL_H_FLOOR: f32 = 280.0;
/// Altura mínima da caixa de instrução (≈ 3 linhas) — o leigo escreve uma frase, não uma palavra.
const INSTRUCTION_MIN_H: f32 = 84.0;

// ───────────────────────────────── render (casca fina gpui) ─────────────────────────────────

/// Pinta o modal sobre o componente [`Modal`] (role=Dialog + aria + oclusão). PURO no layout: as
/// dimensões vêm de [`clamp_frame`]; o conteúdo muda com o [`WebhookModal::stage`].
pub fn render(
    modal: &WebhookModal,
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

    match modal.stage() {
        WebhookStage::Outcome => render_outcome(modal, frame, cx),
        WebhookStage::Pending | WebhookStage::Form => render_form(modal, frame, cx),
    }
}

/// Etapa 1: o formulário (terminal + instrução + nível). `Pending` reusa o formulário com as ações
/// desabilitadas pela ausência de outcome — visualmente o usuário vê o que enviou até o servidor voltar.
fn render_form(
    modal: &WebhookModal,
    frame: crate::ui::Frame,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = theme::active();

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
        .child(node_selector(modal, cx))
        .child(instruction_field(modal, cx))
        .child(autonomy_toggle(modal, cx));

    if modal.stage() == WebhookStage::Pending {
        body = body.child(notice(&t, COPY_PENDING, t.text.muted));
    } else if let Some(err) = modal.error() {
        body = body.child(notice(&t, err, t.state.danger));
    } else if modal.discard_armed() {
        body = body.child(notice(&t, COPY_DISCARD_HINT, t.state.warning));
    }

    Modal::new("webhook-modal", frame)
        .title(COPY_TITLE)
        .aria(COPY_TITLE)
        .dim(true)
        .close(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_cancel(cx)))
        .dismiss_on_backdrop(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_cancel(cx)))
        .body(body)
        .action(ModalAction::new(
            "webhook-cancel",
            COPY_CANCEL,
            ButtonVariant::Secondary,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_cancel(cx)),
        ))
        .action(ModalAction::new(
            "webhook-save",
            COPY_SAVE,
            ButtonVariant::Confirm,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_commit(cx)),
        ))
        .into_any_element()
}

/// Etapa 2: o resultado — endereço + senha (1×) + avisos. Sem campo de entrada; só leitura + copiar.
fn render_outcome(
    modal: &WebhookModal,
    frame: crate::ui::Frame,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = theme::active();
    let outcome = modal.outcome();
    let url = outcome.map(|o| o.url.clone()).unwrap_or_default();
    let secret = outcome.map(|o| o.secret.clone()).unwrap_or_default();

    let body = div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.md)))
        .child(
            div()
                .text_size(px(f32::from(t.typography.size.body)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(t.text.muted))
                .child(COPY_OUTCOME_TITLE),
        )
        .child(readonly_value(
            &t,
            COPY_OUTCOME_URL_LABEL,
            &url,
            "webhook-copy-url",
            COPY_COPY_URL,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_copy_url(cx)),
        ))
        .child(notice(&t, COPY_OUTCOME_TUNNEL_WARN, t.text.muted))
        .child(readonly_value(
            &t,
            COPY_OUTCOME_SECRET_LABEL,
            &secret,
            "webhook-copy-secret",
            COPY_COPY_SECRET,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_copy_secret(cx)),
        ))
        .child(notice(&t, COPY_OUTCOME_SECRET_WARN, t.state.warning));

    Modal::new("webhook-modal", frame)
        .title(COPY_OUTCOME_TITLE)
        .aria(COPY_OUTCOME_TITLE)
        .dim(true)
        .close(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_cancel(cx)))
        .body(body)
        .action(ModalAction::new(
            "webhook-done",
            COPY_DONE,
            ButtonVariant::Confirm,
            cx.listener(|v, _ev: &ClickEvent, _w, cx| v.webhook_cancel(cx)),
        ))
        .into_any_element()
}

/// A lista de terminais vivos (radio-style: clique escolhe; o escolhido ganha a borda de acento). É
/// um "dropdown" achatado em lista — para um roster pequeno, escolher 1 clique vence o popover.
fn node_selector(modal: &WebhookModal, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let t = theme::active();
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.xs)))
        .child(field_label(&t, COPY_NODE_LABEL));

    if modal.live_terminals().is_empty() {
        return col.child(notice(&t, COPY_NODE_EMPTY, t.text.muted));
    }

    let focused_list = modal.focus_field() == WebhookField::Terminal;
    for (idx, name) in modal.live_terminals().iter().enumerate() {
        let chosen = modal.selected_index() == Some(idx);
        let border = if chosen {
            t.accent.confirm
        } else if focused_list {
            t.focus.ring
        } else {
            t.surface.border
        };
        let row_id = SharedString::from(format!("webhook-node-{idx}"));
        col = col.child(
            div()
                .id(row_id)
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(f32::from(t.spacing.sm)))
                .px(px(f32::from(t.spacing.md)))
                .py(px(f32::from(t.spacing.sm)))
                .rounded_content()
                .bg(rgb(if chosen {
                    t.surface.selected_row
                } else {
                    t.surface.chrome
                }))
                .border_1()
                .border_color(rgb(border))
                .text_size(px(f32::from(t.typography.size.body)))
                .font_family(t.typography.family.ui)
                .text_color(rgb(if chosen {
                    t.text.bright
                } else {
                    t.text.primary
                }))
                .cursor_pointer()
                .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                    v.webhook_select_node(idx, cx);
                }))
                .child(SharedString::from(if chosen { "●  " } else { "○  " }))
                .child(SharedString::from(name.clone())),
        );
    }
    col
}

/// O campo de instrução — caixa multilinha (a instrução é a autoridade; o leigo escreve uma frase).
fn instruction_field(modal: &WebhookModal, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let t = theme::active();
    let focused = modal.focus_field() == WebhookField::Instruction;
    let (text_color, content): (u32, SharedString) = if modal.instruction().is_empty() {
        (
            t.text.muted,
            SharedString::from(COPY_INSTRUCTION_PLACEHOLDER),
        )
    } else {
        (
            t.text.bright,
            SharedString::from(modal.instruction().to_string()),
        )
    };
    div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.xs)))
        .child(field_label(&t, COPY_INSTRUCTION_LABEL))
        .child(
            div()
                .id("webhook-instruction")
                .w_full()
                .min_w(px(0.))
                .min_h(px(INSTRUCTION_MIN_H))
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
                .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                    v.webhook_focus(WebhookField::Instruction, cx);
                }))
                .child(content),
        )
}

/// O toggle de nível — dois segmentos (o escolhido pintado de confirmar) + microcopy do gate.
fn autonomy_toggle(modal: &WebhookModal, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let t = theme::active();
    let level = modal.autonomy();
    div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.xs)))
        .child(field_label(&t, COPY_AUTONOMY_LABEL))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(f32::from(t.spacing.sm)))
                .child(autonomy_segment(
                    &t,
                    "webhook-level-notify",
                    COPY_AUTONOMY_NOTIFY,
                    level == WebhookAutonomy::NotifyBefore,
                    cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.webhook_set_autonomy(WebhookAutonomy::NotifyBefore, cx);
                    }),
                ))
                .child(autonomy_segment(
                    &t,
                    "webhook-level-auto",
                    COPY_AUTONOMY_AUTO,
                    level == WebhookAutonomy::Autonomous,
                    cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.webhook_set_autonomy(WebhookAutonomy::Autonomous, cx);
                    }),
                )),
        )
        .child(notice(&t, COPY_AUTONOMY_HINT, t.text.muted))
}

/// Um segmento do toggle de nível — pintado de confirmar quando ativo, neutro quando não.
fn autonomy_segment(
    t: &theme::Theme,
    id: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (bg, fg) = if active {
        (t.accent.confirm, t.text.on_accent)
    } else {
        (t.surface.chrome, t.text.primary)
    };
    div()
        .id(id)
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px(px(f32::from(t.spacing.md)))
        .py(px(f32::from(t.spacing.sm)))
        .rounded_content()
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(if active {
            t.accent.confirm
        } else {
            t.surface.border
        }))
        .text_size(px(f32::from(t.typography.size.body)))
        .font_family(t.typography.family.ui)
        .text_color(rgb(fg))
        .cursor_pointer()
        .on_click(on_click)
        .child(label)
}

/// Uma caixa de leitura (endereço/senha) com botão "Copiar". O valor é selecionável visualmente; o
/// copiar real é uma ação da view (clipboard).
fn readonly_value(
    t: &theme::Theme,
    label: &'static str,
    value: &str,
    copy_id: &'static str,
    copy_label: &'static str,
    on_copy: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(f32::from(t.spacing.xs)))
        .child(field_label(t, label))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(f32::from(t.spacing.sm)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .px(px(f32::from(t.spacing.md)))
                        .py(px(f32::from(t.spacing.sm)))
                        .rounded_content()
                        .bg(rgb(t.surface.raised))
                        .border_1()
                        .border_color(rgb(t.surface.border))
                        .text_size(px(f32::from(t.typography.size.body)))
                        .font_family(t.typography.family.mono)
                        .text_color(rgb(t.text.bright))
                        .child(SharedString::from(value.to_string())),
                )
                .child(
                    div()
                        .id(copy_id)
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(f32::from(t.spacing.md)))
                        .py(px(f32::from(t.spacing.sm)))
                        .rounded_content()
                        .bg(rgb(t.surface.raised))
                        .border_1()
                        .border_color(rgb(t.surface.border))
                        .text_size(px(f32::from(t.typography.size.small)))
                        .font_family(t.typography.family.ui)
                        .text_color(rgb(t.text.primary))
                        .cursor_pointer()
                        .on_click(on_copy)
                        .child(copy_label),
                ),
        )
}

/// Rótulo de campo (pequeno, primário) — uma linha reusável.
fn field_label(t: &theme::Theme, label: &'static str) -> impl IntoElement {
    div()
        .text_size(px(f32::from(t.typography.size.small)))
        .font_family(t.typography.family.ui)
        .text_color(rgb(t.text.primary))
        .child(label)
}

/// Uma linha de aviso/erro na cor semântica dada.
fn notice(t: &theme::Theme, text: &str, color: u32) -> impl IntoElement {
    div()
        .text_size(px(f32::from(t.typography.size.small)))
        .font_family(t.typography.family.ui)
        .text_color(rgb(color))
        .child(SharedString::from(text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modal_with(names: &[&str]) -> WebhookModal {
        WebhookModal::new(names.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn new_modal_starts_on_form_empty_no_commit() {
        let m = modal_with(&["Vendas", "Suporte"]);
        assert_eq!(m.stage(), WebhookStage::Form);
        assert_eq!(m.focus_field(), WebhookField::Terminal);
        assert_eq!(m.instruction(), "");
        assert_eq!(m.autonomy(), WebhookAutonomy::NotifyBefore);
        assert_eq!(m.target_node(), None, "com 2 terminais, nada pré-escolhido");
        assert!(!m.can_commit(), "nada preenchido → não dá para criar");
    }

    #[test]
    fn single_terminal_is_preselected() {
        let m = modal_with(&["Vendas"]);
        assert_eq!(
            m.target_node(),
            Some("Vendas"),
            "único terminal já vem escolhido"
        );
    }

    #[test]
    fn typing_only_appends_to_the_instruction_field() {
        let mut m = modal_with(&["Vendas"]);
        // Foco inicial é Terminal: digitar não escreve nada.
        m.type_char("x");
        assert_eq!(
            m.instruction(),
            "",
            "digitar com foco no terminal não escreve"
        );
        m.set_focus(WebhookField::Instruction);
        m.type_char("o");
        m.type_char("i");
        assert_eq!(m.instruction(), "oi");
    }

    #[test]
    fn backspace_removes_from_instruction() {
        let mut m = modal_with(&["Vendas"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("a");
        m.type_char("b");
        m.backspace();
        assert_eq!(m.instruction(), "a");
    }

    #[test]
    fn tab_cycles_terminal_and_instruction() {
        let mut m = modal_with(&["Vendas"]);
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), WebhookField::Instruction);
        m.cycle_focus(1);
        assert_eq!(m.focus_field(), WebhookField::Terminal, "volta ao início");
        m.cycle_focus(-1);
        assert_eq!(
            m.focus_field(),
            WebhookField::Instruction,
            "Shift-Tab volta"
        );
    }

    #[test]
    fn cycle_node_moves_selection_with_wraparound() {
        let mut m = modal_with(&["Vendas", "Suporte", "Financeiro"]);
        assert_eq!(m.target_node(), None);
        m.cycle_node(1);
        assert_eq!(
            m.target_node(),
            Some("Vendas"),
            "1ª descida escolhe o primeiro"
        );
        m.cycle_node(-1);
        assert_eq!(
            m.target_node(),
            Some("Financeiro"),
            "subir do primeiro dá a volta"
        );
    }

    #[test]
    fn select_node_out_of_range_is_noop() {
        let mut m = modal_with(&["Vendas"]);
        m.select_node(9);
        assert_eq!(
            m.target_node(),
            Some("Vendas"),
            "índice inválido não muda a escolha"
        );
    }

    #[test]
    fn set_autonomy_flips_the_level() {
        let mut m = modal_with(&["Vendas"]);
        assert_eq!(
            m.autonomy(),
            WebhookAutonomy::NotifyBefore,
            "default conservador"
        );
        m.set_autonomy(WebhookAutonomy::Autonomous);
        assert_eq!(m.autonomy(), WebhookAutonomy::Autonomous);
        m.set_autonomy(WebhookAutonomy::NotifyBefore);
        assert_eq!(m.autonomy(), WebhookAutonomy::NotifyBefore);
    }

    #[test]
    fn autonomy_canonical_strings_match_the_frozen_event() {
        // Espelha `default_notify_before()` e a spec 55 §3 — string, não enum.
        assert_eq!(WebhookAutonomy::NotifyBefore.as_str(), "notify_before");
        assert_eq!(WebhookAutonomy::Autonomous.as_str(), "autonomous");
    }

    #[test]
    fn cannot_commit_without_a_node() {
        let mut m = modal_with(&["Vendas", "Suporte"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("faça algo");
        assert!(!m.can_commit(), "sem terminal escolhido não cria");
        assert_eq!(m.commit(), None);
        assert_eq!(m.error(), Some(COPY_ERR_NO_NODE));
        assert_eq!(
            m.stage(),
            WebhookStage::Form,
            "commit recusado não avança o estágio"
        );
    }

    #[test]
    fn cannot_commit_without_an_instruction() {
        // A REGRA-MÃE: a instrução é a autoridade; sem ela a UI recusa salvar (spec 55 §Segurança).
        let mut m = modal_with(&["Vendas"]); // único → terminal já escolhido
        assert!(m.target_node().is_some());
        assert!(
            !m.can_commit(),
            "sem instrução não cria, mesmo com terminal"
        );
        assert_eq!(m.commit(), None);
        assert_eq!(m.error(), Some(COPY_ERR_NO_INSTRUCTION));
    }

    #[test]
    fn whitespace_only_instruction_is_refused() {
        let mut m = modal_with(&["Vendas"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("   ");
        assert!(!m.can_commit(), "instrução só com espaços é vazia");
        assert_eq!(m.commit(), None);
        assert_eq!(m.error(), Some(COPY_ERR_NO_INSTRUCTION));
    }

    #[test]
    fn commit_builds_plan_and_enters_pending() {
        let mut m = modal_with(&["Vendas", "Suporte"]);
        m.select_node(1);
        m.set_focus(WebhookField::Instruction);
        m.type_char("  registre o pedido e me avise  ");
        m.set_autonomy(WebhookAutonomy::Autonomous);
        assert!(m.can_commit());
        let plan = m.commit().expect("commit completo devolve plano");
        assert_eq!(plan.target_node, "Suporte");
        assert_eq!(plan.instruction, "registre o pedido e me avise", "trimada");
        assert_eq!(plan.autonomy_level, "autonomous");
        assert_eq!(
            m.stage(),
            WebhookStage::Pending,
            "após enviar, aguarda o servidor"
        );
    }

    #[test]
    fn debug_of_outcome_redacts_the_secret_keeps_the_url() {
        // Defesa anti Debug-leak (F3): um `{:?}`/tracing futuro NUNCA imprime a senha em claro.
        let o = WebhookOutcome {
            url: "http://127.0.0.1:8787/hook/abc123".to_string(),
            secret: "super-senha-hmac".to_string(),
        };
        let dump = format!("{o:?}");
        assert!(
            !dump.contains("super-senha-hmac"),
            "o secret NÃO pode vazar no Debug: {dump}"
        );
        assert!(dump.contains("<redigido>"), "o secret aparece redigido");
        assert!(
            dump.contains("abc123"),
            "a url segue visível (não é segredo)"
        );
    }

    #[test]
    fn present_outcome_moves_to_outcome_stage() {
        let mut m = modal_with(&["Vendas"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("faça algo");
        let _ = m.commit().expect("commit ok");
        assert_eq!(m.stage(), WebhookStage::Pending);
        m.present_outcome(WebhookOutcome {
            url: "http://127.0.0.1:8787/hook/abc123".to_string(),
            secret: "s3cr3t-hmac".to_string(),
        });
        assert_eq!(m.stage(), WebhookStage::Outcome);
        let o = m.outcome().expect("outcome presente");
        assert_eq!(o.url, "http://127.0.0.1:8787/hook/abc123");
        assert_eq!(o.secret, "s3cr3t-hmac");
    }

    #[test]
    fn escape_with_content_arms_then_closes() {
        let mut m = modal_with(&["Vendas", "Suporte"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("x");
        assert!(!m.escape(), "1º Esc com conteúdo ARMA (não fecha)");
        assert!(m.discard_armed());
        assert!(m.escape(), "2º Esc fecha");
    }

    #[test]
    fn escape_on_empty_form_closes_immediately() {
        let mut m = modal_with(&["Vendas", "Suporte"]);
        assert!(m.escape(), "formulário vazio fecha no 1º Esc");
    }

    #[test]
    fn escape_on_outcome_closes_immediately() {
        let mut m = modal_with(&["Vendas"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("x");
        let _ = m.commit();
        m.present_outcome(WebhookOutcome {
            url: "u".to_string(),
            secret: "s".to_string(),
        });
        assert!(
            m.escape(),
            "no resultado, Esc fecha de imediato (sem arme de descarte)"
        );
    }

    #[test]
    fn typing_disarms_the_discard_confirmation() {
        let mut m = modal_with(&["Vendas"]);
        m.set_focus(WebhookField::Instruction);
        m.type_char("x");
        m.escape(); // arma
        assert!(m.discard_armed());
        m.type_char("y");
        assert!(!m.discard_armed(), "voltar a escrever desarma o descarte");
    }

    #[test]
    fn empty_roster_blocks_commit() {
        let mut m = modal_with(&[]);
        assert!(m.live_terminals().is_empty());
        m.set_focus(WebhookField::Instruction);
        m.type_char("faça algo");
        assert!(!m.can_commit(), "sem terminal no Espaço não há alvo");
        assert_eq!(m.commit(), None);
    }
}
