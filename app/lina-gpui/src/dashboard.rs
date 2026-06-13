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
//! ## Sessão↔nó sem chute (doutrina da camada 3 — 13.9 · rodada 360, passo 7)
//! Match por `cwd` exato contra o binding node↔cwd PERSISTIDO no log (`ProjectedNode.cwd`
//! ← `TerminalSpawned.cwd`, ADR 0022 §3) — a era dos "hints" paralelos acabou: o log é a
//! fonte. Estados HONESTOS da linha de custo ([`CostAttribution`]): sem `cwd` no log
//! (histórico) → "—" não rastreável; `cwd` fora do `ws_root` → custo REAL rotulado
//! "trabalhando fora da pasta do Espaço"; 2+ nós no MESMO cwd → total exibido como
//! COMPARTILHADO (jamais dividido arbitrariamente); sem sessão de hoje → "sem sessão
//! hoje" (≠ não rastreável). "Hoje" é decidido por prefixo de data INJETADO (função
//! pura — lição wall-clock do projeto).
//!
//! ## A11y (critério 5)
//! Cada card expõe `a11y_label` com OS VALORES por extenso (leitor de tela lê estado,
//! motor, custo e atividade). Render: `ElementId` ÚNICO por linha em loops (lição
//! text!-colide-id) e `line_height` explícita — asserções aqui são da LÓGICA (a label),
//! não do TreeUpdate.

// Rodada 360 (passo 7): o caminho de CUSTO (build_cards/adopted_cwds/workspace_cost_today/
// cache de projeção) está WIRADO no render_dashboard. O allow permanece pela superfície
// restante do card (estado/atividade/a11y do `AgentCard`, `totals` do B1): o painel mantém
// o ESTADO vivo do pump — que NÃO é persistido no log a cada transição — e trocar essa
// fonte é a dimensão [PARCIAL] Estado do 360°, de outra rodada.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lina_core::{EventStore, NodeId, ProjectedNode, ProjectedState};
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

    /// Nó COM pasta no log, mas NENHUMA sessão de IA de hoje nela — distinto de "não
    /// rastreável" (rodada 360: distinguir os dois é critério, não polish).
    fn no_session_today() -> Self {
        Self {
            text: "sem sessão hoje".to_owned(),
            usd: 0.0,
            estimated: false,
            has_data: false,
        }
    }

    /// Nó SEM pasta no log (registro anterior ao ADR 0022 §3) — a correlação nó↔sessão
    /// é irreconstituível para sempre: "—" honesto, nunca chute.
    fn untrackable() -> Self {
        Self {
            text: "—".to_owned(),
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
    /// Rodada 360 (passo 7): cache do replay node↔cwd — `(event_count, projeção)`. O
    /// render re-replaya SÓ quando o log tem evento novo ([`projection_cache_is_stale`],
    /// padrão `refresh_cost_paused`); NUNCA por frame.
    pub projected: std::sync::Mutex<Option<(u64, ProjectedState)>>,
}

impl DashWiring {
    #[must_use]
    pub fn new(cli_id: String, ws_root: String) -> Self {
        Self {
            timeline: std::sync::Arc::default(),
            sessions: std::sync::Arc::default(),
            cli_id,
            ws_root,
            projected: std::sync::Mutex::new(None),
        }
    }
}

/// Decisão PURA do cache de projeção (rodada 360, passo 7): re-replay SÓ com evento
/// NOVO no log. `current = None` (store ilegível neste tick) → conserva o snapshot que
/// houver — degradação honesta, nunca um replay às cegas nem um painel zerado.
#[must_use]
pub fn projection_cache_is_stale(cached: Option<u64>, current: Option<u64>) -> bool {
    match (cached, current) {
        (Some(c), Some(n)) => c != n,
        (None, Some(_)) => true,
        (_, None) => false,
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

/// `cwd` vive sob `root`? Consciente da FRONTEIRA de path: `/wsfoo` NÃO está sob `/ws`
/// (o `starts_with` cru diria que sim — sobre-atribuição seria chute). Raiz com `/`
/// final é normalizada; raiz `/` cobre qualquer caminho absoluto.
fn under_root(cwd: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    cwd == root || (cwd.starts_with(root) && cwd[root.len()..].starts_with('/'))
}

/// Pastas ADOTADAS pelos nós do Espaço — o binding node↔cwd do log (ADR 0022 §3).
/// Alimenta o total "Hoje:": sessão na pasta própria de um nó do Espaço CONTA (rotulada
/// como parcela de fora), mesmo sem viver sob o prefixo do `ws_root`.
#[must_use]
pub fn adopted_cwds(state: &ProjectedState) -> BTreeSet<String> {
    state
        .nodes
        .values()
        .filter(|pn| pn.kind == "Terminal")
        .filter_map(|pn| pn.cwd.clone())
        .collect()
}

/// Total do Espaço hoje + parcela de FORA rotulada (rodada 360, passo 7).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceCost {
    /// Texto pronto da superfície (inclui "· inclui …US$ X fora da pasta do Espaço"
    /// quando a parcela existe e é exibível).
    pub line: CostLine,
    /// Parcela (USD, de hoje) atribuída por ADOÇÃO: cwd de nó do Espaço fora do `ws_root`.
    pub outside_usd: f64,
}

/// Custo TOTAL do Espaço hoje: sessões sob o `ws_root` (fronteira de path REAL — `/wsfoo`
/// não é `/ws`) **+** sessões nos cwds ADOTADOS pelos nós do Espaço (binding do log),
/// com a parcela de fora rotulada honestamente no próprio texto. Cada sessão conta NO
/// MÁXIMO uma vez (2+ nós no mesmo cwd não duplicam o total — `adopted` é conjunto).
#[must_use]
pub fn workspace_cost_today(
    sessions: &[Session],
    ws_root: &str,
    adopted: &BTreeSet<String>,
    today_prefix: &str,
) -> WorkspaceCost {
    let mut inside = 0.0f64;
    let mut outside = 0.0f64;
    let mut estimated = false;
    let mut any = false;
    for s in sessions {
        let today = s
            .last_ts
            .as_deref()
            .is_some_and(|ts| ts.starts_with(today_prefix));
        let Some(cwd) = s.cwd.as_deref() else {
            continue;
        };
        if !today {
            continue;
        }
        if under_root(cwd, ws_root) {
            any = true;
            inside += s.cost_usd;
            estimated |= s.cost_estimated || s.source == CostSource::Jsonl;
        } else if adopted.contains(cwd) {
            any = true;
            outside += s.cost_usd;
            estimated |= s.cost_estimated || s.source == CostSource::Jsonl;
        }
    }
    if !any {
        return WorkspaceCost {
            line: CostLine::without_data(),
            outside_usd: 0.0,
        };
    }
    let mut line = CostLine::from_today(inside + outside, estimated);
    // Rotula a parcela de fora SÓ quando ela é exibível (≥ 1 centavo após arredondar) —
    // "inclui US$ 0,00" seria ruído, não honestidade.
    let valor = format!("{outside:.2}");
    if line.has_data && valor != "0.00" {
        let marker = if estimated { "~" } else { "" };
        let valor = valor.replace('.', ",");
        line.text.push_str(&format!(
            " · inclui {marker}US$ {valor} fora da pasta do Espaço"
        ));
    }
    WorkspaceCost {
        line,
        outside_usd: outside,
    }
}

// ═══════ M8-T2 · Mini-status por Espaço (headless — a sidebar do Switcher consome) ═══════

/// Mini-status de UM Espaço na linha do Switcher (M8, spec §1): «{N} Agentes» + ● estado
/// dominante + custo curto de hoje + ⚠ store inacessível. Derivado 100% do log daquele
/// Espaço (inv#4) — nenhum gpui aqui.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMiniStatus {
    /// Terminais VIVOS (`UiState != Encerrado`) na projeção do Espaço.
    pub agents_alive: usize,
    /// Total de terminais (vivos + encerrados). A célula "todos-Encerrado" da spec §1
    /// mostra «{N} Agentes» com N = total e 0 vivos — "sem Agentes ainda" mentiria
    /// (houve time); sem este campo o render não distingue os dois casos.
    pub agents_total: usize,
    /// Estado dominante PIOR-PRIMEIRO (regra da spec §1): `Sem resposta > Esperando
    /// você > Trabalhando > Ocioso > Dormindo > Encerrado`. `None` = sem terminais.
    pub dominant: Option<UiState>,
    /// Forma CURTA do custo de hoje (ex.: `~US$ 0,01`) — sem o sufixo; o «estimado»
    /// viaja no tooltip. `None` = sem dado exibível (a UI mostra «—»), NUNCA «0,00».
    pub cost_short: Option<String>,
    /// Texto completo e honesto p/ o tooltip: com dado, o texto integral da [`CostLine`]
    /// (inclui «(estimado)» e a parcela de fora); sem dado, o fallback honesto.
    pub cost_tooltip: String,
    /// Store que não existe/não abre/não projeta → a linha vira ⚠, NUNCA some
    /// (anti-padrão 6: esconder é mentir).
    pub unreachable: bool,
}

impl WorkspaceMiniStatus {
    fn unreachable() -> Self {
        Self {
            agents_alive: 0,
            agents_total: 0,
            dominant: None,
            cost_short: None,
            cost_tooltip: "sem dados: a pasta deste Espaço não está acessível".to_owned(),
            unreachable: true,
        }
    }
}

/// Posição na regra PIOR-PRIMEIRO da spec §1 (menor = pior = dominante).
fn dominance_rank(s: UiState) -> u8 {
    match s {
        UiState::SemResposta => 0,
        UiState::EsperandoVoce => 1,
        UiState::Trabalhando => 2,
        UiState::Ocioso => 3,
        UiState::Dormindo => 4,
        UiState::Encerrado => 5,
    }
}

/// Forma curta de dinheiro (sem sufixo): `~US$ 0,01` / `US$ 0,01`.
fn short_money(usd: f64, estimated: bool) -> String {
    let valor = format!("{usd:.2}").replace('.', ",");
    let marker = if estimated { "~" } else { "" };
    format!("{marker}US$ {valor}")
}

/// O `events_dir` tem um store? (espelho de `persistence_ui::is_workspace_store`).
/// Checado ANTES de abrir: `EventStore::open` CRIA o store quando falta — efeito
/// colateral proibido numa leitura de mini-status (pasta sumida viraria store fantasma).
fn has_store(events_dir: &Path) -> bool {
    events_dir.join("lina.db").exists() || events_dir.join("log.jsonl").exists()
}

/// Abre o store e projeta, com retry BOUNDED. O `busy_timeout` de 3 s já mora dentro de
/// `EventStore::open` (F1-4-1 crit 5); o retry cobre o residual que o timeout não honra
/// de forma confiável (troca de `journal_mode` — lição W5). Falhou 3×? `None` honesto.
fn project_store_read_only(events_dir: &Path) -> Option<ProjectedState> {
    for _ in 0..3 {
        if let Ok(st) = EventStore::open(events_dir).and_then(|s| s.project()) {
            return Some(st);
        }
    }
    None
}

/// Mini-status de UM Espaço a partir do seu `events_dir`. Tudo INJETADO (`sessions`,
/// `ws_root`, `today_prefix`) — função pura sem relógio nem estado global (lição
/// wall-clock do módulo). Store ausente/corrompido → `unreachable=true`, nunca linha
/// sumida; sem custo apurável → `cost_short=None`, nunca «0,00».
#[must_use]
pub fn workspace_mini_status(
    events_dir: &Path,
    ws_root: &str,
    sessions: &[Session],
    today_prefix: &str,
) -> WorkspaceMiniStatus {
    if !has_store(events_dir) {
        return WorkspaceMiniStatus::unreachable();
    }
    let Some(state) = project_store_read_only(events_dir) else {
        return WorkspaceMiniStatus::unreachable();
    };

    let states: Vec<UiState> = state
        .nodes
        .values()
        .filter(|pn| pn.kind == "Terminal")
        .map(|pn| UiState::from_projection(pn.status.as_deref(), pn.stalled))
        .collect();
    let agents_total = states.len();
    let agents_alive = states.iter().filter(|s| **s != UiState::Encerrado).count();
    let dominant = states.iter().copied().min_by_key(|s| dominance_rank(*s));

    let cost = workspace_cost_today(sessions, ws_root, &adopted_cwds(&state), today_prefix);
    let (cost_short, cost_tooltip) = if cost.line.has_data {
        (
            Some(short_money(cost.line.usd, cost.line.estimated)),
            cost.line.text,
        )
    } else {
        (None, cost.line.text)
    };

    WorkspaceMiniStatus {
        agents_alive,
        agents_total,
        dominant,
        cost_short,
        cost_tooltip,
        unreachable: false,
    }
}

/// Raiz do Espaço a partir do seu `events_dir` — espelho das convenções aceitas por
/// `persistence_ui::list_workspaces` (`<ws>/.lina/events`, `<ws>/events`, ou o próprio
/// dir já sendo o store).
#[must_use]
pub fn ws_root_of(events_dir: &Path) -> PathBuf {
    if events_dir.ends_with(".lina/events") {
        if let Some(root) = events_dir.parent().and_then(Path::parent) {
            return root.to_path_buf();
        }
    } else if events_dir.ends_with("events") {
        if let Some(root) = events_dir.parent() {
            return root.to_path_buf();
        }
    }
    events_dir.to_path_buf()
}

/// Um mini-status guardado com a SUA marca de tempo — staleness declarada, nunca
/// implícita: quem lê decide se está velho com o relógio que tiver.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniStatusEntry {
    pub status: WorkspaceMiniStatus,
    /// Instante (epoch ms) em que o caller computou — INJETADO (sem relógio interno).
    pub computed_at_ms: u64,
}

impl MiniStatusEntry {
    /// Idade do snapshot dado o `now_ms` do caller (saturante — relógio que anda
    /// para trás não vira idade negativa/gigante).
    #[must_use]
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.computed_at_ms)
    }
}

/// Cache do mini-status por ABERTURA do M8 (spec §6-B5): `refresh` projeta cada store
/// 1× e guarda; os frames seguintes leem via [`MiniStatusCache::get`] sem re-projetar.
/// Mitiga o O(N×replay) por abertura; a staleness fica DECLARADA no `computed_at_ms`.
#[derive(Debug, Default)]
pub struct MiniStatusCache {
    by_dir: BTreeMap<PathBuf, MiniStatusEntry>,
}

impl MiniStatusCache {
    /// Projeta 1× cada `events_dir` e substitui o conteúdo do cache (Espaço que saiu
    /// da lista sai do cache — não acumula fantasmas). `computed_at_ms` é injetado
    /// pelo caller e carimbado em TODAS as entradas desta rodada.
    pub fn refresh(
        &mut self,
        entries: &[PathBuf],
        sessions: &[Session],
        today_prefix: &str,
        computed_at_ms: u64,
    ) {
        self.by_dir = entries
            .iter()
            .map(|dir| {
                let root = ws_root_of(dir);
                let status =
                    workspace_mini_status(dir, &root.to_string_lossy(), sessions, today_prefix);
                (
                    dir.clone(),
                    MiniStatusEntry {
                        status,
                        computed_at_ms,
                    },
                )
            })
            .collect();
    }

    /// Último mini-status computado para o `events_dir` (+ o instante, p/ a idade).
    #[must_use]
    pub fn get(&self, events_dir: &Path) -> Option<&MiniStatusEntry> {
        self.by_dir.get(events_dir)
    }
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

/// COMO o custo da linha foi atribuído ao nó — honestidade do painel (rodada 360,
/// passo 7): a ressalva acompanha o número; nunca número pelado quando há ressalva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostAttribution {
    /// `cwd` do log, único entre os nós, dentro do Espaço: o custo é dele.
    Exclusive,
    /// `cwd` do log FORA do `ws_root` (pasta própria) — custo REAL, rotulado.
    OutsideWorkspace,
    /// 2+ nós no MESMO cwd: o total do cwd é exibido como compartilhado — JAMAIS
    /// dividido arbitrariamente (não há base para o rateio; dividir seria chute).
    Shared { agents: usize, outside: bool },
    /// `cwd` no log, mas nenhuma sessão de IA de hoje nele (≠ não rastreável).
    NoSessionToday,
    /// Log histórico sem `cwd` (anterior ao ADR 0022 §3): irreconstituível p/ sempre.
    Untrackable,
}

impl CostAttribution {
    /// Ressalva textual que acompanha o número (`None` = número sem ressalva).
    #[must_use]
    pub fn note(self) -> Option<String> {
        match self {
            CostAttribution::Exclusive | CostAttribution::NoSessionToday => None,
            CostAttribution::OutsideWorkspace => {
                Some("trabalhando fora da pasta do Espaço".to_owned())
            }
            CostAttribution::Shared {
                agents,
                outside: false,
            } => Some(format!("compartilhado: {agents} agentes na mesma pasta")),
            CostAttribution::Shared {
                agents,
                outside: true,
            } => Some(format!(
                "compartilhado: {agents} agentes na mesma pasta, fora do Espaço"
            )),
            CostAttribution::Untrackable => {
                Some("custo não rastreável aqui (registro antigo)".to_owned())
            }
        }
    }
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
    /// COMO o custo foi atribuído (rodada 360) — o render exibe a ressalva junto do número.
    pub attribution: CostAttribution,
    pub tokens: TokenBreakdown,
    /// Frase completa p/ o leitor de tela — contém OS VALORES (critério 5).
    pub a11y_label: String,
}

/// Monta os cards POR TERMINAL a partir das projeções (inv#4: tudo deriva do log).
///
/// Correlação sessão↔nó pelo binding `ProjectedNode.cwd` PERSISTIDO no log
/// (`TerminalSpawned.cwd`, ADR 0022 §3) — match por cwd EXATO; subpasta não herda
/// (atribuir por prefixo seria chute). 2+ nós no mesmo cwd → total COMPARTILHADO,
/// nunca dividido. Sem cwd no log (histórico) → "—" não rastreável; cwd fora do
/// `ws_root` → custo real rotulado. `today_prefix` é `YYYY-MM-DD` INJETADO (função
/// pura; lição wall-clock).
#[must_use]
pub fn build_cards(
    state: &ProjectedState,
    sessions: &[Session],
    ws_root: &str,
    activities: &BTreeMap<NodeId, Activity>,
    today_prefix: &str,
) -> Vec<AgentCard> {
    state
        .nodes
        .iter()
        .filter(|(_, pn)| pn.kind == "Terminal")
        .map(|(node, pn)| {
            // Outros TERMINAIS no mesmo cwd: a atribuição vira compartilhada (nunca rateio).
            let peers = pn.cwd.as_deref().map_or(0, |cwd| {
                state
                    .nodes
                    .iter()
                    .filter(|(n2, p2)| {
                        **n2 != *node && p2.kind == "Terminal" && p2.cwd.as_deref() == Some(cwd)
                    })
                    .count()
            });
            build_card(
                *node,
                pn,
                peers,
                sessions,
                ws_root,
                activities,
                today_prefix,
            )
        })
        .collect()
}

fn build_card(
    node: NodeId,
    pn: &ProjectedNode,
    peers: usize,
    sessions: &[Session],
    ws_root: &str,
    activities: &BTreeMap<NodeId, Activity>,
    today_prefix: &str,
) -> AgentCard {
    let name = pn.name.clone().unwrap_or_else(|| "Terminal".to_owned());
    let engine = EngineBadge::from_cli(pn.cli.as_deref());
    let state = UiState::from_projection(pn.status.as_deref(), pn.stalled);
    let activity = activities.get(&node).copied().unwrap_or(Activity::Idle);

    // Honestidade da atribuição (rodada 360): sem cwd no log → irreconstituível ("—");
    // com cwd → soma das sessões DE HOJE daquele cwd exato; compartilhado/fora rotulados.
    let (cost_today, tokens, attribution) = match pn.cwd.as_deref() {
        None => (
            CostLine::untrackable(),
            TokenBreakdown::default(),
            CostAttribution::Untrackable,
        ),
        Some(cwd) => match aggregate_today(sessions, cwd, today_prefix) {
            None => (
                CostLine::no_session_today(),
                TokenBreakdown::default(),
                CostAttribution::NoSessionToday,
            ),
            Some((line, tk)) => {
                let outside = !under_root(cwd, ws_root);
                let attribution = if peers > 0 {
                    CostAttribution::Shared {
                        agents: peers + 1,
                        outside,
                    }
                } else if outside {
                    CostAttribution::OutsideWorkspace
                } else {
                    CostAttribution::Exclusive
                };
                (line, tk, attribution)
            }
        },
    };

    // A ressalva de atribuição acompanha o valor TAMBÉM no leitor de tela (critério 5).
    let cost_phrase = match attribution.note() {
        Some(n) => format!("{} ({n})", cost_today.text),
        None => cost_today.text.clone(),
    };
    let mut a11y_label = format!(
        "{name}: {}. Motor: {}. Custo de hoje: {cost_phrase}. Atividade: {}.",
        state.label(),
        engine.label,
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
        attribution,
        tokens,
        a11y_label,
    }
}

/// Agrega as sessões DE HOJE do `cwd` exato (um nó pode ter várias num dia — soma).
/// `None` = NENHUMA sessão de hoje nesse cwd ("sem sessão hoje" — distinto de sessão
/// de hoje SEM custo apurável, que devolve a `CostLine` honesta "sem estimativa").
fn aggregate_today(
    sessions: &[Session],
    cwd: &str,
    today_prefix: &str,
) -> Option<(CostLine, TokenBreakdown)> {
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
    if !matched {
        return None;
    }
    tk.thinking_warning = tk.tokens_thinking > tk.tokens_out;
    Some((CostLine::from_today(cost, estimated), tk))
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

impl PanelRect {
    /// Os dois retângulos têm área em comum? Bordas que só ENCOSTAM (gap 0) NÃO contam como
    /// interseção (`<`, não `<=`) — é a regra que deixa dois painéis colados sem sobrepor.
    /// Retângulo degenerado (w/h ≤ 0) nunca intersecta nada (não ocupa área).
    #[must_use]
    pub fn intersects(&self, other: &PanelRect) -> bool {
        if self.w <= 0.0 || self.h <= 0.0 || other.w <= 0.0 || other.h <= 0.0 {
            return false;
        }
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
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

    /// Geometria pura (base do T2): retângulos com área comum intersectam; separados ou apenas
    /// ENCOSTADOS (gap 0) não; degenerado (w/h ≤ 0) nunca. A borda `<` (não `<=`) é o que deixa
    /// dois painéis colados sem reportar sobreposição.
    #[test]
    fn panel_rect_intersection_rule() {
        let a = PanelRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let overlap = PanelRect {
            x: 50.0,
            y: 50.0,
            w: 100.0,
            h: 100.0,
        };
        let touching = PanelRect {
            x: 100.0,
            y: 0.0,
            w: 40.0,
            h: 100.0,
        }; // encosta em x=100
        let apart = PanelRect {
            x: 120.0,
            y: 0.0,
            w: 40.0,
            h: 100.0,
        };
        let degenerate = PanelRect {
            x: 10.0,
            y: 10.0,
            w: 0.0,
            h: 50.0,
        };
        assert!(a.intersects(&overlap));
        assert!(overlap.intersects(&a), "interseção é simétrica");
        assert!(
            !a.intersects(&touching),
            "bordas que só encostam não sobrepõem"
        );
        assert!(!a.intersects(&apart));
        assert!(!a.intersects(&degenerate), "área zero nunca intersecta");
    }

    /// Nó-terminal sintético. `cwd: Some` emite o `TerminalSpawned` com o binding
    /// node↔cwd PERSISTIDO (ADR 0022 §3 — o caminho real da correlação); `cwd: None`
    /// reproduz o LOG HISTÓRICO (nó sem spawn registrado ou spawn sem o campo).
    fn term_node(state: &mut ProjectedState, cli: Option<&str>, cwd: Option<&str>) -> NodeId {
        let node = Uuid::now_v7();
        apply(
            state,
            &DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 0.0,
                y: 0.0,
                requested_by: None,
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
        if let Some(cwd) = cwd {
            apply(
                state,
                &DomainEvent::TerminalSpawned {
                    node,
                    cli: cli.unwrap_or("claude-code").into(),
                    cwd: Some(cwd.into()),
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
    /// Rodada 360: o binding node↔cwd vem do `TerminalSpawned.cwd` do LOG, não de hints.
    #[test]
    fn cards_from_synthetic_log_field_by_field() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"), Some("/work/projeto-a"));
        status(&mut state, node, "Busy");

        let sessions = [session("/work/projeto-a", "2026-06-06T14:02:00.000Z")];
        let cards = build_cards(&state, &sessions, "/work", &busy(node), TODAY);

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
        // Dentro do Espaço, cwd único → atribuição exclusiva, número SEM ressalva.
        assert_eq!(c.attribution, CostAttribution::Exclusive);
        assert_eq!(c.attribution.note(), None);
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
        let node = term_node(&mut state, Some("claude-code"), Some("/w"));
        status(&mut state, node, "Busy");
        let mut s = session("/w", "2026-06-06T10:00:00.000Z");
        s.tokens_thinking = 900;
        s.tokens_out = 100;
        let cards = build_cards(&state, &[s], "/w", &busy(node), TODAY);
        assert!(cards[0].tokens.thinking_warning);
        assert!(
            cards[0].a11y_label.contains("pensando"),
            "aviso leigo no a11y: {}",
            cards[0].a11y_label
        );
    }

    /// Honestidade: sessão sem custo apurável (ex.: modelo fora da tabela de preço)
    /// → "sem estimativa ainda", NUNCA "US$ 0,00" — e DISTINTO de "sem sessão hoje".
    #[test]
    fn cost_without_data_is_honest() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"), Some("/w"));
        status(&mut state, node, "Idle");
        let mut s = session("/w", "2026-06-06T10:00:00.000Z");
        s.cost_usd = 0.0;
        let cards = build_cards(&state, &[s], "/w", &BTreeMap::new(), TODAY);
        let cost = &cards[0].cost_today;
        assert!(!cost.has_data);
        assert!(
            !cost.text.contains("0,00"),
            "zero seria mentira: {}",
            cost.text
        );
        assert!(cost.text.contains("sem estimativa"));
        // A sessão EXISTE hoje (só não tem preço apurável) → atribuição segue exclusiva.
        assert_eq!(cards[0].attribution, CostAttribution::Exclusive);
    }

    /// Distinção honesta (rodada 360): cwd no log SEM sessão de hoje → "sem sessão
    /// hoje" — que NÃO é o "—" de não-rastreável nem o "sem estimativa" do sem-preço.
    #[test]
    fn no_session_today_is_distinct_from_untrackable() {
        let mut state = ProjectedState::default();
        let a = term_node(&mut state, Some("claude-code"), Some("/w"));
        status(&mut state, a, "Busy");

        // Sem sessão nenhuma → "sem sessão hoje".
        let cards = build_cards(&state, &[], "/w", &BTreeMap::new(), TODAY);
        assert!(!cards[0].cost_today.has_data);
        assert_eq!(cards[0].cost_today.text, "sem sessão hoje");
        assert_eq!(cards[0].attribution, CostAttribution::NoSessionToday);
    }

    /// Atribuição ambígua (rodada 360, tarefa 4): 2+ nós no MESMO cwd → o total do cwd
    /// aparece como COMPARTILHADO nos dois cards — jamais dividido arbitrariamente
    /// (e o total do Espaço conta a sessão UMA vez, não duas).
    #[test]
    fn shared_cwd_shows_aggregate_never_split() {
        let mut state = ProjectedState::default();
        let a = term_node(&mut state, Some("claude-code"), Some("/w"));
        status(&mut state, a, "Busy");
        let b = term_node(&mut state, Some("claude-code"), Some("/w"));
        status(&mut state, b, "Busy");

        let sessions = [session("/w", "2026-06-06T10:00:00.000Z")];
        let cards = build_cards(&state, &sessions, "/w", &BTreeMap::new(), TODAY);
        assert_eq!(cards.len(), 2);
        for c in &cards {
            assert!(c.cost_today.has_data, "o agregado do cwd é FATO — exibido");
            assert!(
                (c.cost_today.usd - 0.0123).abs() < 1e-9,
                "total do cwd, nunca rateio: {}",
                c.cost_today.usd
            );
            assert_eq!(
                c.attribution,
                CostAttribution::Shared {
                    agents: 2,
                    outside: false
                }
            );
            let note = c.attribution.note().expect("ressalva obrigatória");
            assert!(note.contains("compartilhado: 2 agentes"), "{note}");
            assert!(c.a11y_label.contains("compartilhado"), "{}", c.a11y_label);
        }
        // Total do Espaço: a sessão compartilhada conta UMA vez.
        let total = workspace_cost_today(&sessions, "/w", &adopted_cwds(&state), TODAY);
        assert!((total.line.usd - 0.0123).abs() < 1e-9, "sem dupla contagem");
    }

    /// "Hoje" é prefixo INJETADO: sessão de ontem não entra no custo de hoje.
    #[test]
    fn today_filter_is_injected_not_wall_clock() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"), Some("/w"));
        status(&mut state, node, "Idle");
        let yesterday = session("/w", "2026-06-05T23:00:00.000Z");
        let cards = build_cards(&state, &[yesterday], "/w", &BTreeMap::new(), TODAY);
        assert!(
            !cards[0].cost_today.has_data,
            "sessão de ontem não é 'hoje'"
        );
        assert_eq!(cards[0].attribution, CostAttribution::NoSessionToday);
    }

    /// Motor desconhecido → fallback honesto (13.9 item 6), nunca chute.
    #[test]
    fn unknown_engine_has_honest_fallback() {
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, None, None);
        status(&mut state, node, "Ready");
        let cards = build_cards(&state, &[], "/w", &BTreeMap::new(), TODAY);
        assert_eq!(cards[0].engine.label, "Terminal (motor desconhecido)");
        assert!(!cards[0].engine.via_spawn);
    }

    /// Log HISTÓRICO (rodada 360, tarefa 6): nó sem `cwd` no log → "—" NÃO rastreável,
    /// mesmo havendo sessões de hoje no disco (correlação irreconstituível — ADR 0022 §3).
    #[test]
    fn old_log_node_without_cwd_is_untrackable() {
        let mut state = ProjectedState::default();
        // Caminho 1 do histórico: nó sem TerminalSpawned algum.
        let a = term_node(&mut state, Some("claude-code"), None);
        status(&mut state, a, "Busy");
        // Caminho 2: spawn ANTIGO sem o campo cwd (replay projeta None).
        let b = term_node(&mut state, Some("claude-code"), None);
        apply(
            &mut state,
            &DomainEvent::TerminalSpawned {
                node: b,
                cli: "claude-code".into(),
                cwd: None,
            },
        );
        status(&mut state, b, "Busy");

        let sessions = [session("/qualquer/lugar", "2026-06-06T10:00:00.000Z")];
        let cards = build_cards(&state, &sessions, "/ws", &BTreeMap::new(), TODAY);
        assert_eq!(cards.len(), 2);
        for c in &cards {
            assert!(!c.cost_today.has_data);
            assert_eq!(c.cost_today.text, "—", "traço honesto, nunca chute");
            assert_eq!(c.attribution, CostAttribution::Untrackable);
            let note = c.attribution.note().expect("o '—' vem com explicação");
            assert!(note.contains("não rastreável"), "{note}");
        }
    }

    /// Pasta própria (rodada 360, tarefa 2): cwd do log FORA do ws_root + sessão de hoje
    /// lá → custo REAL rotulado "trabalhando fora da pasta do Espaço" (fim do invisível).
    #[test]
    fn node_outside_workspace_shows_real_cost_with_label() {
        let mut state = ProjectedState::default();
        let node = term_node(
            &mut state,
            Some("claude-code"),
            Some("/Users/maria/projeto"),
        );
        status(&mut state, node, "Busy");

        let sessions = [session("/Users/maria/projeto", "2026-06-06T09:00:00.000Z")];
        let cards = build_cards(&state, &sessions, "/ws", &BTreeMap::new(), TODAY);
        let c = &cards[0];
        assert!(
            c.cost_today.has_data,
            "custo de pasta própria agora APARECE"
        );
        assert_eq!(c.cost_today.text, "~US$ 0,01 (estimado)");
        assert_eq!(c.attribution, CostAttribution::OutsideWorkspace);
        let note = c.attribution.note().expect("ressalva obrigatória");
        assert_eq!(note, "trabalhando fora da pasta do Espaço");
        assert!(c.a11y_label.contains("fora da pasta"), "{}", c.a11y_label);
    }

    /// Fronteira de path REAL: `/wsfoo` NÃO está sob `/ws` (o starts_with cru mentiria).
    #[test]
    fn ws_root_boundary_is_path_aware() {
        assert!(under_root("/ws/a", "/ws"));
        assert!(under_root("/ws", "/ws"));
        assert!(
            under_root("/ws/a", "/ws/"),
            "raiz com barra final normaliza"
        );
        assert!(under_root("/qualquer/abs", "/"), "raiz '/' cobre absolutos");
        assert!(!under_root("/wsfoo", "/ws"), "prefixo cru sobre-atribuiria");

        // Na prática: nó em /wsfoo com sessão lá → rotulado como FORA do Espaço /ws.
        let mut state = ProjectedState::default();
        let node = term_node(&mut state, Some("claude-code"), Some("/wsfoo"));
        status(&mut state, node, "Busy");
        let sessions = [session("/wsfoo", "2026-06-06T09:00:00.000Z")];
        let cards = build_cards(&state, &sessions, "/ws", &BTreeMap::new(), TODAY);
        assert_eq!(cards[0].attribution, CostAttribution::OutsideWorkspace);
    }

    /// Nó-nota não vira card (cards são POR TERMINAL).
    #[test]
    fn note_nodes_do_not_become_cards() {
        let mut state = ProjectedState::default();
        let _t = term_node(&mut state, Some("claude-code"), None);
        let note = Uuid::now_v7();
        apply(
            &mut state,
            &DomainEvent::NodeAdded {
                node: note,
                kind: "Note".into(),
                x: 1.0,
                y: 1.0,
                requested_by: None,
            },
        );
        let cards = build_cards(&state, &[], "/ws", &BTreeMap::new(), TODAY);
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
        let sessions = [inside, outside_ws, yesterday];

        // Sem cwds adotados: só a sessão do ws E de hoje (comportamento clássico).
        let cost = workspace_cost_today(&sessions, "/ws", &BTreeSet::new(), "2026-06-06");
        assert!(cost.line.has_data);
        assert!(
            (cost.line.usd - 0.0123).abs() < 1e-9,
            "só a sessão do ws E de hoje"
        );
        assert!(cost.line.estimated && cost.line.text.contains("estimado"));
        assert_eq!(cost.outside_usd, 0.0);
        assert!(
            !cost.line.text.contains("fora da pasta"),
            "sem parcela de fora, sem rótulo: {}",
            cost.line.text
        );
    }

    /// Total "Hoje:" (rodada 360, tarefa 3): sessão no cwd ADOTADO por nó do Espaço
    /// (binding do log) SOMA no total — com a parcela de fora rotulada honestamente.
    #[test]
    fn total_includes_adopted_cwds_with_labeled_outside_parcel() {
        let mut state = ProjectedState::default();
        let a = term_node(&mut state, Some("claude-code"), Some("/ws/terminal-a"));
        status(&mut state, a, "Busy");
        let b = term_node(&mut state, Some("claude-code"), Some("/outro/lugar"));
        status(&mut state, b, "Busy");

        let inside = session("/ws/terminal-a", "2026-06-06T10:00:00.000Z");
        let mut adopted_out = session("/outro/lugar", "2026-06-06T11:00:00.000Z");
        adopted_out.session_id = "s2".into();
        adopted_out.cost_usd = 0.0277;
        let mut alheia = session("/de/outra/pessoa", "2026-06-06T12:00:00.000Z");
        alheia.session_id = "s3".into();
        let sessions = [inside, adopted_out, alheia];

        let adopted = adopted_cwds(&state);
        assert!(adopted.contains("/outro/lugar"), "binding do log adotado");
        let total = workspace_cost_today(&sessions, "/ws", &adopted, "2026-06-06");
        // 0.0123 (dentro) + 0.0277 (adotada fora) — a alheia NÃO entra (não adotada).
        assert!((total.line.usd - 0.04).abs() < 1e-9, "{}", total.line.usd);
        assert!((total.outside_usd - 0.0277).abs() < 1e-9);
        assert!(
            total.line.text.contains("~US$ 0,04 (estimado)"),
            "{}",
            total.line.text
        );
        assert!(
            total
                .line
                .text
                .contains("inclui ~US$ 0,03 fora da pasta do Espaço"),
            "parcela de fora rotulada: {}",
            total.line.text
        );
    }

    /// Cache do replay (rodada 360, tarefa 1): re-replay SÓ com evento novo; contador
    /// ilegível conserva o snapshot (nunca replay por frame, nunca painel zerado).
    #[test]
    fn projection_cache_replays_only_on_new_events() {
        assert!(projection_cache_is_stale(None, Some(7)), "1º uso: popula");
        assert!(projection_cache_is_stale(Some(7), Some(9)), "evento novo");
        assert!(
            !projection_cache_is_stale(Some(9), Some(9)),
            "sem evento novo"
        );
        assert!(
            !projection_cache_is_stale(Some(9), None),
            "store ilegível: conserva o que há"
        );
        assert!(!projection_cache_is_stale(None, None), "nada a fazer");
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
        let a = term_node(&mut state, Some("claude-code"), Some("/wa"));
        status(&mut state, a, "Busy");
        let b = term_node(&mut state, Some("codex"), Some("/wb"));
        status(&mut state, b, "Idle");

        let mut s_b = session("/wb", "2026-06-06T11:00:00.000Z");
        s_b.session_id = "sess-2".into();
        s_b.cost_usd = 0.0277;
        s_b.tokens_thinking = 500;
        s_b.tokens_out = 10;
        let sessions = [session("/wa", "2026-06-06T10:00:00.000Z"), s_b];
        let cards = build_cards(&state, &sessions, "/", &BTreeMap::new(), TODAY);

        let t = totals(&cards);
        assert_eq!(t.cards, 2);
        assert!((t.cost_today_usd - 0.04).abs() < 1e-9, "0.0123+0.0277");
        assert!(t.any_estimated, "agregado herda o marcador de estimativa");
        assert_eq!(t.thinking_warnings, 1);
        assert!(t.summary_text.contains("~US$ 0,04"));
        assert!(t.summary_text.contains("estimado"));
    }

    /// Varredura COMBINATÓRIA da correlação node↔cwd↔sessão (matemática pura):
    /// 3 cwds × 2 lotações × 4 cenários de sessão = 24 combos; os invariantes de
    /// honestidade valem em TODOS — nunca chute, nunca rateio, nunca dupla contagem.
    #[test]
    fn attribution_grid_invariants() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Cwd {
            Absent,
            Inside,
            Outside,
        }
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Sess {
            None,
            Today,
            Yesterday,
            TodayZeroCost,
        }
        const WS: &str = "/ws";

        for cwd_kind in [Cwd::Absent, Cwd::Inside, Cwd::Outside] {
            for peer in [false, true] {
                for sess in [
                    Sess::None,
                    Sess::Today,
                    Sess::Yesterday,
                    Sess::TodayZeroCost,
                ] {
                    let ctx = format!("cwd={cwd_kind:?} peer={peer} sess={sess:?}");
                    let cwd = match cwd_kind {
                        Cwd::Absent => None,
                        Cwd::Inside => Some("/ws/a"),
                        Cwd::Outside => Some("/fora/b"),
                    };
                    let mut state = ProjectedState::default();
                    let node = term_node(&mut state, Some("claude-code"), cwd);
                    status(&mut state, node, "Busy");
                    if peer {
                        let p = term_node(&mut state, Some("claude-code"), cwd);
                        status(&mut state, p, "Busy");
                    }
                    // A sessão mora no cwd do nó (ou num canto alheio, p/ o caso Absent).
                    let sess_cwd = cwd.unwrap_or("/alheio");
                    let sessions: Vec<Session> = match sess {
                        Sess::None => vec![],
                        Sess::Today => vec![session(sess_cwd, "2026-06-06T10:00:00.000Z")],
                        Sess::Yesterday => vec![session(sess_cwd, "2026-06-05T10:00:00.000Z")],
                        Sess::TodayZeroCost => {
                            let mut s = session(sess_cwd, "2026-06-06T10:00:00.000Z");
                            s.cost_usd = 0.0;
                            vec![s]
                        }
                    };
                    let cards = build_cards(&state, &sessions, WS, &BTreeMap::new(), TODAY);
                    assert_eq!(cards.len(), if peer { 2 } else { 1 }, "{ctx}");

                    let matched_today =
                        cwd.is_some() && matches!(sess, Sess::Today | Sess::TodayZeroCost);
                    let priced = cwd.is_some() && matches!(sess, Sess::Today);
                    for c in &cards {
                        // 1) Número exibido ⇔ cwd no log + sessão de hoje + custo apurável.
                        assert_eq!(c.cost_today.has_data, priced, "{ctx}");
                        // 2) "—" ⇔ não rastreável ⇔ sem cwd no log.
                        assert_eq!(
                            c.attribution == CostAttribution::Untrackable,
                            cwd.is_none(),
                            "{ctx}"
                        );
                        assert_eq!(c.cost_today.text == "—", cwd.is_none(), "{ctx}");
                        // 3) "sem sessão hoje" ⇔ cwd no log e nenhuma sessão de hoje nele.
                        assert_eq!(
                            c.attribution == CostAttribution::NoSessionToday,
                            cwd.is_some() && !matched_today,
                            "{ctx}"
                        );
                        // 4) A atribuição certa para cada quadrante; compartilhado = TOTAL.
                        match c.attribution {
                            CostAttribution::Shared { agents, outside } => {
                                assert!(peer && matched_today, "{ctx}");
                                assert_eq!(agents, 2, "{ctx}");
                                assert_eq!(outside, cwd_kind == Cwd::Outside, "{ctx}");
                                if priced {
                                    assert!(
                                        (c.cost_today.usd - 0.0123).abs() < 1e-9,
                                        "{ctx}: rateio seria chute"
                                    );
                                }
                            }
                            CostAttribution::OutsideWorkspace => {
                                assert!(
                                    cwd_kind == Cwd::Outside && matched_today && !peer,
                                    "{ctx}"
                                );
                            }
                            CostAttribution::Exclusive => {
                                assert!(cwd_kind == Cwd::Inside && matched_today && !peer, "{ctx}");
                            }
                            CostAttribution::NoSessionToday | CostAttribution::Untrackable => {}
                        }
                        // 5) Toda ressalva chega ao leitor de tela (critério 5).
                        if let Some(note) = c.attribution.note() {
                            assert!(c.a11y_label.contains(&note), "{ctx}: {}", c.a11y_label);
                        }
                    }
                    // 6) Total do Espaço: cada sessão conta NO MÁXIMO uma vez; parcela de
                    //    fora rotulada exatamente quando há custo adotado fora do ws_root.
                    let total = workspace_cost_today(&sessions, WS, &adopted_cwds(&state), TODAY);
                    let expect_total = if priced { 0.0123 } else { 0.0 };
                    let expect_outside = if priced && cwd_kind == Cwd::Outside {
                        0.0123
                    } else {
                        0.0
                    };
                    assert!(
                        (total.line.usd - expect_total).abs() < 1e-9,
                        "{ctx}: {}",
                        total.line.usd
                    );
                    assert!((total.outside_usd - expect_outside).abs() < 1e-9, "{ctx}");
                    assert_eq!(
                        total.line.text.contains("fora da pasta do Espaço"),
                        expect_outside > 0.0,
                        "{ctx}: {}",
                        total.line.text
                    );
                }
            }
        }
    }

    // ═══════════════ M8-T2 · mini-status por Espaço (EventStore REAL em temp dir) ═══════════════

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "lina-dash-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&p).expect("tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Espaço REAL em disco: store de eventos + N terminais com (status, cwd).
    /// Devolve o `events_dir` (layout `<ws>/.lina/events`, o canônico do app).
    fn seed_space(root: &Path, name: &str, terminals: &[(&str, Option<&str>)]) -> PathBuf {
        let events = root.join(".lina").join("events");
        let mut store = lina_core::EventStore::open(&events).expect("open");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: name.into(),
                focus_preset: String::new(),
            })
            .expect("ws");
        for (status, cwd) in terminals {
            let node = Uuid::now_v7();
            store
                .append(&DomainEvent::NodeAdded {
                    node,
                    kind: "Terminal".into(),
                    x: 0.0,
                    y: 0.0,
                    requested_by: None,
                })
                .expect("node");
            if let Some(cwd) = cwd {
                store
                    .append(&DomainEvent::TerminalSpawned {
                        node,
                        cli: "claude-code".into(),
                        cwd: Some((*cwd).into()),
                    })
                    .expect("spawn");
            }
            store
                .append(&DomainEvent::NodeStatusChanged {
                    node,
                    status: (*status).into(),
                    from: String::new(),
                    reason: String::new(),
                })
                .expect("status");
        }
        events
    }

    /// Dominância PIOR-PRIMEIRO da spec §1: 2 vivos em estados mistos → o pior grita.
    /// `Esperando você` (Blocked) domina `Trabalhando` (Busy); vivo SEMPRE domina Encerrado.
    #[test]
    fn mini_status_dominance_is_worst_first_with_mixed_alive_states() {
        let tmp = TempDir::new("dom");
        let events = seed_space(
            tmp.path(),
            "Misto",
            &[("Busy", None), ("Blocked", None), ("Dead", None)],
        );
        let ms = workspace_mini_status(&events, tmp.path().to_str().unwrap(), &[], TODAY);
        assert!(!ms.unreachable);
        assert_eq!(ms.agents_total, 3);
        assert_eq!(ms.agents_alive, 2, "Dead não conta como vivo");
        assert_eq!(ms.dominant, Some(UiState::EsperandoVoce));
        // O dot da linha usa o vocabulário canônico (palavra + token de cor).
        assert_eq!(ms.dominant.unwrap().label(), "Esperando você");
        assert_eq!(ms.dominant.unwrap().color_token(), "ambar");
    }

    /// Espaço sem nenhum terminal: 0/0, sem dominante — a UI mostra «sem Agentes ainda».
    #[test]
    fn mini_status_space_without_agents() {
        let tmp = TempDir::new("vazio");
        let events = seed_space(tmp.path(), "Vazio", &[]);
        let ms = workspace_mini_status(&events, tmp.path().to_str().unwrap(), &[], TODAY);
        assert!(!ms.unreachable);
        assert_eq!(ms.agents_total, 0);
        assert_eq!(ms.agents_alive, 0);
        assert_eq!(ms.dominant, None);
    }

    /// Célula "todos-Encerrado" da spec §1: ≥1 nó e 0 vivos → «{N} Agentes» (N = total)
    /// + dominante Encerrado (cinza-escuro). "sem Agentes ainda" mentiria — houve time.
    #[test]
    fn mini_status_all_dead_keeps_total_and_dominant_encerrado() {
        let tmp = TempDir::new("dead");
        let events = seed_space(tmp.path(), "Findo", &[("Dead", None), ("Dead", None)]);
        let ms = workspace_mini_status(&events, tmp.path().to_str().unwrap(), &[], TODAY);
        assert_eq!(
            ms.agents_total, 2,
            "o total preserva o N da célula todos-Encerrado"
        );
        assert_eq!(ms.agents_alive, 0);
        assert_eq!(ms.dominant, Some(UiState::Encerrado));
        assert_eq!(ms.dominant.unwrap().color_token(), "cinza-escuro");
    }

    /// Anti-padrão 6 (esconder é mentir): store ausente ou corrompido vira linha-⚠
    /// (`unreachable=true`), NUNCA linha sumida. E a leitura NÃO cria store fantasma.
    #[test]
    fn mini_status_unreachable_when_store_missing_or_corrupt() {
        let tmp = TempDir::new("err");

        // (a) Pasta sem store: ⚠ e NENHUM efeito colateral (open criaria — proibido).
        let missing = tmp.path().join("sumiu").join(".lina").join("events");
        let ms = workspace_mini_status(&missing, tmp.path().to_str().unwrap(), &[], TODAY);
        assert!(ms.unreachable);
        assert_eq!(ms.agents_total, 0);
        assert_eq!(ms.cost_short, None);
        assert!(
            !missing.exists(),
            "leitura de mini-status NÃO pode criar store fantasma"
        );

        // (b) Store corrompido (db vira lixo, sem espelho JSONL): ⚠, não pânico.
        let corrupt_dir = tmp.path().join("ruim");
        let events = corrupt_dir.join(".lina").join("events");
        std::fs::create_dir_all(&events).expect("mkdir");
        std::fs::write(events.join("lina.db"), b"isto nao e um sqlite").expect("write");
        let ms = workspace_mini_status(&events, corrupt_dir.to_str().unwrap(), &[], TODAY);
        assert!(ms.unreachable, "store que não abre/projeta → linha-⚠");
    }

    /// Honestidade do custo: Espaço com terminal mas SEM sessão de hoje → `cost_short`
    /// `None` (a UI mostra «—»), NUNCA «0,00»; o tooltip carrega o fallback honesto.
    #[test]
    fn mini_status_cost_without_session_is_none_never_zero() {
        let tmp = TempDir::new("semcusto");
        let events = seed_space(tmp.path(), "Seco", &[("Busy", Some("/w/projeto"))]);
        let ms = workspace_mini_status(&events, "/w", &[], TODAY);
        assert_eq!(ms.cost_short, None);
        assert!(
            !ms.cost_tooltip.contains("0,00"),
            "zero seria mentira: {}",
            ms.cost_tooltip
        );
        assert_eq!(ms.cost_tooltip, "sem estimativa de custo ainda");
    }

    /// Custo COM sessão de hoje: forma curta SEM o sufixo («~US$ 0,01»); o «estimado»
    /// viaja no tooltip (texto integral da CostLine).
    #[test]
    fn mini_status_cost_short_form_without_suffix_estimated_in_tooltip() {
        let tmp = TempDir::new("custo");
        let events = seed_space(tmp.path(), "Caro", &[("Busy", Some("/w/projeto"))]);
        let sessions = [session("/w/projeto", "2026-06-06T14:02:00.000Z")];
        let ms = workspace_mini_status(&events, "/w", &sessions, TODAY);
        assert_eq!(ms.cost_short.as_deref(), Some("~US$ 0,01"));
        assert!(
            !ms.cost_short.unwrap().contains("estimado"),
            "forma curta não carrega o sufixo"
        );
        assert!(
            ms.cost_tooltip.contains("(estimado)"),
            "{}",
            ms.cost_tooltip
        );
    }

    /// Sessão no cwd ADOTADO por nó do Espaço (fora do ws_root) conta no mini-status —
    /// mesmo recorte honesto do `workspace_cost_today` (parcela de fora no tooltip).
    #[test]
    fn mini_status_cost_includes_adopted_cwd_outside_root() {
        let tmp = TempDir::new("fora");
        let events = seed_space(tmp.path(), "Fora", &[("Busy", Some("/outro/lugar"))]);
        let sessions = [session("/outro/lugar", "2026-06-06T14:02:00.000Z")];
        let ms = workspace_mini_status(&events, "/w", &sessions, TODAY);
        assert_eq!(ms.cost_short.as_deref(), Some("~US$ 0,01"));
        assert!(
            ms.cost_tooltip.contains("fora da pasta do Espaço"),
            "{}",
            ms.cost_tooltip
        );
    }

    /// Cache por abertura do M8 (spec §6-B5): `refresh` projeta 1× cada entrada e
    /// carimba o instante INJETADO; `get` devolve o último + base p/ idade; Espaço
    /// que saiu da lista sai do cache (sem fantasmas).
    #[test]
    fn mini_status_cache_refresh_get_and_declared_staleness() {
        let tmp = TempDir::new("cache");
        let ev_a = seed_space(&tmp.path().join("a"), "A", &[("Busy", None)]);
        let ev_b = seed_space(&tmp.path().join("b"), "B", &[]);

        let mut cache = MiniStatusCache::default();
        cache.refresh(&[ev_a.clone(), ev_b.clone()], &[], TODAY, 1_000);

        let a = cache.get(&ev_a).expect("A no cache");
        assert_eq!(a.computed_at_ms, 1_000);
        assert_eq!(a.status.agents_alive, 1);
        assert_eq!(
            a.age_ms(4_500),
            3_500,
            "idade = now - computed_at (injetados)"
        );
        assert_eq!(a.age_ms(500), 0, "relógio andando p/ trás satura em zero");
        assert_eq!(cache.get(&ev_b).expect("B no cache").status.agents_total, 0);

        // Nova abertura SÓ com B: A sai do cache (lista é a verdade da rodada).
        cache.refresh(std::slice::from_ref(&ev_b), &[], TODAY, 2_000);
        assert!(
            cache.get(&ev_a).is_none(),
            "entrada fora da lista não fica fantasma"
        );
        assert_eq!(cache.get(&ev_b).expect("B").computed_at_ms, 2_000);
    }

    /// `ws_root_of`: espelho das convenções de `list_workspaces` — `<ws>/.lina/events`,
    /// `<ws>/events`, ou o próprio dir já sendo o store.
    #[test]
    fn ws_root_of_mirrors_list_workspaces_layouts() {
        use std::path::PathBuf;
        assert_eq!(
            ws_root_of(&PathBuf::from("/ws/proj/.lina/events")),
            PathBuf::from("/ws/proj")
        );
        assert_eq!(
            ws_root_of(&PathBuf::from("/ws/proj/events")),
            PathBuf::from("/ws/proj")
        );
        assert_eq!(
            ws_root_of(&PathBuf::from("/ws/store-direto")),
            PathBuf::from("/ws/store-direto")
        );
    }
}
