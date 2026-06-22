//! F3-5-7 · frente BUFFERS (dono: Terminal K) — `BufferGauge` (spec 53 §11.A).
//!
//! `BufferRegistry`: projeção unificada de ocupação (`BTreeMap<buffer_id, GaugeSnapshot>`,
//! último `BufferOccupancySampled` por `buffer_id` vence). `capacity==0` = ilimitado,
//! `pressure_ratio=0.0`. Anti-amplificação: amostra só quando cruza bucket de 10% OU
//! muda warn/critical (`gauge_warn_ratio` default 0,80). Amostragem no tick do FlushGuard
//! (scrollback) e no drain do mailbox (`mailbox:<node>`). Entrega `buffer_report()` PURO —
//! o braço `lina check --buffers` em router.rs é fiado pelo Maestro (não edite router.rs).
//! Preencha aqui; o `pub mod` já está em lib.rs (não toque lib.rs).
//!
//! **Layering (porta de troca preservada):** `scrollback.rs`/`mailbox.rs` expõem só LEITURAS
//! cruas (`gauge_readings()` → `Vec<GaugeReading>`); este módulo PROJETA (`BufferRegistry`) e
//! decide a emissão (`sample`, anti-amplificação); quem tem o `EventStore` (o supervisor, no tick)
//! apenda o retorno de `sample`. Nenhuma camada de armazenamento conhece o event log (inv #5,
//! #7) — o gauge é projeção, não 2º log de verdade.

use std::collections::BTreeMap;

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};

/// Default do warn (spec 53 §11.A + tabela [12]): pressão a partir da qual o gauge sinaliza um
/// item de atenção. Provisório (calibração final no benchmark W5-1); entra no `SystemParams`.
pub const GAUGE_WARN_RATIO: f32 = 0.80;

/// Passos do bucket de anti-amplificação (10% — mesma doutrina A4 do `CostCeilingHit`).
const BUCKET_STEPS: f32 = 10.0;

/// `used/capacity`; `0.0` quando `capacity==0` (ilimitado/off — evita div-por-zero e sinaliza
/// "desligado", convenção de `token_budget_day`).
#[must_use]
pub fn pressure_ratio(used: u64, capacity: u64) -> f32 {
    if capacity == 0 {
        return 0.0;
    }
    used as f32 / capacity as f32
}

/// O *bucket* de 10% de uma pressão (`0..=10`). `≥ 1.0` cai no bucket `10` (cheio/transbordando).
/// É o degrau que a anti-amplificação observa: duas amostras no mesmo bucket não geram evento novo.
fn pressure_bucket(ratio: f32) -> u8 {
    (ratio * BUCKET_STEPS).floor().clamp(0.0, BUCKET_STEPS) as u8
}

/// Uma LEITURA crua de ocupação — o insumo que `scrollback`/`mailbox` expõem ao amostrador. Não é
/// evento nem projeção; o `pressure_ratio` e a decisão de emitir são derivados aqui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaugeReading {
    /// `"scrollback:<panel>"` | `"mailbox:<node>"` | `"dlq"` | `"pending"` | …
    pub buffer_id: String,
    /// Ocupação atual, na unidade de `unit`.
    pub used: u64,
    /// Capacidade declarada; `0` = ilimitado/off.
    pub capacity: u64,
    /// `"lines"` | `"bytes"` | `"messages"` | `"tokens"`.
    pub unit: String,
}

/// O snapshot PROJETADO de um buffer (espelha o schema do `BufferOccupancySampled`). `f32` no
/// `pressure_ratio` impede derivar `Eq` — só `PartialEq` (a comparação de replay é exata: os bits
/// vêm da mesma serialização).
#[derive(Debug, Clone, PartialEq)]
pub struct GaugeSnapshot {
    pub used: u64,
    pub capacity: u64,
    pub unit: String,
    pub pressure_ratio: f32,
    pub sampled_at_ms: u64,
}

/// Projeção PURA da ocupação dos buffers por replay (molde `CostLedger`/`CodeIntegration`,
/// inv #4/#5/#1 — ZERO LLM): **último `BufferOccupancySampled` por `buffer_id` vence**. `BTreeMap`
/// → ordem estável → replay byte-a-byte idêntico (gate j). Log antigo sem o evento → vazio (default
/// seguro, spec §11.A).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BufferRegistry {
    pub gauges: BTreeMap<String, GaugeSnapshot>,
}

impl BufferRegistry {
    /// Reconstrói do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log; o último por
    /// `buffer_id` vence. Testável com log sintético (`ts` controlado) — não embute relógio.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut gauges: BTreeMap<String, GaugeSnapshot> = BTreeMap::new();
        for record in records {
            // Versão futura/indecodificável: a projeção é DERIVADA, não validadora do log
            // (espelha `Mentality::from_records`) — pula sem panicar.
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            if let DomainEvent::BufferOccupancySampled {
                buffer_id,
                used,
                capacity,
                unit,
                pressure_ratio,
                sampled_at_ms,
            } = event
            {
                gauges.insert(
                    buffer_id,
                    GaugeSnapshot {
                        used,
                        capacity,
                        unit,
                        pressure_ratio,
                        sampled_at_ms,
                    },
                );
            }
        }
        Self { gauges }
    }

    /// O último snapshot conhecido de um buffer (`None` se nunca amostrado).
    #[must_use]
    pub fn snapshot(&self, buffer_id: &str) -> Option<&GaugeSnapshot> {
        self.gauges.get(buffer_id)
    }
}

/// `true` se uma `reading` NOVA merece um `BufferOccupancySampled`, dado o último snapshot conhecido.
/// **Anti-amplificação A4:** emite na PRIMEIRA amostra do buffer, ou quando cruza um *bucket* de
/// 10%, ou quando muda o estado warn (`≥ warn_ratio`). NUNCA por linha — sob output torrencial a
/// pressão fica no mesmo bucket e o evento não se repete.
#[must_use]
pub fn should_emit(prev: Option<&GaugeSnapshot>, reading: &GaugeReading, warn_ratio: f32) -> bool {
    let new_ratio = pressure_ratio(reading.used, reading.capacity);
    match prev {
        None => true,
        Some(p) => {
            pressure_bucket(p.pressure_ratio) != pressure_bucket(new_ratio)
                || (p.pressure_ratio >= warn_ratio) != (new_ratio >= warn_ratio)
        }
    }
}

/// O AMOSTRADOR: dado o registry corrente (últimas amostras) e leituras novas, devolve SÓ os
/// eventos que passam o filtro anti-amplificação. `now_ms` é INJETADO (replay reproduz a decisão).
/// O caller (supervisor no tick do FlushGuard / drain do mailbox) apenda o retorno no `EventStore`.
#[must_use]
pub fn sample(
    prev: &BufferRegistry,
    readings: &[GaugeReading],
    warn_ratio: f32,
    now_ms: u64,
) -> Vec<DomainEvent> {
    readings
        .iter()
        .filter(|r| should_emit(prev.snapshot(&r.buffer_id), r, warn_ratio))
        .map(|r| DomainEvent::BufferOccupancySampled {
            buffer_id: r.buffer_id.clone(),
            used: r.used,
            capacity: r.capacity,
            unit: r.unit.clone(),
            pressure_ratio: pressure_ratio(r.used, r.capacity),
            sampled_at_ms: now_ms,
        })
        .collect()
}

/// Uma linha do relatório `lina check --buffers`: o estado observável de um buffer + se passou do
/// warn. A NARRATIVA ao leigo ("a memória de um terminal está quase cheia") é da UI; aqui é a
/// projeção crua e honesta.
#[derive(Debug, Clone, PartialEq)]
pub struct BufferReportLine {
    pub buffer_id: String,
    pub used: u64,
    pub capacity: u64,
    pub unit: String,
    pub pressure_ratio: f32,
    /// `pressure_ratio ≥ warn` E capacidade declarada (`capacity != 0`). Buffer ilimitado nunca
    /// "pressiona".
    pub over_warn: bool,
}

/// O relatório completo do `--buffers` (ordem estável de `buffer_id`, herdada do `BTreeMap`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BufferReport {
    pub lines: Vec<BufferReportLine>,
}

impl BufferReport {
    /// Algum buffer passou do warn? (alimenta o realce vermelho do painel de saúde / a fila de
    /// atenção, spec §11.F).
    #[must_use]
    pub fn any_over_warn(&self) -> bool {
        self.lines.iter().any(|l| l.over_warn)
    }
}

/// fn PURA — deriva o relatório do registry projetado (warn = `GAUGE_WARN_RATIO`). É o que o braço
/// `lina check --buffers` (fiado pelo Maestro em router.rs) chama.
#[must_use]
pub fn buffer_report(registry: &BufferRegistry) -> BufferReport {
    buffer_report_with_warn(registry, GAUGE_WARN_RATIO)
}

/// Como [`buffer_report`], com o warn injetado (para o `SystemParams` versionado do capítulo [12]).
#[must_use]
pub fn buffer_report_with_warn(registry: &BufferRegistry, warn_ratio: f32) -> BufferReport {
    let lines = registry
        .gauges
        .iter()
        .map(|(buffer_id, g)| BufferReportLine {
            buffer_id: buffer_id.clone(),
            used: g.used,
            capacity: g.capacity,
            unit: g.unit.clone(),
            pressure_ratio: g.pressure_ratio,
            over_warn: g.capacity != 0 && g.pressure_ratio >= warn_ratio,
        })
        .collect();
    BufferReport { lines }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DomainEvent, EventRecord, EventStore};

    /// Registro sintético construído do PRÓPRIO `DomainEvent` (serializado como o `append` faz —
    /// carrega a tag interna `event`), com `ts` controlado. Exercita o decode REAL (`from_record`).
    fn record(event: DomainEvent, ts: u64) -> EventRecord {
        EventRecord {
            seq: 0,
            ts,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa evento de domínio"),
        }
    }

    fn sampled(buffer_id: &str, used: u64, capacity: u64, ts: u64) -> EventRecord {
        record(
            DomainEvent::BufferOccupancySampled {
                buffer_id: buffer_id.into(),
                used,
                capacity,
                unit: "lines".into(),
                pressure_ratio: pressure_ratio(used, capacity),
                sampled_at_ms: ts,
            },
            ts,
        )
    }

    fn reading(buffer_id: &str, used: u64, capacity: u64) -> GaugeReading {
        GaugeReading {
            buffer_id: buffer_id.into(),
            used,
            capacity,
            unit: "lines".into(),
        }
    }

    // ── pressure_ratio: capacity 0 = off (sem div-por-zero) ───────────────────────────────────────

    #[test]
    fn pressure_ratio_zero_capacity_is_off() {
        assert_eq!(
            pressure_ratio(50, 0),
            0.0,
            "capacity 0 = ilimitado/off → 0.0"
        );
    }

    #[test]
    fn pressure_ratio_at_cap_is_one() {
        assert!((pressure_ratio(10_000, 10_000) - 1.0).abs() < f32::EPSILON);
        assert!((pressure_ratio(8_000, 10_000) - 0.8).abs() < 1e-6);
    }

    // ── projeção: último por buffer_id vence; replay idêntico (gate j) ─────────────────────────────

    #[test]
    fn registry_last_sample_per_buffer_wins() {
        let reg = BufferRegistry::from_records(&[
            sampled("scrollback:T", 5_000, 10_000, 100),
            sampled("scrollback:T", 9_000, 10_000, 200),
        ]);
        let g = reg.snapshot("scrollback:T").expect("gauge presente");
        assert_eq!(g.used, 9_000, "a última amostra vence");
        assert!((g.pressure_ratio - 0.9).abs() < 1e-6);
    }

    #[test]
    fn empty_log_is_empty_registry() {
        // Logs antigos sem o evento → BufferRegistry vazio (default seguro, spec §11.A).
        assert!(BufferRegistry::from_records(&[]).gauges.is_empty());
    }

    #[test]
    fn replay_reconstructs_identical_registry() {
        let log = vec![
            sampled("scrollback:A", 1_000, 10_000, 100),
            sampled("mailbox:n1", 10, 256, 150),
            sampled("scrollback:A", 8_000, 10_000, 200),
        ];
        assert_eq!(
            BufferRegistry::from_records(&log),
            BufferRegistry::from_records(&log),
            "replay determinístico — projeção idêntica"
        );
    }

    #[test]
    fn replay_from_event_store() {
        let dir = std::env::temp_dir().join(format!(
            "lina-buffer-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(&dir).expect("abre store temporário");
        store
            .append(&DomainEvent::BufferOccupancySampled {
                buffer_id: "scrollback:T".into(),
                used: 10_000,
                capacity: 10_000,
                unit: "lines".into(),
                pressure_ratio: 1.0,
                sampled_at_ms: 42,
            })
            .expect("append BufferOccupancySampled");
        let reg = BufferRegistry::replay(&store).expect("replay do store");
        let g = reg
            .snapshot("scrollback:T")
            .expect("gauge presente após replay");
        assert_eq!(g.used, 10_000);
        assert!((g.pressure_ratio - 1.0).abs() < f32::EPSILON);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── anti-amplificação A4: bucket de 10% OU mudança de warn — nunca por linha ───────────────────

    #[test]
    fn sample_first_reading_always_emits() {
        let evs = sample(
            &BufferRegistry::default(),
            &[reading("scrollback:T", 100, 10_000)],
            GAUGE_WARN_RATIO,
            1,
        );
        assert_eq!(evs.len(), 1, "a 1ª amostra de um buffer sempre registra");
    }

    #[test]
    fn sample_same_bucket_does_not_emit() {
        // prev em 0,82 (bucket 8, ≥ warn); nova 0,85 (bucket 8, ≥ warn) → mesmo degrau → nada.
        let prev = BufferRegistry::from_records(&[sampled("scrollback:T", 8_200, 10_000, 100)]);
        let evs = sample(
            &prev,
            &[reading("scrollback:T", 8_500, 10_000)],
            GAUGE_WARN_RATIO,
            200,
        );
        assert!(
            evs.is_empty(),
            "mesmo bucket e mesmo estado warn → não emite (anti-amplificação)"
        );
    }

    #[test]
    fn sample_emits_on_bucket_crossing() {
        // 0,75 (bucket 7) → 0,85 (bucket 8): cruzou o bucket de 10% → emite.
        let prev = BufferRegistry::from_records(&[sampled("scrollback:T", 7_500, 10_000, 100)]);
        let evs = sample(
            &prev,
            &[reading("scrollback:T", 8_500, 10_000)],
            GAUGE_WARN_RATIO,
            200,
        );
        assert_eq!(evs.len(), 1, "cruzar bucket de 10% emite");
    }

    #[test]
    fn sample_emits_on_warn_change_within_same_bucket() {
        // warn 0,85 (NÃO alinhado a bucket): prev 0,82 (bucket 8, < warn) → nova 0,88 (bucket 8, ≥ warn).
        // Mesmo bucket, MAS cruzou o warn → emite (cobre warn fora do passo de 10%).
        let prev = BufferRegistry::from_records(&[sampled("scrollback:T", 8_200, 10_000, 100)]);
        let evs = sample(&prev, &[reading("scrollback:T", 8_800, 10_000)], 0.85, 200);
        assert_eq!(
            evs.len(),
            1,
            "mudar de estado warn dentro do mesmo bucket emite"
        );
    }

    #[test]
    fn sample_torrential_emits_once_per_bucket_not_per_line() {
        // O CORAÇÃO do critério: 101 amostras crescendo 0→1.0 (como output torrencial entre ticks)
        // produzem ~1 evento por bucket de 10%, NUNCA um por amostra.
        let mut log: Vec<EventRecord> = Vec::new();
        for step in 0..=100u64 {
            let used = step * 100; // 0, 100, …, 10_000
            let reg = BufferRegistry::from_records(&log);
            let evs = sample(
                &reg,
                &[reading("scrollback:T", used, 10_000)],
                GAUGE_WARN_RATIO,
                step,
            );
            for e in evs {
                log.push(record(e, step));
            }
        }
        let emitted = log.len();
        assert!(
            (10..=12).contains(&emitted),
            "anti-amplificação: ~1 evento por bucket de 10% (11 buckets 0..=10), não 101 — emitiu {emitted}"
        );
    }

    #[test]
    fn sample_unlimited_capacity_never_pressures() {
        // capacity 0 = ilimitado: pressão sempre 0,0 → após a 1ª amostra, crescer não re-emite.
        let prev = BufferRegistry::from_records(&[sampled("dlq", 3, 0, 100)]);
        let evs = sample(&prev, &[reading("dlq", 9_999, 0)], GAUGE_WARN_RATIO, 200);
        assert!(
            evs.is_empty(),
            "buffer ilimitado fica no bucket 0 → não re-emite ao crescer"
        );
    }

    // ── buffer_report: derivado do registry, marca o que passou do warn (gate d/i) ─────────────────

    #[test]
    fn buffer_report_marks_over_warn() {
        let reg = BufferRegistry::from_records(&[
            sampled("scrollback:cheio", 9_500, 10_000, 100), // 0,95 ≥ warn
            sampled("scrollback:ok", 1_000, 10_000, 100),    // 0,10 < warn
            sampled("dlq", 7, 0, 100),                       // ilimitado: nunca over_warn
        ]);
        let report = buffer_report(&reg);
        assert_eq!(report.lines.len(), 3, "uma linha por buffer projetado");
        assert!(report.any_over_warn(), "o painel cheio dispara o aviso");
        let cheio = report
            .lines
            .iter()
            .find(|l| l.buffer_id == "scrollback:cheio")
            .unwrap();
        assert!(cheio.over_warn);
        let ok = report
            .lines
            .iter()
            .find(|l| l.buffer_id == "scrollback:ok")
            .unwrap();
        assert!(!ok.over_warn);
        let dlq = report.lines.iter().find(|l| l.buffer_id == "dlq").unwrap();
        assert!(!dlq.over_warn, "capacity 0 (ilimitado) nunca passa do warn");
    }
}
