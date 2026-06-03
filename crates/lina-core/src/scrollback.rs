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
//! - **Esta camada** (`lina-core`): o disco é o **log append-only autoritativo** de CADA linha
//!   (SQLite WAL, alinhado ao [`crate::EventStore`]); a RAM mantém só um **cache da cauda** de
//!   `cap` linhas (a janela viva / viewport rolável sem tocar o disco). Ao rolar além da janela,
//!   as páginas são **hidratadas do disco sob demanda** — transparente, no core, fora do render.
//!   Nada se perde (invariante #6).
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
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// Cap **default provisório** de linhas mantidas em RAM por painel (janela viva). Espelha
/// `lina_vt::DEFAULT_SCROLLBACK_CAP`. **Provisório**: o benchmark W5-1 calibra o número final —
/// o entregável é o MECANISMO (cap + paginação), não o valor.
pub const DEFAULT_SCROLLBACK_CAP: usize = 10_000;

/// Linhas não-persistidas acumuladas antes de um flush em LOTE ao disco (1 transação). Limita o
/// nº de transações (perf) e o pico de RAM ENTRE flushes.
pub const DEFAULT_FLUSH_BATCH: usize = 2_000;

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
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self {
            cap: DEFAULT_SCROLLBACK_CAP,
            flush_batch: DEFAULT_FLUSH_BATCH,
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
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS scrollback (
    panel TEXT    NOT NULL,
    idx   INTEGER NOT NULL,
    text  TEXT    NOT NULL,
    PRIMARY KEY (panel, idx)
);
";

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

        // Reabertura: `total` e `persisted` por painel vêm de MAX(idx)+1 do disco. O cache da
        // cauda nasce VAZIO (será repovoado pelo stream de output); leituras caem no disco até lá.
        let mut panels = BTreeMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT panel, COALESCE(MAX(idx), -1) + 1 FROM scrollback GROUP BY panel",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (panel, count) = row?;
                let c = count.max(0) as u64;
                panels.insert(
                    panel,
                    PanelBuffer {
                        persisted: c,
                        total: c,
                        ..Default::default()
                    },
                );
            }
        }

        Ok(Self {
            conn,
            db_path,
            cfg,
            panels,
        })
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
        let (start, lines) = match self.panels.get_mut(panel) {
            Some(pb) if pb.total > pb.persisted => {
                let n = (pb.total - pb.persisted) as usize;
                let from = pb.tail_buf.len() - n; // as últimas `n` linhas do cache são as não-persistidas
                let lines: Vec<String> = pb.tail_buf.iter().skip(from).cloned().collect();
                (pb.persisted, lines)
            }
            _ => return Ok(()),
        };

        write_batch(&mut self.conn, panel, start, &lines)?;
        if let Some(pb) = self.panels.get_mut(panel) {
            pb.persisted += lines.len() as u64;
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
}

impl Drop for ScrollbackStore {
    fn drop(&mut self) {
        // Close gracioso: persiste o write-behind pendente (invariante #6 — estado salvo). Em
        // crash duro (kill -9) perde-se no máximo o último lote não-flushado de output (cache de
        // terminal, não estado de domínio — este vive no event log).
        let _ = self.flush_all();
    }
}

// ───────────────────────────── helpers ─────────────────────────────

/// Grava `lines` (índices `start..start+len`) do `panel` em UMA transação (write-behind durável).
fn write_batch(
    conn: &mut Connection,
    panel: &str,
    start: u64,
    lines: &[String],
) -> Result<(), ScrollbackError> {
    let tx = conn.transaction()?;
    {
        let mut stmt =
            tx.prepare("INSERT OR REPLACE INTO scrollback (panel, idx, text) VALUES (?1, ?2, ?3)")?;
        for (i, line) in lines.iter().enumerate() {
            stmt.execute(params![panel, (start + i as u64) as i64, line])?;
        }
    }
    tx.commit()?;
    Ok(())
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
        let cfg = ScrollbackConfig { cap, flush_batch };
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
