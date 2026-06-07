//! **F1-1-5 · Dashboard vivo (P6 §Atividade) — LÓGICA pura do card por terminal.**
//!
//! Módulo **gpui-free, puro e testável** (padrão sedimentado de `inspector.rs`/`canvas.rs`/
//! `gallery.rs`): aqui mora a LÓGICA do dashboard; o render gpui consome no shell.
//!
//! ## O que monta (wireframe AUTORITATIVO: ux-flows fluxo (c), P6)
//! Por terminal: **1) estado (palavra+cor) → 2) custo de hoje (dinheiro) → 3) atividade** —
//! hierarquia fixa do fluxo (c). Badge do motor (CLI detectado, F1-1-1) sempre visível;
//! tokens são jargão e vivem nos "Detalhes técnicos" (divulgação progressiva).
//!
//! ## Vocabulário CANÔNICO de estado (fluxo (c) — fonte única; canvas/P6/Fila consomem daqui)
//! `Trabalhando`(verde) · `Esperando você`(âmbar) · `Ocioso`(cinza) · `Sem resposta`
//! (vermelho-suave) · `Dormindo`(azul-escuro). Extensão registrada: **`Encerrado`** para
//! `Dead` (o fluxo (c) cobre agentes vivos; honestidade > esconder o morto). `Dormindo`
//! existe no enum mas **sem produtor na v1** (teto/pausa — B1/F1-5 futuro). `Starting`/
//! `Running` (pré-lifecycle/legado) mapeiam para `Ocioso` (vivo, sem trabalho VISÍVEL —
//! decisão registrada; o lifecycle F1-0-3 canoniza Ready/Idle/Busy/Blocked/Dead).
//!
//! ## Honestidade de custo (nota da rodada: JSONL subconta até ~75% — pesquisa 13.5)
//! TODO custo exibido leva o marcador **"~" + "(estimado)"** quando a fonte é JSONL
//! ([`Session::cost_estimated`]) — é CRITÉRIO, não polish. O custo vem do `costUSD` da
//! linha (formato antigo) ou DERIVADO de `usage`×preço (`lina_session_watch::pricing` —
//! o formato atual não grava `costUSD`). Sem custo apurável (sem sessão, ambíguo ou
//! modelo fora da tabela de preço) → **"sem estimativa ainda"**, nunca "US$ 0,00"
//! (zero seria mentira). Fonte distinta do teto (`cost.rs` super-conta por bytes de
//! propósito — anti-runaway); este módulo NUNCA alimenta o teto.
//!
//! ## Sessão↔nó sem chute (doutrina da camada 3 — 13.9)
//! Match por `cwd` exato contra os hints do app (que CONHECE o cwd de cada spawn);
//! 2+ nós no mesmo cwd → ambíguo → card SEM números (honesto). "Hoje" é decidido por
//! prefixo de data INJETADO (função pura — lição wall-clock do projeto).
//!
//! ## A11y (critério 5)
//! Cada card expõe `a11y_label` com OS VALORES por extenso (leitor de tela lê estado,
//! motor, custo e atividade). Render: `ElementId` ÚNICO por linha em loops (lição
//! text!-colide-id) e `line_height` explícita — asserções aqui são da LÓGICA (a label),
//! não do TreeUpdate.

// Módulo-biblioteca aguardando o WIRING do render gpui (mesmo padrão de `inspector.rs`:
// API consumida pelo render a cabear; por ora exercida pelos testes). Remover ao wirar.
#![allow(dead_code)]

use std::collections::BTreeMap;

use lina_core::{NodeId, ProjectedNode, ProjectedState};
use lina_session_watch::{CostSource, Session};

use crate::inspector::Activity;

/// Estado do agente no VOCABULÁRIO CANÔNICO do fluxo (c) — palavra + cor, sempre as
/// mesmas nos 3 lugares (nó do canvas, P6, Fila).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiState {
    /// Processo ativo, output fluindo (verde).
    Trabalhando,
    /// Bloqueado em permissão/custódia — descreve o que FAZER, não o que temer (âmbar).
    EsperandoVoce,
    /// Vivo, sem tarefa (cinza).
    Ocioso,
    /// Hang REAL (`NodeStalled` do heartbeat F1-0-3) — reservado a ele (vermelho-suave).
    SemResposta,
    /// Pausado pelo usuário/teto (azul-escuro). SEM produtor na v1 (B1 futuro).
    Dormindo,
    /// `Dead` — sessão do processo encerrada (extensão registrada ao fluxo (c)).
    Encerrado,
}

impl UiState {
    /// Deriva o estado canônico da projeção (`status` de lifecycle + flag `stalled`).
    /// `stalled` vence (é o hang real); `Starting`/`Running`/sem-status → `Ocioso`
    /// (vivo, sem trabalho visível — decisão registrada no doc do módulo).
    #[must_use]
    pub fn from_projection(status: Option<&str>, stalled: bool) -> Self {
        if stalled {
            return UiState::SemResposta;
        }
        match status {
            Some("Busy") => UiState::Trabalhando,
            Some("Blocked") => UiState::EsperandoVoce,
            Some("Dead") => UiState::Encerrado,
            _ => UiState::Ocioso,
        }
    }

    /// Palavra canônica (PT-BR leigo, inv#6).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            UiState::Trabalhando => "Trabalhando",
            UiState::EsperandoVoce => "Esperando você",
            UiState::Ocioso => "Ocioso",
            UiState::SemResposta => "Sem resposta",
            UiState::Dormindo => "Dormindo",
            UiState::Encerrado => "Encerrado",
        }
    }

    /// Token SEMÂNTICO de cor (o `theme.rs` resolve o pixel — nada de cor hardcoded aqui).
    #[must_use]
    pub fn color_token(self) -> &'static str {
        match self {
            UiState::Trabalhando => "verde",
            UiState::EsperandoVoce => "ambar",
            UiState::Ocioso => "cinza",
            UiState::SemResposta => "vermelho-suave",
            UiState::Dormindo => "azul-escuro",
            UiState::Encerrado => "cinza-escuro",
        }
    }
}

/// Badge do motor (CLI) — governança sempre visível, nunca escondida (fluxo (c)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineBadge {
    /// Rótulo humanizado MECANICAMENTE (kebab→Título: `claude-code`→`Claude Code`) —
    /// transformação genérica, nunca conhecimento de CLI no app (inv#3).
    pub label: String,
    /// `true` quando o `cli` veio da projeção (`CliProfileSet` do spawn — camada 1,
    /// a única determinística; 13.9). `false` = fallback honesto.
    pub via_spawn: bool,
}

impl EngineBadge {
    pub fn from_cli(cli: Option<&str>) -> Self {
        match cli {
            Some(id) if !id.trim().is_empty() => Self {
                label: humanize_kebab(id),
                via_spawn: true,
            },
            _ => Self {
                label: "Terminal (motor desconhecido)".to_owned(),
                via_spawn: false,
            },
        }
    }
}

/// `claude-code` → `Claude Code` (capitaliza cada parte; mecânico, sem dicionário).
fn humanize_kebab(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Linha de custo de HOJE — o `~`/`(estimado)` é CRITÉRIO (JSONL subconta até ~75%).
#[derive(Debug, Clone, PartialEq)]
pub struct CostLine {
    /// Texto pronto da superfície (ex.: `~US$ 0,01 (estimado)` ou o fallback honesto).
    pub text: String,
    /// Valor CRU em USD (precisão dos totais — o texto arredonda só na exibição).
    pub usd: f64,
    /// `true` quando a fonte é estimativa (JSONL) — obriga o marcador no texto.
    pub estimated: bool,
    /// `false` = sem número exibível (sem sessão, ambíguo, sem `costUSD` ou fora de hoje).
    pub has_data: bool,
}

impl CostLine {
    fn without_data() -> Self {
        Self {
            text: "sem estimativa de custo ainda".to_owned(),
            usd: 0.0,
            estimated: false,
            has_data: false,
        }
    }

    fn from_today(cost_usd: f64, estimated: bool) -> Self {
        if cost_usd <= 0.0 {
            return Self::without_data(); // zero exibido seria mentira (subcontagem)
        }
        let valor = format!("{cost_usd:.2}").replace('.', ",");
        let text = if estimated {
            format!("~US$ {valor} (estimado)")
        } else {
            format!("US$ {valor}")
        };
        Self {
            text,
            usd: cost_usd,
            estimated,
            has_data: true,
        }
    }
}

/// Tooltip do ⓘ de custo (1 frase, wireframe P6) — constante p/ o render.
pub const COST_TOOLTIP: &str = "calculado pelo uso de IA deste Agente × preço do motor. \
O valor exato aparece na fatura do seu provedor.";

// ═══════════ Timeline viva (wiring F1-1-3→F1-1-5): hooks → atividade do card ═══════════

/// Janela (ms) em que um hook de trabalho recente conta como "Trabalhando" mesmo sem
/// evento de fim — depois disso a timeline se declara sem opinião (cai no fallback).
pub const ACTIVITY_FRESH_MS: u64 = 10_000;

/// Última palavra dos hooks por terminal (chave = NOME do nó — é o `node_id` atribuído
/// pelo TOKEN de spawn; identidade A2A única do roster). Alimentada pela drenagem do
/// `HookListener` (bridge), lida pelo render. Telemetria NÃO-decisória (13.5 achado 8).
#[derive(Debug, Default)]
pub struct Timeline {
    last: BTreeMap<String, lina_hooks::HookEvent>,
}

impl Timeline {
    /// Absorve um evento normalizado do listener (último por nó vence).
    pub fn note(&mut self, ev: lina_hooks::HookEvent) {
        self.last.insert(ev.node_id.clone(), ev);
    }

    /// Atividade derivada do último hook do nó — `now_ms` é INJETADO (lição wall-clock):
    /// - `Stop` → `Idle` ("terminou a resposta"), sem janela (o fim é explícito);
    /// - hook de trabalho há menos de [`ACTIVITY_FRESH_MS`] → `Busy` + a ferramenta;
    /// - sem hook / hook velho → `None` (o caller cai na fonte anterior: damage/Idle).
    #[must_use]
    pub fn activity(&self, name: &str, now_ms: u64) -> Option<(Activity, Option<String>)> {
        let ev = self.last.get(name)?;
        if ev.kind == lina_hooks::HookKind::Stop {
            return Some((Activity::Idle, None));
        }
        if now_ms.saturating_sub(ev.ts) <= ACTIVITY_FRESH_MS {
            return Some((Activity::Busy, ev.tool_name.clone()));
        }
        None
    }

    /// Chegada (`ts` do listener) do último hook do nó — base da medição de latência
    /// evento→card (`[DASH]`).
    #[must_use]
    pub fn last_ts(&self, name: &str) -> Option<u64> {
        self.last.get(name).map(|e| e.ts)
    }
}

/// Fios do dashboard que o BOOT liga e o root consome (1 parâmetro no construtor).
pub struct DashWiring {
    /// Última palavra dos hooks por nó (alimentada por `HooksShared::spawn_drain`).
    pub timeline: std::sync::Arc<std::sync::Mutex<Timeline>>,
    /// Snapshot das sessões da camada 3 (alimentado pela thread do `SessionWatch`).
    pub sessions: std::sync::Arc<std::sync::Mutex<Vec<Session>>>,
    /// `id` do perfil de injeção (badge do motor — v1: todos os terminais usam o mesmo).
    pub cli_id: String,
    /// Raiz do workspace (string) — recorte honesto do custo do Espaço.
    pub ws_root: String,
}

impl DashWiring {
    #[must_use]
    pub fn new(cli_id: String, ws_root: String) -> Self {
        Self {
            timeline: std::sync::Arc::default(),
            sessions: std::sync::Arc::default(),
            cli_id,
            ws_root,
        }
    }
}

/// Prefixo `YYYY-MM-DD` (UTC) do instante `epoch_ms` — algoritmo civil determinístico,
/// sem dependência de data. **Limitação registrada (v1):** o "hoje" é o dia UTC (desvio
/// de até 3h vs Brasília); timezone local é melhoria futura — preferimos o recorte
/// honesto e testável a uma dependência nova nesta fiação.
#[must_use]
pub fn utc_day_prefix(epoch_ms: u64) -> String {
    let days = (epoch_ms / 86_400_000) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `days` desde 1970-01-01 → (ano, mês, dia) no calendário civil (Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Custo TOTAL do Espaço hoje (sessões cujo `cwd` vive sob o `ws_root` e cujo último
/// movimento é de hoje) — v1 da tela: o total é atribuível com certeza ao Espaço; a
/// atribuição POR NÓ continua exigindo hint não-ambíguo (sem chute — doutrina 13.9).
#[must_use]
pub fn workspace_cost_today(sessions: &[Session], ws_root: &str, today_prefix: &str) -> CostLine {
    let mut cost = 0.0f64;
    let mut estimated = false;
    let mut any = false;
    for s in sessions {
        let in_ws = s.cwd.as_deref().is_some_and(|c| c.starts_with(ws_root));
        let today = s
            .last_ts
            .as_deref()
            .is_some_and(|ts| ts.starts_with(today_prefix));
        if in_ws && today {
            any = true;
            cost += s.cost_usd;
            estimated |= s.cost_estimated || s.source == CostSource::Jsonl;
        }
    }
    if !any {
        return CostLine::without_data();
    }
    CostLine::from_today(cost, estimated)
}

/// Breakdown de tokens — mora nos "Detalhes técnicos" (dobrado; jargão fora da superfície).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenBreakdown {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache: u64,
    pub tokens_thinking: u64,
    /// 13.5 item 5: thinking > output → warning (subestimação real + opacidade do thinking).
    pub thinking_warning: bool,
}

/// Card de UM terminal — a LÓGICA do P6 §Atividade que o render consome.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentCard {
    pub node: NodeId,
    pub name: String,
    pub engine: EngineBadge,
    pub state: UiState,
    pub activity: Activity,
    pub cost_today: CostLine,
    pub tokens: TokenBreakdown,
    /// Frase completa p/ o leitor de tela — contém OS VALORES (critério 5).
    pub a11y_label: String,
}

/// Monta os cards POR TERMINAL a partir das projeções (inv#4: tudo deriva do log).
///
/// - `hints`: `(node, cwd)` dos spawns — o app CONHECE o cwd; match por cwd EXATO,
///   2+ nós no mesmo cwd = ambíguo → sem números (doutrina camada 3: nunca chutar).
/// - `activities`: atividade por nó derivada do damage (ausente → `Idle`).
/// - `today_prefix`: `YYYY-MM-DD` INJETADO (função pura; lição wall-clock).
#[must_use]
pub fn build_cards(
    state: &ProjectedState,
    sessions: &[Session],
    hints: &[(NodeId, String)],
    activities: &BTreeMap<NodeId, Activity>,
    today_prefix: &str,
) -> Vec<AgentCard> {
    state
        .nodes
        .iter()
        .filter(|(_, pn)| pn.kind == "Terminal")
        .map(|(node, pn)| build_card(*node, pn, sessions, hints, activities, today_prefix))
        .collect()
}

fn build_card(
    node: NodeId,
    pn: &ProjectedNode,
    sessions: &[Session],
    hints: &[(NodeId, String)],
    activities: &BTreeMap<NodeId, Activity>,
    today_prefix: &str,
) -> AgentCard {
    let name = pn.name.clone().unwrap_or_else(|| "Terminal".to_owned());
    let engine = EngineBadge::from_cli(pn.cli.as_deref());
    let state = UiState::from_projection(pn.status.as_deref(), pn.stalled);
    let activity = activities.get(&node).copied().unwrap_or(Activity::Idle);

    // Sessões do nó: cwd do hint, único entre os nós (ambíguo → sem números).
    let my_cwd = hints.iter().find(|(n, _)| *n == node).map(|(_, c)| c);
    let unambiguous =
        my_cwd.is_some_and(|cwd| hints.iter().filter(|(n, c)| c == cwd && *n != node).count() == 0);
    let (cost_today, tokens) = match (my_cwd, unambiguous) {
        (Some(cwd), true) => aggregate_today(sessions, cwd, today_prefix),
        _ => (CostLine::without_data(), TokenBreakdown::default()),
    };

    let mut a11y_label = format!(
        "{name}: {}. Motor: {}. Custo de hoje: {}. Atividade: {}.",
        state.label(),
        engine.label,
        cost_today.text,
        activity.label()
    );
    if tokens.thinking_warning {
        a11y_label.push_str(" Atenção: gastando mais pensando do que produzindo.");
    }

    AgentCard {
        node,
        name,
        engine,
        state,
        activity,
        cost_today,
        tokens,
        a11y_label,
    }
}

/// Agrega as sessões DE HOJE do `cwd` (um nó pode ter várias sessões num dia — soma).
fn aggregate_today(
    sessions: &[Session],
    cwd: &str,
    today_prefix: &str,
) -> (CostLine, TokenBreakdown) {
    let mut cost = 0.0f64;
    let mut estimated = false;
    let mut tk = TokenBreakdown::default();
    let mut matched = false;
    for s in sessions {
        let same_cwd = s.cwd.as_deref() == Some(cwd);
        let today = s
            .last_ts
            .as_deref()
            .is_some_and(|ts| ts.starts_with(today_prefix));
        if !(same_cwd && today) {
            continue;
        }
        matched = true;
        cost += s.cost_usd;
        estimated |= s.cost_estimated || s.source == CostSource::Jsonl;
        tk.tokens_in += s.tokens_in;
        tk.tokens_out += s.tokens_out;
        tk.tokens_cache += s.tokens_cache;
        tk.tokens_thinking += s.tokens_thinking;
    }
    tk.thinking_warning = tk.tokens_thinking > tk.tokens_out;
    if !matched {
        return (CostLine::without_data(), TokenBreakdown::default());
    }
    (CostLine::from_today(cost, estimated), tk)
}

// ═══════════ Clamp do painel aos bounds da janela — LÓGICA pura (gpui não roda headless) ═══════════

/// Largura preferida do painel "Atividade e custos" (P6 — encolhe antes de vazar).
pub const PANEL_PREFERRED_W: f32 = 340.0;
/// Inset do topo (sob a topbar) e da base (sobre o footer) — os mesmos px da fiação.
pub const PANEL_TOP_INSET: f32 = 44.0;
pub const PANEL_BOTTOM_INSET: f32 = 28.0;

/// Retângulo final do painel DENTRO da janela (origem no canto sup. esquerdo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// **Geometria clampada do painel** — função PURA que o render gpui consome (bug da
/// rodada: o painel fixava `w=340 / top=44 / bottom=28` sem olhar a janela e VAZAVA a
/// borda direita no app do fundador). Invariantes (testadas): `x ≥ 0`, `x+w ≤ win_w`,
/// `y ≥ 0`, `y+h ≤ win_h` — em QUALQUER tamanho de janela, inclusive degenerado (0×0).
/// Altura do conteúdo > `h` é problema do scroll interno (render), não desta função.
#[must_use]
pub fn dashboard_panel_rect(win_w: f32, win_h: f32) -> PanelRect {
    let win_w = win_w.max(0.0);
    let win_h = win_h.max(0.0);
    // Largura: a preferida, encolhida ao vão da janela; ancorado à direita sem vazar.
    let w = PANEL_PREFERRED_W.min(win_w);
    let x = win_w - w;
    // Altura: entre os insets; janela baixa demais → encolhe até 0 (nunca negativo).
    let y = PANEL_TOP_INSET.min(win_h);
    let bottom = (win_h - PANEL_BOTTOM_INSET).clamp(y, win_h);
    let h = bottom - y;
    PanelRect { x, y, w, h }
}

/// Totais do Espaço (base do B1 — Medidor): agrega os cards de hoje.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceTotals {
    pub cards: usize,
    pub cost_today_usd: f64,
    pub any_estimated: bool,
    pub thinking_warnings: usize,
    /// Texto pronto da barra (ex.: `Hoje: ~US$ 0,04 (estimado) · 2 agentes`).
    pub summary_text: String,
}

/// Agrega os cards no total do Espaço (o `~estimado` propaga se QUALQUER fonte estima).
#[must_use]
pub fn totals(cards: &[AgentCard]) -> WorkspaceTotals {
    let cost: f64 = cards
        .iter()
        .filter(|c| c.cost_today.has_data)
        .map(|c| c.cost_today.usd)
        .sum();
    let any_estimated = cards
        .iter()
        .any(|c| c.cost_today.has_data && c.cost_today.estimated);
    let thinking_warnings = cards.iter().filter(|c| c.tokens.thinking_warning).count();
    let valor = format!("{cost:.2}").replace('.', ",");
    let marker = if any_estimated { "~" } else { "" };
    let sufixo = if any_estimated { " (estimado)" } else { "" };
    let summary_text = format!(
        "Hoje: {marker}US$ {valor}{sufixo} · {} agentes",
        cards.len()
    );
    WorkspaceTotals {
        cards: cards.len(),
        cost_today_usd: cost,
        any_estimated,
        thinking_warnings,
        summary_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_core::{apply, DomainEvent};
    use uuid::Uuid;

    fn term_node(state: &mut ProjectedState, cli: Option<&str>) -> NodeId {
        let node = Uuid::now_v7();
        apply(
            state,
            &DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
            },
        );
        apply(
            state,
            &DomainEvent::NodeRenamed {
                node,
                name: "Ajudante Dev".into(),
            },
        );
        if let Some(cli) = cli {
            apply(
                state,
                &DomainEvent::CliProfileSet {
                    node,
                    profile: cli.into(),
                },
            );
        }
        node
    }

    fn status(state: &mut ProjectedState, node: NodeId, to: &str) {
        apply(
            state,
            &DomainEvent::NodeStatusChanged {
                node,
                status: to.into(),
                from: String::new(),
                reason: String::new(),
            },
        );
    }

    fn session(cwd: &str, ts: &str) -> Session {
        Session {
            cli: "claude-code".into(),
            session_id: "sess-1".into(),
            tokens_in: 155,
            tokens_out: 105,
            tokens_cache: 40,
            tokens_thinking: 8,
            cost_usd: 0.0123,
            cost_estimated: true,
            model: Some("claude-opus-4-8".into()),
            cwd: Some(cwd.into()),
            last_ts: Some(ts.into()),
            subagents: vec![],
            tools: vec!["Bash".into()],
            source: lina_session_watch::CostSource::Jsonl,
        }
    }

    fn busy(node: NodeId) -> BTreeMap<NodeId, Activity> {
        BTreeMap::from([(node, Activity::Busy)])
    }

    const TODAY: &str = "2026-06-06";

    /// Vocabulário canônico do fluxo (c): tabela COMPLETA lifecycle → palavra+cor.
    #[test]
    fn state_vocabulary_maps_lifecycle_to_canonical_words() {
        let cases = [
            (Some("Busy"), false, "Trabalhando", "verde"),
            (Some("Blocked"), false, "Esperando você", "ambar"),
            (Some("Ready"), false, "Ocioso", "cinza"),
            (Some("Idle"), false, "Ocioso", "cinza"),
            // Hang real (NodeStalled): vermelho-suave, reservado ao stall.
            (Some("Busy"), true, "Sem resposta", "vermelho-suave"),
            (Some("Dead"), false, "Encerrado", "cinza-escuro"),
            // Pré-lifecycle/legado → vivo sem trabalho visível.
            (Some("Starting"), false, "Ocioso", "cinza"),
            (Some("Running"), false, "Ocioso", "cinza"),
        ];
        for (st, stalled, word, color) in cases {
            let ui = UiState::from_projection(st, stalled);
            assert_eq!(ui.label(), word, "status {st:?} stalled={stalled}");
            assert_eq!(ui.color_token(), color, "cor de {word}");
        }
        // Dormindo: no vocabulário (teto/pausa B1 futuro) — sem produtor v1.
        assert_eq!(UiState::Dormindo.label(), "Dormindo");
        assert_eq!(UiState::Dormindo.color_token(), "azul-escuro");
    }

    /// Critério 2 da story: log sintético → projeção → card com EXATAMENTE estes números.
    #[test]
    fn cards_from_synthetic_log_field_by_field() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"));
        status(&mut state, node, "Busy");

        let sessions = [session("/work/projeto-a", "2026-06-06T14:02:00.000Z")];
        let hints = [(node, "/work/projeto-a".to_string())];
        let cards = build_cards(&state, &sessions, &hints, &busy(node), TODAY);

        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(c.name, "Ajudante Dev");
        // Badge do motor: humanização MECÂNICA (kebab→Título), nunca conhecimento de CLI.
        assert_eq!(c.engine.label, "Claude Code");
        assert!(
            c.engine.via_spawn,
            "cli da projeção = detecção via spawn (camada 1)"
        );
        assert_eq!(c.state.label(), "Trabalhando");
        // Custo de HOJE com o marcador honesto (critério 4) — SEMPRE que source=jsonl.
        assert!(c.cost_today.has_data);
        assert!(c.cost_today.estimated);
        assert_eq!(c.cost_today.text, "~US$ 0,01 (estimado)");
        // Detalhes técnicos: breakdown campo a campo (13.5 item 5).
        assert_eq!(c.tokens.tokens_in, 155);
        assert_eq!(c.tokens.tokens_out, 105);
        assert_eq!(c.tokens.tokens_cache, 40);
        assert_eq!(c.tokens.tokens_thinking, 8);
        assert!(!c.tokens.thinking_warning, "8 thinking < 105 output");
        // A11y: o leitor de tela lê OS VALORES.
        for needle in [
            "Ajudante Dev",
            "Trabalhando",
            "Claude Code",
            "0,01",
            "estimado",
        ] {
            assert!(
                c.a11y_label.contains(needle),
                "a11y_label deve conter '{needle}': {}",
                c.a11y_label
            );
        }
    }

    /// Critério 3: thinking > output → warning presente na projeção do card.
    #[test]
    fn thinking_warning_when_thinking_dominates() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"));
        status(&mut state, node, "Busy");
        let mut s = session("/w", "2026-06-06T10:00:00.000Z");
        s.tokens_thinking = 900;
        s.tokens_out = 100;
        let hints = [(node, "/w".to_string())];
        let cards = build_cards(&state, &[s], &hints, &busy(node), TODAY);
        assert!(cards[0].tokens.thinking_warning);
        assert!(
            cards[0].a11y_label.contains("pensando"),
            "aviso leigo no a11y: {}",
            cards[0].a11y_label
        );
    }

    /// Honestidade: sessão sem custo apurável (ex.: modelo fora da tabela de preço)
    /// → "sem estimativa ainda", NUNCA "US$ 0,00".
    #[test]
    fn cost_without_data_is_honest() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"));
        status(&mut state, node, "Idle");
        let mut s = session("/w", "2026-06-06T10:00:00.000Z");
        s.cost_usd = 0.0;
        let hints = [(node, "/w".to_string())];
        let cards = build_cards(&state, &[s], &hints, &BTreeMap::new(), TODAY);
        let cost = &cards[0].cost_today;
        assert!(!cost.has_data);
        assert!(
            !cost.text.contains("0,00"),
            "zero seria mentira: {}",
            cost.text
        );
        assert!(cost.text.contains("sem estimativa"));
    }

    /// Sem chute (doutrina camada 3): sem sessão correlacionada OU cwd ambíguo → sem números.
    #[test]
    fn unmatched_or_ambiguous_session_yields_no_numbers() {
        let mut state = ProjectedState::default();
        let a = term_node(&mut state, Some("claude-code"));
        status(&mut state, a, "Busy");

        // Sem sessão nenhuma → fallback honesto do motor + sem custo.
        let cards = build_cards(&state, &[], &[(a, "/w".into())], &BTreeMap::new(), TODAY);
        assert!(!cards[0].cost_today.has_data);

        // 2 nós no MESMO cwd → ambíguo → nenhum dos dois ganha os números.
        let b = term_node(&mut state, Some("claude-code"));
        status(&mut state, b, "Busy");
        let hints = [(a, "/w".to_string()), (b, "/w".to_string())];
        let cards = build_cards(
            &state,
            &[session("/w", "2026-06-06T10:00:00.000Z")],
            &hints,
            &BTreeMap::new(),
            TODAY,
        );
        assert_eq!(cards.len(), 2);
        for c in &cards {
            assert!(!c.cost_today.has_data, "ambíguo nunca chuta números");
        }
    }

    /// "Hoje" é prefixo INJETADO: sessão de ontem não entra no custo de hoje.
    #[test]
    fn today_filter_is_injected_not_wall_clock() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"));
        status(&mut state, node, "Idle");
        let hints = [(node, "/w".to_string())];
        let yesterday = session("/w", "2026-06-05T23:00:00.000Z");
        let cards = build_cards(&state, &[yesterday], &hints, &BTreeMap::new(), TODAY);
        assert!(
            !cards[0].cost_today.has_data,
            "sessão de ontem não é 'hoje'"
        );
    }

    /// Motor desconhecido → fallback honesto (13.9 item 6), nunca chute.
    #[test]
    fn unknown_engine_has_honest_fallback() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, None);
        status(&mut state, node, "Ready");
        let cards = build_cards(&state, &[], &[(node, "/w".into())], &BTreeMap::new(), TODAY);
        assert_eq!(cards[0].engine.label, "Terminal (motor desconhecido)");
        assert!(!cards[0].engine.via_spawn);
    }

    /// Nó-nota não vira card (cards são POR TERMINAL).
    #[test]
    fn note_nodes_do_not_become_cards() {
        let mut state = ProjectedState::default();
        let _t = term_node(&mut state, Some("claude-code"));
        let note = Uuid::now_v7();
        apply(
            &mut state,
            &DomainEvent::NodeAdded {
                node: note,
                kind: "Note".into(),
                x: 1.0,
                y: 1.0,
            },
        );
        let cards = build_cards(&state, &[], &[], &BTreeMap::new(), TODAY);
        assert_eq!(cards.len(), 1, "só o terminal vira card");
    }

    /// Timeline: hook fresco → Busy+tool; Stop → Idle; velho/desconhecido → None.
    #[test]
    fn timeline_derives_activity_from_hooks_with_injected_clock() {
        let mut tl = Timeline::default();
        let ev = |kind, tool: Option<&str>, ts| lina_hooks::HookEvent {
            node_id: "Ajudante Dev".into(),
            kind,
            tool_name: tool.map(str::to_owned),
            subagent_id: None,
            parent_id: None,
            ts,
            trusted: false,
            // Aditivos do seam F1-1-6 — a timeline não os consome (ainda).
            message: None,
            tool_input: None,
        };
        // Sem hook → sem opinião.
        assert_eq!(tl.activity("Ajudante Dev", 1_000), None);
        // PreToolUse fresco → Trabalhando + ferramenta.
        tl.note(ev(lina_hooks::HookKind::PreToolUse, Some("Bash"), 1_000));
        assert_eq!(
            tl.activity("Ajudante Dev", 2_000),
            Some((Activity::Busy, Some("Bash".into())))
        );
        // Velho (> janela) → sem opinião (cai no fallback).
        assert_eq!(
            tl.activity("Ajudante Dev", 1_000 + ACTIVITY_FRESH_MS + 1),
            None
        );
        // Stop → Idle explícito, sem janela.
        tl.note(ev(lina_hooks::HookKind::Stop, None, 5_000));
        assert_eq!(
            tl.activity("Ajudante Dev", 99_000),
            Some((Activity::Idle, None))
        );
    }

    /// Dia UTC determinístico (sem dependência) + recorte do custo do Espaço.
    #[test]
    fn utc_day_prefix_and_workspace_cost_today() {
        // 2026-06-06T14:00:00Z = 1780754400000 ms.
        assert_eq!(utc_day_prefix(1_780_754_400_000), "2026-06-06");
        assert_eq!(utc_day_prefix(0), "1970-01-01");

        let inside = session("/ws/terminal-a", "2026-06-06T10:00:00.000Z");
        let mut outside_ws = session("/outro/lugar", "2026-06-06T10:00:00.000Z");
        outside_ws.session_id = "s2".into();
        let mut yesterday = session("/ws/terminal-b", "2026-06-05T10:00:00.000Z");
        yesterday.session_id = "s3".into();
        let cost = workspace_cost_today(&[inside, outside_ws, yesterday], "/ws", "2026-06-06");
        assert!(cost.has_data);
        assert!(
            (cost.usd - 0.0123).abs() < 1e-9,
            "só a sessão do ws E de hoje"
        );
        assert!(cost.estimated && cost.text.contains("estimado"));
    }

    /// Clamp do painel: janela confortável usa a geometria preferida, exata.
    #[test]
    fn panel_rect_in_comfortable_window_uses_preferred_geometry() {
        let r = dashboard_panel_rect(1200.0, 800.0);
        assert_eq!(
            r,
            PanelRect {
                x: 1200.0 - PANEL_PREFERRED_W,
                y: PANEL_TOP_INSET,
                w: PANEL_PREFERRED_W,
                h: 800.0 - PANEL_TOP_INSET - PANEL_BOTTOM_INSET,
            }
        );
    }

    /// O BUG da rodada: o painel nunca pode vazar a borda direita — janela mais
    /// estreita que a largura preferida ENCOLHE o painel (x trava em 0).
    #[test]
    fn panel_rect_never_leaks_past_right_edge() {
        let r = dashboard_panel_rect(200.0, 800.0);
        assert_eq!(r.w, 200.0, "encolhe ao vão da janela");
        assert_eq!(r.x, 0.0);
        assert!(r.x + r.w <= 200.0, "borda direita respeitada");
    }

    /// Janela baixa demais: altura degrada até 0 — nunca negativa, nunca abaixo do rodapé.
    #[test]
    fn panel_rect_degrades_height_in_short_windows() {
        let r = dashboard_panel_rect(1200.0, 60.0); // só 16px entre topbar e footer
        assert!(r.h >= 0.0);
        assert!(r.y + r.h <= 60.0, "nunca vaza a base");
        let r0 = dashboard_panel_rect(1200.0, 30.0); // nem o inset do topo cabe
        assert!(r0.h >= 0.0 && r0.y + r0.h <= 30.0);
    }

    /// Invariantes em GRADE de tamanhos (inclui 0×0 e negativos defensivos):
    /// o painel SEMPRE cabe na janela — é o contrato que o render gpui consome.
    #[test]
    fn panel_rect_invariants_hold_for_any_window_size() {
        for &w in &[
            0.0, 1.0, 27.0, 44.0, 200.0, 339.0, 340.0, 341.0, 1024.0, 3840.0,
        ] {
            for &h in &[0.0, 1.0, 28.0, 44.0, 72.0, 73.0, 600.0, 2160.0] {
                let r = dashboard_panel_rect(w, h);
                assert!(r.x >= 0.0, "x≥0 em {w}x{h}: {r:?}");
                assert!(r.y >= 0.0, "y≥0 em {w}x{h}: {r:?}");
                assert!(r.w >= 0.0 && r.h >= 0.0, "dimensões ≥0 em {w}x{h}: {r:?}");
                assert!(r.x + r.w <= w + 1e-3, "vaza direita em {w}x{h}: {r:?}");
                assert!(r.y + r.h <= h + 1e-3, "vaza base em {w}x{h}: {r:?}");
                assert!(r.w <= PANEL_PREFERRED_W, "nunca mais largo que o preferido");
            }
        }
        // Entrada negativa defensiva (viewport ainda não medido) → rect nulo, sem NaN.
        let r = dashboard_panel_rect(-10.0, -10.0);
        assert_eq!((r.x, r.y, r.w, r.h), (0.0, 0.0, 0.0, 0.0));
    }

    /// Totais do workspace (base do B1): soma custo de hoje + conta warnings.
    #[test]
    fn workspace_totals_aggregate() {
        let mut state = ProjectedState::default();
        let a = term_node(&mut state, Some("claude-code"));
        status(&mut state, a, "Busy");
        let b = term_node(&mut state, Some("codex"));
        status(&mut state, b, "Idle");

        let mut s_b = session("/wb", "2026-06-06T11:00:00.000Z");
        s_b.session_id = "sess-2".into();
        s_b.cost_usd = 0.0277;
        s_b.tokens_thinking = 500;
        s_b.tokens_out = 10;
        let sessions = [session("/wa", "2026-06-06T10:00:00.000Z"), s_b];
        let hints = [(a, "/wa".to_string()), (b, "/wb".to_string())];
        let cards = build_cards(&state, &sessions, &hints, &BTreeMap::new(), TODAY);

        let t = totals(&cards);
        assert_eq!(t.cards, 2);
        assert!((t.cost_today_usd - 0.04).abs() < 1e-9, "0.0123+0.0277");
        assert!(t.any_estimated, "agregado herda o marcador de estimativa");
        assert_eq!(t.thinking_warnings, 1);
        assert!(t.summary_text.contains("~US$ 0,04"));
        assert!(t.summary_text.contains("estimado"));
    }
}
