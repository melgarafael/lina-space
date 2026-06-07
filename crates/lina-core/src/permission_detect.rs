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

use crate::events::{fnv1a, DomainEvent, PermissionEvidence};

/// Janela máxima (ms) entre o `PreToolUse` pendente e a `Notification` para a
/// correlação valer. No fluxo real os dois chegam em sub-segundo; 15s cobre listener
/// sob carga sem deixar uma pendência VELHA promover uma notification qualquer.
pub const CORRELATION_WINDOW_MS: u64 = 15_000;
/// Quantas linhas não-vazias do FUNDO do viewport o fallback varre (prompt de
/// permissão mora na cauda; diálogos com opções abaixo da pergunta cabem na janela).
pub const GRID_SCAN_ROWS: usize = 6;

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
    /// Pedidos emitidos pelo fallback de grid.
    pub emitted_grid: u64,
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
    /// Total de pedidos emitidos (todas as camadas).
    #[must_use]
    pub fn emitted_total(&self) -> u64 {
        self.emitted_hook + self.emitted_grid
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

/// Bookkeeping efêmero por nó (NUNCA persistido — só o veredito vira evento, mesma
/// disciplina do heartbeat ADR 0019 §5).
#[derive(Debug, Default)]
struct NodeTrack {
    pending: Option<PendingTool>,
    call_seq: u64,
    /// Última pendência JÁ emitida — `Notification` repetida do mesmo pedido não
    /// re-emite (anti-amplificação, ADR 0003/0005).
    last_emitted_call: Option<u64>,
    /// `(hash, row)` da linha candidata JÁ emitida e ainda na tela — re-arma quando o
    /// prompt some OU muda de conteúdo OU aparece em OUTRA linha do viewport (pedido
    /// NOVO de texto idêntico: a tela rolou, o prompt novo nasce noutra row; o
    /// pendente parado fica na mesma — calibração da rodada reduzida, ver relatório).
    armed_grid: Option<(u64, usize)>,
}

/// O detector multi-camada. Um por workspace; estado por nó.
#[derive(Debug, Default)]
pub struct PermissionDetector {
    nodes: HashMap<String, NodeTrack>,
    telemetry: DetectionTelemetry,
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
                Some(PermissionAsk {
                    node_id: node_id.to_string(),
                    tool: pending.tool.clone(),
                    detail: pending.detail.clone(),
                    evidence: PermissionEvidence::Hook,
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
    /// candidato; re-arma quando o prompt some ou muda.
    pub fn observe_grid(
        &mut self,
        node_id: &str,
        vt: &dyn VtBackend,
        idle: bool,
        now_ms: u64,
    ) -> Option<PermissionAsk> {
        let candidate = scan_grid(vt);
        let track = self.nodes.entry(node_id.to_string()).or_default();
        let Some((line, row)) = candidate else {
            track.armed_grid = None; // prompt sumiu (respondido/rolou) → re-arma
            return None;
        };
        let line_hash = fnv1a(line.as_bytes());
        if !idle {
            self.telemetry.grid_suppressed_busy += 1;
            return None;
        }
        if track.armed_grid == Some((line_hash, row)) {
            return None; // mesmo prompt na MESMA linha — pedido já emitido, pendente
        }
        track.armed_grid = Some((line_hash, row));
        self.telemetry.emitted_grid += 1;
        Some(PermissionAsk {
            node_id: node_id.to_string(),
            tool: None,
            detail: Some(line),
            evidence: PermissionEvidence::Grid,
            stable_id: stable_id(node_id, &format!("grid:{line_hash:016x}:{row}"), now_ms),
            ts: now_ms,
        })
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

    /// O `to_event` produz o `DomainEvent::PermissionAsked` campo a campo.
    #[test]
    fn permission_ask_converts_to_domain_event() {
        let ask = PermissionAsk {
            node_id: "n1".into(),
            tool: Some("Bash".into()),
            detail: Some("git push".into()),
            evidence: PermissionEvidence::Hook,
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
            } => {
                assert_eq!(node_id, "n1");
                assert_eq!(tool.as_deref(), Some("Bash"));
                assert_eq!(detail.as_deref(), Some("git push"));
                assert_eq!(evidence, PermissionEvidence::Hook);
                assert_eq!(stable_id, "abc");
            }
            other => panic!("evento errado: {other:?}"),
        }
    }
}
