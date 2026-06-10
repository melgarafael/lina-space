//! F1-1-6 · Detecção de permissão bloqueante (multi-camada).
//!
//! ## Arquitetura (story F1-1-6; fonte 13.13)
//! - **Camada 1 — hook `Notification` + correlação de contexto.** O payload da
//!   `Notification` é genérico (sem `tool_name` — issue #32952); a recomendação 13.13
//!   é correlacionar com o `PreToolUse` precedente da sessão. Aqui a correlação é
//!   **estrutural, por ordem de eventos**: no Claude Code o `PreToolUse` dispara ANTES
//!   da checagem de permissão e o `PostToolUse` só dispara DEPOIS da aprovação — logo
//!   `Notification` chegando com tool PENDENTE (`PreToolUse` sem `PostToolUse`, janela
//!   curta) **é** o pedido de permissão, e a tool do cache é o contexto ("Bash", não só
//!   "precisa de aprovação"). `Notification` SEM pendência (ex.: aviso de idle-60s) é
//!   descartada — corta o FP estrutural da camada primária sem matching de string do
//!   `message` (i18n/forjável) e sem campo novo no pipeline F1-1-3.
//! - **Camada 2 — fallback por regex no grid** (CLIs sem hook), lido pela trait
//!   [`VtBackend`] (âncora §3 — nunca acopla o alacritty): padrões `(y/n)`/`[Y/n]`/
//!   `yes/no`/`Continue?` no FIM de uma das últimas linhas não-vazias, SOB `idle`
//!   (prompt parado esperando input — script que IMPRIME `(y/n)` e segue rodando não
//!   está idle). A taxa de FP é **medida**, nunca assumida zero (issue #28174 refuta a
//!   meta — aspiracional por evidência): o contador vive na [`DetectionTelemetry`],
//!   alimentado por rótulo externo (ground truth do harness / botão "Não era um
//!   pedido" do ux-flows §fila-de-atenção).
//!
//! ## Doutrina (gate humano — CLAUDE.md)
//! Detecção é OBSERVABILIDADE: nenhum campo daqui decide aprovação/injeção (isso é
//! F1-1-7/8, atrás do ADR de segurança). `tool`/`detail` nascem de payload de hook
//! (`trusted: false` por contrato do pipeline) — servem ao toast, jamais a autoridade.
//!
//! ## Padrão do módulo
//! Puro e dirigido pelo chamador (sem thread/relógio próprio), como o
//! `lifecycle::LifecycleEngine` — `now_ms` é injetado, testável headless. Política de
//! camadas é do chamador: nó com `HooksCapability::HttpHooks` alimenta só
//! [`PermissionDetector::observe_hook`]; nó `GridOnly` alimenta só
//! [`PermissionDetector::observe_grid`] (evita dupla emissão do mesmo pedido).

use std::collections::HashMap;
use std::sync::OnceLock;

use lina_vt::VtBackend;
use regex::Regex;

use crate::events::{fnv1a, DomainEvent, EventStore, PermissionEvidence, PromptKind, StoreError};

/// Janela máxima (ms) entre o `PreToolUse` pendente e a `Notification` para a
/// correlação valer. No fluxo real os dois chegam em sub-segundo; 15s cobre listener
/// sob carga sem deixar uma pendência VELHA promover uma notification qualquer.
pub const CORRELATION_WINDOW_MS: u64 = 15_000;
/// Quantas linhas não-vazias do FUNDO do viewport o fallback varre (prompt de
/// permissão mora na cauda; diálogos com opções abaixo da pergunta cabem na janela).
pub const GRID_SCAN_ROWS: usize = 6;
/// R2b: linhas não-vazias da cauda varridas pela âncora de CHOICE (o rodapé da caixa
/// de múltipla escolha mora na última linha do widget).
pub const CHOICE_SCAN_ROWS: usize = 6;
/// R2b: lookback (linhas do viewport, acima do rodapé) para localizar a PERGUNTA da
/// caixa (`…?`) — a caixa do Claude Code tem pergunta + N opções + rodapé.
pub const CHOICE_QUESTION_LOOKBACK: usize = 14;

/// R2b: um prompt ARMADO que SAIU do grid (respondido no terminal, cancelado ou
/// rolou) — o sinal que tira o item da fila quando o gesto acontece LÁ (ação primária
/// do `choice` é "ir para o terminal"). Drenado por
/// [`PermissionDetector::take_cleared`]; o [`PermissionWatch::scan`] o converte no
/// evento `PermissionPromptCleared` (semântica deliberada do contrato: NÃO é
/// `PermissionDismissed` — o pedido era real, contaminaria a telemetria de FP — nem
/// `PermissionResolved` — ninguém decidiu pela fila; o log guarda fatos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearedPrompt {
    pub node_id: String,
    pub stable_id: String,
    pub kind: PromptKind,
}

/// Sinal de hook normalizado que o detector consome — ESPELHO mínimo dos kinds do
/// pipeline F1-1-3 (`lina-hooks`), duplicado de propósito (mesmo padrão do bootstrap:
/// o core não depende do crate do listener; quem liga as pontas é o app).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSignal<'a> {
    /// `PreToolUse`: a tool vai rodar (ANTES do gate de permissão). `tool` =
    /// `tool_name` do payload; `detail` = resumo legível do input (`"git push
    /// origin/master"`) quando o pipeline o expuser (hoje `None` — ver relatório).
    PreToolUse {
        tool: Option<&'a str>,
        detail: Option<&'a str>,
    },
    /// `PostToolUse`: a tool RODOU (aprovada ou sem gate) — limpa a pendência.
    PostToolUse,
    /// `Notification`: payload genérico — vira pedido SÓ se houver pendência (camada 1).
    Notification,
    /// `Stop` (fim do turno): pendência que sobrou é stale — limpa sem emitir.
    TurnEnd,
}

/// Um pedido de permissão DETECTADO — o valor pronto para virar
/// [`DomainEvent::PermissionAsked`] (o append é do chamador; emissão desacoplada,
/// mesmo padrão do pipeline de hooks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionAsk {
    pub node_id: String,
    pub tool: Option<String>,
    pub detail: Option<String>,
    pub evidence: PermissionEvidence,
    /// R2b: tipo do prompt ([`PromptKind`] canônico do evento — `yn|choice|trust`).
    /// `Choice` nasce da âncora de CHROME do rodapé, nunca de lista numerada.
    pub kind: PromptKind,
    /// R2b (ADR 0021 §1, Captura 1): hash da região do prompt no instante da detecção
    /// ([`crate::approval::prompt_snapshot_hash`], K default). `None` no caminho de
    /// hook (sem grid à mão).
    pub vt_snapshot_hash: Option<String>,
    /// Chave de idempotência `(session, tool_call, ts)` — padrão OpenCode (13.13
    /// achado 4). `session` = `node_id` (atribuído por TOKEN de spawn — nunca campo
    /// de payload); `tool_call` = seq da pendência (hook) ou hash da linha (grid).
    pub stable_id: String,
    pub ts: u64,
}

impl PermissionAsk {
    /// Converte no evento de domínio (o chamador apenda no `EventStore`).
    #[must_use]
    pub fn to_event(&self) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: self.node_id.clone(),
            tool: self.tool.clone(),
            detail: self.detail.clone(),
            evidence: self.evidence,
            stable_id: self.stable_id.clone(),
            vt_snapshot_hash: self.vt_snapshot_hash.clone(),
            prompt_kind: self.kind,
        }
    }
}

/// Telemetria da detecção — base do protocolo de medição de FP (critério 5).
/// `false_positives` NUNCA é asserido zero (#28174): é alimentado por rótulo EXTERNO
/// (ground truth do harness agora; botão "Não era um pedido" na F1-1-7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectionTelemetry {
    /// Pedidos emitidos pela camada primária (hook correlacionado).
    pub emitted_hook: u64,
    /// Pedidos emitidos pelo fallback de grid (padrão y/n).
    pub emitted_grid: u64,
    /// R2b: pedidos emitidos pelo padrão de CHOICE (âncora de chrome) — contado em
    /// separado para a calibração de FP distinguir as famílias.
    pub emitted_choice: u64,
    /// `Notification` descartadas SEM pendência correlacionável (anti-FP estrutural
    /// da camada 1 — inclui avisos de idle/elicitation do CLI).
    pub notifications_uncorrelated: u64,
    /// Candidatos de grid suprimidos por `idle == false` (texto `(y/n)` passando em
    /// output vivo — a armadilha da #28174 cortada pela condição de idle).
    pub grid_suppressed_busy: u64,
    /// FPs CONFIRMADOS por rótulo externo (nunca auto-inferidos).
    pub false_positives: u64,
}

impl DetectionTelemetry {
    /// Total de pedidos emitidos (todas as camadas e famílias — hook, grid y/n, choice).
    #[must_use]
    pub fn emitted_total(&self) -> u64 {
        self.emitted_hook + self.emitted_grid + self.emitted_choice
    }

    /// Precisão = (emitidos − FP confirmados) ÷ emitidos. `None` sem emissões
    /// (indefinida — não inventamos 100%).
    #[must_use]
    pub fn precision(&self) -> Option<f64> {
        let emitted = self.emitted_total();
        if emitted == 0 {
            return None;
        }
        let tp = emitted.saturating_sub(self.false_positives);
        #[allow(clippy::cast_precision_loss)] // contadores ≪ 2^52
        Some(tp as f64 / emitted as f64)
    }
}

/// `PreToolUse` aguardando desfecho (sem `PostToolUse`) — o cache de correlação.
#[derive(Debug, Clone)]
struct PendingTool {
    tool: Option<String>,
    detail: Option<String>,
    /// Seq monotônico por nó — o componente `tool_call` do `stable_id`.
    call_seq: u64,
    ts: u64,
}

/// Um candidato de grid JÁ emitido e ainda na tela (y/n OU choice). O `stable_id`
/// fica retido para o sinal de *cleared* quando o prompt sair do grid (R2b).
#[derive(Debug, Clone)]
struct ArmedPrompt {
    hash: u64,
    row: usize,
    stable_id: String,
    kind: PromptKind,
}

/// Bookkeeping efêmero por nó (NUNCA persistido — só o veredito vira evento, mesma
/// disciplina do heartbeat ADR 0019 §5).
#[derive(Debug, Default)]
struct NodeTrack {
    pending: Option<PendingTool>,
    call_seq: u64,
    /// Última pendência JÁ emitida — `Notification` repetida do mesmo pedido não
    /// re-emite (anti-amplificação, ADR 0003/0005).
    last_emitted_call: Option<u64>,
    /// Candidato y/n JÁ emitido e ainda na tela — re-arma quando o prompt some OU muda
    /// de conteúdo OU aparece em OUTRA linha do viewport (pedido NOVO de texto
    /// idêntico: a tela rolou, o prompt novo nasce noutra row; o pendente parado fica
    /// na mesma — calibração da rodada reduzida, ver relatório).
    armed_grid: Option<ArmedPrompt>,
    /// R2b: candidato de CHOICE JÁ emitido (estado independente do y/n — os padrões
    /// armam/desarmam sem interferir um no outro).
    armed_choice: Option<ArmedPrompt>,
}

/// O detector multi-camada. Um por workspace; estado por nó.
#[derive(Debug, Default)]
pub struct PermissionDetector {
    nodes: HashMap<String, NodeTrack>,
    telemetry: DetectionTelemetry,
    /// R2b: prompts armados que SAÍRAM do grid desde o último
    /// [`PermissionDetector::take_cleared`] (sinal "respondido no terminal").
    cleared: Vec<ClearedPrompt>,
}

impl PermissionDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// **Camada 1** — consome um sinal de hook do nó. Devolve `Some` quando uma
    /// `Notification` correlaciona com a pendência (pedido de permissão detectado).
    pub fn observe_hook(
        &mut self,
        node_id: &str,
        signal: HookSignal<'_>,
        now_ms: u64,
    ) -> Option<PermissionAsk> {
        let track = self.nodes.entry(node_id.to_string()).or_default();
        match signal {
            HookSignal::PreToolUse { tool, detail } => {
                track.call_seq += 1;
                // "Cache do último PreToolUse por sessão" (story): o ÚLTIMO vence.
                track.pending = Some(PendingTool {
                    tool: tool.map(str::to_string),
                    detail: detail.map(str::to_string),
                    call_seq: track.call_seq,
                    ts: now_ms,
                });
                None
            }
            HookSignal::PostToolUse | HookSignal::TurnEnd => {
                track.pending = None;
                None
            }
            HookSignal::Notification => {
                let Some(pending) = track.pending.as_ref() else {
                    // Sem tool pendente não há o que aprovar: é aviso de idle/afins.
                    self.telemetry.notifications_uncorrelated += 1;
                    return None;
                };
                if now_ms.saturating_sub(pending.ts) > CORRELATION_WINDOW_MS {
                    self.telemetry.notifications_uncorrelated += 1;
                    return None;
                }
                if track.last_emitted_call == Some(pending.call_seq) {
                    return None; // mesmo pedido, notification duplicada — já emitido
                }
                track.last_emitted_call = Some(pending.call_seq);
                self.telemetry.emitted_hook += 1;
                // R2b: a tool dá a pista do tipo (`AskUserQuestion` = caixa de múltipla
                // escolha). Contexto de exibição apenas; no merge da fila o GRID é o
                // dono do prompt_kind (hook nunca rebaixa Choice→Yn).
                let kind = if pending.tool.as_deref() == Some("AskUserQuestion") {
                    PromptKind::Choice
                } else {
                    PromptKind::Yn
                };
                Some(PermissionAsk {
                    node_id: node_id.to_string(),
                    tool: pending.tool.clone(),
                    detail: pending.detail.clone(),
                    evidence: PermissionEvidence::Hook,
                    kind,
                    // Caminho de hook não tem o grid à mão — sem Captura 1 aqui (a
                    // fila completa o hash com o do par de grid quando houver merge).
                    vt_snapshot_hash: None,
                    stable_id: stable_id(node_id, &format!("call:{}", pending.call_seq), now_ms),
                    ts: now_ms,
                })
            }
        }
    }

    /// **Camada 2 (fallback)** — varre o grid do nó pela trait [`VtBackend`].
    /// `idle` vem do chamador (sem output novo na janela de amostragem / lifecycle):
    /// um prompt de verdade está PARADO esperando input; output vivo com `(y/n)` no
    /// texto não emite (contado em `grid_suppressed_busy`). Emite UMA vez por
    /// candidato; re-arma quando o prompt some ou muda — e o sumiço de um candidato
    /// ARMADO vira um [`ClearedPrompt`] (drenado por [`Self::take_cleared`]).
    ///
    /// R2b: dois padrões independentes, CHOICE testado primeiro (âncora de CHROME do
    /// rodapé — mais específica; a pergunta de uma caixa de escolha pode conter
    /// "Do you want to proceed?" e casaria o y/n por engano).
    pub fn observe_grid(
        &mut self,
        node_id: &str,
        vt: &dyn VtBackend,
        idle: bool,
        now_ms: u64,
    ) -> Option<PermissionAsk> {
        let choice = scan_choice(vt);
        let yn = scan_grid(vt);
        let track = self.nodes.entry(node_id.to_string()).or_default();

        // Sumiço ⇒ re-arma + registra o cleared (respondido no terminal/cancelado).
        if choice.is_none() {
            if let Some(armed) = track.armed_choice.take() {
                self.cleared.push(ClearedPrompt {
                    node_id: node_id.to_string(),
                    stable_id: armed.stable_id,
                    kind: armed.kind,
                });
            }
        }
        if yn.is_none() {
            if let Some(armed) = track.armed_grid.take() {
                self.cleared.push(ClearedPrompt {
                    node_id: node_id.to_string(),
                    stable_id: armed.stable_id,
                    kind: armed.kind,
                });
            }
        }

        // ── CHOICE (R2b): âncora de chrome + idle ──
        if let Some(c) = choice {
            // O rodapé é estático ("Enter to select…"): o conteúdo que distingue UM
            // pedido do próximo é a PERGUNTA — ela entra no hash de re-arm.
            let hash =
                fnv1a(format!("{}|{}", c.footer, c.question.as_deref().unwrap_or("")).as_bytes());
            if !idle {
                self.telemetry.grid_suppressed_busy += 1;
                return None;
            }
            let same = track
                .armed_choice
                .as_ref()
                .is_some_and(|a| a.hash == hash && a.row == c.row);
            if same {
                return None; // mesma caixa pendente — pedido já emitido
            }
            // Caixa NOVA no lugar da antiga (sem passar por "sem candidato"): o
            // pedido anterior foi respondido — registra o cleared dele.
            if let Some(old) = track.armed_choice.take() {
                self.cleared.push(ClearedPrompt {
                    node_id: node_id.to_string(),
                    stable_id: old.stable_id,
                    kind: old.kind,
                });
            }
            let sid = stable_id(node_id, &format!("choice:{hash:016x}:{}", c.row), now_ms);
            track.armed_choice = Some(ArmedPrompt {
                hash,
                row: c.row,
                stable_id: sid.clone(),
                kind: PromptKind::Choice,
            });
            self.telemetry.emitted_choice += 1;
            return Some(PermissionAsk {
                node_id: node_id.to_string(),
                tool: None,
                detail: c.question,
                evidence: PermissionEvidence::Grid,
                kind: PromptKind::Choice,
                vt_snapshot_hash: Some(crate::approval::prompt_snapshot_hash(
                    vt,
                    crate::approval::PROMPT_REGION_ROWS,
                )),
                stable_id: sid,
                ts: now_ms,
            });
        }

        // ── y/n (fluxo F1-1-6, intacto) ──
        let (line, row) = yn?;
        let line_hash = fnv1a(line.as_bytes());
        if !idle {
            self.telemetry.grid_suppressed_busy += 1;
            return None;
        }
        let same = track
            .armed_grid
            .as_ref()
            .is_some_and(|a| a.hash == line_hash && a.row == row);
        if same {
            return None; // mesmo prompt na MESMA linha — pedido já emitido, pendente
        }
        if let Some(old) = track.armed_grid.take() {
            self.cleared.push(ClearedPrompt {
                node_id: node_id.to_string(),
                stable_id: old.stable_id,
                kind: old.kind,
            });
        }
        let sid = stable_id(node_id, &format!("grid:{line_hash:016x}:{row}"), now_ms);
        track.armed_grid = Some(ArmedPrompt {
            hash: line_hash,
            row,
            stable_id: sid.clone(),
            kind: PromptKind::Yn,
        });
        self.telemetry.emitted_grid += 1;
        Some(PermissionAsk {
            node_id: node_id.to_string(),
            tool: None,
            detail: Some(line),
            evidence: PermissionEvidence::Grid,
            kind: PromptKind::Yn,
            vt_snapshot_hash: Some(crate::approval::prompt_snapshot_hash(
                vt,
                crate::approval::PROMPT_REGION_ROWS,
            )),
            stable_id: sid,
            ts: now_ms,
        })
    }

    /// R2b: drena os prompts armados que SAÍRAM do grid desde a última chamada — o
    /// sinal "a pergunta foi respondida/abandonada NO TERMINAL" que tira o item da
    /// fila (ação primária do `choice` resolve lá). Vira `PermissionPromptCleared`
    /// quando a variante aditiva aterrissar (pedido conjunto detecção+UI ao Arquiteto).
    #[must_use]
    pub fn take_cleared(&mut self) -> Vec<ClearedPrompt> {
        std::mem::take(&mut self.cleared)
    }

    /// Telemetria corrente (leitura para dashboard/harness).
    #[must_use]
    pub fn telemetry(&self) -> &DetectionTelemetry {
        &self.telemetry
    }

    /// Registra um FP CONFIRMADO por rótulo externo (ground truth do harness; na
    /// F1-1-7, o botão "Não era um pedido" do ux-flows). Nunca auto-inferido.
    pub fn record_false_positive(&mut self) {
        self.telemetry.false_positives += 1;
    }
}

/// `stable_id` determinístico de `(session, tool_call, ts)` — hex compacto. `session`
/// = `node_id` (token de spawn); dois pedidos seguidos do mesmo nó diferem por
/// `tool_call` E `ts`.
fn stable_id(node_id: &str, marker: &str, ts: u64) -> String {
    format!(
        "{:016x}",
        fnv1a(format!("{node_id}|{marker}|{ts}").as_bytes())
    )
}

/// Regex do fallback (compilada 1×): padrão de prompt interativo no FIM da linha —
/// `(y/n)`/`[Y/n]`/`(yes/no)`/`yes/no`/`Continue?`/`Do you want to proceed?`, com
/// cauda de no máximo 4 chars de pontuação/espaço (`: > » ? ⟩`). Texto que apenas
/// CONTÉM o padrão no meio da linha não casa — junto com a condição de idle, é o que
/// corta a armadilha da #28174 (impressão não-interativa de `(y/n)`).
fn pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(\((?:y/n|yes/no)\)|\[(?:y/n|yes/no)\]|\byes/no\b|continue\?|do you want to proceed\??)[\s:>»❯?]{0,4}$",
        )
        .unwrap_or_else(|e| unreachable!("regex literal do fallback inválida: {e}"))
    })
}

/// `true` se a linha (sem trailing whitespace) termina num padrão de prompt y/n.
/// Função PURA — unit-testável sem grid.
#[must_use]
pub fn is_permission_prompt_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    !trimmed.is_empty() && pattern().is_match(trimmed)
}

/// Varre as últimas [`GRID_SCAN_ROWS`] linhas não-vazias do viewport (da última para
/// cima) e devolve a primeira candidata `(texto, row)`. Lê SÓ pela trait (texto já
/// parseado, sem ANSI — `row_text` resolve cores/escapes no motor VT).
fn scan_grid(vt: &dyn VtBackend) -> Option<(String, usize)> {
    let (_, rows) = vt.dims();
    let mut seen = 0_usize;
    for row in (0..rows).rev() {
        let text = vt.row_text(row);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if is_permission_prompt_line(trimmed) {
            return Some((trimmed.to_string(), row));
        }
        seen += 1;
        if seen >= GRID_SCAN_ROWS {
            break;
        }
    }
    None
}

// ───────────────── R2b · padrão "pergunta de escolha" (âncora de CHROME) ─────────────────

/// R2b: `true` se a linha é o RODAPÉ da caixa de múltipla escolha do Claude Code —
/// chrome observado na tela do fundador (2026-06-07 20:30):
/// `Enter to select · ↑/↓ to navigate · Esc to cancel`.
///
/// A âncora exige TRÊS sinais de chrome na MESMA linha: `enter to
/// select|submit|confirm` **e** `to navigate` **e** (`esc to …` OU o separador `·`)
/// — vocabulário de widget navegável, não de conteúdo. O 3º sinal veio da rodada de
/// calibração R2b: prosa citando DUAS frases ("docs: print Enter to select and arrows
/// to navigate") furava; o rodapé real sempre carrega o Esc/separador. Lista numerada
/// NUNCA é sinal (toda lista markdown é numerada — FP explosivo, proibido pela
/// story). Resíduo conhecido (medido, nunca jurado zero): linha que cita o chrome
/// COMPLETO com separadores — indistinguível por texto; trap rotulada
/// `chrome-cite-idle` do protocolo de calibração.
#[must_use]
pub fn is_choice_chrome_line(line: &str) -> bool {
    let l = line.trim().to_ascii_lowercase();
    if l.is_empty() {
        return false;
    }
    let select = l.contains("enter to select")
        || l.contains("enter to submit")
        || l.contains("enter to confirm");
    let third = l.contains("esc to") || l.contains('·');
    select && l.contains("to navigate") && third
}

/// Candidato de caixa de escolha: rodapé de chrome + a pergunta localizada acima.
#[derive(Debug, Clone)]
struct ChoiceCandidate {
    /// A pergunta da caixa (linha não-vazia mais próxima ACIMA do rodapé terminando em
    /// `?`, bordas de caixa removidas) — contexto de exibição do toast; `None` quando
    /// não localizável dentro do lookback.
    question: Option<String>,
    /// O texto do rodapé (a âncora).
    footer: String,
    /// Row do rodapé no viewport.
    row: usize,
}

/// R2b: varre as últimas [`CHOICE_SCAN_ROWS`] linhas não-vazias do fundo procurando o
/// rodapé de chrome; ao achar, sobe até [`CHOICE_QUESTION_LOOKBACK`] linhas pela
/// pergunta (`…?`). Lê SÓ pela trait (texto pós-emulador).
fn scan_choice(vt: &dyn VtBackend) -> Option<ChoiceCandidate> {
    let (_, rows) = vt.dims();
    let mut seen = 0_usize;
    for row in (0..rows).rev() {
        let text = vt.row_text(row);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if is_choice_chrome_line(trimmed) {
            let lo = row.saturating_sub(CHOICE_QUESTION_LOOKBACK);
            let question = (lo..row).rev().find_map(|q| {
                // Remove borda de caixa (│ ┃ |) e espaços das pontas antes de testar o `?`.
                let raw = vt.row_text(q);
                let clean = raw.trim().trim_matches(['│', '┃', '┆', '|']).trim();
                (clean.ends_with('?')).then(|| clean.to_string())
            });
            return Some(ChoiceCandidate {
                question,
                footer: trimmed.to_string(),
                row,
            });
        }
        seen += 1;
        if seen >= CHOICE_SCAN_ROWS {
            break;
        }
    }
    None
}

// ───────────────── R2b · fiação VIVA (grids reais → EventStore do workspace) ─────────────────

/// R2b: silêncio de OUTPUT do PTY (ms) que conta como `idle` para o fallback de grid
/// — mesma calibração do protocolo de FP (`permission_fp`). Um prompt de verdade está
/// PARADO; saída viva nunca emite.
pub const WATCH_IDLE_MS: u64 = 1_500;

/// Razão canônica da transição de lifecycle quando um prompt bloqueante é detectado
/// (`NodeStatusChanged.reason`) — o cabeçalho "Idle" parado numa pergunta é estado
/// mentiroso (direção do fundador, R2b).
pub const BLOCKED_REASON_PROMPT: &str = "permission_prompt";

/// R2b: fiação opcional do lifecycle — ao emitir um pedido, transiciona o nó para
/// `Blocked` pela porta canônica ([`crate::LifecycleEngine::transition`]; fato antes
/// do efeito, no-op se já `Blocked`). O `map` traduz o `NodeId` do pty-host para o
/// `NodeId` do Supervisor — o mapeamento pertence ao app (autoridade única de
/// identidade é o Supervisor; o core não o adivinha).
pub struct LifecycleWiring<'a> {
    pub engine: &'a mut crate::LifecycleEngine,
    pub supervisor: &'a crate::Supervisor,
    /// pty-host `NodeId` → Supervisor `NodeId` (`None` = nó sem par no roster).
    pub map: &'a dyn Fn(crate::NodeId) -> Option<crate::NodeId>,
}

/// R2b (tarefa 2): o LOOP VIVO da detecção — varre os grids reais do [`crate::PtyHost`],
/// deriva `idle` por silêncio de OUTPUT (bytes lidos do PTY estáveis — sinal do
/// produtor, não proxy de render), respeita o mute por nó (projeção do
/// `NodeDetectionMuted` derivada do LOG) e apenda cada `PermissionAsked` no
/// `EventStore` do workspace.
///
/// Caller-driven, sem thread própria (padrão do módulo): o app chama
/// [`PermissionWatch::scan`] no tick. Cadência ≤ 1s ⇒ latência de detecção ≤
/// `idle_after_ms` + 1 tick (< 3s, critério observável da story).
///
/// Alimente [`PermissionWatch::observe_event`] com o log (replay no boot + eventos
/// vivos) para a projeção de mute convergir — replay NUNCA escreve nem apenda
/// (entrada é `&DomainEvent`, sem porta).
pub struct PermissionWatch {
    det: PermissionDetector,
    /// Por nó: (bytes lidos na última amostra, `now_ms` da última MUDANÇA de output).
    activity: HashMap<crate::NodeId, (u64, u64)>,
    idle_after_ms: u64,
    /// Projeção `NodeDetectionMuted` (último evento do nó vence) — derivada do log.
    muted: HashMap<String, bool>,
}

impl Default for PermissionWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionWatch {
    #[must_use]
    pub fn new() -> Self {
        Self::with_idle_after(WATCH_IDLE_MS)
    }

    /// Janela de idle customizada (testes/calibração).
    #[must_use]
    pub fn with_idle_after(idle_after_ms: u64) -> Self {
        Self {
            det: PermissionDetector::new(),
            activity: HashMap::new(),
            idle_after_ms,
            muted: HashMap::new(),
        }
    }

    /// Aplica um evento do log à projeção de mute (boot/replay + vivo). Nó mutado tem
    /// o fallback de GRID desligado na varredura (a fila aplica a mesma regra na
    /// projeção dela — defesa em profundidade nas duas pontas).
    pub fn observe_event(&mut self, event: &DomainEvent) {
        if let DomainEvent::NodeDetectionMuted { node_id, muted } = event {
            self.muted.insert(node_id.clone(), *muted);
        }
    }

    /// Detector interno (telemetria de FP / calibração).
    #[must_use]
    pub fn detector(&self) -> &PermissionDetector {
        &self.det
    }

    /// Rótulo externo de FP (repasse — ver [`PermissionDetector::record_false_positive`]).
    pub fn record_false_positive(&mut self) {
        self.det.record_false_positive();
    }

    /// UMA varredura dos terminais vivos do host: deriva `idle` por nó, roda o
    /// detector de grid e **apenda** cada pedido no `EventStore` do workspace.
    /// Com [`LifecycleWiring`], também transiciona o nó detectado para `Blocked`
    /// (`NodeStatusChanged{reason: permission_prompt}` — no-op se já estava).
    ///
    /// Fecho R2b: prompts que SUMIRAM do grid (respondidos direto no terminal) são
    /// drenados no fim da varredura e apendados como `PermissionPromptCleared` — a
    /// fila os remove no fold, sem decisão fabricada.
    ///
    /// Devolve o que ESTA varredura emitiu ([`ScanOutcome`]) para o app rotear ao
    /// vivo (fila/toast) sem re-projetar o log.
    pub fn scan(
        &mut self,
        host: &crate::PtyHost,
        store: &mut EventStore,
        mut lifecycle: Option<LifecycleWiring<'_>>,
        now_ms: u64,
    ) -> Result<ScanOutcome, StoreError> {
        let mut out = Vec::new();
        for node in host.nodes() {
            let node_str = node.to_string();
            if self.muted.get(&node_str).copied().unwrap_or(false) {
                continue; // fallback de grid DESLIGADO para o nó (hook não passa aqui)
            }
            let Some(metrics) = host.metrics(node) else {
                continue;
            };
            let entry = self
                .activity
                .entry(node)
                .or_insert((metrics.bytes_read, now_ms));
            if metrics.bytes_read != entry.0 {
                *entry = (metrics.bytes_read, now_ms);
            }
            let idle = now_ms.saturating_sub(entry.1) >= self.idle_after_ms;
            let ask = host
                .with_grid(node, |vt| {
                    self.det.observe_grid(&node_str, vt, idle, now_ms)
                })
                .flatten();
            let Some(ask) = ask else { continue };
            // Fato antes do efeito: o pedido entra no log do workspace JÁ na varredura.
            store.append(&ask.to_event())?;
            if let Some(w) = lifecycle.as_mut() {
                if let Some(sup_node) = (w.map)(node) {
                    // `Blocked` pela porta canônica do dono da state-machine; falha de
                    // transição não derruba a varredura (o pedido JÁ está auditado).
                    if let Err(e) = w.engine.transition(
                        w.supervisor,
                        store,
                        sup_node,
                        crate::NodeStatus::Blocked,
                        BLOCKED_REASON_PROMPT,
                    ) {
                        tracing::warn!(node = %node_str, error = %e,
                            "detecção emitiu pedido mas a transição p/ Blocked falhou");
                    }
                }
            }
            out.push(ask);
        }
        // R2b item 5 (fecho): prompts respondidos DIRETO no terminal — o sumiço do
        // grid vira `PermissionPromptCleared` no log (fato observado, sem decisão
        // fabricada); a fila remove o item no fold.
        let cleared = self.det.take_cleared();
        for c in &cleared {
            store.append(&DomainEvent::PermissionPromptCleared {
                node_id: c.node_id.clone(),
                stable_id: c.stable_id.clone(),
            })?;
        }
        Ok(ScanOutcome { asks: out, cleared })
    }
}

/// O que UMA varredura do [`PermissionWatch`] emitiu (e já apendou no log) — para o
/// app rotear ao vivo sem re-projetar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// Pedidos novos (`PermissionAsked` apendados nesta varredura).
    pub asks: Vec<PermissionAsk>,
    /// Prompts respondidos no terminal (`PermissionPromptCleared` apendados).
    pub cleared: Vec<ClearedPrompt>,
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lina_vt::AlacrittyBackend;

    const T0: u64 = 1_750_000_000_000;

    // ───────────── camada 1: hook + correlação ─────────────

    /// Critério 1 (núcleo): `PreToolUse` pendente + `Notification` → pedido COM o
    /// contexto da tool; evidence=hook.
    #[test]
    fn notification_with_pending_pretooluse_emits_with_tool_context() {
        let mut det = PermissionDetector::new();
        assert!(det
            .observe_hook(
                "node-a",
                HookSignal::PreToolUse {
                    tool: Some("Bash"),
                    detail: Some("git push origin/master"),
                },
                T0,
            )
            .is_none());
        let ask = det
            .observe_hook("node-a", HookSignal::Notification, T0 + 200)
            .expect("notification correlacionada deve emitir");
        assert_eq!(ask.tool.as_deref(), Some("Bash"));
        assert_eq!(ask.detail.as_deref(), Some("git push origin/master"));
        assert_eq!(ask.evidence, PermissionEvidence::Hook);
        assert_eq!(det.telemetry().emitted_hook, 1);
    }

    /// Anti-FP estrutural (#32952): `Notification` SEM pendência (aviso de idle) é
    /// descartada e CONTADA — não vira pedido.
    #[test]
    fn notification_without_pending_is_discarded_and_counted() {
        let mut det = PermissionDetector::new();
        assert!(det
            .observe_hook("node-a", HookSignal::Notification, T0)
            .is_none());
        assert_eq!(det.telemetry().notifications_uncorrelated, 1);
        assert_eq!(det.telemetry().emitted_total(), 0);
    }

    /// `PostToolUse` fecha a pendência (tool rodou sem gate) — a `Notification`
    /// posterior não correlaciona. Idem para fim de turno (`Stop`).
    #[test]
    fn posttooluse_and_turnend_clear_pending() {
        for closer in [HookSignal::PostToolUse, HookSignal::TurnEnd] {
            let mut det = PermissionDetector::new();
            det.observe_hook(
                "n",
                HookSignal::PreToolUse {
                    tool: Some("Bash"),
                    detail: None,
                },
                T0,
            );
            det.observe_hook("n", closer, T0 + 50);
            assert!(
                det.observe_hook("n", HookSignal::Notification, T0 + 100)
                    .is_none(),
                "pendência fechada por {closer:?} não pode correlacionar"
            );
        }
    }

    /// Pendência VELHA (fora da janela) não promove uma notification qualquer.
    #[test]
    fn stale_pending_outside_window_does_not_correlate() {
        let mut det = PermissionDetector::new();
        det.observe_hook(
            "n",
            HookSignal::PreToolUse {
                tool: Some("Bash"),
                detail: None,
            },
            T0,
        );
        assert!(det
            .observe_hook(
                "n",
                HookSignal::Notification,
                T0 + CORRELATION_WINDOW_MS + 1
            )
            .is_none());
        assert_eq!(det.telemetry().notifications_uncorrelated, 1);
    }

    /// Critério 4: dois pedidos seguidos do MESMO nó → 2 `stable_id` distintos.
    #[test]
    fn two_sequential_asks_same_node_distinct_stable_ids() {
        let mut det = PermissionDetector::new();
        det.observe_hook(
            "n",
            HookSignal::PreToolUse {
                tool: Some("Bash"),
                detail: Some("rm -rf build"),
            },
            T0,
        );
        let ask1 = det
            .observe_hook("n", HookSignal::Notification, T0 + 100)
            .expect("ask 1");
        // humano aprovou → a tool roda → PostToolUse fecha o ciclo.
        det.observe_hook("n", HookSignal::PostToolUse, T0 + 5_000);
        det.observe_hook(
            "n",
            HookSignal::PreToolUse {
                tool: Some("Bash"),
                detail: Some("rm -rf build"),
            },
            T0 + 6_000,
        );
        let ask2 = det
            .observe_hook("n", HookSignal::Notification, T0 + 6_100)
            .expect("ask 2");
        assert_ne!(ask1.stable_id, ask2.stable_id);
    }

    /// `Notification` DUPLICADA do mesmo pedido (mesma pendência) não re-emite —
    /// anti-amplificação.
    #[test]
    fn duplicate_notification_same_pending_emits_once() {
        let mut det = PermissionDetector::new();
        det.observe_hook(
            "n",
            HookSignal::PreToolUse {
                tool: Some("Write"),
                detail: None,
            },
            T0,
        );
        assert!(det
            .observe_hook("n", HookSignal::Notification, T0 + 100)
            .is_some());
        assert!(det
            .observe_hook("n", HookSignal::Notification, T0 + 200)
            .is_none());
        assert_eq!(det.telemetry().emitted_hook, 1);
    }

    // ───────────── camada 2: fallback de grid ─────────────

    /// Backend REAL (alacritty atrás da trait) com o prompt sintético do critério 2.
    fn grid_with(bytes: &[u8]) -> AlacrittyBackend {
        let mut vt = AlacrittyBackend::new(80, 24);
        vt.advance(bytes);
        vt
    }

    /// Critério 2 (metade real): grid sintético com `(y/n)` + idle → evento com
    /// `evidence=grid` e a linha do prompt como detail.
    #[test]
    fn grid_yn_prompt_idle_emits_grid_evidence() {
        let vt = grid_with(b"$ ./deploy.sh\r\nDeploy to production? (y/n) ");
        let mut det = PermissionDetector::new();
        let ask = det
            .observe_grid("node-g", &vt, true, T0)
            .expect("prompt y/n idle deve emitir");
        assert_eq!(ask.evidence, PermissionEvidence::Grid);
        assert_eq!(ask.tool, None, "fallback não tem contexto de tool");
        assert_eq!(ask.detail.as_deref(), Some("Deploy to production? (y/n)"));
        assert_eq!(det.telemetry().emitted_grid, 1);
    }

    /// Critério 2 (metade FP, #28174): candidato FALSO — terminal IDLE cuja última
    /// linha imprime `(y/n)` como texto — É emitido (limite conhecido do mecanismo) e
    /// o rótulo externo o conta na telemetria. SEM assert de zero: o teste prova que
    /// a MEDIÇÃO funciona, não que FP não existe.
    #[test]
    fn grid_false_candidate_is_counted_in_fp_telemetry_no_zero_assert() {
        // Script terminou DEIXANDO o texto-armadilha como última linha (não-interativo).
        let vt = grid_with(b"INFO: prompts use the marker (y/n)");
        let mut det = PermissionDetector::new();
        let ask = det.observe_grid("node-g", &vt, true, T0);
        assert!(
            ask.is_some(),
            "a armadilha #28174 engana o fallback — por design medimos, não juramos zero"
        );
        det.record_false_positive(); // ground truth do harness: NÃO era pedido
        assert_eq!(det.telemetry().false_positives, 1);
        assert_eq!(det.telemetry().precision(), Some(0.0));
    }

    /// Output VIVO contendo `(y/n)` (build/log passando) não emite: a condição de
    /// idle corta a maior família de FP — e o corte é contado (telemetria).
    #[test]
    fn grid_busy_candidate_is_suppressed_and_counted() {
        let vt = grid_with(b"building...\r\nvalid answers are (y/n)");
        let mut det = PermissionDetector::new();
        assert!(det.observe_grid("node-g", &vt, false, T0).is_none());
        assert_eq!(det.telemetry().grid_suppressed_busy, 1);
        assert_eq!(det.telemetry().emitted_total(), 0);
    }

    /// O MESMO prompt amostrado N vezes emite UMA vez; o prompt sumir re-arma; um
    /// pedido novo (mesmo texto) emite de novo com `stable_id` distinto (critério 4
    /// na camada de grid + replay-friendly: 2 pedidos = 2 eventos, não N amostras).
    #[test]
    fn grid_same_prompt_emits_once_rearms_when_cleared() {
        let mut vt = grid_with(b"Continue? (y/n) ");
        let mut det = PermissionDetector::new();
        let ask1 = det.observe_grid("n", &vt, true, T0).expect("emissão 1");
        assert!(det.observe_grid("n", &vt, true, T0 + 500).is_none());
        assert!(det.observe_grid("n", &vt, true, T0 + 1_000).is_none());
        assert_eq!(det.telemetry().emitted_grid, 1, "1 pedido = 1 evento");

        // Humano respondeu: o script segue e o prompt some do fundo do grid.
        vt.advance(b"y\r\nrunning step 2...\r\ndone.\r\n$ ");
        assert!(det.observe_grid("n", &vt, true, T0 + 2_000).is_none());

        // Pedido NOVO, texto idêntico → re-emite com stable_id distinto.
        vt.advance(b"\r\nContinue? (y/n) ");
        let ask2 = det
            .observe_grid("n", &vt, true, T0 + 3_000)
            .expect("emissão 2 (re-armado)");
        assert_ne!(ask1.stable_id, ask2.stable_id);
        assert_eq!(det.telemetry().emitted_grid, 2);
    }

    /// Padrões POSITIVOS e NEGATIVOS da regex de prompt (pura, sem grid).
    #[test]
    fn permission_prompt_line_patterns() {
        for positive in [
            "Continue? (y/n)",
            "Overwrite file? [Y/n]",
            "apply changes (y/N)? ",
            "Are you sure? (yes/no)",
            "Please answer yes/no: ",
            "Continue?",
            "Do you want to proceed?",
            "DO YOU WANT TO PROCEED",
        ] {
            assert!(
                is_permission_prompt_line(positive),
                "deveria casar: {positive:?}"
            );
        }
        for negative in [
            "",
            "   ",
            "the (y/n) marker appears mid-sentence in this log line",
            "comparing yes/no answers across 300 samples",
            "to continue? press enter and wait for the build",
            "$ cargo build --release",
        ] {
            assert!(
                !is_permission_prompt_line(negative),
                "NÃO deveria casar: {negative:?}"
            );
        }
    }

    /// `scan_grid` respeita a janela: candidato ALÉM das últimas `GRID_SCAN_ROWS`
    /// linhas não-vazias (prompt antigo que rolou) não conta.
    #[test]
    fn grid_scan_window_ignores_old_scrolled_prompt() {
        let mut vt = AlacrittyBackend::new(80, 24);
        vt.advance(b"Continue? (y/n) \r\n"); // prompt antigo, já respondido
        for i in 0..GRID_SCAN_ROWS + 2 {
            vt.advance(format!("log line {i}\r\n").as_bytes());
        }
        vt.advance(b"$ ");
        let mut det = PermissionDetector::new();
        assert!(det.observe_grid("n", &vt, true, T0).is_none());
    }

    /// O `to_event` produz o `DomainEvent::PermissionAsked` campo a campo — inclusive
    /// os aditivos R2b (`prompt_kind` + `vt_snapshot_hash`, a Captura 1).
    #[test]
    fn permission_ask_converts_to_domain_event() {
        let ask = PermissionAsk {
            node_id: "n1".into(),
            tool: Some("Bash".into()),
            detail: Some("git push".into()),
            evidence: PermissionEvidence::Hook,
            kind: PromptKind::Choice,
            vt_snapshot_hash: Some("cafe".into()),
            stable_id: "abc".into(),
            ts: T0,
        };
        match ask.to_event() {
            DomainEvent::PermissionAsked {
                node_id,
                tool,
                detail,
                evidence,
                stable_id,
                vt_snapshot_hash,
                prompt_kind,
            } => {
                assert_eq!(node_id, "n1");
                assert_eq!(tool.as_deref(), Some("Bash"));
                assert_eq!(detail.as_deref(), Some("git push"));
                assert_eq!(evidence, PermissionEvidence::Hook);
                assert_eq!(stable_id, "abc");
                assert_eq!(vt_snapshot_hash.as_deref(), Some("cafe"));
                assert_eq!(prompt_kind, PromptKind::Choice);
            }
            other => panic!("evento errado: {other:?}"),
        }
    }

    // ───────────── R2b · padrão CHOICE (âncora de chrome) ─────────────

    /// Bytes da caixa de múltipla escolha REAL (chrome da tela do fundador 20:30):
    /// pergunta + opções numeradas + rodapé `Enter to select · ↑/↓ to navigate · Esc
    /// to cancel`.
    fn choice_box(question: &str) -> Vec<u8> {
        format!(
            "{question}\r\n\r\n\u{276f} 1. Op\u{e7}\u{e3}o A\r\n     Primeira op\u{e7}\u{e3}o\r\n  2. Op\u{e7}\u{e3}o B\r\n  3. Chat about this\r\n\r\nEnter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel "
        )
        .into_bytes()
    }

    /// Positivos e negativos da âncora de chrome (pura). NEGATIVO CRÍTICO: lista
    /// numerada/markdown NUNCA é sinal (proibição da story — FP explosivo).
    #[test]
    fn choice_chrome_line_patterns() {
        for positive in [
            "Enter to select · ↑/↓ to navigate · Esc to cancel",
            "enter to select - up/down to navigate - esc to cancel",
            "Space to toggle · Enter to submit · ↑/↓ to navigate",
            "Enter to confirm · ↑/↓ to navigate · Esc to exit",
        ] {
            assert!(
                is_choice_chrome_line(positive),
                "deveria casar: {positive:?}"
            );
        }
        for negative in [
            "",
            "1. Primeira opção da lista",
            "2. Segunda opção — markdown comum",
            "- [ ] item de checklist",
            "Enter to select the best tool for the job", // só 1 sinal
            "use arrows to navigate the docs site",      // só 1 sinal
            // 2 sinais sem o 3º (esc/separador) — a prosa que furou a calibração R2b:
            "docs: TUI widgets print Enter to select and arrows to navigate as footer",
            "$ cargo build --release",
        ] {
            assert!(
                !is_choice_chrome_line(negative),
                "NÃO deveria casar: {negative:?}"
            );
        }
    }

    /// Caixa de escolha real + idle ⇒ emite `kind=Choice`, `evidence=Grid`, detail =
    /// a PERGUNTA, e a Captura 1 (`vt_snapshot_hash`) anexada (ADR 0021 §1).
    #[test]
    fn grid_choice_box_idle_emits_choice_kind_with_question_and_capture1() {
        let vt = grid_with(&choice_box(
            "Isto é um teste de múltipla escolha. Qual opção você quer selecionar?",
        ));
        let mut det = PermissionDetector::new();
        let ask = det
            .observe_grid("node-c", &vt, true, T0)
            .expect("caixa de escolha idle deve emitir");
        assert_eq!(ask.kind, PromptKind::Choice);
        assert_eq!(ask.evidence, PermissionEvidence::Grid);
        assert_eq!(
            ask.detail.as_deref(),
            Some("Isto é um teste de múltipla escolha. Qual opção você quer selecionar?"),
            "o toast mostra a PERGUNTA, não o rodapé"
        );
        let expected_hash =
            crate::approval::prompt_snapshot_hash(&vt, crate::approval::PROMPT_REGION_ROWS);
        assert_eq!(
            ask.vt_snapshot_hash.as_deref(),
            Some(expected_hash.as_str()),
            "Captura 1 anexada na detecção"
        );
        assert_eq!(det.telemetry().emitted_choice, 1);
        assert_eq!(det.telemetry().emitted_grid, 0, "não é o padrão y/n");
    }

    /// PROIBIÇÃO da story: lista numerada SEM o rodapé de chrome (markdown comum,
    /// terminal idle) NÃO emite nada.
    #[test]
    fn grid_numbered_list_without_chrome_never_emits() {
        let vt = grid_with(
            b"Plano de hoje:\r\n  1. Revisar o ADR\r\n  2. Rodar os testes\r\n  3. Fechar a rodada\r\n",
        );
        let mut det = PermissionDetector::new();
        assert!(
            det.observe_grid("n", &vt, true, T0).is_none(),
            "lista numerada não é pergunta"
        );
        assert_eq!(det.telemetry().emitted_total(), 0);
    }

    /// Caixa VIVA (output ainda fluindo) é suprimida pela condição de idle — mesma
    /// família anti-FP do y/n, contada na telemetria.
    #[test]
    fn grid_choice_busy_is_suppressed() {
        let vt = grid_with(&choice_box("Pergunta?"));
        let mut det = PermissionDetector::new();
        assert!(det.observe_grid("n", &vt, false, T0).is_none());
        assert_eq!(det.telemetry().grid_suppressed_busy, 1);
    }

    /// A MESMA caixa amostrada N vezes emite 1×; o sumiço re-arma E produz o sinal de
    /// CLEARED (respondido no terminal — contrato pedido pela fila/UI); uma caixa NOVA
    /// re-emite com stable_id distinto.
    #[test]
    fn grid_choice_emits_once_and_clear_signals_answered_in_terminal() {
        let mut vt = grid_with(&choice_box("Qual opção?"));
        let mut det = PermissionDetector::new();
        let ask1 = det.observe_grid("n", &vt, true, T0).expect("emissão 1");
        assert!(det.observe_grid("n", &vt, true, T0 + 500).is_none());
        assert_eq!(det.telemetry().emitted_choice, 1, "1 pedido = 1 evento");
        assert!(det.take_cleared().is_empty(), "nada respondido ainda");

        // Humano foi ao terminal e respondeu: a caixa some, o CLI segue.
        vt.advance(b"\x1b[2J\x1b[HSelecionou: Op\xc3\xa7\xc3\xa3o A\r\nseguindo...\r\n$ ");
        assert!(det.observe_grid("n", &vt, true, T0 + 2_000).is_none());
        let cleared = det.take_cleared();
        assert_eq!(
            cleared,
            vec![ClearedPrompt {
                node_id: "n".into(),
                stable_id: ask1.stable_id.clone(),
                kind: PromptKind::Choice,
            }],
            "sumiço da caixa = pergunta respondida NO TERMINAL"
        );
        assert!(det.take_cleared().is_empty(), "drenado uma vez");

        // Pergunta NOVA → pedido novo, stable_id distinto.
        vt.advance(b"\x1b[2J\x1b[H");
        vt.advance(&choice_box("Outra pergunta diferente?"));
        let ask2 = det
            .observe_grid("n", &vt, true, T0 + 3_000)
            .expect("emissão 2");
        assert_ne!(ask1.stable_id, ask2.stable_id);
        assert_eq!(ask2.detail.as_deref(), Some("Outra pergunta diferente?"));
    }

    /// CHOICE tem precedência sobre y/n quando a PERGUNTA da caixa casaria o padrão
    /// y/n ("Do you want to proceed?") — um pedido, kind=Choice, não dois.
    #[test]
    fn choice_takes_precedence_over_yn_inside_the_box() {
        let vt = grid_with(&choice_box("Do you want to proceed?"));
        let mut det = PermissionDetector::new();
        let ask = det.observe_grid("n", &vt, true, T0).expect("deve emitir");
        assert_eq!(ask.kind, PromptKind::Choice, "chrome decide o tipo");
        assert_eq!(det.telemetry().emitted_choice, 1);
        assert_eq!(det.telemetry().emitted_grid, 0, "não duplica como y/n");
        // E a próxima amostra não emite o y/n por baixo.
        let mut det2 = det;
        assert!(det2.observe_grid("n", &vt, true, T0 + 500).is_none());
    }

    /// O y/n também ganhou o sinal de cleared (a UI remove o item quando o humano
    /// responde direto no terminal — vale para as duas famílias).
    #[test]
    fn grid_yn_clear_also_signals() {
        let mut vt = grid_with(b"Continue? (y/n) ");
        let mut det = PermissionDetector::new();
        let ask = det.observe_grid("n", &vt, true, T0).expect("emite y/n");
        assert_eq!(ask.kind, PromptKind::Yn);
        vt.advance(b"y\r\ndone\r\n$ ");
        assert!(det.observe_grid("n", &vt, true, T0 + 1_000).is_none());
        let cleared = det.take_cleared();
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].stable_id, ask.stable_id);
        assert_eq!(cleared[0].kind, PromptKind::Yn);
    }

    /// Hook com tool `AskUserQuestion` rotula o pedido como Choice (contexto de
    /// exibição; o GRID é o dono do kind no merge da fila).
    #[test]
    fn hook_askuserquestion_tool_labels_choice() {
        let mut det = PermissionDetector::new();
        det.observe_hook(
            "n",
            HookSignal::PreToolUse {
                tool: Some("AskUserQuestion"),
                detail: None,
            },
            T0,
        );
        let ask = det
            .observe_hook("n", HookSignal::Notification, T0 + 100)
            .expect("emite");
        assert_eq!(ask.kind, PromptKind::Choice);
        assert_eq!(ask.vt_snapshot_hash, None, "hook não tem grid à mão");
    }

    // ───────────── R2b · fiação viva (PermissionWatch sobre PTY real) ─────────────

    /// O critério observável da story, na metade do core: um terminal REAL bloqueado
    /// numa caixa de escolha ⇒ o `scan` do watch apenda `PermissionAsked` com
    /// `prompt_kind=choice` no EventStore em < 3s (do prompt visível ao append),
    /// transiciona o nó para `Blocked` via lifecycle (cabeçalho "Idle" parado na
    /// pergunta é estado mentiroso) e o nó MUTADO no log nunca emite.
    #[cfg(unix)] // F1-6-8: spawna `sh -c` em PTY real — runtime-Unix (tabela ci-3so-triagem.md).
    #[test]
    fn watch_appends_choice_ask_within_sla_blocks_node_and_respects_mute() {
        use std::time::{Duration, Instant};

        fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
            let start = Instant::now();
            loop {
                if cond() {
                    return true;
                }
                if start.elapsed() >= timeout {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        const T: Duration = Duration::from_secs(5);
        let script = "printf 'Qual op\u{e7}\u{e3}o voc\u{ea} quer selecionar?\\n  1. Op\u{e7}\u{e3}o A\\n  2. Op\u{e7}\u{e3}o B\\nEnter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel '; read x";

        let mut host = crate::PtyHost::new();
        let node = host
            .spawn(crate::PtyCommand::new("sh").arg("-c").arg(script), 100, 24)
            .expect("spawn caixa de escolha");
        let muted_node = host
            .spawn(crate::PtyCommand::new("sh").arg("-c").arg(script), 100, 24)
            .expect("spawn nó mutado");

        // Lifecycle real: roster do Supervisor + engine + mapa pty→sup (papel do app).
        let sup = crate::Supervisor::new();
        let sup_node = sup.register("@Terminal A", None, Box::new(std::io::sink()));
        sup.set_status(sup_node, crate::NodeStatus::Idle)
            .expect("status inicial Idle — o 'estado mentiroso' da tela do fundador");
        let mut engine = crate::LifecycleEngine::new();

        let dir = std::env::temp_dir().join(format!("lina-r2b-watch-{}", uuid::Uuid::now_v7()));
        let mut store = EventStore::open(&dir).expect("store");

        let mut watch = PermissionWatch::new();
        // O nó 2 está MUTADO no log (projeção derivada — `NodeDetectionMuted`).
        watch.observe_event(&DomainEvent::NodeDetectionMuted {
            node_id: muted_node.to_string(),
            muted: true,
        });

        // Espera o prompt aparecer nos DOIS grids (condição, não sleep).
        for n in [node, muted_node] {
            assert!(
                poll_until(T, || host
                    .with_grid(n, |vt| vt.last_nonempty_line())
                    .is_some_and(|l| l.contains("Enter to select"))),
                "rodapé da caixa deveria aparecer no grid"
            );
        }
        let prompt_visible = Instant::now();

        // Bombeia o scan como o app fará no tick (250ms), com relógio real.
        let map = |n: crate::NodeId| (n == node).then_some(sup_node);
        let mut emitted = Vec::new();
        assert!(
            poll_until(T, || {
                let out = watch
                    .scan(
                        &host,
                        &mut store,
                        Some(LifecycleWiring {
                            engine: &mut engine,
                            supervisor: &sup,
                            map: &map,
                        }),
                        crate::now_ms(),
                    )
                    .expect("scan");
                emitted.extend(out.asks);
                !emitted.is_empty()
            }),
            "o watch deveria emitir o pedido do nó não-mutado"
        );
        let latency = prompt_visible.elapsed();

        // <3s do prompt visível ao append (critério da story; idle=1.5s + ticks).
        assert!(
            latency < Duration::from_secs(3),
            "latência {latency:?} estourou o SLA de 3s"
        );
        assert_eq!(emitted.len(), 1, "só o nó NÃO-mutado emite");
        assert_eq!(emitted[0].node_id, node.to_string());
        assert_eq!(emitted[0].kind, PromptKind::Choice);

        // O evento está NO LOG do workspace com os campos R2b (payload persistido).
        let recs = store.events().expect("events");
        let asked: Vec<_> = recs
            .iter()
            .filter(|r| r.kind == "PermissionAsked")
            .collect();
        assert_eq!(
            asked.len(),
            1,
            "1 PermissionAsked no log (mutado não entra)"
        );
        assert_eq!(asked[0].payload["prompt_kind"], "choice");
        assert_eq!(asked[0].payload["node_id"], node.to_string());
        assert!(
            asked[0].payload["vt_snapshot_hash"].is_string(),
            "Captura 1 persistida"
        );

        // Lifecycle: o nó saiu do 'Idle' mentiroso para Blocked, com evento auditado.
        assert_eq!(
            sup.get(sup_node).map(|i| i.status),
            Some(crate::NodeStatus::Blocked),
            "nó parado numa pergunta não pode dizer Idle"
        );
        assert!(
            recs.iter().any(|r| r.kind == "NodeStatusChanged"
                && r.payload["status"] == "Blocked"
                && r.payload["reason"] == BLOCKED_REASON_PROMPT),
            "transição auditada no log"
        );

        // Scans seguintes não re-emitem (pedido pendente armado).
        let again = watch
            .scan(&host, &mut store, None, crate::now_ms())
            .expect("scan 2");
        assert!(again.asks.is_empty(), "mesma caixa pendente não duplica");
        assert!(
            again.cleared.is_empty(),
            "caixa ainda na tela: nada cleared"
        );

        host.kill(node).ok();
        host.kill(muted_node).ok();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fecho R2b (caminho real): o humano responde a caixa DIRETO no terminal ⇒ o
    /// prompt some do grid ⇒ o próximo scan apenda `PermissionPromptCleared` no
    /// EventStore com o `stable_id` original. (Só a EMISSÃO — o fold de remoção na
    /// fila é do Arquiteto, provado lá.)
    #[cfg(unix)] // F1-6-8: spawna `sh -c` em PTY real — runtime-Unix (tabela ci-3so-triagem.md).
    #[test]
    fn watch_emits_prompt_cleared_when_answered_in_terminal() {
        use std::time::{Duration, Instant};

        fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
            let start = Instant::now();
            loop {
                if cond() {
                    return true;
                }
                if start.elapsed() >= timeout {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        const T: Duration = Duration::from_secs(5);
        // A resposta LIMPA a tela (como a TUI real, que redesenha sem a caixa).
        let script = "printf 'Qual op\u{e7}\u{e3}o?\\n  1. A\\n  2. B\\nEnter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel '; read x; printf '\\033[2J\\033[Hrespondido: %s\\n' \"$x\"; sleep 60";

        let mut host = crate::PtyHost::new();
        let node = host
            .spawn(crate::PtyCommand::new("sh").arg("-c").arg(script), 100, 24)
            .expect("spawn");
        let dir = std::env::temp_dir().join(format!("lina-r2b-clr-{}", uuid::Uuid::now_v7()));
        let mut store = EventStore::open(&dir).expect("store");
        let mut watch = PermissionWatch::new();

        assert!(
            poll_until(T, || host
                .with_grid(node, |vt| vt.last_nonempty_line())
                .is_some_and(|l| l.contains("Enter to select"))),
            "caixa deveria aparecer"
        );
        // Detecta o pedido (bombeia até o idle de output vencer).
        let mut asked = Vec::new();
        assert!(
            poll_until(T, || {
                let out = watch
                    .scan(&host, &mut store, None, crate::now_ms())
                    .expect("scan");
                asked.extend(out.asks);
                !asked.is_empty()
            }),
            "pedido deveria ser emitido"
        );
        let stable_id = asked[0].stable_id.clone();

        // O humano vai ao terminal e responde — a caixa some (tela limpa).
        host.write(node, b"1\r").expect("resposta humana");
        assert!(
            poll_until(T, || host
                .with_grid(node, |vt| vt.last_nonempty_line())
                .is_some_and(|l| l.contains("respondido: 1"))),
            "a resposta deveria limpar a caixa"
        );

        // O próximo scan observa o sumiço e EMITE o cleared no log.
        let mut cleared = Vec::new();
        assert!(
            poll_until(T, || {
                let out = watch
                    .scan(&host, &mut store, None, crate::now_ms())
                    .expect("scan pós-resposta");
                cleared.extend(out.cleared);
                !cleared.is_empty()
            }),
            "o sumiço da caixa deveria emitir o cleared"
        );
        assert_eq!(cleared[0].stable_id, stable_id, "o id do pedido original");
        assert_eq!(cleared[0].kind, PromptKind::Choice);

        // Persistido no log do workspace, payload campo a campo.
        let recs = store.events().expect("events");
        let ev = recs
            .iter()
            .find(|r| r.kind == "PermissionPromptCleared")
            .expect("PermissionPromptCleared no log");
        assert_eq!(ev.payload["stable_id"], stable_id);
        assert_eq!(ev.payload["node_id"], node.to_string());

        host.kill(node).ok();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
