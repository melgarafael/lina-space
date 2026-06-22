//! `attention_ui` — **F1-1-7: fila de atenção UNIFICADA + toast (permissão + custódia)**.
//!
//! Apresentação da fila de atenção no canvas: toast (countdown ~6s, pause-on-hover,
//! desativado com fila >1), badge com contagem (pulsante quando há item Escalated ≥5min),
//! som opcional 1×/30s (mute persistido em Ajustes) e o painel compacto da fila com
//! «Não era um pedido» (dismiss/FP) e «Silenciar detecção deste terminal» (mute por nó).
//!
//! **Fronteira dura (ADR 0021 §6):** NENHUM byte é escrito no PTY a partir daqui —
//! aprovar/recusar só REGISTRA a decisão (`PermissionResolved`); o write é F1-1-8.
//! A custódia (gate ⌘⏎ existente, `CustodyDesk`) é APRESENTADA junto na fila, mas o
//! gesto continua pelo canal atual (`confirm_requested`/`reject_requested`) — zero regressão.
//!
//! **Quem ordena é o CORE**: a projeção `lina_core::AttentionQueue` (Arquiteto, F1-1-7
//! core) é a fonte única de ordenação (custódia > permissão, round-robin por nó) e de
//! escalação (≥5 min); a fiação app↔core mora no [`crate::bridge::AttentionHub`]. Este
//! módulo só decide COMO apresentar: qual item vira toast, countdown ativo?, estado do
//! badge, copy, som — **funções puras testáveis** (relógio SEMPRE injetado em ms; gpui
//! não roda headless). O render gpui é uma casca fina sobre elas. Cores 100% em tokens
//! de [`crate::theme`] (o lint anti-hardcode varre este arquivo).

use lina_core::attention::PromptKind;
use lina_core::{AttentionEvidence, AttentionItem, AttentionKind};

use crate::dashboard;

// ═══════════════════════════ toast: qual item + countdown (PURO) ═══════════════════════════

/// O QUE o toast mostra (13.13 achado 7 + ux-flows E1): nada, UM pedido completo, ou o
/// colapso «+N pedidos aguardando» (nunca empilhar toasts — anti-padrão 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastView {
    None,
    /// Índice (em `items`) do ÚNICO pendente — toast cheio com countdown.
    Single(usize),
    /// Nº total de pendentes — toast colapsado SEM countdown.
    Collapsed(usize),
}

/// Deriva o toast da fila corrente (já ordenada pelo core).
#[must_use]
pub fn toast_view(items: &[AttentionItem]) -> ToastView {
    match items.len() {
        0 => ToastView::None,
        1 => ToastView::Single(0),
        n => ToastView::Collapsed(n),
    }
}

/// Countdown do toast SÓ com exatamente 1 pendência (13.13 achado 7: "o timer não é
/// crítico e pode ser desativado se a fila tiver mais de um item").
#[must_use]
pub fn countdown_active(queue_len: usize) -> bool {
    queue_len == 1
}

// ═══════════════════════════ badge: contagem + pulso (PURO) ═══════════════════════════

/// Estado do badge 🔔 da topbar: conta PENDÊNCIAS reais (não "não-lidas" — quando zera é
/// porque o humano decidiu tudo) e pulsa SÓ com item Escalated (ADR 0021 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadgeView {
    pub count: usize,
    pub pulsing: bool,
}

/// Deriva o badge da fila corrente (o render aplica reduce-motion ao pulso).
#[must_use]
pub fn badge_view(items: &[AttentionItem]) -> BadgeView {
    BadgeView {
        count: items.len(),
        pulsing: items
            .iter()
            .any(|i| i.state == lina_core::AttentionState::Escalated),
    }
}

// ═══════════════════════════ copy (PURA — zero jargão, comando visível) ═══════════════════════════

/// R2b: a ação primária do item é IR ATÉ O TERMINAL? `true` exatamente para permissão
/// **não-Yn** (`Choice`/`Trust` — tela 20:30 do fundador): injetar `y` numa caixa de
/// escolha seria input errado e injeção remota de escolha NÃO existe nesta fase — o
/// gesto resolve no terminal (o core reforça: `resolve` devolve `None` p/ não-Yn).
/// Custódia nunca é goto-only (o gate dela é o ⌘⏎ da pump; o campo é nominal).
#[must_use]
pub fn is_goto_only(item: &AttentionItem) -> bool {
    // FIX-2: GuardAsk (ask do guard via hook PreToolUse) é SEMPRE goto-only — o y/n é respondido NO
    // terminal (injeção remota não existe nesta fase). Permissão não-Yn (Choice/Trust) idem.
    // W3 (#22): DeliveredNoProgress é OBSERVABILIDADE — nunca aprovável (alerta + foco; o Maestro age
    // re-despachando, não com y/n). Reusa o caminho não-aprovável: a ação primária é olhar o terminal.
    // F3-4-4: CodeConflict é "humano árbitro NO TERMINAL" (spec 36 §2) — o gesto é rebase/
    // renegociar/desconflitar no fluxo de código, NUNCA um Aprovar/Recusar remoto. Goto-only
    // (sem botões: o core nem o resolve — vive em `code_conflicts`, não em `permissions`).
    item.kind == AttentionKind::GuardAsk
        || item.kind == AttentionKind::DeliveredNoProgress
        || item.kind == AttentionKind::CodeConflict
        || (item.kind == AttentionKind::Permission && item.prompt_kind != PromptKind::Yn)
}

/// Copy do toast (story F1-1-7, com a **ARBITRAGEM do Maestro**: ux-flows vence —
/// «Esc nunca decide», então a copy promete só o que o gesto FAZ: ⌘⏎ aprova; Esc
/// deixa para depois — recusar é clique explícito na fila). Sem `detail` (payload
/// degradado), a frase fica leiga sem colchetes vazios. Evidência GRID (heurística,
/// FP medido) ganha o rótulo «conteúdo não verificado» (ADR 0021 R1 — confiabilidade
/// não-uniforme é EXIBIDA, nunca escondida). Custódia reusa o `display` do gate
/// existente INTACTO (zero regressão — o copy é do broker).
///
/// **R2b (direção do fundador — todo bloqueante alerta):** `Choice`/`Trust` NÃO são
/// aprováveis daqui — a copy convida a ir até o terminal e NUNCA promete aprovação.
#[must_use]
pub fn toast_copy(item: &AttentionItem) -> String {
    match item.kind {
        AttentionKind::Custody => item.detail.clone().unwrap_or_default(),
        // SEAM-1: banner de spawn (cascata) — copy leiga; ⌘⏎ deixa nascer, Esc recusa (gate humano).
        AttentionKind::Spawn => match &item.detail {
            Some(d) => format!("Um agente quer {d} — ⌘⏎ deixa nascer · Esc recusa",),
            None => "Um agente quer trazer um especialista pro time — ⌘⏎ deixa nascer · Esc recusa"
                .to_string(),
        },
        // FIX-2: ask do guard (hook PreToolUse) — o agente está BLOQUEADO num y/n que a fila não
        // pegava (formato ≠ dialog nativo; em bypass o nativo nem existe). Copy leiga + foco; o
        // gesto resolve NO terminal (sem injeção remota).
        AttentionKind::GuardAsk => match &item.detail {
            Some(cmd) => format!(
                "{} quer rodar: {cmd} — vá até o terminal aprovar",
                item.node_id
            ),
            None => format!(
                "{} está bloqueado esperando sua aprovação — vá até o terminal",
                item.node_id
            ),
        },
        // W3 (#22): o nó recebeu o handoff e voltou a Idle SEM começar (despacho engolido). Aviso
        // honesto — não interrompe com decisão; convida a olhar (o Maestro re-despacha, nunca y/n).
        AttentionKind::DeliveredNoProgress => format!(
            "{} recebeu a tarefa e ainda não começou — vale dar uma olhada",
            item.node_id
        ),
        // F3-4-4 (spec 36 §2): conflito de código — um colega tocou um arquivo que ESTE nó
        // reservou (claim × paths do commit). Aviso honesto que pede direção; o gesto é no
        // terminal (alinhar/desconflitar), nunca um y/n remoto. Copy leiga — o `detail` do core
        // carrega o técnico (paths/branch) para o painel rico (deferido). Precedência guard-ask.
        AttentionKind::CodeConflict => format!(
            "{} tem um conflito de código — um colega mexeu num arquivo que ele reservou; vá até o terminal alinhar",
            item.node_id
        ),
        AttentionKind::Permission => match item.prompt_kind {
            PromptKind::Choice => format!(
                "{} fez uma pergunta e aguarda sua escolha — vá até o terminal",
                item.node_id
            ),
            PromptKind::Trust => format!(
                "{} pede uma confirmação de segurança — vá até o terminal",
                item.node_id
            ),
            PromptKind::Yn => {
                let tag = match item.evidence {
                    AttentionEvidence::Grid => " · conteúdo não verificado",
                    _ => "",
                };
                match &item.detail {
                    Some(d) => format!(
                        "{} aguarda [{d}] — ⌘⏎ aprova · Esc deixa para depois{tag}",
                        item.node_id
                    ),
                    None => format!(
                        "{} aguarda sua aprovação — ⌘⏎ aprova · Esc deixa para depois{tag}",
                        item.node_id
                    ),
                }
            }
        },
    }
}

/// R2b 1b: copy do estado vazio da fila — só afirma «sem precisar de você» quando
/// NENHUM terminal está ocupado (um nó Blocked projeta como ocupado na UI; nunca
/// mentir sobre ele — direção do fundador). Com trabalho em curso a frase é neutra.
#[must_use]
pub fn empty_state_copy(busy_terminals: usize) -> String {
    match busy_terminals {
        0 => "Tudo em dia ✓ — seu time está trabalhando sem precisar de você.".to_string(),
        1 => "Nenhuma pendência agora — 1 terminal trabalhando.".to_string(),
        n => format!("Nenhuma pendência agora — {n} terminais trabalhando."),
    }
}

/// Entrada persistente da escalação (ADR 0021 §3): «{nó} espera você há N min».
#[must_use]
pub fn escalated_copy(item: &AttentionItem, now_ms: u64) -> String {
    let min = now_ms.saturating_sub(item.created_ts) / 60_000;
    format!("{} espera você há {min} min", item.node_id)
}

/// Idade legível do item no painel («há 5 s» / «há 2 min» / «há 1 h») — relógio
/// injetado; nunca negativa.
#[must_use]
pub fn age_label(created_ts: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(created_ts) / 1_000;
    if s < 60 {
        format!("há {s} s")
    } else if s < 3_600 {
        format!("há {} min", s / 60)
    } else {
        format!("há {} h", s / 3_600)
    }
}

// ═══════════════════════════ som 1×/30s (decisão PURA; o efeito mora no render) ═══════════════════════════

/// Intervalo mínimo entre toques (ADR 0021 §3 / 13.13 achado 9: som é convite, não sirene).
pub const SOUND_INTERVAL_MS: u64 = 30_000;

/// Decide se o lembrete sonoro toca AGORA: fila >0, sem mute, e ≥30 s desde o último
/// toque (`None` = nunca tocou nesta sessão). PURA — o side-effect fica no chamador.
#[must_use]
pub fn should_play_sound(
    queue_len: usize,
    muted: bool,
    elapsed_since_last_ms: Option<u64>,
) -> bool {
    queue_len > 0 && !muted && elapsed_since_last_ms.is_none_or(|e| e >= SOUND_INTERVAL_MS)
}

// ═══════════════════════════ countdown ~6s com pause-on-hover (PURO) ═══════════════════════════

/// Duração do toast (13.13 achado 7 — padrão HPE/Forma 36).
pub const TOAST_MS: u64 = 6_000;

/// Countdown do toast com **pause-on-hover**: o tempo PAUSADO não consome o countdown.
/// Relógio SEMPRE injetado em ms (testável headless; produção converte de `Instant`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastTimer {
    born_ms: u64,
    /// Tempo pausado ACUMULADO (hovers encerrados).
    paused_accum_ms: u64,
    /// Início do hover corrente (`Some` = congelado agora).
    hover_since: Option<u64>,
}

impl ToastTimer {
    #[must_use]
    pub fn new(now_ms: u64) -> Self {
        Self {
            born_ms: now_ms,
            paused_accum_ms: 0,
            hover_since: None,
        }
    }

    /// O ponteiro ENTROU no toast → congela o countdown (idempotente sob hovers repetidos).
    pub fn hover_start(&mut self, now_ms: u64) {
        if self.hover_since.is_none() {
            self.hover_since = Some(now_ms);
        }
    }

    /// O ponteiro SAIU do toast → o tempo congelado vira pausa acumulada.
    pub fn hover_end(&mut self, now_ms: u64) {
        if let Some(since) = self.hover_since.take() {
            self.paused_accum_ms += now_ms.saturating_sub(since);
        }
    }

    /// Tempo ATIVO decorrido (descontadas as pausas, inclusive o hover em curso).
    fn active_elapsed_ms(&self, now_ms: u64) -> u64 {
        let frozen_now = self
            .hover_since
            .map_or(0, |since| now_ms.saturating_sub(since));
        now_ms
            .saturating_sub(self.born_ms)
            .saturating_sub(self.paused_accum_ms)
            .saturating_sub(frozen_now)
    }

    /// Quanto resta do countdown (0 = expirou).
    #[must_use]
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        TOAST_MS.saturating_sub(self.active_elapsed_ms(now_ms))
    }

    /// O toast expirou? (o item NÃO some — segue no badge/fila; só o toast se recolhe.)
    #[must_use]
    pub fn expired(&self, now_ms: u64) -> bool {
        self.remaining_ms(now_ms) == 0
    }

    /// Fração restante `1.0 → 0.0` — dirige a barra visual do countdown.
    #[must_use]
    pub fn fraction_remaining(&self, now_ms: u64) -> f32 {
        self.remaining_ms(now_ms) as f32 / TOAST_MS as f32
    }
}

// ═══════════════════════════ máquina de estados do toast (memória de UI; gpui-free) ═══════════════════════════

/// A MEMÓRIA do toast entre frames: qual single está no ar (e seu countdown), quais
/// chaves já se recolheram (expirou/«Depois» — toast não renasce p/ a mesma chave;
/// **o item NÃO some**: segue na fila/badge — ux-flows E1/anti-padrão 3), e o snooze do
/// colapsado (re-aparece quando a CONTAGEM muda — pedido novo é sinal novo). Decisões
/// derivam das puras [`toast_view`]/[`ToastTimer`]; aqui só vive o estado. gpui-free.
#[derive(Debug, Default)]
pub struct ToastMachine {
    /// Single no ar: (stable_id, countdown).
    current: Option<(String, ToastTimer)>,
    /// Chaves cujo toast já se recolheu (expirado ou «Depois»).
    done: std::collections::BTreeSet<String>,
    /// «Depois» com fila >1: a contagem snoozada (≠ contagem atual ⇒ re-mostra).
    collapsed_snooze: Option<usize>,
    /// O ponteiro está sobre o toast? (replicado p/ o timer do PRÓXIMO single).
    hovering: bool,
}

impl ToastMachine {
    /// Avança a máquina para a fila corrente (chamar a cada heartbeat/frame de dados).
    pub fn tick(&mut self, items: &[AttentionItem], now_ms: u64) {
        match toast_view(items) {
            ToastView::None => {
                // Fila zerou: tudo decidido — memórias limpam (chave futura = toast novo).
                self.current = None;
                self.done.clear();
                self.collapsed_snooze = None;
            }
            ToastView::Single(idx) => {
                self.collapsed_snooze = None;
                let key = items[idx].stable_id.as_str();
                if self.done.contains(key) {
                    self.current = None;
                    return;
                }
                match &mut self.current {
                    Some((k, timer)) if k == key => {
                        if timer.expired(now_ms) {
                            self.done.insert(key.to_string());
                            self.current = None;
                        }
                    }
                    _ => {
                        let mut timer = ToastTimer::new(now_ms);
                        if self.hovering {
                            timer.hover_start(now_ms);
                        }
                        self.current = Some((key.to_string(), timer));
                    }
                }
            }
            ToastView::Collapsed(_) => {
                // Countdown desativado com fila >1 (13.13 achado 7).
                self.current = None;
            }
        }
    }

    /// O ponteiro entrou/saiu do toast (pause-on-hover).
    pub fn hover(&mut self, hovering: bool, now_ms: u64) {
        self.hovering = hovering;
        if let Some((_, timer)) = &mut self.current {
            if hovering {
                timer.hover_start(now_ms);
            } else {
                timer.hover_end(now_ms);
            }
        }
    }

    /// «Depois»/Esc-sem-decidir: recolhe o toast corrente SEM decidir (o item segue na
    /// fila + badge — nada se perde).
    pub fn snooze(&mut self, items: &[AttentionItem]) {
        match toast_view(items) {
            ToastView::Single(idx) => {
                self.done.insert(items[idx].stable_id.clone());
                self.current = None;
            }
            ToastView::Collapsed(n) => self.collapsed_snooze = Some(n),
            ToastView::None => {}
        }
    }

    /// O que está VISÍVEL agora (respeitando expiração/done/snooze) — o render desenha
    /// exatamente isto; `None` = sem toast (badge segue contando).
    #[must_use]
    pub fn visible(&self, items: &[AttentionItem], now_ms: u64) -> Option<ToastView> {
        match toast_view(items) {
            ToastView::None => None,
            ToastView::Single(idx) => match &self.current {
                Some((k, timer)) if k == &items[idx].stable_id && !timer.expired(now_ms) => {
                    Some(ToastView::Single(idx))
                }
                _ => None,
            },
            ToastView::Collapsed(n) => {
                (self.collapsed_snooze != Some(n)).then_some(ToastView::Collapsed(n))
            }
        }
    }

    /// O countdown do single corrente (p/ a barra) — `None` fora do single.
    #[must_use]
    pub fn timer(&self) -> Option<&ToastTimer> {
        self.current.as_ref().map(|(_, t)| t)
    }
}

// ═══════════════════════════ pulso do badge (determinístico por ms) ═══════════════════════════

/// Período do pulso do badge escalado (~1.2 s — suave, sem flash).
pub const PULSE_PERIOD_MS: u64 = 1_200;

/// Opacidade do badge pulsante: onda TRIANGULAR determinística sobre o relógio injetado
/// (pico 1.0 → vale 0.45 → pico). O chamador aplica reduce-motion (pulso suprimido =
/// opacidade fixa 1.0) — mesma disciplina do pulso A2A (W4-3/W4-6).
#[must_use]
pub fn pulse_alpha(now_ms: u64) -> f32 {
    let phase = (now_ms % PULSE_PERIOD_MS) as f32 / PULSE_PERIOD_MS as f32;
    let tri = (1.0 - 2.0 * phase).abs(); // 1 → 0 → 1 ao longo do período
    0.45 + 0.55 * tri
}

// ═══════════════════════════ efeito sonoro (fora da thread de UI; best-effort) ═══════════════════════════

/// Toca o lembrete 1× (curto, som de sistema — zero asset próprio), numa thread
/// descartável. Falha é SILENCIOSA na tela (badge/pulso já cobrem — ux-flows: "som não
/// pôde tocar → silencioso") e logada no stderr.
pub fn play_attention_sound() {
    #[cfg(target_os = "macos")]
    std::thread::spawn(|| {
        let ok = std::process::Command::new("afplay")
            .arg("/System/Library/Sounds/Tink.aiff")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("lina-gpui: [ATT] lembrete sonoro indisponível (afplay) — badge cobre");
        }
    });
    #[cfg(not(target_os = "macos"))]
    eprintln!("lina-gpui: [ATT] lembrete sonoro sem player nesta plataforma — badge cobre");
}

// ═══════════════════════════ geometria do toast vs painel "Atividade e custo" (PURA) ═══════════════════════════

/// Largura preferida do toast (canto inferior direito). O render encolhe ANTES de invadir a
/// coluna do dashboard — a não-sobreposição é inviolável; a largura é que cede.
pub const TOAST_W: f32 = 480.0;
/// Margem do toast às bordas da janela.
const TOAST_MARGIN: f32 = 16.0;
/// Folga do toast ao rodapé (acima do footer — não cobre o input em foco).
const TOAST_BOTTOM_INSET: f32 = 56.0;
/// Respiro entre o toast e a coluna do painel "Atividade e custo" quando ambos estão abertos.
const TOAST_DASH_GAP: f32 = 12.0;

/// **Geometria do toast que NUNCA colide com o painel "Atividade e custo" (T2).** Função PURA
/// (viewport → rect) que o render consome. Com o dashboard aberto, o toast vive à ESQUERDA da
/// coluna dele (separação horizontal com [`TOAST_DASH_GAP`]) — assim a não-interseção vale para
/// QUALQUER altura real do toast (medida pelo gpui), não só a modelada aqui. O `h` retornado é o
/// PIOR CASO (caixa cheia, do topo útil ao rodapé): se nem ela invade o dashboard, nada invade.
/// Janela estreita → o toast encolhe até ceder a vez, nunca sobrepõe (largura cede, colisão não).
#[must_use]
pub fn toast_rect(win_w: f32, win_h: f32, dashboard_open: bool) -> dashboard::PanelRect {
    let win_w = win_w.max(0.0);
    let win_h = win_h.max(0.0);
    // Limite direito da área do toast: à ESQUERDA da coluna do dashboard (com respiro) quando ele
    // está aberto; senão, a borda da janela menos a margem. Essa separação horizontal é o que
    // garante a não-interseção — independe da altura real do toast.
    let right_limit = if dashboard_open {
        (dashboard::dashboard_panel_rect(win_w, win_h).x - TOAST_DASH_GAP).max(0.0)
    } else {
        (win_w - TOAST_MARGIN).max(0.0)
    };
    // Largura preferida encolhida ao vão [margem, right_limit]; cede até sumir antes de invadir.
    let avail = (right_limit - TOAST_MARGIN).max(0.0);
    let w = TOAST_W.min(avail);
    let x = (right_limit - w).max(0.0);
    // Altura: PIOR CASO (caixa cheia, do topo útil ao rodapé) — o teste exige separação
    // horizontal, válida para a altura real (menor) que o gpui medir.
    let y = dashboard::PANEL_TOP_INSET.min(win_h);
    let bottom = (win_h - TOAST_BOTTOM_INSET).clamp(y, win_h);
    let h = bottom - y;
    dashboard::PanelRect { x, y, w, h }
}

// ═══════════════════════════ render gpui (casca FINA sobre as puras) ═══════════════════════════
//
// Toda cor vem de `theme::active()` via parâmetro (lint anti-hardcode varre este
// arquivo); `ElementId` ÚNICO por linha/botão (lição AccessKit da Fase 0);
// `line_height` EXPLÍCITA nos blocos de texto (lição do clip do grid W5).

use gpui::{div, prelude::*, px, rgb, text, AnyElement, ClickEvent, Context, Role, Window};

use crate::a11y_live::{live_region, Politeness};
use crate::theme::Theme;
use crate::ui::{Button, ButtonSize, ButtonVariant};
use crate::WorkspaceView;

/// Largura do painel compacto da fila.
const PANEL_W: f32 = 400.0;

/// Botão padrão da fila (id único + role + aria_label — teclado/leitor de tela).
fn btn<F>(
    cx: &mut Context<WorkspaceView>,
    id: impl Into<gpui::ElementId>,
    label: impl Into<String>,
    (bg, fg): (u32, u32),
    on_click: F,
) -> AnyElement
where
    F: Fn(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>) + 'static,
{
    let label: String = label.into();
    Button::new(id, label)
        .variant(ButtonVariant::Custom(bg, fg))
        .size(ButtonSize::Sm)
        .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| on_click(v, w, cx)))
        .into_any_element()
}

/// O sino 🔔 da topbar: contagem de pendências REAIS + pulso quando há item Escalated
/// (suprimido sob reduce-motion — opacidade fixa). Clique abre/fecha o painel da fila.
pub fn render_badge(
    items: &[lina_core::AttentionItem],
    panel_open: bool,
    reduce_motion: bool,
    now_ms: u64,
    th: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let badge = badge_view(items);
    let (bg, fg, label) = if badge.count > 0 {
        (
            th.state.warning,
            th.text.on_emphasis,
            format!("🔔 {}", badge.count),
        )
    } else {
        (th.surface.raised, th.text.primary, "🔔".to_string())
    };
    let alpha = if badge.pulsing && !reduce_motion {
        pulse_alpha(now_ms)
    } else {
        1.0
    };
    let aria = if badge.count > 0 {
        format!(
            "Fila de atenção: {} pedido(s) aguardando sua decisão",
            badge.count
        )
    } else {
        // R2b 1b: neutro — "tudo em dia" pleno é afirmação do painel (que conhece os
        // terminais ocupados); o sino sem contagem só diz que não há pendências.
        "Fila de atenção: sem pendências".to_string()
    };
    let mut el = div()
        .id("att-badge")
        .px_3()
        .py_1()
        .rounded_md()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .opacity(alpha)
        .cursor_pointer()
        .role(Role::Button)
        .aria_label(aria.clone())
        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| v.attention_toggle_panel(cx)))
        .child(text!(label));
    if panel_open {
        el = el.border_2().border_color(rgb(th.focus.ring));
    }
    // F2-2-2 / ADR 0028: a contagem/escalação da fila muda SEM foco — o sino (que segue um Button
    // clicável) nasce compondo o live-region. Cortesia: ESCALAÇÃO (pulsando) → Assertive (interrompe,
    // mesmo com o toast adiado); contagem → Polite (o toast já deu o detalhe assertive do item; o sino
    // só resume a fila, sem martelar). Id estável ⇒ anuncia na MUDANÇA da contagem, não a cada frame.
    let courtesy = if badge.pulsing {
        Politeness::Assertive
    } else {
        Politeness::Polite
    };
    live_region("att-badge-live", aria, courtesy, el).into_any_element()
}

/// O toast TO (canto inferior direito): single com countdown (pause-on-hover) ou
/// colapsado «+N pedidos» (countdown DESATIVADO — 13.13 achado 7). Nunca empilha.
#[allow(clippy::too_many_arguments)] // render de fiação: viewport + dashboard_open entram p/ a geometria T2.
pub fn render_toast(
    items: &[lina_core::AttentionItem],
    visible: ToastView,
    timer: Option<&ToastTimer>,
    now_ms: u64,
    viewport: (f32, f32),
    dashboard_open: bool,
    th: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    // T2: posição/largura HORIZONTAL que não invade a coluna "Atividade e custo" (à esquerda dela,
    // com folga, quando aberta). A ancoragem vertical segue no rodapé — a altura é a do conteúdo.
    let rect = toast_rect(viewport.0, viewport.1, dashboard_open);
    let mut card = div()
        .id("att-toast")
        .absolute()
        .left(px(rect.x))
        .bottom(px(TOAST_BOTTOM_INSET)) // acima do rodapé — não cobre nós nem o input em foco
        .w(px(rect.w))
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_md()
        .bg(rgb(th.surface.panel))
        .border_2()
        .text_size(px(12.0))
        .line_height(px(18.0))
        .on_hover(cx.listener(|v, hovered: &bool, _w, cx| {
            v.attention_hover(*hovered);
            cx.notify();
        }));
    // F2-2-2 / ADR 0028: o toast de PERMISSÃO/CUSTÓDIA é o estado mais crítico — não pode ser mudo
    // sem foco. O `Role::Status` cru NÃO auto-anuncia neste SHA (gap do a11y.rs); então o toast nasce
    // compondo o live-region: `announce` (capturado por modo) é anunciado ASSERTIVE (interrompe — há
    // uma decisão pendente). Substitui o `aria_label`-só de cada modo (canal único: o live carrega o
    // label+value+live).
    let mut announce = String::new();
    match visible {
        ToastView::Single(idx) => {
            let item = &items[idx];
            let copy = toast_copy(item);
            announce = copy.clone(); // o que o live-region anuncia (assertive) ao surgir o toast
            let is_custody = item.kind == AttentionKind::Custody;
            let selo = if is_custody {
                "custódia"
            } else if item.kind == AttentionKind::DeliveredNoProgress {
                "aviso"
            } else {
                "permissão"
            };
            // Custódia ganha selo visual distinto (borda de acento — ux-flows E1).
            card = card
                .border_color(rgb(if is_custody {
                    th.accent.primary
                } else {
                    th.state.warning
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .px_2()
                                .rounded_md()
                                .bg(rgb(th.surface.raised_alt))
                                .text_color(rgb(th.text.bright))
                                .text_size(px(10.0))
                                .child(text!(selo)),
                        )
                        .child(
                            div()
                                .text_color(rgb(th.text.muted))
                                .text_size(px(10.0))
                                .child(text!(age_label(item.created_ts, now_ms))),
                        ),
                )
                .child(div().text_color(rgb(th.text.primary)).child(text!(copy)));
            // R2b: pergunta (Choice/Trust) → a ação primária é IR ATÉ O TERMINAL
            // (o gesto resolve lá; aprovar/recusar não existem p/ esse formato).
            let primary: AnyElement = if is_goto_only(item) {
                let node = item.node_id.clone();
                btn(
                    cx,
                    "att-toast-goto",
                    "→ Ir até o terminal",
                    (th.accent.action, th.text.on_accent),
                    move |v, w, cx| v.attention_goto_node(&node, w, cx),
                )
            } else {
                btn(
                    cx,
                    "att-toast-open",
                    "Ver e decidir",
                    (th.accent.action, th.text.on_accent),
                    |v, _w, cx| v.attention_open_panel(cx),
                )
            };
            let mut actions = div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(primary)
                .child(btn(
                    cx,
                    "att-toast-later",
                    "Depois",
                    (th.surface.raised, th.text.primary),
                    |v, _w, cx| v.attention_snooze(cx),
                ));
            // FIX-2: GuardAsk não é detecção heurística — sem «não era um pedido» (o ask do guard é
            // determinístico; não há FP a rotular). Custódia também não (gate brokerado).
            if !is_custody && item.kind != AttentionKind::GuardAsk {
                let sid = item.stable_id.clone();
                actions = actions.child(btn(
                    cx,
                    "att-toast-fp",
                    "Não era um pedido",
                    (th.surface.raised, th.text.muted),
                    move |v, _w, cx| v.attention_dismiss(&sid, cx),
                ));
            }
            card = card.child(actions);
            // Barra do countdown (SÓ no single — fila >1 desativa o timer).
            if let Some(t) = timer.filter(|_| countdown_active(items.len())) {
                let frac = t.fraction_remaining(now_ms).clamp(0.0, 1.0);
                card = card.child(
                    div()
                        .h(px(3.0))
                        .w_full()
                        .rounded_md()
                        .bg(rgb(th.surface.raised))
                        .child(
                            div()
                                .h(px(3.0))
                                .w(px((rect.w - 24.0).max(0.0) * frac))
                                .rounded_md()
                                .bg(rgb(th.accent.primary)),
                        ),
                );
            }
        }
        ToastView::Collapsed(n) => {
            announce = format!("{n} pedidos aguardando a sua decisão");
            card = card
                .border_color(rgb(th.state.warning))
                .child(
                    div()
                        .text_color(rgb(th.text.primary))
                        .child(text!(format!("+{n} pedidos aguardando a sua decisão"))),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(btn(
                            cx,
                            "att-toast-open",
                            "Ver fila",
                            (th.accent.action, th.text.on_accent),
                            |v, _w, cx| v.attention_open_panel(cx),
                        ))
                        .child(btn(
                            cx,
                            "att-toast-later",
                            "Depois",
                            (th.surface.raised, th.text.primary),
                            |v, _w, cx| v.attention_snooze(cx),
                        )),
                );
        }
        ToastView::None => {}
    }
    // None (nada visível) → sem anúncio; senão o toast nasce compondo o live-region (assertive:
    // há uma decisão pendente — permissão/custódia/escalação interrompe a fala).
    if announce.is_empty() {
        card.into_any_element()
    } else {
        live_region("att-toast-live", announce, Politeness::Assertive, card).into_any_element()
    }
}

/// O painel compacto da fila (mailbox): itens NA ORDEM DO CORE (custódia > permissão,
/// round-robin), idade, entrada Escalated persistente, e as ações por classe —
/// permissão decide AQUI (registra evento; zero PTY); custódia mostra o gesto
/// EXISTENTE (⌘⏎/⌘⇧⏎ na frente do gate — nenhum caminho novo de decisão).
#[allow(clippy::too_many_arguments)] // render de fiação: os fios da view entram aqui (idioma do construtor do root).
pub fn render_panel(
    items: &[lina_core::AttentionItem],
    muted_nodes: &std::collections::BTreeSet<String>,
    desk_front: Option<&str>,
    busy_terminals: usize,
    viewport: (f32, f32),
    now_ms: u64,
    th: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let x = (viewport.0 - PANEL_W - 12.0).max(8.0);
    let h = (viewport.1 - 120.0).max(160.0);
    let mut rows = div().flex().flex_col().gap_2();
    if items.is_empty() {
        // Estado vazio é RECOMPENSA (ux-flows) — mas HONESTO (R2b 1b): só promete
        // «sem precisar de você» quando nenhum terminal está ocupado/bloqueado.
        rows = rows.child(
            div()
                .p_3()
                .text_color(rgb(th.text.secondary))
                .line_height(px(18.0))
                .child(text!(empty_state_copy(busy_terminals))),
        );
    }
    for (i, item) in items.iter().enumerate() {
        let i64id = i as u64;
        let is_custody = item.kind == AttentionKind::Custody;
        let escalated = item.state == lina_core::AttentionState::Escalated;
        let selo = if item.kind == AttentionKind::DeliveredNoProgress {
            // W3 (#22): aviso de monitoramento (recebeu, não começou) — nem custódia nem permissão.
            "aviso".to_string()
        } else if item.kind == AttentionKind::CodeConflict {
            // F3-4-4: conflito de código (um colega tocou arquivo reservado) — não é permissão.
            "conflito".to_string()
        } else {
            match (is_custody, item.evidence) {
                (true, _) => "custódia".to_string(),
                (false, AttentionEvidence::Grid) => "permissão · não verificado".to_string(),
                (false, _) => "permissão".to_string(),
            }
        };
        let mut row = div()
            .id(("att-row", i64id))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(rgb(th.surface.card))
            .border_1()
            .border_color(rgb(if escalated {
                th.state.warning
            } else if is_custody {
                th.accent.primary
            } else {
                th.surface.border
            }))
            .line_height(px(17.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .rounded_md()
                            .bg(rgb(th.surface.raised_alt))
                            .text_color(rgb(th.text.bright))
                            .text_size(px(10.0))
                            .child(text!(selo)),
                    )
                    .child(
                        div()
                            .text_color(rgb(th.text.primary))
                            .text_size(px(12.0))
                            .child(text!(item.node_id.clone())),
                    )
                    .child(
                        div()
                            .text_color(rgb(th.text.muted))
                            .text_size(px(10.0))
                            .child(text!(age_label(item.created_ts, now_ms))),
                    ),
            );
        // O comando/resumo exato em monospace (transparência: segurança pede o literal). EXCEÇÃO
        // F3-4-4: o `detail` do CodeConflict é técnico (paths/branch/"rebase" — vocabulário da spec
        // 36 p/ o painel rico, deferido); ao leigo vaza jargão (inv #6), e o toast já dá a mensagem
        // legível. Suprimido aqui até o painel rico traduzi-lo. (GuardAsk segue mostrando o cmd —
        // ali o literal É a transparência de segurança que o usuário precisa ver antes de aprovar.)
        if let Some(detail) = item
            .detail
            .as_ref()
            .filter(|_| item.kind != AttentionKind::CodeConflict)
        {
            row = row.child(
                div()
                    .id(("att-detail", i64id))
                    .font_family(th.typography.family.mono)
                    .text_size(px(11.0))
                    .text_color(rgb(th.text.secondary))
                    .overflow_hidden()
                    .child(text!(detail.clone())),
            );
        }
        if escalated {
            row = row.child(
                div()
                    .id(("att-esc", i64id))
                    .text_color(rgb(th.state.warning))
                    .text_size(px(11.0))
                    .child(text!(escalated_copy(item, now_ms))),
            );
        }
        if is_custody {
            // ZERO caminho novo: o gesto da custódia segue sendo o ⌘⏎/⌘⇧⏎ existente.
            let hint = if desk_front == Some(item.stable_id.as_str()) {
                "⌘⏎ aprova · ⌘⇧⏎ recusa (frente do gate)"
            } else {
                "na fila do gate — decide ao chegar na frente (⌘⏎)"
            };
            row = row.child(
                div()
                    .text_color(rgb(th.text.muted))
                    .text_size(px(11.0))
                    .child(text!(hint)),
            );
        } else if is_goto_only(item) {
            // R2b: pergunta (Choice/Trust) e FIX-2 GuardAsk — o gesto resolve NO TERMINAL; aqui só
            // navegação (nunca aprovar/recusar — o core devolve None p/ não-Yn, defesa em
            // profundidade). FP/mute só para PERMISSÃO detectada (heurística do grid, sujeita a
            // falso-positivo); GuardAsk é gate determinístico do guard — não há FP a rotular, e
            // «silenciar detecção» mutaria o grid de permissão do nó (efeito colateral indevido).
            let node_go = item.node_id.clone();
            let mut acts = div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(btn(
                    cx,
                    ("att-goto", i64id),
                    "→ Ir até o terminal",
                    (th.accent.action, th.text.on_accent),
                    move |v, w, cx| v.attention_goto_node(&node_go, w, cx),
                ));
            if item.kind == AttentionKind::Permission {
                let node = item.node_id.clone();
                let node_muted = muted_nodes.contains(&node);
                let sid_fp = item.stable_id.clone();
                acts = acts
                    .child(btn(
                        cx,
                        ("att-fp", i64id),
                        "Não era um pedido",
                        (th.surface.raised, th.text.primary),
                        move |v, _w, cx| v.attention_dismiss(&sid_fp, cx),
                    ))
                    .child(btn(
                        cx,
                        ("att-mute", i64id),
                        if node_muted {
                            "🔔 Reativar detecção deste terminal"
                        } else {
                            "🔕 Silenciar detecção deste terminal"
                        },
                        (th.surface.raised, th.text.primary),
                        move |v, _w, cx| v.attention_toggle_mute(&node, cx),
                    ));
            }
            row = row.child(acts);
        } else {
            let node = item.node_id.clone();
            let node_muted = muted_nodes.contains(&node);
            let sid_ok = item.stable_id.clone();
            let sid_no = item.stable_id.clone();
            let sid_fp = item.stable_id.clone();
            row = row.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(btn(
                        cx,
                        ("att-approve", i64id),
                        "✓ Aprovar",
                        (th.accent.confirm, th.text.on_accent),
                        move |v, _w, cx| v.attention_approve(&sid_ok, cx),
                    ))
                    .child(btn(
                        cx,
                        ("att-deny", i64id),
                        "✗ Recusar",
                        (th.state.danger, th.text.on_emphasis),
                        move |v, _w, cx| v.attention_deny(&sid_no, cx),
                    ))
                    .child(btn(
                        cx,
                        ("att-fp", i64id),
                        "Não era um pedido",
                        (th.surface.raised, th.text.primary),
                        move |v, _w, cx| v.attention_dismiss(&sid_fp, cx),
                    ))
                    .child(btn(
                        cx,
                        ("att-mute", i64id),
                        if node_muted {
                            "🔔 Reativar detecção deste terminal"
                        } else {
                            "🔕 Silenciar detecção deste terminal"
                        },
                        (th.surface.raised, th.text.primary),
                        move |v, _w, cx| v.attention_toggle_mute(&node, cx),
                    )),
            );
        }
        rows = rows.child(row);
    }
    div()
        .id("att-panel")
        .absolute()
        .left(px(x))
        .top(px(52.0)) // sob a topbar
        .w(px(PANEL_W))
        .h(px(h))
        .overflow_hidden()
        .rounded_md()
        .bg(rgb(th.surface.panel))
        .border_1()
        .border_color(rgb(th.surface.border))
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(rgb(th.text.bright))
                        .child(text!(format!("Fila de Atenção ({})", items.len()))),
                )
                .child(btn(
                    cx,
                    "att-panel-close",
                    "✕",
                    (th.surface.raised, th.text.primary),
                    |v, _w, cx| v.attention_toggle_panel(cx),
                )),
        )
        .child(
            // Scroll INTERNO das linhas (lição do onboarding/dashboard): nunca vaza.
            div()
                .id("att-panel-rows")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(rows),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_core::{AttentionState, ESCALATE_AFTER_MS};

    /// Item de teste mínimo sobre o CONTRATO REAL do core (`lina_core::AttentionItem`).
    fn item(id: &str, node: &str, kind: AttentionKind, ts: u64) -> AttentionItem {
        AttentionItem {
            stable_id: id.into(),
            node_id: node.into(),
            kind,
            detail: Some(format!("detalhe {id}")),
            evidence: match kind {
                AttentionKind::Custody | AttentionKind::Spawn => AttentionEvidence::Custody,
                AttentionKind::Permission | AttentionKind::GuardAsk => AttentionEvidence::Hook,
                // Espelha a projeção real do core (estrutural, derivado do log — não heurístico).
                AttentionKind::DeliveredNoProgress | AttentionKind::CodeConflict => {
                    AttentionEvidence::Hook
                }
            },
            created_ts: ts,
            state: AttentionState::Pending,
            prompt_kind: lina_core::attention::PromptKind::Yn,
            vt_snapshot_hash: None,
        }
    }

    /// Item de PERGUNTA (R2b): permissão com `prompt_kind` não-Yn.
    fn question(id: &str, node: &str, pk: lina_core::attention::PromptKind) -> AttentionItem {
        let mut it = item(id, node, AttentionKind::Permission, 100);
        it.prompt_kind = pk;
        it
    }

    // ─────────────── T2: toast × painel "Atividade e custo" NÃO se sobrepõem ───────────────

    /// O cerne do T2: com o painel "Atividade e custo" ABERTO, o toast e o dashboard NÃO podem
    /// ter área em comum em NENHUM tamanho de janela (inclusive estreito). A geometria modela o
    /// toast no PIOR CASO (altura cheia) — se essa caixa não invade a coluna do dashboard, a real
    /// (mais baixa) também não. Falha com o stub que ancora à direita ignorando o dashboard.
    #[test]
    fn toast_never_overlaps_dashboard_when_both_open() {
        for (w, h) in [
            (1440.0, 900.0),
            (1280.0, 800.0),
            (1024.0, 768.0),
            (900.0, 600.0),
            (760.0, 520.0),
            (520.0, 420.0),
        ] {
            let dash = crate::dashboard::dashboard_panel_rect(w, h);
            let toast = toast_rect(w, h, true);
            assert!(
                !toast.intersects(&dash),
                "toast {toast:?} sobrepõe o dashboard {dash:?} em {w}x{h}"
            );
        }
    }

    /// Sem o dashboard, o toast volta ao canto inferior direito com a margem padrão (largura
    /// cheia quando cabe) — a correção do T2 não rouba espaço quando não há com o que colidir.
    #[test]
    fn toast_anchors_right_when_dashboard_closed() {
        let r = toast_rect(1440.0, 900.0, false);
        assert_eq!(r.w, TOAST_W, "largura cheia quando há espaço");
        assert!(
            (r.x + r.w - (1440.0 - 16.0)).abs() < f32::EPSILON,
            "borda direita do toast = janela − margem: {r:?}"
        );
    }

    /// Viewport degenerada (até 0×0) nunca gera rect negativo nem sai da janela — a geometria é
    /// total (clampada), como a do dashboard. Sem panic, sem largura/altura abaixo de zero.
    #[test]
    fn toast_rect_is_bounded_for_any_viewport() {
        for (w, h) in [(0.0, 0.0), (10.0, 10.0), (320.0, 200.0), (2560.0, 1440.0)] {
            for open in [false, true] {
                let r = toast_rect(w, h, open);
                assert!(r.w >= 0.0 && r.h >= 0.0, "sem dimensão negativa: {r:?}");
                assert!(r.x >= 0.0 && r.y >= 0.0, "origem não-negativa: {r:?}");
                assert!(
                    r.x + r.w <= w + f32::EPSILON,
                    "não vaza à direita: {r:?} em {w}x{h}"
                );
                assert!(
                    r.y + r.h <= h + f32::EPSILON,
                    "não vaza embaixo: {r:?} em {w}x{h}"
                );
            }
        }
    }

    // ─────────────────── R2b: Question/Choice — copy + ação primária ───────────────────

    /// R2b (tela 20:30 do fundador): `choice`/`trust` NÃO são aprováveis remotamente —
    /// a copy convida a IR ATÉ O TERMINAL e NUNCA promete aprovar/recusar (injetar `y`
    /// numa caixa de escolha seria input errado; injeção de escolha não existe na fase).
    #[test]
    fn toast_copy_choice_and_trust_invite_to_terminal_never_approve() {
        use lina_core::attention::PromptKind;
        let c = question("q1", "Terminal A", PromptKind::Choice);
        assert_eq!(
            toast_copy(&c),
            "Terminal A fez uma pergunta e aguarda sua escolha — vá até o terminal"
        );
        let t = question("q2", "Terminal B", PromptKind::Trust);
        let copy = toast_copy(&t);
        assert!(
            copy.contains("confirmação de segurança") && copy.contains("vá até o terminal"),
            "{copy}"
        );
        for it in [c, t] {
            let copy = toast_copy(&it);
            assert!(
                !copy.contains("aprova") && !copy.contains("⌘⏎"),
                "pergunta nunca promete aprovação remota: {copy}"
            );
        }
    }

    /// A ação primária é GOTO (foco no terminal) exatamente p/ permissão não-Yn;
    /// y/n e custódia mantêm seus gestos atuais.
    #[test]
    fn goto_only_for_non_yn_permission() {
        use lina_core::attention::PromptKind;
        assert!(!is_goto_only(&item("p", "A", AttentionKind::Permission, 1)));
        assert!(is_goto_only(&question("q", "A", PromptKind::Choice)));
        assert!(is_goto_only(&question("q", "A", PromptKind::Trust)));
        // Custódia nunca é goto-only (gate ⌘⏎ da pump), mesmo com o campo nominal.
        let mut c = item("c", "B", AttentionKind::Custody, 1);
        c.prompt_kind = PromptKind::Choice;
        assert!(!is_goto_only(&c));
    }

    /// F3-4-4: CodeConflict é goto-only (humano árbitro NO TERMINAL, spec 36 §2) — nunca um
    /// Aprovar/Recusar remoto (seria botão morto: o core não o resolve, vive em `code_conflicts`).
    /// E o toast é leigo: sem jargão de git (rebase/branch/merge/HEAD) e sem prometer aprovação.
    #[test]
    fn code_conflict_is_goto_only_and_leiga() {
        let it = item("cc", "Terminal B", AttentionKind::CodeConflict, 1);
        assert!(
            is_goto_only(&it),
            "conflito de código resolve NO TERMINAL, nunca por botão remoto"
        );
        let copy = toast_copy(&it);
        for jargao in ["rebase", "branch", "merge", "head", "git"] {
            assert!(
                !copy.to_lowercase().contains(jargao),
                "toast do conflito vaza jargão '{jargao}': {copy}"
            );
        }
        assert!(
            !copy.contains("Aprovar") && !copy.contains("⌘⏎"),
            "conflito nunca promete aprovação remota: {copy}"
        );
    }

    /// FIX-2: o ASK do guard é goto-only (alerta+foco; o y/n é respondido NO terminal) e a copy
    /// leiga nomeia o nó + convida a ir até o terminal, NUNCA prometendo o gesto de aprovação local.
    #[test]
    fn guard_ask_is_goto_only_with_leiga_copy() {
        let g = item(
            "guard:Bug Finder",
            "Bug Finder",
            AttentionKind::GuardAsk,
            100,
        );
        assert!(is_goto_only(&g), "GuardAsk resolve no terminal (goto-only)");
        let copy = toast_copy(&g);
        assert!(
            copy.contains("Bug Finder") && copy.contains("vá até o terminal"),
            "copy nomeia o nó e convida ao terminal: {copy}"
        );
        assert!(
            !copy.contains("⌘⏎"),
            "nunca promete o gesto de aprovação local (resolve no terminal): {copy}"
        );
    }

    // ─────────────────── R2b 1b: empty-state honesto ───────────────────

    /// O estado vazio só afirma «sem precisar de você» quando NENHUM terminal está
    /// ocupado/bloqueado; com trabalho em curso a copy é neutra e conta os terminais
    /// (nunca mentir sobre um nó possivelmente Blocked — direção do fundador).
    #[test]
    fn empty_state_copy_is_honest_about_busy_terminals() {
        assert_eq!(
            empty_state_copy(0),
            "Tudo em dia ✓ — seu time está trabalhando sem precisar de você."
        );
        assert_eq!(
            empty_state_copy(1),
            "Nenhuma pendência agora — 1 terminal trabalhando."
        );
        assert_eq!(
            empty_state_copy(3),
            "Nenhuma pendência agora — 3 terminais trabalhando."
        );
        for n in 1..5 {
            assert!(
                !empty_state_copy(n).contains("sem precisar de você"),
                "com trabalho em curso, nunca prometer 'sem precisar de você'"
            );
        }
    }

    // ─────────────────────── toast: qual item + countdown ativo? ───────────────────────

    /// Fila vazia → sem toast; 1 item → toast cheio DESSE item; >1 → colapsa em contador
    /// («+N pedidos aguardando» — nunca empilhar toasts, ux-flows anti-padrão 6).
    #[test]
    fn toast_view_single_full_multiple_collapsed() {
        assert_eq!(toast_view(&[]), ToastView::None);
        let one = vec![item("p1", "A", AttentionKind::Permission, 100)];
        assert_eq!(toast_view(&one), ToastView::Single(0));
        let two = vec![
            item("p1", "A", AttentionKind::Permission, 100),
            item("c1", "B", AttentionKind::Custody, 200),
        ];
        assert_eq!(toast_view(&two), ToastView::Collapsed(2));
    }

    /// Countdown SÓ com exatamente 1 pendência (13.13 achado 7: fila >1 desativa o timer;
    /// fila 0 não tem o que contar).
    #[test]
    fn countdown_active_only_with_single_item() {
        assert!(!countdown_active(0));
        assert!(countdown_active(1));
        assert!(!countdown_active(2));
        assert!(!countdown_active(9));
    }

    // ─────────────────────── badge: contagem + pulso ───────────────────────

    /// Badge conta PENDÊNCIAS reais e pulsa SÓ com item Escalated (≥5 min).
    #[test]
    fn badge_view_counts_and_pulses_on_escalation() {
        assert_eq!(
            badge_view(&[]),
            BadgeView {
                count: 0,
                pulsing: false
            }
        );
        let calm = vec![item("p1", "A", AttentionKind::Permission, 100)];
        assert_eq!(
            badge_view(&calm),
            BadgeView {
                count: 1,
                pulsing: false
            }
        );
        let mut hot = vec![
            item("p1", "A", AttentionKind::Permission, 100),
            item("c1", "B", AttentionKind::Custody, 200),
        ];
        hot[1].state = AttentionState::Escalated;
        assert_eq!(
            badge_view(&hot),
            BadgeView {
                count: 2,
                pulsing: true
            }
        );
    }

    // ─────────────────────── copy do toast + entrada escalada ───────────────────────

    /// Copy do toast (story F1-1-7 + ARBITRAGEM do Maestro: ux-flows vence — «Esc
    /// nunca decide», então a copy NÃO promete recusa por Esc; Esc deixa p/ depois).
    /// Permissão via GRID ganha o rótulo de evidência não verificada (ADR 0021 R1);
    /// sem detail a frase degrada leiga; custódia reusa o display do gate intacto.
    #[test]
    fn toast_copy_follows_story_and_labels_grid_evidence() {
        let mut p = item("p1", "Terminal B", AttentionKind::Permission, 100);
        p.detail = Some("git push origin/master".into());
        assert_eq!(
            toast_copy(&p),
            "Terminal B aguarda [git push origin/master] — ⌘⏎ aprova · Esc deixa para depois"
        );
        assert!(
            !toast_copy(&p).contains("recusa"),
            "a copy NUNCA promete recusa por Esc (arbitragem: Esc não decide)"
        );
        p.evidence = AttentionEvidence::Grid;
        let copy = toast_copy(&p);
        assert!(copy.contains("conteúdo não verificado"), "{copy}");

        p.detail = None;
        let copy = toast_copy(&p);
        assert!(
            copy.starts_with("Terminal B aguarda sua aprovação"),
            "sem detail → frase leiga sem colchetes vazios: {copy}"
        );

        let mut c = item("c1", "Terminal A", AttentionKind::Custody, 100);
        c.detail = Some("🔒 CUSTODIA: 'deploy' pedido por A — ⌘⏎ aprova".into());
        assert_eq!(
            toast_copy(&c),
            "🔒 CUSTODIA: 'deploy' pedido por A — ⌘⏎ aprova",
            "custódia: display do gate intacto"
        );
    }

    /// Idade legível do item no painel: segundos < 1 min, minutos < 1 h, depois horas.
    #[test]
    fn age_label_humanizes_seconds_minutes_hours() {
        assert_eq!(age_label(0, 5_000), "há 5 s");
        assert_eq!(age_label(0, 59_999), "há 59 s");
        assert_eq!(age_label(0, 60_000), "há 1 min");
        assert_eq!(age_label(0, 12 * 60_000 + 30_000), "há 12 min");
        assert_eq!(age_label(0, 2 * 3_600_000 + 60_000), "há 2 h");
        // relógio para trás: nunca negativo.
        assert_eq!(age_label(10_000, 5_000), "há 0 s");
    }

    /// Entrada persistente da escalação (ADR 0021 §3): «Terminal X espera você há N min».
    /// Usa o `ESCALATE_AFTER_MS` do CORE — fonte única do limiar (sem cópia local).
    #[test]
    fn escalated_copy_names_node_and_minutes() {
        let p = item("p1", "Terminal X", AttentionKind::Permission, 0);
        assert_eq!(
            escalated_copy(&p, ESCALATE_AFTER_MS),
            "Terminal X espera você há 5 min"
        );
        assert_eq!(
            escalated_copy(&p, 12 * 60 * 1000 + 30_000),
            "Terminal X espera você há 12 min"
        );
    }

    // ─────────────────────── som 1×/30s (decisão PURA; efeito fica fora) ───────────────────────

    /// Som SÓ com fila >0 (ADR 0021 §3), nunca sob mute, no MÁXIMO 1× a cada 30 s.
    /// `elapsed = None` = nunca tocou nesta sessão.
    #[test]
    fn sound_plays_at_most_once_per_30s_when_queue_nonempty() {
        assert!(!should_play_sound(0, false, None), "fila vazia: silêncio");
        assert!(!should_play_sound(3, true, None), "mute vence tudo");
        assert!(should_play_sound(1, false, None), "1º toque da sessão");
        assert!(!should_play_sound(1, false, Some(SOUND_INTERVAL_MS - 1)));
        assert!(should_play_sound(1, false, Some(SOUND_INTERVAL_MS)));
    }

    // ─────────────────────── countdown ~6s com pause-on-hover ───────────────────────

    /// O timer expira após 6 s de tempo ATIVO; o hover CONGELA o countdown (pause-on-hover,
    /// 13.13 achado 7) e o tempo pausado NÃO conta. Relógio injetado (ms) — headless.
    #[test]
    fn toast_timer_six_seconds_with_pause_on_hover() {
        let t0 = 10_000_u64;
        let t = ToastTimer::new(t0);
        assert_eq!(t.remaining_ms(t0), TOAST_MS);
        assert!(!t.expired(t0 + TOAST_MS - 1));
        assert!(t.expired(t0 + TOAST_MS));

        // Hover aos +1s congela; 2s de hover não consomem countdown.
        let mut t = ToastTimer::new(t0);
        t.hover_start(t0 + 1000);
        assert_eq!(t.remaining_ms(t0 + 1000), TOAST_MS - 1000);
        assert_eq!(
            t.remaining_ms(t0 + 3000),
            TOAST_MS - 1000,
            "congelado durante o hover"
        );
        t.hover_end(t0 + 3000);
        // Restam 5s ativos → expira só em t0+3000+5000.
        assert!(!t.expired(t0 + 7999));
        assert!(t.expired(t0 + 8000));
    }

    /// Fração restante p/ a barra do countdown: 1.0 ao nascer → 0.0 na expiração (clampada).
    #[test]
    fn toast_timer_fraction_drives_countdown_bar() {
        let t0 = 0_u64;
        let t = ToastTimer::new(t0);
        assert!((t.fraction_remaining(t0) - 1.0).abs() < f32::EPSILON);
        assert!((t.fraction_remaining(t0 + TOAST_MS / 2) - 0.5).abs() < 0.01);
        assert_eq!(t.fraction_remaining(t0 + TOAST_MS + 999), 0.0, "clampada");
    }

    // ─────────────────────── pulso do badge (determinístico por ms) ───────────────────────

    /// Onda triangular determinística: pico 1.0 no início do período, vale no meio, e
    /// SEMPRE dentro de [0.45, 1.0] — sem flash (a11y) e sem depender de relógio interno.
    #[test]
    fn pulse_alpha_is_bounded_triangle_wave() {
        assert!((pulse_alpha(0) - 1.0).abs() < 0.01);
        assert!((pulse_alpha(PULSE_PERIOD_MS / 2) - 0.45).abs() < 0.01);
        assert!((pulse_alpha(PULSE_PERIOD_MS) - 1.0).abs() < 0.01);
        for ms in (0..PULSE_PERIOD_MS * 3).step_by(37) {
            let a = pulse_alpha(ms);
            assert!((0.45..=1.0).contains(&a), "alpha {a} fora dos limites");
        }
    }

    // ─────────────────────── máquina de estados do toast (memória de UI) ───────────────────────

    /// Item único: o toast aparece, EXPIRA aos 6 s e NÃO renasce para a mesma chave —
    /// mas o item segue na fila (badge): "o toast some, o item NÃO" (ux-flows E1).
    #[test]
    fn toast_machine_single_expires_and_never_resurrects_same_key() {
        let mut m = ToastMachine::default();
        let items = vec![item("p1", "A", AttentionKind::Permission, 100)];
        m.tick(&items, 1_000);
        assert_eq!(m.visible(&items, 1_000), Some(ToastView::Single(0)));
        // ainda vivo aos 5.9s…
        m.tick(&items, 1_000 + TOAST_MS - 100);
        assert_eq!(
            m.visible(&items, 1_000 + TOAST_MS - 100),
            Some(ToastView::Single(0))
        );
        // …expira aos 6s e NÃO volta nos ticks seguintes.
        m.tick(&items, 1_000 + TOAST_MS);
        assert_eq!(m.visible(&items, 1_000 + TOAST_MS), None);
        m.tick(&items, 1_000 + TOAST_MS + 500);
        assert_eq!(m.visible(&items, 1_000 + TOAST_MS + 500), None);
    }

    /// «Depois» (snooze) recolhe o toast SEM decidir; com fila >1 o colapsado some e
    /// SÓ re-aparece quando a contagem MUDA (pedido novo = sinal novo).
    #[test]
    fn toast_machine_snooze_hides_until_count_changes() {
        let mut m = ToastMachine::default();
        let two = vec![
            item("p1", "A", AttentionKind::Permission, 100),
            item("p2", "B", AttentionKind::Permission, 200),
        ];
        m.tick(&two, 1_000);
        assert_eq!(m.visible(&two, 1_000), Some(ToastView::Collapsed(2)));
        m.snooze(&two);
        m.tick(&two, 1_500);
        assert_eq!(m.visible(&two, 1_500), None, "snoozado");
        // chega um 3º pedido → o colapsado re-aparece com a contagem nova.
        let three = vec![
            two[0].clone(),
            two[1].clone(),
            item("p3", "C", AttentionKind::Permission, 300),
        ];
        m.tick(&three, 2_000);
        assert_eq!(m.visible(&three, 2_000), Some(ToastView::Collapsed(3)));
    }

    /// Fila zerada LIMPA as memórias (done/snooze): um pedido futuro (chave nova)
    /// ganha toast novo; e um único item snoozado some sem decidir.
    #[test]
    fn toast_machine_resets_memory_when_queue_drains() {
        let mut m = ToastMachine::default();
        let items = vec![item("p1", "A", AttentionKind::Permission, 100)];
        m.tick(&items, 1_000);
        m.snooze(&items);
        m.tick(&items, 1_100);
        assert_eq!(m.visible(&items, 1_100), None, "single snoozado (Depois)");
        // decidiu/resolveu → fila vazia → memórias limpam.
        m.tick(&[], 2_000);
        let fresh = vec![item("p9", "A", AttentionKind::Permission, 2_500)];
        m.tick(&fresh, 2_500);
        assert_eq!(
            m.visible(&fresh, 2_500),
            Some(ToastView::Single(0)),
            "chave nova ganha toast novo"
        );
    }

    /// Pause-on-hover atravessa a máquina: hovering congela o countdown do single.
    #[test]
    fn toast_machine_hover_freezes_countdown() {
        let mut m = ToastMachine::default();
        let items = vec![item("p1", "A", AttentionKind::Permission, 100)];
        m.tick(&items, 0);
        m.hover(true, 1_000);
        m.tick(&items, 10_000); // 9s de hover — sem hover teria expirado
        assert_eq!(
            m.visible(&items, 10_000),
            Some(ToastView::Single(0)),
            "congelado sob o ponteiro"
        );
        m.hover(false, 10_000);
        // consumiu 1s ativo antes do hover → expira 5s após soltar.
        m.tick(&items, 14_999);
        assert_eq!(m.visible(&items, 14_999), Some(ToastView::Single(0)));
        m.tick(&items, 15_000);
        assert_eq!(m.visible(&items, 15_000), None);
    }

    /// Single de outro item (chave nova após decidir o anterior) reinicia o countdown.
    #[test]
    fn toast_machine_new_single_key_restarts_timer() {
        let mut m = ToastMachine::default();
        let a = vec![item("p1", "A", AttentionKind::Permission, 100)];
        m.tick(&a, 0);
        m.tick(&a, TOAST_MS); // expira p1
        assert_eq!(m.visible(&a, TOAST_MS), None);
        // p1 resolvido; chega p2 (fila nunca zerou? zerou — entre os ticks).
        m.tick(&[], TOAST_MS + 100);
        let b = vec![item("p2", "B", AttentionKind::Permission, 200)];
        m.tick(&b, TOAST_MS + 200);
        assert_eq!(m.visible(&b, TOAST_MS + 200), Some(ToastView::Single(0)));
    }
}
