//! Scrollback com **cap por painel** + **paginação em disco** (W5-2).
//!
//! ## O problema
//! `(#painéis × scrollback)` sem teto é a classe de leak que estourou a RAM de Ghostty/Warp
//! (~41 GB) sob output torrencial de CLI de IA. Manter TODO o histórico de N terminais em RAM
//! não escala.
//!
//! ## A solução (duas camadas, complementares)
//! - **Camada VT** (`lina-vt`): o ring-buffer interno do alacritty (`Config::scrolling_history`)
//!   já descarta linhas antigas da RAM acima do cap — mas as PERDE.
//! - **Esta camada** (`lina-core`): o disco é o **log append-only** do scrollback (SQLite WAL,
//!   alinhado ao [`crate::EventStore`]); a RAM mantém só um **cache da cauda** de `cap` linhas
//!   (a janela viva / viewport rolável sem tocar o disco). Ao rolar além da janela, as páginas
//!   são **hidratadas do disco sob demanda** — transparente, no core, fora do render.
//!
//! ## Durabilidade: a fronteira honesta (F1-6-5)
//! A escrita é **write-behind em lote** — a durabilidade tem uma JANELA, não é "cada linha
//! instantânea no disco". O contrato real (cada item nomeia o teste verde que o prova):
//! - **Encerramento gracioso** (`Drop` do `ScrollbackStore`) e **sinais educados** (SIGTERM/
//!   SIGINT/SIGHUP, via `FlushGuard`) drenam TODO o write-behind pendente → **zero perda
//!   byte-idêntica** (`a_sigterm_com_pendentes_zero_perda_byte_identica`).
//! - Sob **`kill -9`** (SIGKILL, não-capturável) perde-se **no máximo** o write-behind ainda
//!   não-flushado; o `FlushGuard` (F1-5-6) encolhe essa janela para ~`idle_for` de output
//!   (1-2s) drenando os painéis ociosos (`b_idle_drain_persiste_em_ate_2s_visivel_a_leitor_externo`).
//!   É **cache de terminal**, NÃO estado de domínio — este vive no event log, durável à parte.
//! - O que JÁ foi flushado nunca se perde e reidrata byte-idêntico na reabertura
//!   (`paged_lines_hydrate_from_disk_byte_identical`, `reopen_continues_and_recovers_paged_line`).
//! - **Retenção (F1-5-9):** linhas além de `DEFAULT_RETENTION_DAYS` (30d) expiram no job diário;
//!   a leitura responde vazio + sinaliza `expired_before`, **nunca erro nem dado fantasma**
//!   (`ret_d_leitura_pos_expiracao_sinaliza_expirado_nao_erro`).
//!
//! ## Modelo de índices (por painel)
//! Cada linha tem um `idx` GLOBAL monotônico (0-based). Estado:
//! - `total`: nº de linhas empurradas (= próximo `idx`).
//! - `persisted`: linhas `[0, persisted)` já DURÁVEIS no disco.
//! - `tail_buf`: cache em RAM das linhas recentes, CONTÍGUO `[total − tail_buf.len(), total)`.
//!
//! **Invariante:** `total − tail_buf.len() ≤ persisted ≤ total` — toda linha NÃO-persistida está
//! no `tail_buf` (nunca é evictada antes de ir ao disco); o `tail_buf` pode AINDA cachear algumas
//! linhas já persistidas, até o teto `cap`. Escrita em disco é **write-behind em lote** (1 transação
//! a cada `flush_batch` linhas) — cada linha é gravada UMA vez; a janela permanece como cache.
//!
//! ## Concorrência
//! Single-thread (a `Connection` do rusqlite não é `Sync`) — mesmo contrato do [`crate::EventStore`].
//! O `scrollback.db` é um arquivo SEPARADO, mas um leitor externo (`lina check --tail N`) pode
//! abri-lo enquanto o app escreve → `busy_timeout` + retry-bounded na troca de `journal_mode=WAL`
//! (mesmo footgun do event store). Leitores de outro processo enxergam até o último `flush`.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// Cap **default provisório** de linhas mantidas em RAM por painel (janela viva). Espelha
/// `lina_vt::DEFAULT_SCROLLBACK_CAP`. **Provisório**: o benchmark W5-1 calibra o número final —
/// o entregável é o MECANISMO (cap + paginação), não o valor.
pub const DEFAULT_SCROLLBACK_CAP: usize = 10_000;

/// Linhas não-persistidas acumuladas antes de um flush em LOTE ao disco (1 transação). Limita o
/// nº de transações (perf) e o pico de RAM ENTRE flushes.
pub const DEFAULT_FLUSH_BATCH: usize = 2_000;

/// F1-5-9: retenção default do cache de output em DIAS (anti-"Warp 41GB"). `0` = retenção
/// DESLIGADA (nada expira) — escolha explícita, nunca "apagar tudo".
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Um dia em millis (janela do job de retenção).
const DAY_MS: u64 = 86_400_000;

/// Erros do scrollback store.
#[derive(Debug, Error)]
pub enum ScrollbackError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuração do scrollback store.
#[derive(Debug, Clone, Copy)]
pub struct ScrollbackConfig {
    /// Linhas mantidas em RAM por painel (janela viva). Acima disto, paginação em disco.
    pub cap: usize,
    /// Linhas não-persistidas acumuladas antes de um flush em lote ao disco.
    pub flush_batch: usize,
    /// F1-5-9: dias de retenção do cache de output no disco (default 30; configurável
    /// por workspace). Linhas mais velhas que isto somem no job diário. `0` = desligado.
    /// O event log do DOMÍNIO nunca passa por aqui (inv. #4 — isto é só o cache).
    pub retention_days: u32,
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self {
            cap: DEFAULT_SCROLLBACK_CAP,
            flush_batch: DEFAULT_FLUSH_BATCH,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

/// Buffer em-RAM de um painel: cache da cauda + contadores.
#[derive(Default)]
struct PanelBuffer {
    /// Cache CONTÍGUO das linhas recentes: `[total − tail_buf.len(), total)`. Inclui todas as
    /// linhas ainda não-persistidas, mais (até o cap) linhas já persistidas para scroll rápido.
    tail_buf: VecDeque<String>,
    /// Linhas já DURÁVEIS no disco: `[0, persisted)`.
    persisted: u64,
    /// Total de linhas já empurradas (= próximo `idx`).
    total: u64,
    /// F1-5-6: instante do ÚLTIMO `push_line` — o sinal de ociosidade do idle-drain
    /// (monotônico; `None` = painel reidratado sem output novo, nunca pendente).
    last_push: Option<Instant>,
    /// F1-5-9 (revisão): `ts` do último lote flushado — piso do carimbo do próximo
    /// lote (relógio de parede que recua nunca quebra o prefixo de expiração).
    /// Reidratado de `MAX(ts)` na abertura.
    last_ts: u64,
    /// F1-5-9: piso de expiração — linhas `[0, expired_before)` foram removidas pela
    /// retenção (leitura responde vazio + a UI/API sinalizam "expirado", nunca erro).
    expired_before: u64,
}

impl PanelBuffer {
    /// `idx` da 1ª linha do `tail_buf` (a fronteira RAM↔disco para leitura).
    fn tail_start(&self) -> u64 {
        self.total - self.tail_buf.len() as u64
    }
}

/// Store de scrollback de um workspace: SQLite WAL (`scrollback.db`) + cache da cauda por painel.
///
/// Mantém só `cap` linhas/painel em RAM; o resto é paginado e hidratado do disco sob demanda.
pub struct ScrollbackStore {
    conn: Connection,
    db_path: PathBuf,
    cfg: ScrollbackConfig,
    panels: BTreeMap<String, PanelBuffer>,
    /// F1-5-9: relógio INJETÁVEL (epoch ms) — carimba o `ts` dos lotes e decide o corte
    /// da retenção. Default: `SystemTime`. Testes injetam um relógio fixo (`set_clock`).
    clock: Box<dyn Fn() -> u64 + Send>,
    /// F1-5-9: último instante (ms do `clock`) em que o job de retenção rodou. Em-memória
    /// (zera no boot → o job roda no 1º tick do guard; um DELETE vazio é barato).
    last_retention_ms: u64,
    /// F1-5-6: métricas do drain (janela real de exposição do write-behind).
    stats: DrainStats,
}

/// F1-5-6: métricas observáveis do flush de durabilidade — "linhas pendentes no momento
/// do flush" é a medida da janela real de exposição a um crash duro.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrainStats {
    /// Nº de flushes disparados pelo idle-drain (1 por painel drenado).
    pub idle_drains: u64,
    /// Linhas TOTAIS persistidas pelos drains (idle + sinal).
    pub lines: u64,
    /// Linhas pendentes no momento do ÚLTIMO drain (idle ou sinal).
    pub last_pending: u64,
    /// Nº de `flush_all` disparados por SINAL (SIGTERM/SIGINT/SIGHUP).
    pub signal_flushes: u64,
}

/// F1-5-9: resultado de uma passada do job de retenção.
#[derive(Debug, Clone, Copy)]
pub struct RetentionReport {
    /// Linhas removidas do disco nesta passada.
    pub deleted: u64,
    /// O corte usado (`now − retention_days`), em epoch ms.
    pub cutoff_ms: u64,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS scrollback (
    panel TEXT    NOT NULL,
    idx   INTEGER NOT NULL,
    text  TEXT    NOT NULL,
    PRIMARY KEY (panel, idx)
);
CREATE TABLE IF NOT EXISTS scrollback_meta (
    panel          TEXT    PRIMARY KEY,
    next_idx       INTEGER NOT NULL,
    expired_before INTEGER NOT NULL DEFAULT 0
);
";

/// F1-5-9: migração IDEMPOTENTE do schema W5-2 → F1-5-9. A coluna `ts` (epoch ms, por
/// LOTE — decisão da story: mais barato e suficiente p/ retenção diária) entra com
/// `ALTER TABLE` tolerante a re-execução; linhas antigas (fixture/produção pré-F1-5-9)
/// ganham o `ts` da MIGRAÇÃO — honesto e documentado: elas começam a contar a partir
/// de agora, nunca expiram retroativamente no 1º boot.
fn migrate(conn: &Connection, now_ms: u64) -> Result<(), ScrollbackError> {
    let has_ts: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('scrollback') WHERE name = 'ts'",
        [],
        |r| r.get(0),
    )?;
    if has_ts == 0 {
        conn.execute_batch("ALTER TABLE scrollback ADD COLUMN ts INTEGER;")?;
    }
    // Índice de `ts` (revisão): sem ele, o DELETE diário da retenção e o UPDATE abaixo
    // são full-scan segurando o Mutex global do store — a BAIXA-iii que a story manda
    // não piorar. NULLs entram no índice → o `WHERE ts IS NULL` também o usa.
    conn.execute_batch("CREATE INDEX IF NOT EXISTS scrollback_ts ON scrollback(ts);")?;
    // Idempotente por construção: só carimba quem está NULL (re-execução é no-op).
    conn.execute(
        "UPDATE scrollback SET ts = ?1 WHERE ts IS NULL",
        params![now_ms as i64],
    )?;
    // Semeia a META dos painéis legados (revisão): um painel pré-F1-5-9 que expirar
    // INTEIRO antes de qualquer flush novo precisa do `next_idx` durável para a
    // sequência não regredir — e do piso `expired_before` para sinalizar "expirado".
    conn.execute_batch(
        "INSERT OR IGNORE INTO scrollback_meta (panel, next_idx, expired_before)
         SELECT panel, MAX(idx) + 1, 0 FROM scrollback GROUP BY panel;",
    )?;
    Ok(())
}

impl ScrollbackStore {
    /// Abre (ou cria) o store em `dir` com a config default.
    pub fn open_default(dir: impl AsRef<Path>) -> Result<Self, ScrollbackError> {
        Self::open(dir, ScrollbackConfig::default())
    }

    /// Abre (ou cria) o store em `dir`. Reidrata os contadores por painel a partir do disco
    /// (reabertura continua a numeração de `idx` de onde parou — nada se perde).
    pub fn open(dir: impl AsRef<Path>, cfg: ScrollbackConfig) -> Result<Self, ScrollbackError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join("scrollback.db");

        let conn = Connection::open(&db_path)?;
        // Mesmo footgun do EventStore: `busy_timeout` ANTES de qualquer escrita (a troca de
        // journal e o CREATE TABLE pegam write-lock) — um leitor externo (`lina check`) pode
        // disputar o lock. O WAL deixa leituras passarem em paralelo.
        conn.busy_timeout(Duration::from_millis(3000))?;
        enable_wal(&conn)?;
        conn.execute_batch(SCHEMA)?;
        // F1-5-9: schema antigo (W5-2) ganha `ts` aqui — idempotente, sem perda.
        migrate(&conn, system_now_ms())?;

        // Reabertura: `total` e `persisted` por painel vêm de MAX(idx)+1 do disco,
        // reconciliado com a META durável (F1-5-9): se a retenção apagou TODAS as linhas
        // de um painel, `next_idx` preserva a sequência — `idx` nunca regride. O cache da
        // cauda nasce VAZIO (será repovoado pelo stream de output); leituras caem no disco até lá.
        let mut panels: BTreeMap<String, PanelBuffer> = BTreeMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT panel, COALESCE(MAX(idx), -1) + 1, COALESCE(MAX(ts), 0)
                 FROM scrollback GROUP BY panel",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (panel, count, max_ts) = row?;
                let c = count.max(0) as u64;
                panels.insert(
                    panel,
                    PanelBuffer {
                        persisted: c,
                        total: c,
                        last_ts: max_ts.max(0) as u64,
                        ..Default::default()
                    },
                );
            }
        }
        {
            let mut stmt =
                conn.prepare("SELECT panel, next_idx, expired_before FROM scrollback_meta")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (panel, next_idx, expired_before) = row?;
                let pb = panels.entry(panel).or_default();
                let seq = next_idx.max(0) as u64;
                pb.total = pb.total.max(seq);
                pb.persisted = pb.persisted.max(seq);
                pb.expired_before = expired_before.max(0) as u64;
            }
        }

        Ok(Self {
            conn,
            db_path,
            cfg,
            panels,
            clock: Box::new(system_now_ms),
            last_retention_ms: 0,
            stats: DrainStats::default(),
        })
    }

    /// F1-5-9: injeta o relógio (epoch ms) usado no carimbo `ts` dos lotes e no corte da
    /// retenção. Seam de teste/simulação — produção fica no default (`SystemTime`).
    pub fn set_clock(&mut self, clock: impl Fn() -> u64 + Send + 'static) {
        self.clock = Box::new(clock);
    }

    /// Empurra uma linha de scrollback no painel. Write-behind: a linha vai ao cache da cauda;
    /// a cada `flush_batch` linhas não-persistidas, o lote é gravado no disco; linhas antigas JÁ
    /// persistidas são evictadas da RAM para manter a janela viva em ~`cap` (teto de RAM).
    pub fn push_line(
        &mut self,
        panel: &str,
        line: impl Into<String>,
    ) -> Result<(), ScrollbackError> {
        let flush_batch = self.cfg.flush_batch.max(1) as u64;
        let need_flush = {
            let pb = self.panels.entry(panel.to_string()).or_default();
            pb.tail_buf.push_back(line.into());
            pb.total += 1;
            // F1-5-6: o sinal de atividade do idle-drain — sob output contínuo o painel
            // nunca fica ocioso e o drain não dispara (critério d).
            pb.last_push = Some(Instant::now());
            pb.total - pb.persisted >= flush_batch
        };
        // Drena o write-behind em lote quando enche (fora do borrow de `pb`, pois toca o disco).
        if need_flush {
            self.flush(panel)?;
        }
        // Evicta da RAM as linhas da FRENTE já persistidas, até a janela voltar ao cap. NUNCA
        // evicta linha não-persistida (perderia dado antes do disco).
        let cap = self.cfg.cap;
        if let Some(pb) = self.panels.get_mut(panel) {
            while pb.tail_buf.len() > cap && pb.tail_start() < pb.persisted {
                pb.tail_buf.pop_front();
            }
        }
        Ok(())
    }

    /// Grava no disco, em UMA transação, as linhas ainda não-persistidas do painel (o SUFIXO
    /// não-persistido do cache da cauda). As linhas PERMANECEM no cache (são só espelhadas);
    /// em erro, `persisted` não avança → nada se perde e o próximo flush re-tenta.
    pub fn flush(&mut self, panel: &str) -> Result<(), ScrollbackError> {
        let (start, lines, ts_floor) = match self.panels.get_mut(panel) {
            Some(pb) if pb.total > pb.persisted => {
                let n = (pb.total - pb.persisted) as usize;
                let from = pb.tail_buf.len() - n; // as últimas `n` linhas do cache são as não-persistidas
                let lines: Vec<String> = pb.tail_buf.iter().skip(from).cloned().collect();
                (pb.persisted, lines, pb.last_ts)
            }
            _ => return Ok(()),
        };

        // (revisão) `ts` MONOTÔNICO por painel: o relógio de parede pode recuar
        // (NTP/ajuste manual); um lote novo nunca carimba ts menor que o anterior —
        // assim o conjunto expirado é sempre um PREFIXO de idx e `expired_before`
        // nunca mente ("dado fantasma" invertido do critério B-d).
        let ts = (self.clock)().max(ts_floor);
        write_batch(&mut self.conn, panel, start, &lines, ts)?;
        if let Some(pb) = self.panels.get_mut(panel) {
            pb.persisted += lines.len() as u64;
            pb.last_ts = ts;
        }
        Ok(())
    }

    /// Paga em disco o write-behind de TODOS os painéis.
    pub fn flush_all(&mut self) -> Result<(), ScrollbackError> {
        let panels: Vec<String> = self.panels.keys().cloned().collect();
        for p in panels {
            self.flush(&p)?;
        }
        Ok(())
    }

    /// `wal_checkpoint(TRUNCATE)` — materializa o WAL no `.db` (usado para medir o tamanho do
    /// arquivo de paginação de forma estável).
    pub fn checkpoint(&mut self) -> Result<(), ScrollbackError> {
        self.conn
            .pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        Ok(())
    }

    /// Total de linhas já empurradas no painel (= próximo `idx`).
    #[must_use]
    pub fn total_lines(&self, panel: &str) -> u64 {
        self.panels.get(panel).map_or(0, |pb| pb.total)
    }

    /// Linhas já PERSISTIDAS no disco para o painel (contador em-RAM).
    #[must_use]
    pub fn disk_line_count(&self, panel: &str) -> u64 {
        self.panels.get(panel).map_or(0, |pb| pb.persisted)
    }

    /// Nº REAL de linhas paginadas no disco (`COUNT(*)` — medição direta da tabela).
    pub fn disk_rows(&self, panel: &str) -> Result<u64, ScrollbackError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM scrollback WHERE panel = ?1",
            params![panel],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// Linha de índice GLOBAL `idx` do painel, hidratando do disco se preciso. `None` se `idx`
    /// não existe. Serve do cache da cauda (RAM) se recente; senão hidrata do disco.
    pub fn line(&self, panel: &str, idx: u64) -> Result<Option<String>, ScrollbackError> {
        let Some(pb) = self.panels.get(panel) else {
            return Ok(None);
        };
        if idx >= pb.total {
            return Ok(None);
        }
        let tail_start = pb.tail_start();
        if idx >= tail_start {
            return Ok(pb.tail_buf.get((idx - tail_start) as usize).cloned());
        }
        // Hidrata do disco (linha já fora da janela viva; `idx < tail_start ≤ persisted`).
        let text = self
            .conn
            .query_row(
                "SELECT text FROM scrollback WHERE panel = ?1 AND idx = ?2",
                params![panel, idx as i64],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(text)
    }

    /// Faixa `[lo, hi)` de linhas (clampada a `[0, total)`), atravessando disco→janela viva.
    /// O trecho em disco vem em UMA query (não linha-a-linha).
    pub fn range(&self, panel: &str, lo: u64, hi: u64) -> Result<Vec<String>, ScrollbackError> {
        let Some(pb) = self.panels.get(panel) else {
            return Ok(Vec::new());
        };
        let hi = hi.min(pb.total);
        let lo = lo.min(hi);
        if lo == hi {
            return Ok(Vec::new());
        }
        let tail_start = pb.tail_start();
        let mut out = Vec::with_capacity((hi - lo) as usize);

        // Disco: [lo, min(hi, tail_start)).
        let disk_hi = hi.min(tail_start);
        if lo < disk_hi {
            let mut stmt = self.conn.prepare(
                "SELECT text FROM scrollback WHERE panel = ?1 AND idx >= ?2 AND idx < ?3 ORDER BY idx ASC",
            )?;
            let rows = stmt.query_map(params![panel, lo as i64, disk_hi as i64], |r| {
                r.get::<_, String>(0)
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        // Janela viva (RAM): [max(lo, tail_start), hi).
        let ram_lo = lo.max(tail_start);
        for i in ram_lo..hi {
            out.push(pb.tail_buf[(i - tail_start) as usize].clone());
        }
        Ok(out)
    }

    /// As últimas `n` linhas do painel (base do `lina check --tail N`), hidratando do disco o que
    /// não estiver na janela viva.
    pub fn tail(&self, panel: &str, n: usize) -> Result<Vec<String>, ScrollbackError> {
        let total = self.total_lines(panel);
        let lo = total.saturating_sub(n as u64);
        self.range(panel, lo, total)
    }

    /// Bytes de linha REALMENTE retidos em RAM por este painel (cache da cauda):
    /// `Σ (text.len() + size_of::<String>())`. É a "RAM do painel" — medição direta do que o
    /// store segura no heap, independente de quantas linhas já passaram. NÃO inclui o cache de
    /// páginas do SQLite (isso é RSS de processo, medido à parte).
    #[must_use]
    pub fn ram_bytes(&self, panel: &str) -> usize {
        let overhead = std::mem::size_of::<String>();
        self.panels
            .get(panel)
            .map_or(0, |pb| pb.tail_buf.iter().map(|l| l.len() + overhead).sum())
    }

    /// Caminho do arquivo de paginação (`scrollback.db`).
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Cap configurado (linhas/painel em RAM).
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cfg.cap
    }

    /// F3-5-7 (`BufferGauge`, spec 53 §11.A): LEITURAS de ocupação — uma por painel
    /// (`scrollback:<panel>`). `used` = linhas na janela viva = `min(total, cap)` (satura no cap
    /// quando o backlog cresce — o resto pagina para o disco); `capacity` = `cap`; unidade
    /// `"lines"`. É SÓ leitura crua: o amostrador (`buffer_registry::sample`) aplica a
    /// anti-amplificação e quem tem o `EventStore` (o supervisor, no tick do `FlushGuard`) apenda o
    /// `BufferOccupancySampled`. Não toca o event log (inv #5 — gauge é projeção, não 2º log).
    /// Ordem estável por `buffer_id`.
    #[must_use]
    pub fn gauge_readings(&self) -> Vec<crate::buffer_registry::GaugeReading> {
        let cap = self.cfg.cap as u64;
        self.panels
            .iter()
            .map(|(panel, pb)| crate::buffer_registry::GaugeReading {
                buffer_id: format!("scrollback:{panel}"),
                used: pb.total.min(cap),
                capacity: cap,
                unit: "lines".to_string(),
            })
            .collect()
    }

    // ───────────────────── F1-5-6: idle-drain + métricas ─────────────────────

    /// F1-5-6: linhas no write-behind (ainda NÃO duráveis) do painel — a janela de
    /// exposição a um crash duro neste instante.
    #[must_use]
    pub fn pending_lines(&self, panel: &str) -> u64 {
        self.panels
            .get(panel)
            .map_or(0, |pb| pb.total - pb.persisted)
    }

    /// F1-5-6: soma do write-behind pendente de TODOS os painéis.
    #[must_use]
    pub fn pending_total(&self) -> u64 {
        self.panels.values().map(|pb| pb.total - pb.persisted).sum()
    }

    /// F1-5-6: métricas do drain de durabilidade (idle + sinal).
    #[must_use]
    pub fn drain_stats(&self) -> DrainStats {
        self.stats
    }

    /// F1-5-6: drena o write-behind dos painéis OCIOSOS — sem `push_line` há pelo menos
    /// `idle_for` E com pendência. Sob output torrencial o `last_push` se renova a cada
    /// linha e o drain NUNCA dispara (não é flush-por-linha disfarçado — critério d).
    /// Devolve o nº de linhas persistidas nesta passada.
    pub fn drain_idle(&mut self, idle_for: Duration) -> Result<u64, ScrollbackError> {
        let now = Instant::now();
        let idle: Vec<(String, u64)> = self
            .panels
            .iter()
            .filter_map(|(name, pb)| {
                let pending = pb.total - pb.persisted;
                let ocioso = pb
                    .last_push
                    .is_some_and(|t| now.duration_since(t) >= idle_for);
                (pending > 0 && ocioso).then(|| (name.clone(), pending))
            })
            .collect();
        let mut drained = 0u64;
        for (panel, pending) in idle {
            self.flush(&panel)?;
            self.stats.idle_drains += 1;
            self.stats.lines += pending;
            self.stats.last_pending = pending;
            drained += pending;
        }
        Ok(drained)
    }

    /// F1-5-6: `flush_all` do caminho de SINAL — registra a métrica (linhas pendentes no
    /// momento do flush = a janela que o handler salvou) antes de persistir.
    pub fn flush_all_for_signal(&mut self) -> Result<(), ScrollbackError> {
        let pending = self.pending_total();
        self.flush_all()?;
        self.stats.signal_flushes += 1;
        self.stats.lines += pending;
        self.stats.last_pending = pending;
        Ok(())
    }

    // ───────────────────── F1-5-9: retenção configurável ─────────────────────

    /// F1-5-9: piso de expiração do painel — linhas `[0, expired_before)` foram removidas
    /// pela retenção. É O SINAL para UI/API responderem "histórico expirado" (a leitura
    /// em si devolve vazio/`None`, nunca erro).
    #[must_use]
    pub fn expired_before(&self, panel: &str) -> u64 {
        self.panels.get(panel).map_or(0, |pb| pb.expired_before)
    }

    /// F1-5-9: roda o job de retenção AGORA: remove do disco linhas com `ts` além de
    /// `retention_days` (relógio injetável), atualiza os pisos `expired_before` (durável,
    /// na meta) e evicta do cache em RAM o prefixo expirado — "expirado" some em TODO
    /// lugar, não só no disco. `retention_days == 0` → desligado (no-op).
    ///
    /// A sequência de `idx` NUNCA regride: `next_idx` na meta sobrevive até à expiração
    /// total do painel (reabertura continua de onde parou). O tamanho do `.db` estabiliza
    /// por REUSO de páginas livres (freelist) — decisão pelo custo medido: `VACUUM` só
    /// encolhe o arquivo, não muda a propriedade anti-crescimento, e custa O(db).
    pub fn run_retention(&mut self) -> Result<RetentionReport, ScrollbackError> {
        let now = (self.clock)();
        self.last_retention_ms = now;
        if self.cfg.retention_days == 0 {
            return Ok(RetentionReport {
                deleted: 0,
                cutoff_ms: 0,
            });
        }
        let cutoff = now.saturating_sub(u64::from(self.cfg.retention_days) * DAY_MS);
        let deleted = self.conn.execute(
            "DELETE FROM scrollback WHERE ts IS NOT NULL AND ts < ?1",
            params![cutoff as i64],
        )? as u64;
        if deleted > 0 {
            // Piso durável por painel: o MIN(idx) sobrevivente — ou `next_idx` quando o
            // painel expirou INTEIRO (max(x,y) escalar do SQLite preserva pisos antigos).
            self.conn.execute_batch(
                "UPDATE scrollback_meta SET expired_before = MAX(expired_before,
                    COALESCE((SELECT MIN(idx) FROM scrollback
                              WHERE scrollback.panel = scrollback_meta.panel), next_idx));",
            )?;
            // Re-deriva os pisos em memória e evicta do cache o prefixo expirado (só
            // linhas JÁ persistidas — o invariante do write-behind segue intacto).
            let floors: Vec<(String, u64)> = {
                let mut stmt = self
                    .conn
                    .prepare("SELECT panel, expired_before FROM scrollback_meta")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                let mut out = Vec::new();
                for row in rows {
                    let (panel, floor) = row?;
                    out.push((panel, floor.max(0) as u64));
                }
                out
            };
            for (panel, floor) in floors {
                if let Some(pb) = self.panels.get_mut(&panel) {
                    pb.expired_before = floor;
                    while pb.tail_start() < floor && pb.tail_start() < pb.persisted {
                        pb.tail_buf.pop_front();
                    }
                }
            }
        }
        Ok(RetentionReport {
            deleted,
            cutoff_ms: cutoff,
        })
    }

    /// F1-5-9: o gatilho DIÁRIO do job (chamado a cada tick do idle-drain — thread única,
    /// decisão da story): roda a retenção se passou ≥1 dia do relógio injetável desde a
    /// última passada. No boot (`last_retention_ms == 0`) roda na primeira oportunidade.
    pub fn maybe_run_retention(&mut self) -> Result<Option<RetentionReport>, ScrollbackError> {
        if self.cfg.retention_days == 0 {
            return Ok(None);
        }
        let now = (self.clock)();
        if now.saturating_sub(self.last_retention_ms) >= DAY_MS || self.last_retention_ms == 0 {
            return Ok(Some(self.run_retention()?));
        }
        Ok(None)
    }
}

impl Drop for ScrollbackStore {
    fn drop(&mut self) {
        // Close gracioso: persiste o write-behind pendente (invariante #6 — estado salvo). Em
        // crash duro (kill -9) perde-se no máximo o último lote não-flushado de output (cache de
        // terminal, não estado de domínio — este vive no event log). Com o `FlushGuard` ativo
        // (F1-5-6), a janela real encolhe para ~`idle_for` (1-2s) de output + sinais cobertos.
        let _ = self.flush_all();
    }
}

// ───────────────────────────── helpers ─────────────────────────────

/// Grava `lines` (índices `start..start+len`) do `panel` em UMA transação (write-behind durável).
/// F1-5-9: o lote inteiro carimba o MESMO `ts` (decisão da story: por lote, não por linha) e a
/// META durável (`next_idx`) avança NA MESMA transação — a sequência de `idx` sobrevive até à
/// expiração TOTAL do painel (o `expired_before` existente é preservado no upsert).
fn write_batch(
    conn: &mut Connection,
    panel: &str,
    start: u64,
    lines: &[String],
    ts_ms: u64,
) -> Result<(), ScrollbackError> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO scrollback (panel, idx, text, ts) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (i, line) in lines.iter().enumerate() {
            stmt.execute(params![
                panel,
                (start + i as u64) as i64,
                line,
                ts_ms as i64
            ])?;
        }
        tx.execute(
            "INSERT INTO scrollback_meta (panel, next_idx, expired_before) VALUES (?1, ?2, 0)
             ON CONFLICT(panel) DO UPDATE SET next_idx = excluded.next_idx",
            params![panel, (start + lines.len() as u64) as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Epoch ms do relógio de sistema (o default do `clock` injetável).
fn system_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `true` se o erro do rusqlite é `SQLITE_BUSY` (disputa de lock transitória).
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _) if err.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

/// Liga `journal_mode=WAL` tolerando a corrida de setup entre conexões (mesmo footgun do
/// EventStore: a troca de journal não honra `busy_timeout` de forma confiável). Retry BOUNDED.
fn enable_wal(conn: &Connection) -> Result<(), ScrollbackError> {
    const TRIES: u32 = 50; // ~50 × 20ms = 1s
    let mut left = TRIES;
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) && left > 1 => {
                left -= 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// **Pico de RSS do processo** (best-effort, unix) — corroboração da medição de RAM. Em macOS
/// `ru_maxrss` vem em BYTES; em Linux/BSD, em KILOBYTES. Pico (não decai) basta para provar que
/// nunca estouramos a RAM: com o cap, fica ordens de grandeza abaixo do unbounded.
#[cfg(unix)]
#[must_use]
pub fn peak_rss_bytes() -> Option<u64> {
    // SAFETY: `getrusage` só ESCREVE em `usage` (struct zerada do tamanho correto, via libc) e
    // lê a constante `RUSAGE_SELF`. Sem aliasing nem ponteiros emprestados.
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return None;
        }
        let maxrss = usage.ru_maxrss as u64;
        let bytes = if cfg!(target_os = "macos") {
            maxrss // já em bytes
        } else {
            maxrss.saturating_mul(1024) // KiB → bytes
        };
        Some(bytes)
    }
}

/// Em não-unix, o pico de RSS não é coletado aqui (a medição-gate é [`ScrollbackStore::ram_bytes`]).
#[cfg(not(unix))]
#[must_use]
pub fn peak_rss_bytes() -> Option<u64> {
    None
}

// ───────────────────── F1-5-6: FlushGuard (job ÚNICO de durabilidade) ─────────────────────

/// F1-5-6: o último sinal fatal recebido e ainda não tratado (0 = nenhum). O handler SÓ
/// grava este atômico — único trabalho async-signal-safe possível ali (flush toca SQLite
/// e Mutex, proibidos em handler); a thread do guard observa e faz o resto.
#[cfg(unix)]
static PENDING_SIGNAL: AtomicI32 = AtomicI32::new(0);

/// Handler instalado para `SIGTERM`/`SIGINT`/`SIGHUP`: grava o nº do sinal e retorna.
/// O processo NÃO morre aqui — o guard drena o write-behind e re-emite o sinal com a
/// disposição default (o pai vê a morte pelo sinal correto).
#[cfg(unix)]
extern "C" fn flush_signal_handler(sig: libc::c_int) {
    PENDING_SIGNAL.store(sig, Ordering::SeqCst);
}

/// Instala o handler de flush para os três sinais de término "educado".
#[cfg(unix)]
/// (revisão) Os três sinais de término "educado" e o bit de cada um na máscara de
/// instalados (para o último Drop restaurar SÓ o que foi instalado).
#[cfg(unix)]
const FLUSH_SIGNALS: [(libc::c_int, u32); 3] = [
    (libc::SIGTERM, 0b001),
    (libc::SIGINT, 0b010),
    (libc::SIGHUP, 0b100),
];

/// (revisão) Guards vivos que pediram sinais: o ÚLTIMO Drop restaura a disposição
/// default — sem isso, um guard dropado deixaria o processo "imatável" (o handler
/// gravaria a flag e ninguém re-emitiria o sinal).
#[cfg(unix)]
static SIGNAL_GUARDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// (revisão) Bitmask dos sinais cujo handler FOI instalado de fato (`SIG_IGN` herdado
/// é respeitado — convenção POSIX de `nohup`/background-shell: o bit fica 0).
#[cfg(unix)]
static INSTALLED_MASK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(unix)]
fn install_flush_signal_handlers() {
    // SAFETY: `signal(2)` com um handler `extern "C"` que só faz um store atômico
    // (async-signal-safe). O cast fn→ptr→sighandler_t é o contrato da API do libc.
    // A disposição ANTERIOR é consultada: `SIG_IGN` herdado (nohup, shell sem
    // job-control) é restaurado na hora — quem pediu para ignorar segue ignorando.
    unsafe {
        for (sig, bit) in FLUSH_SIGNALS {
            let prev = libc::signal(sig, flush_signal_handler as *const () as libc::sighandler_t);
            if prev == libc::SIG_IGN {
                libc::signal(sig, libc::SIG_IGN);
            } else {
                INSTALLED_MASK.fetch_or(bit, Ordering::SeqCst);
            }
        }
    }
}

/// (revisão) Restaura `SIG_DFL` para os sinais que NÓS instalamos — chamado pelo
/// Drop do ÚLTIMO guard com sinais (o processo volta ao comportamento default).
#[cfg(unix)]
fn uninstall_flush_signal_handlers() {
    // SAFETY: restaura a disposição default apenas dos sinais marcados na máscara.
    unsafe {
        let mask = INSTALLED_MASK.swap(0, Ordering::SeqCst);
        for (sig, bit) in FLUSH_SIGNALS {
            if mask & bit != 0 {
                libc::signal(sig, libc::SIG_DFL);
            }
        }
    }
}

/// Configuração do [`FlushGuard`].
#[derive(Debug, Clone, Copy)]
pub struct FlushGuardConfig {
    /// Painel sem `push_line` por este intervalo (e com pendência) → `flush(panel)`.
    /// A story fixa 1-2s; default 1.5s.
    pub idle_for: Duration,
    /// Período do tick do job (latência máxima do caminho de sinal e do idle-check).
    pub tick: Duration,
    /// Instala os handlers de `SIGTERM`/`SIGINT`/`SIGHUP` → `flush_all` antes de morrer.
    /// (Unix; no Windows o equivalente — console ctrl handler — é costura pós-bring-up.)
    pub handle_signals: bool,
}

impl Default for FlushGuardConfig {
    fn default() -> Self {
        Self {
            idle_for: Duration::from_millis(1_500),
            tick: Duration::from_millis(250),
            handle_signals: true,
        }
    }
}

/// F1-5-6: o job ÚNICO de durabilidade do scrollback — **uma** thread para todos os
/// painéis (nota load-bearing do 13.16: nunca uma thread por painel disputando o Mutex
/// global do store — a BAIXA-iii do red-team não piora: este é +1 cliente breve por tick,
/// pela MESMA serialização de sempre). Responsabilidades, no mesmo loop:
/// 1. idle-drain: painel ocioso 1-2s com write-behind → `flush(panel)`;
/// 2. sinais fatais: `SIGTERM`/`SIGINT`/`SIGHUP` → `flush_all` + re-raise (zero perda);
/// 3. F1-5-9: o job DIÁRIO de retenção (sem segunda thread).
///
/// Pare-o dropando (sinaliza e dá join). O `Drop` do store continua cobrindo o
/// encerramento limpo; o guard cobre o que o `Drop` não vê (sinal fatal, ociosidade).
pub struct FlushGuard {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    /// O guard pediu handlers de sinal (participa do contador global e da
    /// restauração no Drop).
    handle_signals: bool,
}

impl FlushGuard {
    /// Sobe a thread do guard sobre o store COMPARTILHADO do workspace (a mesma
    /// serialização dos reader-threads do `PtyHost`).
    ///
    /// **Contrato de sinais (revisão):** os handlers são INSTALADOS só depois de a
    /// thread subir (falha de spawn não deixa handler órfão) e são processo-globais —
    /// no máximo **um caminho de sinal por processo**: com 2+ guards (futuro
    /// multi-workspace, ADR 0010), o guard que observar o sinal drena só o SEU store
    /// antes do re-raise; quando isso virar caso real, o caminho de sinal precisa de
    /// um registro de stores (registrado na entrega da fatia). `SIG_IGN` herdado
    /// (nohup) é respeitado por sinal.
    ///
    /// ⚠️ Nunca drope o guard segurando o lock do store NA MESMA thread: o Drop dá
    /// `join` numa thread que pega esse lock — deadlock garantido.
    pub fn start(
        store: Arc<Mutex<ScrollbackStore>>,
        cfg: FlushGuardConfig,
    ) -> Result<Self, ScrollbackError> {
        let stop = Arc::new(AtomicBool::new(false));
        let join = std::thread::Builder::new()
            .name("lina-scrollback-flush-guard".into())
            .spawn({
                let stop = Arc::clone(&stop);
                move || guard_loop(&store, cfg, &stop)
            })?;
        #[cfg(unix)]
        if cfg.handle_signals {
            SIGNAL_GUARDS.fetch_add(1, Ordering::SeqCst);
            install_flush_signal_handlers();
        }
        Ok(Self {
            stop,
            join: Some(join),
            handle_signals: cfg.handle_signals,
        })
    }
}

impl Drop for FlushGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        // (revisão) O ÚLTIMO guard com sinais restaura a disposição default — um
        // processo sem guard nunca fica "imatável" (handler gravando flag que ninguém
        // consome). Na SUBSTITUIÇÃO (start_flush_guard re-chamado), o guard novo já
        // incrementou o contador antes deste Drop → nada é desinstalado.
        #[cfg(unix)]
        if self.handle_signals && SIGNAL_GUARDS.fetch_sub(1, Ordering::SeqCst) == 1 {
            uninstall_flush_signal_handlers();
        }
    }
}

/// O loop do guard (ver [`FlushGuard`]). Erros de disco são LOGADOS e a próxima passada
/// re-tenta (o write-behind não avança em erro — nada se perde em silêncio).
fn guard_loop(store: &Arc<Mutex<ScrollbackStore>>, cfg: FlushGuardConfig, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        #[cfg(unix)]
        {
            let sig = PENDING_SIGNAL.swap(0, Ordering::SeqCst);
            if sig != 0 {
                {
                    let mut s = crate::lock(store);
                    if let Err(e) = s.flush_all_for_signal() {
                        tracing::error!(%sig, error = %e, "flush de sinal falhou — write-behind pode se perder");
                    } else {
                        tracing::info!(
                            %sig,
                            pendentes = s.drain_stats().last_pending,
                            "scrollback drenado pelo handler de sinal"
                        );
                    }
                }
                // SAFETY: restaura a disposição DEFAULT e re-emite o MESMO sinal — o
                // processo morre com a semântica correta (o pai vê SIGTERM/SIGINT/SIGHUP).
                unsafe {
                    libc::signal(sig, libc::SIG_DFL);
                    libc::raise(sig);
                }
                return; // inalcançável: a disposição default dos 3 sinais termina o processo
            }
        }
        {
            let mut s = crate::lock(store);
            if let Err(e) = s.drain_idle(cfg.idle_for) {
                tracing::warn!(error = %e, "idle-drain falhou; re-tenta no próximo tick");
            }
            if let Err(e) = s.maybe_run_retention() {
                tracing::warn!(error = %e, "job de retenção falhou; re-tenta no próximo tick");
            }
        }
        std::thread::sleep(cfg.tick);
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// Diretório temporário único; removido no Drop (best-effort).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-sb-{tag}-{}", Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
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

    /// Linhas curtas vão e voltam pela RAM (abaixo do cap, sem tocar o disco).
    #[test]
    fn push_and_read_from_ram_roundtrip() {
        let tmp = TempDir::new("rt");
        let mut s = ScrollbackStore::open(
            tmp.path(),
            ScrollbackConfig {
                cap: 100,
                flush_batch: 8,
                retention_days: 30,
            },
        )
        .expect("open");
        let p = "A";
        for i in 0..10u64 {
            s.push_line(p, format!("l{i}")).unwrap();
        }
        assert_eq!(s.total_lines(p), 10);
        assert_eq!(s.line(p, 0).unwrap().as_deref(), Some("l0"));
        assert_eq!(s.line(p, 9).unwrap().as_deref(), Some("l9"));
        assert_eq!(s.line(p, 10).unwrap(), None, "idx além do total → None");
        assert_eq!(s.tail(p, 3).unwrap(), vec!["l7", "l8", "l9"]);
    }

    /// Gate (d) F3-5: 12.000 linhas (cap 10.000) → a leitura do gauge `scrollback:<panel>` SATURA
    /// no cap (`used==cap`, pressão ≈ 1,0) e as linhas além do cap reidratam do disco byte-idênticas.
    #[test]
    fn gauge_reading_saturates_at_cap_and_rehydrates_byte_identical() {
        use crate::buffer_registry::{self, BufferRegistry};
        let tmp = TempDir::new("gauge");
        let mut s = ScrollbackStore::open(
            tmp.path(),
            ScrollbackConfig {
                cap: 10_000,
                flush_batch: 2_000,
                retention_days: 30,
            },
        )
        .expect("open");
        let p = "T";
        let pushed: Vec<String> = (0..12_000u64).map(|i| format!("linha-{i:05}")).collect();
        for l in &pushed {
            s.push_line(p, l.clone()).unwrap();
        }
        s.flush(p).unwrap();

        // A leitura crua satura no cap mesmo com 12k empurradas (RAM = janela viva de `cap`).
        let readings = s.gauge_readings();
        assert_eq!(readings.len(), 1, "um buffer por painel");
        let r = &readings[0];
        assert_eq!(r.buffer_id, "scrollback:T");
        assert_eq!(r.capacity, 10_000);
        assert_eq!(r.used, 10_000, "RAM satura no cap mesmo com 12k empurradas");
        assert_eq!(r.unit, "lines");
        assert!(
            (buffer_registry::pressure_ratio(r.used, r.capacity) - 1.0).abs() < f32::EPSILON,
            "pressão ≈ 1,0 no cap"
        );

        // As 12k linhas voltam byte-idênticas (disco paginado + janela viva).
        let back = s.range(p, 0, 12_000).unwrap();
        assert_eq!(back, pushed, "12k linhas reidratam byte-idênticas");

        // A amostragem produz exatamente 1 evento; a projeção reproduz a pressão ≈ 1,0.
        let evs = buffer_registry::sample(
            &BufferRegistry::default(),
            &readings,
            buffer_registry::GAUGE_WARN_RATIO,
            1,
        );
        assert_eq!(evs.len(), 1, "1ª amostra do buffer emite 1 evento");
    }

    /// Acima do cap, as linhas antigas são PAGINADAS e hidratadas do disco byte-idênticas;
    /// `tail`/`range` atravessam disco→janela na ordem certa.
    #[test]
    fn paged_lines_hydrate_from_disk_byte_identical() {
        let tmp = TempDir::new("page");
        let mut s = ScrollbackStore::open(
            tmp.path(),
            ScrollbackConfig {
                cap: 4,
                flush_batch: 2,
                retention_days: 30,
            },
        )
        .expect("open");
        let p = "term";
        for i in 0..20u64 {
            s.push_line(p, format!("linha-{i:03}")).unwrap();
        }
        s.flush(p).unwrap();

        assert_eq!(s.total_lines(p), 20);
        assert!(
            s.disk_line_count(p) >= 16,
            "esperava ≥16 paginadas (20 - cap 4)"
        );

        // Linha bem antiga: só na RAM se ≥ tail_start; aqui já fora da janela → hidrata do disco.
        assert_eq!(s.line(p, 0).unwrap().as_deref(), Some("linha-000"));
        assert_eq!(s.line(p, 7).unwrap().as_deref(), Some("linha-007"));
        // Linha recente: na janela viva.
        assert_eq!(s.line(p, 19).unwrap().as_deref(), Some("linha-019"));

        // tail(6) cruza disco (14..16) + janela (16..20).
        let tail = s.tail(p, 6).unwrap();
        let expected: Vec<String> = (14..20).map(|i| format!("linha-{i:03}")).collect();
        assert_eq!(tail, expected);

        // range arbitrário cruzando a fronteira disco/janela.
        let r = s.range(p, 2, 18).unwrap();
        let exp: Vec<String> = (2..18).map(|i| format!("linha-{i:03}")).collect();
        assert_eq!(r, exp);
    }

    /// Reabrir o store do disco continua a numeração e recupera linha paginada byte-idêntica.
    #[test]
    fn reopen_continues_and_recovers_paged_line() {
        let tmp = TempDir::new("reopen");
        let cfg = ScrollbackConfig {
            cap: 4,
            flush_batch: 4,
            retention_days: 30,
        };
        {
            let mut s = ScrollbackStore::open(tmp.path(), cfg).expect("open");
            for i in 0..20u64 {
                s.push_line("X", format!("v{i}")).unwrap();
            }
            s.flush("X").unwrap();
        } // dropa → conexão fechada

        let mut s2 = ScrollbackStore::open(tmp.path(), cfg).expect("reopen");
        assert_eq!(s2.total_lines("X"), 20, "total reidratado do disco");
        assert_eq!(
            s2.line("X", 3).unwrap().as_deref(),
            Some("v3"),
            "paginada pós-reabertura"
        );
        // Continua a numeração: a próxima linha é idx 20.
        s2.push_line("X", "v20").unwrap();
        assert_eq!(s2.total_lines("X"), 21);
        assert_eq!(s2.line("X", 20).unwrap().as_deref(), Some("v20"));
    }

    /// Painéis são isolados (índices e conteúdo independentes na mesma `scrollback.db`).
    #[test]
    fn panels_are_isolated() {
        let tmp = TempDir::new("multi");
        let mut s = ScrollbackStore::open(
            tmp.path(),
            ScrollbackConfig {
                cap: 2,
                flush_batch: 2,
                retention_days: 30,
            },
        )
        .expect("open");
        for i in 0..6u64 {
            s.push_line("p1", format!("um-{i}")).unwrap();
            s.push_line("p2", format!("dois-{i}")).unwrap();
        }
        s.flush_all().unwrap();
        assert_eq!(s.total_lines("p1"), 6);
        assert_eq!(s.total_lines("p2"), 6);
        assert_eq!(s.line("p1", 0).unwrap().as_deref(), Some("um-0"));
        assert_eq!(s.line("p2", 0).unwrap().as_deref(), Some("dois-0"));
        assert_eq!(s.tail("p1", 1).unwrap(), vec!["um-5"]);
        assert_eq!(s.tail("p2", 1).unwrap(), vec!["dois-5"]);
    }

    /// **Critério de aceite W5-2**: despeja > 1M linhas num painel e prova, por MEDIÇÃO:
    /// (a) a RAM do painel ESTABILIZA dentro de ±10% do teto (cap × bytes/linha), NÃO cresce
    /// monotonicamente; (b) linha já paginada é recuperada BYTE-IDÊNTICA (inclusive pós-reabertura);
    /// (c) o arquivo de paginação cresceu com o excedente. RSS de processo é reportado (corroboração).
    #[test]
    fn scrollback_cap_caps_ram() {
        let tmp = TempDir::new("ram");
        let cap = 10_000usize;
        let flush_batch = 4_096usize;
        let cfg = ScrollbackConfig {
            cap,
            flush_batch,
            retention_days: 30,
        };
        let mut s = ScrollbackStore::open(tmp.path(), cfg).expect("open");
        let p = "painel-firehose";

        // Linha de tamanho FIXO conhecido: "ln " + idx zero-pad 8 + " " + 48×'x' = 60 bytes.
        let payload = "x".repeat(48);
        let make = |i: u64| format!("ln {i:08} {payload}");
        let text_len = make(0).len();
        assert_eq!(text_len, 60, "sanity: linha de tamanho fixo");
        // bytes/linha = texto + overhead do String (ptr+len+cap = size_of::<String>()).
        let bpl = text_len + std::mem::size_of::<String>();

        const TOTAL: u64 = 1_050_000; // > 1M
        let flush_samples_at = [100_000u64, 500_000, 1_000_000];
        let raw_sample_at = 750_000u64; // amostra SEM flush → prova o teto duro
        let mut flushed_ram: Vec<usize> = Vec::new();
        let mut raw_ram_peak = 0usize;

        for i in 0..TOTAL {
            s.push_line(p, make(i)).expect("push");
            let n = i + 1;
            if flush_samples_at.contains(&n) {
                // Achata o write-behind no disco → sobra só a janela viva (cap linhas).
                s.flush(p).expect("flush");
                flushed_ram.push(s.ram_bytes(p));
            }
            if n == raw_sample_at {
                raw_ram_peak = s.ram_bytes(p); // SEM flush
            }
        }
        s.flush(p).expect("flush final");

        let ceiling = cap * bpl; // teto da janela viva estabilizada
        let hard_ceiling = (cap + flush_batch) * bpl; // teto INCLUINDO o write-behind

        // (a.1) cada amostra pós-flush fica dentro de ±10% do teto.
        for (k, &r) in flushed_ram.iter().enumerate() {
            let lo = ceiling * 90 / 100;
            let hi = ceiling * 110 / 100;
            assert!(
                r >= lo && r <= hi,
                "amostra {k}: ram_bytes={r} fora de ±10% do teto {ceiling} [{lo},{hi}]"
            );
        }
        // (a.2) as amostras NÃO crescem entre si (≤10% de variação) → não-monotônica/estável.
        let min = *flushed_ram.iter().min().unwrap();
        let max = *flushed_ram.iter().max().unwrap();
        assert!(
            max * 100 <= min * 110,
            "RAM cresceu >10% entre amostras (min={min}, max={max}) — sinal de leak"
        );
        // (a.3) o pico REAL (entre flushes) respeita o teto duro (janela + write-behind).
        assert!(
            raw_ram_peak <= hard_ceiling,
            "pico de RAM {raw_ram_peak} > teto duro {hard_ceiling} (cap+flush_batch)"
        );
        // (a.4) o teto é DRAMATICAMENTE menor que o unbounded (a classe de leak evitada).
        let unbounded = TOTAL as usize * bpl;
        assert!(
            hard_ceiling < unbounded / 50,
            "teto {hard_ceiling} não é ≪ unbounded {unbounded}"
        );

        // (b) recuperação BYTE-IDÊNTICA de uma linha há muito paginada (fora da RAM).
        let early = 12_345u64;
        assert!(
            early < s.total_lines(p) - cap as u64,
            "idx de teste precisa estar paginado"
        );
        assert_eq!(
            s.line(p, early).expect("line").as_deref(),
            Some(make(early).as_str()),
            "linha paginada NÃO recuperada byte-idêntica"
        );
        // ...e uma linha na janela viva.
        let recent = TOTAL - 3;
        assert_eq!(
            s.line(p, recent).expect("line").as_deref(),
            Some(make(recent).as_str())
        );
        // tail(N) atravessa disco→RAM (W5-2 task 4).
        let tail = s.tail(p, 5).expect("tail");
        let expected: Vec<String> = (TOTAL - 5..TOTAL).map(make).collect();
        assert_eq!(tail, expected, "tail não casa as últimas linhas");

        // (c) o arquivo de paginação cresceu com o excedente (medição direta).
        s.checkpoint().expect("checkpoint");
        let disk_rows = s.disk_rows(p).expect("disk_rows");
        assert!(
            disk_rows >= TOTAL - cap as u64 - flush_batch as u64,
            "disco não recebeu o excedente paginado: {disk_rows} (esperado ~{})",
            TOTAL
        );
        let db_len = std::fs::metadata(s.db_path()).expect("metadata").len();
        assert!(
            db_len > (disk_rows * text_len as u64) / 2,
            "arquivo de paginação pequeno demais: {db_len} bytes"
        );

        // (b') persistência real: reabre do disco e re-confirma byte-idêntico.
        drop(s);
        let s2 = ScrollbackStore::open(tmp.path(), cfg).expect("reopen");
        assert_eq!(s2.total_lines(p), TOTAL, "total perdido na reabertura");
        assert_eq!(
            s2.line(p, early).expect("line").as_deref(),
            Some(make(early).as_str()),
            "recuperação pós-reabertura não byte-idêntica"
        );

        // RSS de processo (best-effort, corroboração — o GATE é ram_bytes, determinístico).
        let rss = peak_rss_bytes();
        if let Some(rss) = rss {
            // pico de RSS bem abaixo do que o unbounded teria exigido (×4 = folga p/ cache/harness).
            assert!(
                (rss as usize) < unbounded * 4,
                "pico de RSS {rss} suspeito vs unbounded {unbounded}"
            );
        }
        eprintln!(
            "[W5-2 scrollback_cap_caps_ram] total={TOTAL} bpl={bpl}B \
             teto(janela)={ceiling}B teto_duro={hard_ceiling}B unbounded~={unbounded}B | \
             ram_pos-flush={flushed_ram:?} pico_raw={raw_ram_peak}B \
             disk_rows={disk_rows} db_file={db_len}B peak_rss={rss:?}"
        );
    }
}
