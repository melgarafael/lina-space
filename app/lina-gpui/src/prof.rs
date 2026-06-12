//! F1-5-1 · Sonda **`[PROF]`** — decomposição do frametime (a `[FPS]` mede o AGREGADO; esta
//! decompõe). Lógica **gpui-free** (agregação de janela, percentis, top-K, formato das linhas)
//! testável headless; o `main.rs` só carimba `Instant`s nas fronteiras de fase do `render`.
//!
//! **Fases medidas (honestidade da medição — o que cada número É):**
//! - `poll` — do início do `render` até a entrada do loop de cards: poll/update do model,
//!   amostra da `[FPS]`, limpeza de estado (z-order/seleção), a11y.
//! - `assemble` — o loop de cards inteiro (montagem de elementos POR PAINEL; inclui o lock do
//!   grid + snapshot `screen()` — contenção com o pump É custo real da thread de render).
//!   Painéis suspensos (culled) dão `continue` e não geram amostra por-painel: a diferença
//!   entre `assemble` e a soma de TODAS as amostras por-painel é o custo de iterar/classificar
//!   os suspensos. ATENÇÃO de leitura: a linha top-K é só uma JANELA (os K mais caros, com o
//!   denominador "de N medidos") — painel fora do top-K ≠ custo zero; para o custo POR PAINEL
//!   completo (ex.: gate F1-5-3 — quanto custam os painéis OCIOSOS desenhados), suba o K com
//!   `LINA_PROF_TOPK=N` (≥ drawn) na sessão de medição.
//! - `chrome` — do fim do loop até o `render` retornar (topbar/rodapé/pulso/dashboard/modais).
//! - `layout_paint` — do retorno do `render` até o paint do SENTINELA (um `gpui::canvas` de
//!   tamanho zero, último filho da cena): cobre o layout (taffy) + paint dos elementos. É a
//!   **melhor aproximação disponível no gpui** sem forkar o framework.
//! - `present_vsync` — do paint do sentinela até o início do `render` seguinte: submit/present
//!   da GPU **+ a espera do próximo animation-frame (vsync)**. Em saturação (frametime ≫
//!   período do vsync) a espera tende a zero e o número aproxima o custo real de present.
//!
//! **Padrão da sonda `[FPS]` mantido:** janela ~120 frames, percentis nearest-rank, lacunas de
//! OCIOSIDADE descartadas (frame fechado com intervalo > [`PROF_IDLE_GAP_MS`] não entra na
//! janela — medimos *enquanto desenha*), sem spam (1 bloco por janela). Overhead: com
//! `LINA_PROF` ausente/`0` a sonda é UM check de bool por frame — o protocolo de overhead
//! (critério c) compara o `[FPS]` (sempre ativo) entre `LINA_PROF=0` e `=1` na MESMA carga.
//!
//! **F2-0-1 — métricas de gate (estrutura SuperTuxKart/CapFrameX da régua D0(d)):** além de
//! p50/p95, a janela emite **p99** e **`excess_time` (% do TEMPO acima do orçamento)** =
//! soma do frametime dos frames que estouraram [`PROF_BUDGET_MS`] ÷ tempo ativo total. O
//! orçamento é PARÂMETRO (`16.6` default = 60Hz; `LINA_PROF_BUDGET_MS=8.33` para 120Hz) —
//! a meta de plataforma muda, a sonda não. O fechamento da janela também produz um
//! [`ProfWindowSummary`] serializável (numerador/denominador BRUTOS — re-derivável por
//! replay, inv#4): a fiação que o converte em evento do log é costura (`events.rs`); a
//! sonda só acumula e entrega via [`Probe::take_window_summaries`].
//!
//! **LoDPI (gap §II.8 da onda V):** [`Probe::observe_scale_factor`] loga o scale factor da
//! janela e AVISA quando rodando em 1x — nitidez de glifo só é validável a olho num monitor
//! 1080p real; a sonda garante que a sessão de tela SAIBA em que escala mediu.
//!
//! **Cena de estresse canônica (reprodutível, gate F2):**
//! ```sh
//! LINA_PROF=1 LINA_LOAD_ACTIVE=10 cargo run -p lina-gpui --release
//! # + pan/zoom contínuos por 60s; gate: p95 ≤ 16.6ms E excess_time < 1%
//! # (condição "Steady" da régua: excess_time < 0.1%)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Janela de amostragem, em FRAMES (espelha `FPS_WINDOW` da sonda `[FPS]` no `main.rs`).
pub const PROF_WINDOW: usize = 120;
/// Teto de TEMPO ATIVO (ms) por janela (espelha `FPS_WINDOW_MS`): fecha a janela ao acumular
/// ~2s de render ativo mesmo abaixo de [`PROF_WINDOW`] frames — FPS baixo não segura o bloco.
pub const PROF_WINDOW_MS: f64 = 2000.0;
/// Teto de intervalo entre frames (espelha `IDLE_GAP_MS`): acima disto o frame fechou contra
/// uma janela OCIOSA (quiescência do loop de animação), não foi "frame lento" — descartado.
pub const PROF_IDLE_GAP_MS: f64 = 250.0;
/// Quantos painéis o ranking "mais caros" reporta por janela.
pub const PROF_TOP_K: usize = 5;
/// Orçamento de frametime (ms) do gate F2 — 16.6 = 60Hz. Override: `LINA_PROF_BUDGET_MS`
/// (ex.: `8.33` para a meta 120Hz). Parâmetro da SONDA, não da plataforma.
pub const PROF_BUDGET_MS: f64 = 16.6;

/// `LINA_PROF=1|true|on|yes` liga a sonda (mesmo idioma de `LINA_DEMO`/`LINA_DASH`).
#[must_use]
pub fn enabled_from_env() -> bool {
    matches!(
        std::env::var("LINA_PROF")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

/// Percentil (nearest-rank) de um slice **já ordenado** de ms. `p` em `[0.0, 100.0]`; slice
/// vazio → `0.0`. Barato e sem alocar: índice = `round(p/100 · (n-1))`, saturado (sem panic).
/// (Movida da sonda `[FPS]` do `main.rs` — fonte única para as duas sondas.)
#[must_use]
pub fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0 * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// O carimbo do SENTINELA de paint: o closure de paint (`gpui::canvas`) grava o instante em um
/// atômico (nanos desde a época da sonda; `0` = vazio) — lock-free, sem Mutex envenenável.
pub struct PaintSlot {
    epoch: Instant,
    nanos: AtomicU64,
}

impl PaintSlot {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            nanos: AtomicU64::new(0),
        }
    }

    /// Chamado DENTRO da fase de paint do gpui (closure do sentinela). `+1` distingue do `0`
    /// ("sem carimbo") — o erro de 1ns é irrelevante na escala de ms.
    pub fn stamp(&self) {
        let n = u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX - 1);
        self.nanos.store(n + 1, Ordering::Relaxed);
    }

    /// Consome o carimbo (reseta para vazio) — cada frame usa o seu, nunca um stale.
    fn take(&self) -> Option<Instant> {
        let n = self.nanos.swap(0, Ordering::Relaxed);
        (n > 0).then(|| self.epoch + Duration::from_nanos(n - 1))
    }
}

/// Custo de montagem de UM painel em UM frame (fase `assemble`, atribuível).
pub struct PanelSample {
    pub name: String,
    pub ms: f64,
    /// Runs (spans estilo-contíguo) do grid montados — proxy de quads no shell: cada run vira
    /// ~1 quad de fundo + 1 run de texto. Layout/paint não são atribuíveis por painel no gpui.
    pub runs: usize,
}

/// Os carimbos que o `render` do `main.rs` colhe nas fronteiras de fase de UM frame.
pub struct RenderMarks {
    pub t0: Instant,
    pub poll_end: Instant,
    pub assemble_end: Instant,
    pub end: Instant,
    pub live: usize,
    pub drawn: usize,
    pub runs: usize,
}

/// Um frame FECHADO (fechamento ocorre no início do render seguinte — só aí o frametime e o
/// carimbo do sentinela existem).
pub struct FrameSample {
    pub frame_ms: f64,
    pub cpu_render_ms: f64,
    pub poll_ms: f64,
    pub assemble_ms: f64,
    pub chrome_ms: f64,
    /// `None` = o sentinela não pintou dentro da janela válida deste frame (ex.: 1º frame).
    pub layout_paint_ms: Option<f64>,
    pub present_vsync_ms: Option<f64>,
    pub live: usize,
    pub drawn: usize,
    pub runs: usize,
}

/// Resumo SERIALIZÁVEL de uma janela fechada — o payload do evento de métrica do [PROF]
/// (contrato §VI do épico F2; a fiação `summary → evento` é costura em `events.rs`).
/// Carrega numerador (`excess_ms`) e denominador (`active_ms`) BRUTOS: a régua re-deriva a
/// % acima do orçamento por replay somando janelas — exato, sem média-de-porcentagens.
/// Aditivo (`serde(default)`): replay de log antigo sem algum campo nunca quebra.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfWindowSummary {
    /// Frames fechados (válidos, não-ociosos) na janela.
    pub frames: usize,
    /// Orçamento vigente quando a janela fechou (16.6 default; `LINA_PROF_BUDGET_MS`).
    pub budget_ms: f64,
    /// Denominador: soma do frametime de TODOS os frames da janela (tempo ativo, ms).
    pub active_ms: f64,
    /// Numerador: soma do frametime dos frames com `frame_ms > budget_ms` (ms). A estrutura
    /// CapFrameX conta o frame inteiro que estourou, não só o excedente.
    pub excess_ms: f64,
    /// Quantos frames estouraram o orçamento (legibilidade; redundante com excess_ms).
    pub frames_over_budget: usize,
    pub frame_p50_ms: f64,
    pub frame_p95_ms: f64,
    pub frame_p99_ms: f64,
    /// Último scale factor observado ([`Probe::observe_scale_factor`]); `None` = não fiado.
    pub scale_factor: Option<f64>,
}

impl ProfWindowSummary {
    /// % do tempo acima do orçamento (métrica do gate: <1% passa; <0.1% = "Steady").
    #[must_use]
    pub fn excess_time_pct(&self) -> f64 {
        if self.active_ms <= 0.0 {
            return 0.0;
        }
        self.excess_ms / self.active_ms * 100.0
    }
}

/// Frame em aberto: medido neste `render`, fechado no próximo (precisa do intervalo + paint).
struct Pending {
    marks: RenderMarks,
    panels: Vec<PanelSample>,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Fecha um frame pendente contra o início do render seguinte (`now`) e o carimbo do sentinela.
/// `None` = o frame fechou contra OCIOSIDADE (intervalo > [`PROF_IDLE_GAP_MS`]) — descartado,
/// mesma honestidade da `[FPS]` ("medimos enquanto desenha").
fn close_frame(
    pending: Pending,
    now: Instant,
    paint_at: Option<Instant>,
) -> Option<(FrameSample, Vec<PanelSample>)> {
    let m = &pending.marks;
    let frame_ms = ms(now.saturating_duration_since(m.t0));
    if frame_ms > PROF_IDLE_GAP_MS {
        return None;
    }
    let (layout_paint_ms, present_vsync_ms) = match paint_at {
        // Carimbo válido: caiu ENTRE o fim do render deste frame e o início do seguinte.
        Some(p) if p >= m.end && p <= now => (
            Some(ms(p.saturating_duration_since(m.end))),
            Some(ms(now.saturating_duration_since(p))),
        ),
        _ => (None, None),
    };
    let frame = FrameSample {
        frame_ms,
        cpu_render_ms: ms(m.end.saturating_duration_since(m.t0)),
        poll_ms: ms(m.poll_end.saturating_duration_since(m.t0)),
        assemble_ms: ms(m.assemble_end.saturating_duration_since(m.poll_end)),
        chrome_ms: ms(m.end.saturating_duration_since(m.assemble_end)),
        layout_paint_ms,
        present_vsync_ms,
        live: m.live,
        drawn: m.drawn,
        runs: m.runs,
    };
    Some((frame, pending.panels))
}

/// Estatística por-painel acumulada na janela corrente.
#[derive(Default)]
struct PanelStats {
    ms: Vec<f64>,
    runs_last: usize,
}

/// Agregador da janela: acumula frames fechados + amostras por-painel; ao fechar a janela
/// (count OU tempo ativo) devolve as linhas `[PROF]` e zera.
struct Collector {
    /// Quantos painéis a linha top-K mostra ([`PROF_TOP_K`]; override `LINA_PROF_TOPK` — a
    /// sessão de tela sobe para ≥drawn quando o gate F1-5-3 precisar do custo POR PAINEL
    /// completo, incluindo os BARATOS/ociosos que nunca entrariam num top-5).
    top_k: usize,
    /// Orçamento de frametime (ms) do gate ([`PROF_BUDGET_MS`]; override `LINA_PROF_BUDGET_MS`).
    budget_ms: f64,
    /// Último scale factor observado — carimba o [`ProfWindowSummary`] (medição em 1x ≠ 2x).
    scale_factor: Option<f64>,
    frames: Vec<FrameSample>,
    panels: BTreeMap<String, PanelStats>,
}

impl Default for Collector {
    fn default() -> Self {
        Self {
            top_k: PROF_TOP_K,
            budget_ms: PROF_BUDGET_MS,
            scale_factor: None,
            frames: Vec::new(),
            panels: BTreeMap::new(),
        }
    }
}

/// `"p50=1.2ms p95=3.4ms"` sobre amostras NÃO-ordenadas; vazio → `"p50=-- p95=--"` (honesto:
/// "sem dado" não é `0.0ms`).
fn fmt_p50_p95(samples: &mut [f64]) -> String {
    if samples.is_empty() {
        return "p50=-- p95=--".to_string();
    }
    samples.sort_by(f64::total_cmp);
    format!(
        "p50={:.1}ms p95={:.1}ms",
        percentile_sorted(samples, 50.0),
        percentile_sorted(samples, 95.0)
    )
}

impl Collector {
    fn push(
        &mut self,
        frame: FrameSample,
        panels: Vec<PanelSample>,
    ) -> Option<(Vec<String>, ProfWindowSummary)> {
        for p in panels {
            let st = self.panels.entry(p.name).or_default();
            st.ms.push(p.ms);
            st.runs_last = p.runs;
        }
        self.frames.push(frame);
        let active_ms: f64 = self.frames.iter().map(|f| f.frame_ms).sum();
        if self.frames.len() >= PROF_WINDOW || active_ms >= PROF_WINDOW_MS {
            let report = (self.report_lines(), self.window_summary());
            self.frames.clear();
            self.panels.clear();
            return Some(report);
        }
        None
    }

    /// O [`ProfWindowSummary`] da janela corrente (numerador/denominador brutos + percentis).
    fn window_summary(&self) -> ProfWindowSummary {
        let mut frame: Vec<f64> = self.frames.iter().map(|f| f.frame_ms).collect();
        frame.sort_by(f64::total_cmp);
        let over: Vec<f64> = frame
            .iter()
            .copied()
            .filter(|&ms| ms > self.budget_ms)
            .collect();
        ProfWindowSummary {
            frames: frame.len(),
            budget_ms: self.budget_ms,
            // `+ 0.0` normaliza o `-0.0` que `Iterator::sum` devolve para lista vazia
            // (acumulador inicial IEEE) — senão o evento serializa `-0.0` e a linha
            // imprime `excess_time=-0.00%`.
            active_ms: frame.iter().sum::<f64>() + 0.0,
            excess_ms: over.iter().sum::<f64>() + 0.0,
            frames_over_budget: over.len(),
            frame_p50_ms: percentile_sorted(&frame, 50.0),
            frame_p95_ms: percentile_sorted(&frame, 95.0),
            frame_p99_ms: percentile_sorted(&frame, 99.0),
            scale_factor: self.scale_factor,
        }
    }

    /// As linhas do bloco `[PROF]` da janela corrente (1 linha de fases + 1 de top-K painéis).
    fn report_lines(&self) -> Vec<String> {
        let n = self.frames.len();
        let pick = |f: fn(&FrameSample) -> f64| -> Vec<f64> { self.frames.iter().map(f).collect() };
        let mut frame = pick(|f| f.frame_ms);
        let mut cpu = pick(|f| f.cpu_render_ms);
        let mut poll = pick(|f| f.poll_ms);
        let mut assemble = pick(|f| f.assemble_ms);
        let mut chrome = pick(|f| f.chrome_ms);
        let mut layout: Vec<f64> = self
            .frames
            .iter()
            .filter_map(|f| f.layout_paint_ms)
            .collect();
        let mut present: Vec<f64> = self
            .frames
            .iter()
            .filter_map(|f| f.present_vsync_ms)
            .collect();
        let mut live: Vec<f64> = self.frames.iter().map(|f| f.live as f64).collect();
        let mut drawn: Vec<f64> = self.frames.iter().map(|f| f.drawn as f64).collect();
        let mut runs: Vec<f64> = self.frames.iter().map(|f| f.runs as f64).collect();
        live.sort_by(f64::total_cmp);
        drawn.sort_by(f64::total_cmp);
        runs.sort_by(f64::total_cmp);

        // Métricas do gate F2 (p99 + excess_time) saem do MESMO resumo que vira evento —
        // o que o grep do stderr lê e o que o replay re-deriva nunca divergem.
        let s = self.window_summary();
        let mut lines = vec![format!(
            "[PROF] frames={n} live={} drawn={} | frame {} p99={:.1}ms | budget={:.1}ms excess_time={:.2}% over={}/{n} | cpu_render {} | poll {} | assemble {} | chrome {} | layout_paint {} | present_vsync {} | runs p50={}",
            percentile_sorted(&live, 50.0) as usize,
            percentile_sorted(&drawn, 50.0) as usize,
            fmt_p50_p95(&mut frame),
            s.frame_p99_ms,
            s.budget_ms,
            s.excess_time_pct(),
            s.frames_over_budget,
            fmt_p50_p95(&mut cpu),
            fmt_p50_p95(&mut poll),
            fmt_p50_p95(&mut assemble),
            fmt_p50_p95(&mut chrome),
            fmt_p50_p95(&mut layout),
            fmt_p50_p95(&mut present),
            percentile_sorted(&runs, 50.0) as usize,
        )];

        // Top-K painéis mais caros da janela, por p95 da fase assemble (desc).
        let mut ranked: Vec<(&String, f64, f64, usize)> = self
            .panels
            .iter()
            .map(|(name, st)| {
                let mut s = st.ms.clone();
                s.sort_by(f64::total_cmp);
                (
                    name,
                    percentile_sorted(&s, 50.0),
                    percentile_sorted(&s, 95.0),
                    st.runs_last,
                )
            })
            .collect();
        ranked.sort_by(|a, b| f64::total_cmp(&b.2, &a.2));
        if !ranked.is_empty() {
            let tops: Vec<String> = ranked
                .iter()
                .take(self.top_k.max(1))
                .map(|(name, p50, p95, runs)| {
                    format!("{name} p50={p50:.2}ms p95={p95:.2}ms runs={runs}")
                })
                .collect();
            // "de N medidos" = o DENOMINADOR: quantos painéis têm amostra na janela. Sem ele,
            // a ausência de um painel no top-K seria ilegível (caro demais nunca? barato?).
            lines.push(format!(
                "[PROF] top{} (de {} medidos) assemble: {}",
                self.top_k.max(1),
                ranked.len(),
                tops.join(" · ")
            ));
        }
        lines
    }

    /// Introspecção dos testes de gating (a sonda DESLIGADA não pode acumular nada).
    #[cfg(test)]
    fn frames_in_window(&self) -> usize {
        self.frames.len()
    }
}

/// A sonda viva no `WorkspaceView`. DESLIGADA (`enabled=false`): todo método retorna
/// imediatamente — custo de UM check de bool por frame (o protocolo de overhead compara o
/// `[FPS]` entre `LINA_PROF=0` e `=1`).
pub struct Probe {
    pub enabled: bool,
    paint: Arc<PaintSlot>,
    pending: Option<Pending>,
    cur_panels: Vec<PanelSample>,
    collector: Collector,
    /// Resumos de janelas FECHADAS aguardando dreno ([`Self::take_window_summaries`]) — a
    /// fiação `summary → evento do log` é costura (`events.rs`/`main.rs`), não desta sonda.
    summaries: Vec<ProfWindowSummary>,
}

impl Probe {
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            paint: Arc::new(PaintSlot::new()),
            pending: None,
            cur_panels: Vec::new(),
            collector: Collector::default(),
            summaries: Vec::new(),
        }
    }

    #[must_use]
    pub fn from_env() -> Self {
        let mut probe = Self::new(enabled_from_env());
        // `LINA_PROF_TOPK=N` (≥1) — a sessão de tela sobe o K para ver TODOS os painéis
        // (gate F1-5-3 precisa do custo dos baratos/ociosos, não só dos top-5 caros).
        if let Some(k) = std::env::var("LINA_PROF_TOPK")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            probe.collector.top_k = k.max(1);
        }
        // `LINA_PROF_BUDGET_MS=8.33` — orçamento do gate (default 16.6 = 60Hz). Finito e >0:
        // orçamento absurdo silenciaria o excess_time (sem dado ≠ "passou").
        if let Some(b) = std::env::var("LINA_PROF_BUDGET_MS")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|b| b.is_finite() && *b > 0.0)
        {
            probe.collector.budget_ms = b;
        }
        probe
    }

    /// O slot que o closure de paint do SENTINELA carimba (clone barato de `Arc`).
    #[must_use]
    pub fn paint_slot(&self) -> Arc<PaintSlot> {
        Arc::clone(&self.paint)
    }

    /// Início de um `render`: fecha o frame ANTERIOR (frametime + sentinela) e devolve as
    /// linhas `[PROF]` se a janela fechou (vazio na imensa maioria dos frames).
    pub fn begin_frame(&mut self, now: Instant) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let paint_at = self.paint.take();
        let Some(pending) = self.pending.take() else {
            return Vec::new();
        };
        match close_frame(pending, now, paint_at) {
            Some((frame, panels)) => match self.collector.push(frame, panels) {
                Some((lines, summary)) => {
                    self.summaries.push(summary);
                    lines
                }
                None => Vec::new(),
            },
            None => Vec::new(), // fechou contra ociosidade — descartado (não envenena p50/p95)
        }
    }

    /// Drena os resumos de janelas fechadas desde o último dreno — a fiação (costura) os
    /// converte em eventos aditivos do log (contrato §VI do F2; inv#4: replay re-deriva).
    // dead_code: o chamador é a fiação em `main.rs`/`events.rs` (costura F2-0-1, pendente).
    #[allow(dead_code)]
    pub fn take_window_summaries(&mut self) -> Vec<ProfWindowSummary> {
        std::mem::take(&mut self.summaries)
    }

    /// LoDPI check (F2, gap §II.8): registra o scale factor da janela e devolve a linha
    /// `[PROF]` quando ele MUDA (1ª observação inclusa) — com AVISO em 1x, onde a nitidez
    /// de glifo precisa ser validada a olho num monitor 1080p REAL. Chamável todo frame
    /// (barato; só fala na mudança). Carimba também o [`ProfWindowSummary`].
    // dead_code: o chamador é a fiação no `render` do `main.rs` (costura F2-0-1, pendente).
    #[allow(dead_code)]
    pub fn observe_scale_factor(&mut self, scale: f64) -> Option<String> {
        if !self.enabled || self.collector.scale_factor == Some(scale) {
            return None;
        }
        self.collector.scale_factor = Some(scale);
        let aviso = if scale < 1.5 {
            " AVISO: LoDPI (~1x) — valide a nitidez do texto a olho num monitor 1080p real"
        } else {
            ""
        };
        Some(format!("[PROF] scale_factor={scale:.2}{aviso}"))
    }

    /// Um painel DESENHADO terminou de montar neste frame.
    pub fn panel_done(&mut self, name: &str, elapsed: Duration, runs: usize) {
        if !self.enabled {
            return;
        }
        self.cur_panels.push(PanelSample {
            name: name.to_string(),
            ms: ms(elapsed),
            runs,
        });
    }

    /// Fim do `render`: registra os carimbos do frame corrente (fechado no próximo begin).
    pub fn finish_frame(&mut self, marks: RenderMarks) {
        if !self.enabled {
            return;
        }
        self.pending = Some(Pending {
            marks,
            panels: std::mem::take(&mut self.cur_panels),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn marks(base: Instant, off_ms: u64) -> RenderMarks {
        // Frame sintético: poll 2ms, assemble 6ms, chrome 1ms (cpu_render = 9ms).
        RenderMarks {
            t0: t(base, off_ms),
            poll_end: t(base, off_ms + 2),
            assemble_end: t(base, off_ms + 8),
            end: t(base, off_ms + 9),
            live: 28,
            drawn: 14,
            runs: 1900,
        }
    }

    /// Percentil nearest-rank: vazio → 0.0; extremos saturam sem panic; mediana correta.
    #[test]
    fn percentile_nearest_rank() {
        assert_eq!(percentile_sorted(&[], 50.0), 0.0);
        assert_eq!(percentile_sorted(&[7.0], 50.0), 7.0);
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile_sorted(&v, 0.0), 1.0);
        assert_eq!(percentile_sorted(&v, 100.0), 100.0);
        assert_eq!(percentile_sorted(&v, 50.0), 51.0); // round(0.5·99)=50 → v[50]=51
        assert_eq!(percentile_sorted(&v, 95.0), 95.0); // round(0.95·99)=94 → v[94]=95
    }

    /// A decomposição de um frame fechado bate com os carimbos: cpu=9 (poll=2, assemble=6,
    /// chrome=1), layout_paint=4 (end→paint), present_vsync=3 (paint→próximo render), frame=16.
    #[test]
    fn close_frame_decomposes_phases() {
        let base = Instant::now();
        let pending = Pending {
            marks: marks(base, 0),
            panels: vec![],
        };
        let (f, _) = close_frame(pending, t(base, 16), Some(t(base, 13)))
            .expect("frame contínuo não pode ser descartado");
        assert!((f.frame_ms - 16.0).abs() < 0.01);
        assert!((f.cpu_render_ms - 9.0).abs() < 0.01);
        assert!((f.poll_ms - 2.0).abs() < 0.01);
        assert!((f.assemble_ms - 6.0).abs() < 0.01);
        assert!((f.chrome_ms - 1.0).abs() < 0.01);
        assert!((f.layout_paint_ms.expect("sentinela válido") - 4.0).abs() < 0.01);
        assert!((f.present_vsync_ms.expect("sentinela válido") - 3.0).abs() < 0.01);
    }

    /// Frame fechado contra OCIOSIDADE (intervalo > PROF_IDLE_GAP_MS) é DESCARTADO — mesma
    /// honestidade da [FPS]: a quietude do loop de animação não é "frame lento".
    #[test]
    fn close_frame_discards_idle_gap() {
        let base = Instant::now();
        let pending = Pending {
            marks: marks(base, 0),
            panels: vec![],
        };
        assert!(close_frame(pending, t(base, 1000), None).is_none());
    }

    /// Carimbo do sentinela FORA da janela válida (stale de um frame velho) → fases pós-render
    /// viram None (não inventamos número).
    #[test]
    fn close_frame_rejects_stale_paint_stamp() {
        let base = Instant::now();
        let pending = Pending {
            marks: marks(base, 20),
            panels: vec![],
        };
        // Carimbo ANTES do fim do render deste frame (t=13 < end=29) = stale.
        let (f, _) = close_frame(pending, t(base, 36), Some(t(base, 13))).expect("contínuo");
        assert!(f.layout_paint_ms.is_none());
        assert!(f.present_vsync_ms.is_none());
    }

    /// A janela fecha por CONTAGEM (~PROF_WINDOW frames) e o bloco sai com o formato pinado —
    /// é o contrato que o Maestro grepa no stderr (critério a).
    #[test]
    fn window_closes_by_count_with_pinned_format() {
        let base = Instant::now();
        let mut probe = Probe::new(true);
        let mut got: Vec<String> = Vec::new();
        // Frames de 16ms: 120×16 = 1920ms < 2000ms → fecha por contagem, não por tempo.
        for i in 0..=PROF_WINDOW as u64 {
            got.extend(probe.begin_frame(t(base, i * 16)));
            probe.panel_done("Carga 01 (burst)", Duration::from_millis(2), 210);
            probe.panel_done("Carga 02 (spinner)", Duration::from_millis(1), 80);
            probe.finish_frame(marks(base, i * 16));
        }
        assert_eq!(got.len(), 2, "1 linha de fases + 1 de top-K");
        let l = &got[0];
        assert!(l.starts_with("[PROF] frames=120"), "linha: {l}");
        for key in [
            "live=28",
            "drawn=14",
            "frame p50=",
            "cpu_render p50=",
            "poll p50=",
            "assemble p50=",
            "chrome p50=",
            "layout_paint p50=",
            "present_vsync p50=",
            "runs p50=1900",
        ] {
            assert!(l.contains(key), "faltou `{key}` em: {l}");
        }
        assert!(got[1].starts_with("[PROF] top"), "linha 2: {}", got[1]);
        assert!(got[1].contains("Carga 01 (burst)"));
    }

    /// A janela também fecha por TEMPO ATIVO acumulado (~PROF_WINDOW_MS) — FPS baixo não
    /// segura o relatório (espelho do FPS_WINDOW_MS da [FPS]).
    #[test]
    fn window_closes_early_by_active_time() {
        let base = Instant::now();
        let mut probe = Probe::new(true);
        let mut reports = 0usize;
        let mut frames_at_first_report = 0usize;
        // Frames de 100ms (10fps): 2000ms/100ms = 20 frames << 120.
        for i in 0..=25u64 {
            let lines = probe.begin_frame(t(base, i * 100));
            if !lines.is_empty() && reports == 0 {
                frames_at_first_report = i as usize;
            }
            reports += usize::from(!lines.is_empty());
            probe.finish_frame(marks(base, i * 100));
        }
        assert_eq!(reports, 1, "fechou exatamente uma janela");
        assert!(
            frames_at_first_report < PROF_WINDOW / 2,
            "fechou por tempo ({frames_at_first_report} frames), não por contagem"
        );
    }

    /// GATING (não-vacuoso): sonda DESLIGADA não acumula NADA — nem frames, nem painéis —
    /// enquanto a LIGADA, com a mesma sequência, acumula. Remover o gate quebra este teste.
    #[test]
    fn disabled_probe_accumulates_nothing() {
        let base = Instant::now();
        let run = |enabled: bool| -> usize {
            let mut probe = Probe::new(enabled);
            for i in 0..10u64 {
                let _ = probe.begin_frame(t(base, i * 16));
                probe.panel_done("X", Duration::from_millis(1), 10);
                probe.finish_frame(marks(base, i * 16));
            }
            probe.collector.frames_in_window() + probe.cur_panels.len()
        };
        assert_eq!(run(false), 0, "desligada: zero acumulado");
        assert!(run(true) > 0, "ligada: acumula (prova o gate, não o vácuo)");
    }

    /// Top-K ranqueia por p95 DESC e respeita o teto K — os mais caros aparecem, o barato
    /// além do K não.
    #[test]
    fn top_k_ranks_by_p95_and_caps() {
        let mut c = Collector::default();
        let base = Instant::now();
        // 6 painéis com custos distintos: P6 (6ms) > P5 > … > P1 (1ms). K=5 → P1 fica fora.
        for i in 0..PROF_WINDOW as u64 {
            let panels: Vec<PanelSample> = (1..=6usize)
                .map(|p| PanelSample {
                    name: format!("P{p}"),
                    ms: p as f64,
                    runs: p * 10,
                })
                .collect();
            let pending = Pending {
                marks: marks(base, i * 16),
                panels,
            };
            let (f, ps) = close_frame(pending, t(base, i * 16 + 16), None).expect("contínuo");
            if let Some((lines, _)) = c.push(f, ps) {
                let top = &lines[1];
                let pos = |n: &str| top.find(n).unwrap_or(usize::MAX);
                assert!(pos("P6") < pos("P5"), "ordem por p95 desc: {top}");
                assert!(pos("P5") < pos("P2"), "ordem por p95 desc: {top}");
                assert!(top.contains("P2"), "K=5 inclui o 5º: {top}");
                assert!(!top.contains("P1 "), "o 6º (mais barato) fica fora: {top}");
                return;
            }
        }
        panic!("janela nunca fechou");
    }

    /// Sem carimbo do sentinela na janela inteira, as fases pós-render saem como `--`
    /// (sem dado ≠ 0.0ms — honestidade do relatório).
    #[test]
    fn missing_paint_stamp_reports_dashes() {
        let mut c = Collector::default();
        let base = Instant::now();
        for i in 0..PROF_WINDOW as u64 {
            let pending = Pending {
                marks: marks(base, i * 16),
                panels: vec![],
            };
            let (f, ps) = close_frame(pending, t(base, i * 16 + 16), None).expect("contínuo");
            if let Some((lines, _)) = c.push(f, ps) {
                assert!(
                    lines[0].contains("layout_paint p50=--"),
                    "sem dado → `--`: {}",
                    lines[0]
                );
                return;
            }
        }
        panic!("janela nunca fechou");
    }

    /// `top_k` configurável (LINA_PROF_TOPK na sessão de tela): K=2 mostra SÓ os 2 mais
    /// caros, e o DENOMINADOR "de N medidos" expõe quantos painéis têm amostra — sem ele,
    /// painel ausente do top-K seria ilegível (a leitura do gate F1-5-3 depende disto).
    #[test]
    fn top_k_is_configurable_and_shows_denominator() {
        let mut c = Collector {
            top_k: 2,
            ..Collector::default()
        };
        let base = Instant::now();
        for i in 0..PROF_WINDOW as u64 {
            let panels: Vec<PanelSample> = (1..=4usize)
                .map(|p| PanelSample {
                    name: format!("P{p}"),
                    ms: p as f64,
                    runs: p,
                })
                .collect();
            let pending = Pending {
                marks: marks(base, i * 16),
                panels,
            };
            let (f, ps) = close_frame(pending, t(base, i * 16 + 16), None).expect("contínuo");
            if let Some((lines, _)) = c.push(f, ps) {
                let top = &lines[1];
                assert!(
                    top.starts_with("[PROF] top2 (de 4 medidos)"),
                    "linha: {top}"
                );
                assert!(
                    top.contains("P4") && top.contains("P3"),
                    "2 mais caros: {top}"
                );
                assert!(
                    !top.contains("P2 ") && !top.contains("P1 "),
                    "K=2 corta: {top}"
                );
                return;
            }
        }
        panic!("janela nunca fechou");
    }

    /// Roundtrip do PaintSlot: stamp → take devolve um instante na época certa; o take
    /// CONSOME (segundo take = None — um frame nunca lê o carimbo de outro).
    #[test]
    fn paint_slot_stamp_take_roundtrip() {
        let slot = PaintSlot::new();
        assert!(slot.take().is_none(), "nasce vazio");
        slot.stamp();
        let got = slot.take().expect("carimbado");
        assert!(got >= slot.epoch);
        assert!(slot.take().is_none(), "take consome");
    }

    /// Frames sintéticos de duração exata (frame = intervalo até o próximo begin; as fases
    /// internas não importam para as métricas de orçamento).
    fn run_frames(probe: &mut Probe, durations_ms: &[u64]) -> Vec<String> {
        let base = Instant::now();
        let mut at = 0u64;
        let mut lines = Vec::new();
        for &d in durations_ms {
            lines.extend(probe.begin_frame(t(base, at)));
            probe.finish_frame(marks(base, at));
            at += d;
        }
        lines.extend(probe.begin_frame(t(base, at)));
        lines
    }

    /// F2-0-1 (gate): excess_time = soma do frametime dos frames ACIMA do orçamento ÷ tempo
    /// ativo total (estrutura CapFrameX: o frame que estourou conta INTEIRO). 118×10ms +
    /// 2×130ms → excess = 260/1440 ≈ 18.06%, over=2, e o p99 captura os outliers que o p95
    /// esconde (em 120 amostras o nearest-rank do p99 é o índice 118 — 2 outliers entram)
    /// — exatamente o porquê do gate exigir as duas métricas.
    #[test]
    fn excess_time_counts_whole_offending_frames() {
        let mut probe = Probe::new(true);
        let mut durations = vec![10u64; PROF_WINDOW - 2];
        durations.extend([130, 130]);
        let _ = run_frames(&mut probe, &durations);
        let s = probe.take_window_summaries().pop().expect("janela fechou");
        assert_eq!(s.frames, PROF_WINDOW);
        assert_eq!(s.frames_over_budget, 2);
        assert!((s.active_ms - 1440.0).abs() < 0.5, "active={}", s.active_ms);
        assert!((s.excess_ms - 260.0).abs() < 0.5, "excess={}", s.excess_ms);
        assert!(
            (s.excess_time_pct() - 260.0 / 1440.0 * 100.0).abs() < 0.1,
            "pct={}",
            s.excess_time_pct()
        );
        assert!(s.frame_p95_ms < 16.6, "p95 não vê os 2 outliers");
        assert!(s.frame_p99_ms > 16.6, "p99 vê: {}", s.frame_p99_ms);
    }

    /// O orçamento é PARÂMETRO: com budget=8.33 (meta 120Hz), frames de 10ms — verdes a
    /// 60Hz — passam a contar como excesso. A meta muda, a sonda não.
    #[test]
    fn budget_is_parametric() {
        let mut probe = Probe::new(true);
        probe.collector.budget_ms = 8.33;
        let _ = run_frames(&mut probe, &[10u64; PROF_WINDOW]);
        let s = probe.take_window_summaries().pop().expect("janela fechou");
        assert!((s.budget_ms - 8.33).abs() < f64::EPSILON);
        assert_eq!(s.frames_over_budget, PROF_WINDOW, "todo frame estoura 8.33");
        assert!((s.excess_time_pct() - 100.0).abs() < 0.01);
    }

    /// A linha [PROF] carrega as métricas do gate (p99 + budget + excess_time + over) — é o
    /// formato que o Maestro grepa no stderr na cena de estresse canônica.
    #[test]
    fn report_line_carries_gate_metrics() {
        let mut probe = Probe::new(true);
        let lines = run_frames(&mut probe, &[10u64; PROF_WINDOW]);
        let l = lines.first().expect("janela fechou");
        for key in ["p99=", "budget=16.6ms", "excess_time=0.00%", "over=0/120"] {
            assert!(l.contains(key), "faltou `{key}` em: {l}");
        }
    }

    /// inv#4 (re-derivação por replay): o resumo serializa com numerador/denominador BRUTOS;
    /// deserializar JSON SEM campos novos não quebra (`serde(default)` — evento aditivo) e a
    /// % re-derivada de duas janelas agregadas é exata (soma de somas, não média de %).
    #[test]
    fn summary_roundtrips_and_rederives_by_aggregation() {
        let s = ProfWindowSummary {
            frames: 120,
            budget_ms: PROF_BUDGET_MS,
            active_ms: 1320.0,
            excess_ms: 130.0,
            frames_over_budget: 1,
            frame_p50_ms: 10.0,
            frame_p95_ms: 10.0,
            frame_p99_ms: 130.0,
            scale_factor: Some(2.0),
        };
        let json = serde_json::to_string(&s).expect("serializa");
        let back: ProfWindowSummary = serde_json::from_str(&json).expect("deserializa");
        assert_eq!(back, s);
        // Log antigo (campos faltando) → defaults, replay nunca quebra.
        let old: ProfWindowSummary =
            serde_json::from_str(r#"{"frames":60,"active_ms":600.0}"#).expect("aditivo");
        assert_eq!(old.frames, 60);
        assert_eq!(old.excess_ms, 0.0);
        assert_eq!(old.scale_factor, None);
        // Agregação entre janelas: (130+0)/(1320+600) — exato a partir dos brutos.
        let agg = (s.excess_ms + old.excess_ms) / (s.active_ms + old.active_ms) * 100.0;
        assert!((agg - 130.0 / 1920.0 * 100.0).abs() < 1e-9);
    }

    /// LoDPI (gap §II.8): 1ª observação loga; repetição NÃO loga (chamável todo frame);
    /// mudança loga de novo; escala 1x carrega o AVISO, 2x não; o resumo da janela carimba
    /// a escala em que se mediu. Sonda desligada: silêncio total.
    #[test]
    fn scale_factor_logs_on_change_and_warns_on_lodpi() {
        let mut probe = Probe::new(true);
        let first = probe.observe_scale_factor(1.0).expect("1ª observação loga");
        assert!(first.starts_with("[PROF] scale_factor=1.00"), "{first}");
        assert!(first.contains("AVISO"), "1x avisa: {first}");
        assert!(probe.observe_scale_factor(1.0).is_none(), "repetição cala");
        let hi = probe.observe_scale_factor(2.0).expect("mudança loga");
        assert!(!hi.contains("AVISO"), "2x não avisa: {hi}");

        let _ = run_frames(&mut probe, &[10u64; PROF_WINDOW]);
        let s = probe.take_window_summaries().pop().expect("janela fechou");
        assert_eq!(s.scale_factor, Some(2.0), "resumo carimba a escala");

        let mut off = Probe::new(false);
        assert!(off.observe_scale_factor(1.0).is_none(), "desligada cala");
    }

    /// O dreno de resumos CONSOME (segundo take = vazio) e a sonda DESLIGADA nunca acumula
    /// resumo — extensão do gating F1-5-1 às métricas novas.
    #[test]
    fn summaries_drain_consumes_and_respects_gate() {
        let mut probe = Probe::new(true);
        let _ = run_frames(&mut probe, &[10u64; PROF_WINDOW]);
        assert_eq!(probe.take_window_summaries().len(), 1);
        assert!(probe.take_window_summaries().is_empty(), "dreno consome");

        let mut off = Probe::new(false);
        let _ = run_frames(&mut off, &[10u64; PROF_WINDOW]);
        assert!(off.take_window_summaries().is_empty(), "desligada: nada");
    }
}
