//! W5-1 — instrumentação reutilizável do benchmark de carga (Trilho A, Onda 5).
//!
//! Mede a parte de CORE/PTY/RAM/threads/A2A que É observável headless. **NÃO** mede
//! FPS de render (o gpui não roda headless — ver `RELATORIO-W5-1.md`). O binário que
//! dirige a matriz N∈{10..50} vive em `src/bin/bench_load.rs`; aqui ficam os tijolos
//! reutilizáveis: amostragem de RAM (sysinfo) + threads (API de SO), agregação de
//! latência e classificação da curva de RSS — estas duas últimas são PURAS e testadas.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lina_cli_profiles::CliProfile;
use lina_pty::{PtyCommand, PtyManager};
use lina_vt::{AlacrittyBackend, VtBackend};
use serde::Serialize;
use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::{
    deliver_a2a, lock, A2aEnvelope, InjectPolicy, NodeId, Recipient, RolePolicy, Supervisor,
};

/// Grid VT compartilhado (mesmo alias de `app::bridge::Grid`): a reader-thread o avança
/// e o A2A o sente via `GridSense`.
pub type Grid = Arc<Mutex<Box<dyn VtBackend>>>;

/// Agregação de latência sobre uma amostra de durações. Método de percentil:
/// **nearest-rank** (1-indexado): `rank = ceil(p/100 · n)`, valor = `ordenado[rank-1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LatencyStats {
    pub n: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

impl LatencyStats {
    /// Agrega `samples` em p50/p95/max/mean (ms). Amostra vazia → tudo zero, `n = 0`.
    /// Independe da ordem de entrada (ordena internamente).
    #[must_use]
    pub fn from_durations(samples: &[Duration]) -> Self {
        let n = samples.len();
        if n == 0 {
            return Self {
                n: 0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                max_ms: 0.0,
                mean_ms: 0.0,
            };
        }
        let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms.sort_by(|a, b| a.total_cmp(b));

        // nearest-rank (1-indexado): rank = ceil(p/100 · n), valor = ordenado[rank-1].
        let pct = |p: f64| -> f64 {
            let rank = ((p / 100.0) * n as f64).ceil() as usize;
            ms[rank.clamp(1, n) - 1]
        };
        let sum: f64 = ms.iter().sum();
        Self {
            n,
            p50_ms: pct(50.0),
            p95_ms: pct(95.0),
            max_ms: ms[n - 1],
            mean_ms: sum / n as f64,
        }
    }
}

/// Veredito sobre a curva de RSS ao longo do tempo (item 3 da story: a RAM cresce
/// monotonicamente sob acúmulo de saída? → justifica o scrollback-cap do W5-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RssTrend {
    /// Cresceu além da tolerância relativa (RAM não estabiliza — precisa de cap).
    Growing,
    /// Variou dentro da tolerância (estabilizou).
    Plateau,
    /// Encolheu além da tolerância (liberou memória).
    Shrinking,
    /// Amostras insuficientes (< 2 pontos) para decidir.
    Insufficient,
}

/// Classifica a curva de RSS comparando o último ponto com o primeiro, com tolerância
/// relativa `rel_tol` (ex.: `0.05` = 5%). `< 2` pontos → [`RssTrend::Insufficient`].
#[must_use]
pub fn classify_rss_trend(series: &[u64], rel_tol: f64) -> RssTrend {
    if series.len() < 2 {
        return RssTrend::Insufficient;
    }
    let first = series[0];
    let last = series[series.len() - 1];
    let base = first.max(1) as f64;
    let rel = (last as f64 - first as f64) / base;
    if rel > rel_tol {
        RssTrend::Growing
    } else if rel < -rel_tol {
        RssTrend::Shrinking
    } else {
        RssTrend::Plateau
    }
}

// ───────────────────────── amostragem de processo (RAM + threads) ─────────────────────────

/// Snapshot do processo: RSS (bytes, via `sysinfo`) + contagem de threads do SO.
/// `threads = None` quando o SO não expõe (sysinfo NÃO reporta thread-count, então
/// caímos numa API nativa — `proc_pidinfo` no macOS, `/proc/self/status` no Linux).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProcStats {
    pub rss_bytes: u64,
    pub threads: Option<usize>,
}

/// Amostra o RSS (resident set, bytes) e o nº de threads do PRÓPRIO processo.
/// Reusa um `System` entre chamadas (refresh só do nosso PID — barato).
#[must_use]
pub fn sample_self(sys: &mut System) -> ProcStats {
    let rss_bytes = match get_current_pid() {
        Ok(pid) => {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::everything(),
            );
            sys.process(pid).map_or(0, sysinfo::Process::memory)
        }
        Err(_) => 0,
    };
    ProcStats {
        rss_bytes,
        threads: thread_count(),
    }
}

/// Threads vivas do processo, via API nativa do SO (sysinfo não expõe isto).
#[cfg(target_os = "macos")]
#[must_use]
pub fn thread_count() -> Option<usize> {
    // `proc_pidinfo(PROC_PIDTASKINFO)` devolve `proc_taskinfo`, cujo `pti_threadnum`
    // é o nº de threads kernel do processo — medição real, não estimativa. `getpid()`
    // devolve `c_int` direto (sem cast u32→i32 do `std::process::id()`).
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let ret = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            std::ptr::addr_of_mut!(info).cast::<libc::c_void>(),
            size,
        )
    };
    if ret == size {
        Some(info.pti_threadnum as usize)
    } else {
        None
    }
}

/// Threads vivas via `/proc/self/status` (campo `Threads:`).
#[cfg(target_os = "linux")]
#[must_use]
pub fn thread_count() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|v| v.trim().parse().ok())
}

/// Sem API nativa de thread-count fora de macOS/Linux (ex.: Windows fica p/ W5-6).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[must_use]
pub fn thread_count() -> Option<usize> {
    None
}

/// Amostra o RSS a cada `interval` durante `window`, devolvendo a série (bytes). Use
/// com terminais já em rajada para expor se a RAM cresce monotonicamente (item 3).
#[must_use]
pub fn rss_curve(sys: &mut System, window: Duration, interval: Duration) -> Vec<u64> {
    let start = Instant::now();
    let mut series = Vec::new();
    loop {
        series.push(sample_self(sys).rss_bytes);
        if start.elapsed() >= window {
            break;
        }
        thread::sleep(interval);
    }
    series
}

// ───────────────────────── harness de carga (N PTYs reais) ─────────────────────────

/// Um terminal vivo do harness: o `NodeId` no Supervisor + o grid VT compartilhado
/// (sensável por `deliver_a2a` via `GridSense`) + o handle da reader-thread.
pub struct LiveTerminal {
    node: NodeId,
    key: String,
    grid: Grid,
    reader: Option<JoinHandle<()>>,
}

impl LiveTerminal {
    /// `NodeId` deste terminal no roster do Supervisor.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }
    /// Grid VT compartilhado (o mesmo que a reader-thread avança e que o A2A sente).
    #[must_use]
    pub fn grid(&self) -> &Grid {
        &self.grid
    }
}

/// Harness headless que espelha `app::bridge::wire_terminal` (que vive no crate de UI,
/// fora do workspace): para cada terminal sobe um PTY real (`PtyManager`), dá o writer
/// ao `Supervisor` (1 thread serial-writer) e sobe a própria reader-thread (1 thread)
/// que avança o grid compartilhado — daí o esperado **~2N threads**. Não emite
/// `GridDelta` (não há consumidor de UI headless; emitir num canal não-drenado poluiria
/// a curva de RAM com backlog em vez de scrollback de VT).
pub struct LoadHarness {
    pty: PtyManager,
    sup: Arc<Supervisor>,
    cols: u16,
    rows: u16,
    terminals: Vec<LiveTerminal>,
}

impl LoadHarness {
    /// Harness vazio com terminais de `cols`×`rows`.
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            pty: PtyManager::new(),
            sup: Arc::new(Supervisor::new()),
            cols,
            rows,
            terminals: Vec::new(),
        }
    }

    /// O `Supervisor` compartilhado (para rotear/suspender/inspecionar de fora).
    #[must_use]
    pub fn supervisor(&self) -> &Arc<Supervisor> {
        &self.sup
    }

    /// Terminais vivos, em ordem de criação.
    #[must_use]
    pub fn live(&self) -> &[LiveTerminal] {
        &self.terminals
    }

    /// Spawna um terminal real rodando `cmd`: PTY → writer ao Supervisor (papel `role`)
    /// → reader-thread que avança o grid. Devolve o `NodeId` do Supervisor.
    pub fn spawn_terminal(
        &mut self,
        name: &str,
        role: &str,
        cmd: PtyCommand,
    ) -> anyhow::Result<NodeId> {
        let key = format!("bench-{}", self.terminals.len());
        self.pty.spawn(key.clone(), cmd, self.cols, self.rows)?;
        let writer = self.pty.take_writer(key.clone())?;
        let node = self.sup.register(name, Some(role.to_string()), writer);
        let reader = self.pty.clone_reader(key.clone())?;

        let grid: Grid = Arc::new(Mutex::new(Box::new(AlacrittyBackend::new(
            self.cols, self.rows,
        ))));
        let reader_grid = Arc::clone(&grid);
        let handle = thread::Builder::new()
            .name(format!("bench-reader-{key}"))
            .spawn(move || read_into_grid(reader, &reader_grid))?;

        self.terminals.push(LiveTerminal {
            node,
            key,
            grid,
            reader: Some(handle),
        });
        Ok(node)
    }

    /// Encerra todos os PTYs (SIGTERM→SIGKILL) e dá join nas reader-threads — o EOF do
    /// master destrava o loop de leitura. Idempotente.
    pub fn shutdown(&mut self) {
        let keys: Vec<String> = self.terminals.iter().map(|t| t.key.clone()).collect();
        for key in keys {
            let _ = self.pty.kill(key, Duration::from_secs(2));
        }
        for t in &mut self.terminals {
            if let Some(h) = t.reader.take() {
                let _ = h.join();
            }
        }
        self.terminals.clear();
    }
}

impl Drop for LoadHarness {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Loop da reader-thread headless: lê o master e avança o grid sob lock até EOF/erro
/// (master fechado quando o filho sai). Espelha o reader de `wire_terminal`, sem
/// emissão de `GridDelta`.
fn read_into_grid(mut reader: Box<dyn Read + Send>, grid: &Grid) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut g = lock(grid);
                g.advance(&buf[..n]);
                g.reset_damage();
            }
        }
    }
}

// ───────────────────────── medições A2A + culling ─────────────────────────

/// Resultado de uma rodada de fan-out A2A: as latências das entregas bem-sucedidas e o
/// nº de falhas (ex.: timeout de lock sob contenção extrema). Falhas NÃO são silenciadas;
/// são contadas e reportadas.
#[derive(Debug, Clone, Default)]
pub struct FanOutResult {
    pub latencies: Vec<Duration>,
    pub failures: usize,
}

/// Dispara `deliver_a2a` CONCORRENTE (fan-out real) de `from` para cada `(target, grid)`,
/// cronometrando cada entrega ponta-a-ponta (wait_ready → paste → submit_delay → Enter).
/// Cada alvo tem seu próprio lock de PTY, então a contenção medida é a do `Mutex` do
/// grid (reader vs. sensor) e dos locks internos do Supervisor — o sinal sensível a N.
#[must_use]
pub fn fan_out_a2a_latency(
    sup: &Arc<Supervisor>,
    from: NodeId,
    targets: &[(NodeId, Grid)],
    text: &str,
    profile: &Arc<CliProfile>,
) -> FanOutResult {
    let mut handles = Vec::with_capacity(targets.len());
    for (target, grid) in targets {
        let sup = Arc::clone(sup);
        let target = *target;
        let grid = Arc::clone(grid);
        let text = text.to_string();
        let profile = Arc::clone(profile);
        handles.push(thread::spawn(move || {
            let start = Instant::now();
            let ok = deliver_a2a(
                &sup,
                target,
                from,
                &text,
                &profile,
                &grid,
                InjectPolicy::AllowAll,
            )
            .is_ok();
            (start.elapsed(), ok)
        }));
    }
    let mut out = FanOutResult::default();
    for h in handles {
        match h.join() {
            Ok((dur, true)) => out.latencies.push(dur),
            Ok((_, false)) => out.failures += 1,
            // Panic numa thread de entrega NÃO é silenciado: loga e conta como falha
            // (não o confunde com uma falha normal de entrega/lock timeout).
            Err(_) => {
                eprintln!("[bench] thread de entrega A2A entrou em panic — contada como falha");
                out.failures += 1;
            }
        }
    }
    out
}

/// Quantos alvos vivos um broadcast de `from` resolve (excluindo o remetente). Base do
/// teste de **culling lógico**: nós não-vivos (suspensos via `mark_dead`) saem da
/// resolução → 0 trabalho de entrega de core. Não muta nada (usa `route`, que só lê o
/// roster e publica o evento de presença).
#[must_use]
pub fn broadcast_targets(sup: &Supervisor, from: NodeId) -> usize {
    let env = A2aEnvelope::new(from, Recipient::Broadcast, None);
    sup.route(&env, RolePolicy::All).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn latency_empty_is_all_zero() {
        let s = LatencyStats::from_durations(&[]);
        assert_eq!(s.n, 0);
        assert_eq!(s.p50_ms, 0.0);
        assert_eq!(s.p95_ms, 0.0);
        assert_eq!(s.max_ms, 0.0);
        assert_eq!(s.mean_ms, 0.0);
    }

    #[test]
    fn latency_single_sample_is_that_value() {
        let s = LatencyStats::from_durations(&[ms(10)]);
        assert_eq!(s.n, 1);
        assert_eq!(s.p50_ms, 10.0);
        assert_eq!(s.p95_ms, 10.0);
        assert_eq!(s.max_ms, 10.0);
        assert_eq!(s.mean_ms, 10.0);
    }

    #[test]
    fn latency_four_samples_nearest_rank() {
        // ordenado [10,20,30,40]: p50 rank=ceil(.5*4)=2 -> 20; p95 rank=ceil(.95*4)=4 -> 40.
        let s = LatencyStats::from_durations(&[ms(10), ms(20), ms(30), ms(40)]);
        assert_eq!(s.n, 4);
        assert_eq!(s.p50_ms, 20.0);
        assert_eq!(s.p95_ms, 40.0);
        assert_eq!(s.max_ms, 40.0);
        assert_eq!(s.mean_ms, 25.0);
    }

    #[test]
    fn latency_is_order_independent() {
        let a = LatencyStats::from_durations(&[ms(40), ms(10), ms(30), ms(20)]);
        let b = LatencyStats::from_durations(&[ms(10), ms(20), ms(30), ms(40)]);
        assert_eq!(a, b);
    }

    #[test]
    fn rss_trend_flat_is_plateau() {
        assert_eq!(
            classify_rss_trend(&[100, 100, 100], 0.05),
            RssTrend::Plateau
        );
    }

    #[test]
    fn rss_trend_doubling_is_growing() {
        assert_eq!(
            classify_rss_trend(&[100, 150, 200], 0.05),
            RssTrend::Growing
        );
    }

    #[test]
    fn rss_trend_small_wobble_within_tol_is_plateau() {
        // 100 -> 103 = +3% < 5% de tolerância.
        assert_eq!(classify_rss_trend(&[100, 103], 0.05), RssTrend::Plateau);
    }

    #[test]
    fn rss_trend_drop_is_shrinking() {
        assert_eq!(classify_rss_trend(&[200, 100], 0.05), RssTrend::Shrinking);
    }

    #[test]
    fn rss_trend_one_point_is_insufficient() {
        assert_eq!(classify_rss_trend(&[100], 0.05), RssTrend::Insufficient);
        assert_eq!(classify_rss_trend(&[], 0.05), RssTrend::Insufficient);
    }
}
