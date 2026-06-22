//! F3-5-8 · frente DISCO (dono: Terminal M) — o `DiskBudget` sob gate humano custodiado.
//!
//! Probe periódico (`DISK_PROBE_SECS` 300) → `used_pct≥0,85` = `DiskPressureSignaled{warn}`;
//! `≥0,95` = `{critical}` + varre candidatos + `DiskReclaimProposed`. A proposta entra como
//! `AttentionKind::Custody` (gate humano — extensão em `attention.rs`).
//!
//! **REGRA-MÃE (ADR 0043, IRREVERSÍVEL):** poda é destrutiva → **zero `DiskReclaimExecuted` sem o
//! gesto humano**; `approved_by` é EXIBIÇÃO, a autoridade é o gesto custodiado (broker
//! [`run_disk_reclaim`]). Forjar `approved_by` no payload NÃO dispara poda (família ADR 0007).
//!
//! **CAMADA PURA aqui** (classificação + decisão de candidatos + projeção [`DiskBudget`], testável
//! com disco SIMULADO injetando `used_bytes/total_bytes`) **SEPARADA da execução shell** (achado de
//! dogfooding #36: `rm`/`cargo clean` dispara `permission_prompt` — a poda real é do broker
//! custodiado, fora do core; o probe FS fica atrás de [`DiskProbe`]).

use std::io;
use std::path::Path;

use crate::events::{DomainEvent, EventRecord, EventStore, ReclaimCandidate, StoreError};

/// Execução custodiada da poda (a porta IRREVERSÍVEL) — re-exportada do broker para que a
/// superfície pública da frente DISCO seja UMA só (`lina_core::disk_budget::*`), sem que o dispatch
/// precise tocar a linha de re-export de `lib.rs` (dono = Maestro). Move o plumbing em vez de
/// negociar a costura (doutrina "fronteira de re-export").
pub use crate::broker::{run_disk_reclaim, DiskReclaimOutcome};

// ───────────────────────────── thresholds FIXOS (spec 53 §11.C / ADR 0043) ─────────────────────

/// `used/total` que cruza para `warn` (aviso; observabilidade livre em todo nível).
pub const DISK_WARN_RATIO: f32 = 0.85;
/// `used/total` que cruza para `critical` (dispara a PROPOSTA de poda — nunca a poda em si).
pub const DISK_CRITICAL_RATIO: f32 = 0.95;
/// Período do probe, em segundos (a cadência do loop é do app; o core só fixa o valor).
pub const DISK_PROBE_SECS: u64 = 300;

// ──────────────────────────────────── camada PURA: pressão ──────────────────────────────────────

/// Nível de pressão de disco — derivado puramente de `used/total` por [`classify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskPressure {
    /// Abaixo de `warn` — nada a sinalizar.
    Ok,
    /// `≥ DISK_WARN_RATIO` — aviso (sem proposta de poda).
    Warn,
    /// `≥ DISK_CRITICAL_RATIO` — aviso + PROPOSTA de poda (se há o que recuperar).
    Critical,
}

impl DiskPressure {
    /// Rótulo do `threshold_crossed` do evento (`None` para `Ok` — `Ok` não apenda nada).
    fn threshold_label(self) -> Option<&'static str> {
        match self {
            DiskPressure::Ok => None,
            DiskPressure::Warn => Some("warn"),
            DiskPressure::Critical => Some("critical"),
        }
    }
}

/// `used/total → (used_pct, nível)`. `total == 0` ⇒ `(0.0, Ok)` — volume desconhecido NÃO sinaliza
/// pressão (degradação honesta, nunca chute; e sem divisão por zero).
#[must_use]
pub fn classify(used_bytes: u64, total_bytes: u64) -> (f32, DiskPressure) {
    if total_bytes == 0 {
        return (0.0, DiskPressure::Ok);
    }
    let used_pct = used_bytes as f32 / total_bytes as f32;
    let level = if used_pct >= DISK_CRITICAL_RATIO {
        DiskPressure::Critical
    } else if used_pct >= DISK_WARN_RATIO {
        DiskPressure::Warn
    } else {
        DiskPressure::Ok
    };
    (used_pct, level)
}

// ──────────────────────────────── snapshot + decisão (apenda eventos) ────────────────────────────

/// Um retrato do disco para a camada pura decidir — REAL (via [`probe_snapshot`]/[`OsDiskProbe`]) ou
/// SIMULADO (injetado no teste). `path` é o rótulo do escopo (`"workspace_target"` | `"app_target"`
/// | `"disk_total"`); `candidates` já vêm VARRIDOS (a varredura toca o FS e fica fora da decisão
/// pura — lição #36).
#[derive(Debug, Clone, PartialEq)]
pub struct DiskSnapshot {
    /// Rótulo do escopo medido (vira `DiskPressureSignaled.path`).
    pub path: String,
    /// Bytes ocupados no volume.
    pub used_bytes: u64,
    /// Capacidade total do volume.
    pub total_bytes: u64,
    /// Candidatos de poda já varridos (vazio ⇒ nada a propor, mesmo em `critical`).
    pub candidates: Vec<ReclaimCandidate>,
}

impl DiskSnapshot {
    /// Bytes livres = `total - used` (saturante: `used > total` improvável ⇒ 0, nunca panic).
    #[must_use]
    pub fn free_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.used_bytes)
    }

    /// Total recuperável = soma dos bytes dos candidatos.
    #[must_use]
    pub fn reclaimable_bytes(&self) -> u64 {
        self.candidates.iter().map(|c| c.bytes).sum()
    }
}

/// O que a avaliação de um snapshot produziu (para o caller/teste; o efeito durável é o log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskAssessment {
    /// Nível classificado.
    pub pressure: DiskPressure,
    /// `true` se uma `DiskReclaimProposed` foi apendada (só em `critical` E com algo a recuperar).
    pub proposed: bool,
}

/// **A decisão PURA (ZERO LLM, ZERO execução):** dado um snapshot, sinaliza pressão e — só em
/// `critical` e com candidatos — PROPÕE a poda. NUNCA apaga: `DiskReclaimProposed` é proposta, jamais
/// execução (a poda é do broker custodiado [`run_disk_reclaim`], ADR 0043 §4). `Ok` não apenda nada.
/// `now_ms` carimba os `*_at_ms` dos eventos (o `ts` do registro é a autoridade temporal — o app
/// passa o relógio real; o teste injeta um valor fixo).
pub fn evaluate(
    snapshot: &DiskSnapshot,
    now_ms: u64,
    store: &mut EventStore,
) -> Result<DiskAssessment, StoreError> {
    let (used_pct, pressure) = classify(snapshot.used_bytes, snapshot.total_bytes);
    let Some(threshold) = pressure.threshold_label() else {
        return Ok(DiskAssessment {
            pressure,
            proposed: false,
        });
    };
    store.append(&DomainEvent::DiskPressureSignaled {
        path: snapshot.path.clone(),
        used_bytes: snapshot.used_bytes,
        free_bytes: snapshot.free_bytes(),
        used_pct,
        threshold_crossed: threshold.to_string(),
        signaled_at_ms: now_ms,
    })?;
    let reclaimable = snapshot.reclaimable_bytes();
    let proposed = pressure == DiskPressure::Critical && reclaimable > 0;
    if proposed {
        store.append(&DomainEvent::DiskReclaimProposed {
            candidates: snapshot.candidates.clone(),
            reclaimable_bytes: reclaimable,
            proposed_at_ms: now_ms,
        })?;
    }
    Ok(DiskAssessment { pressure, proposed })
}

/// ID estável de uma proposta de poda — derivado dos CAMINHOS (ordenados), o vínculo determinístico
/// proposta↔aprovação↔execução que o gesto referencia (ADR 0021 §5: o gesto referencia o id, nunca
/// texto/posição). Mesmos caminhos ⇒ mesmo id no replay (a fila de atenção e a projeção convergem).
#[must_use]
pub fn reclaim_stable_id(paths: &[String]) -> String {
    let mut sorted: Vec<&str> = paths.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    format!("disk-reclaim:{}", sorted.join("|"))
}

/// Formata bytes em GB/MB/KB legível ao leigo (pt-br, vírgula decimal — ex.: `"30,0 GB"`). Para a
/// copy do aviso de disco custodiado (critério (k): "aviso legível"). Aritmética inteira (sem float).
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    const KB: u64 = 1 << 10;
    if bytes >= GB {
        format!("{},{} GB", bytes / GB, (bytes % GB) * 10 / GB)
    } else if bytes >= MB {
        format!("{},{} MB", bytes / MB, (bytes % MB) * 10 / MB)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{bytes} B")
    }
}

// ─────────────────────────── projeção event-sourced (UI + replay byte-a-byte) ───────────────────

/// `DiskPressureSignaled` projetado — o último sinal de pressão (o que a UI exibe).
#[derive(Debug, Clone, PartialEq)]
pub struct PressureState {
    /// Escopo sinalizado.
    pub path: String,
    /// `used/total` no sinal.
    pub used_pct: f32,
    /// `"warn"` | `"critical"`.
    pub threshold: String,
    /// Bytes livres no sinal.
    pub free_bytes: u64,
}

/// `DiskReclaimProposed` projetado — a proposta PENDENTE de gesto (some quando executada).
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalState {
    /// Candidatos propostos.
    pub candidates: Vec<ReclaimCandidate>,
    /// Total recuperável proposto.
    pub reclaimable_bytes: u64,
    /// EXIBIÇÃO: quem aprovou (de `DiskReclaimApproved`), se já aprovado. **NUNCA autoridade** — a
    /// poda só existe via `DiskReclaimExecuted`, nascido do broker custodiado (ADR 0007/0043).
    pub approved_by: Option<String>,
    /// Id estável da proposta (`reclaim_stable_id` dos caminhos).
    pub stable_id: String,
}

/// `DiskReclaimExecuted` projetado — a poda que de fato ocorreu (a ÚNICA fonte de "executado").
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutedState {
    /// Bytes recuperados pela poda.
    pub reclaimed_bytes: u64,
    /// Caminhos apagados.
    pub paths: Vec<String>,
}

/// Projeção event-sourced do estado de disco — o que a UI mostra e o replay reconstrói byte-a-byte
/// (critério (j)). Fold por [`DiskBudget::observe`]; reconstruível por [`DiskBudget::replay`].
///
/// **A salvaguarda na projeção:** `approved_by` é só EXIBIÇÃO em [`ProposalState`]; o estado
/// "executado" ([`DiskBudget::is_reclaim_executed`]) vem EXCLUSIVAMENTE de `DiskReclaimExecuted` —
/// jamais de `DiskReclaimApproved`. Forjar `approved_by` no log não move este ponteiro.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiskBudget {
    pressure: Option<PressureState>,
    proposal: Option<ProposalState>,
    executed: Option<ExecutedState>,
}

impl DiskBudget {
    /// O fold — único caminho de mutação por evento (replay ≡ live).
    pub fn observe(&mut self, event: &DomainEvent) {
        match event {
            DomainEvent::DiskPressureSignaled {
                path,
                free_bytes,
                used_pct,
                threshold_crossed,
                ..
            } => {
                self.pressure = Some(PressureState {
                    path: path.clone(),
                    used_pct: *used_pct,
                    threshold: threshold_crossed.clone(),
                    free_bytes: *free_bytes,
                });
            }
            DomainEvent::DiskReclaimProposed {
                candidates,
                reclaimable_bytes,
                ..
            } => {
                let paths: Vec<String> = candidates.iter().map(|c| c.path.clone()).collect();
                self.proposal = Some(ProposalState {
                    candidates: candidates.clone(),
                    reclaimable_bytes: *reclaimable_bytes,
                    approved_by: None,
                    stable_id: reclaim_stable_id(&paths),
                });
            }
            DomainEvent::DiskReclaimApproved {
                candidate_paths,
                approved_by,
                ..
            } => {
                // EXIBIÇÃO apenas: carimba quem aprovou na proposta correspondente. NÃO marca
                // "executado" (a autoridade é o gesto custodiado, não este campo — ADR 0007).
                let stable_id = reclaim_stable_id(candidate_paths);
                if let Some(p) = self.proposal.as_mut() {
                    if p.stable_id == stable_id {
                        p.approved_by = Some(approved_by.clone());
                    }
                }
            }
            DomainEvent::DiskReclaimExecuted {
                reclaimed_bytes,
                paths,
                ..
            } => {
                self.executed = Some(ExecutedState {
                    reclaimed_bytes: *reclaimed_bytes,
                    paths: paths.clone(),
                });
                self.proposal = None; // proposta consumida pela poda.
            }
            _ => {}
        }
    }

    /// Reconstrói a projeção varrendo o log (replay ≡ estado vivo).
    #[must_use]
    pub fn replay(records: &[EventRecord]) -> Self {
        let mut budget = Self::default();
        for rec in records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                budget.observe(&ev);
            }
        }
        budget
    }

    /// Último sinal de pressão (o que a UI exibe), se houver.
    #[must_use]
    pub fn pressure(&self) -> Option<&PressureState> {
        self.pressure.as_ref()
    }

    /// Proposta PENDENTE de gesto (some após a poda), se houver.
    #[must_use]
    pub fn pending_proposal(&self) -> Option<&ProposalState> {
        self.proposal.as_ref()
    }

    /// `true` SOMENTE se uma poda de fato ocorreu (`DiskReclaimExecuted`). Nunca verdadeiro por
    /// `DiskReclaimApproved` (a prova de que `approved_by` é exibição, não autoridade).
    #[must_use]
    pub fn is_reclaim_executed(&self) -> bool {
        self.executed.is_some()
    }

    /// Bytes recuperados pela poda (0 se nada foi executado).
    #[must_use]
    pub fn reclaimed_bytes(&self) -> u64 {
        self.executed.as_ref().map_or(0, |e| e.reclaimed_bytes)
    }
}

// ───────────────────────────── probe REAL (a ÚNICA parte que toca o FS) ──────────────────────────

/// Sonda do disco — a fronteira FS, fora da decisão pura e da execução (poda = broker). `volume_usage`
/// lê o volume; `dir_size` mede um candidato. NÃO apaga nada. Atrás de trait para a camada pura ser
/// 100% testável com snapshot SIMULADO (e o worker não rodar varredura real — lição #36).
pub trait DiskProbe {
    /// `(used_bytes, total_bytes)` do volume que contém `path`.
    ///
    /// # Errors
    /// Falha se nenhum volume montado contém `path` (ou erro de leitura do SO).
    fn volume_usage(&self, path: &Path) -> io::Result<(u64, u64)>;

    /// Bytes ocupados por um diretório candidato (soma recursiva; 0 se ausente/inacessível).
    fn dir_size(&self, path: &Path) -> u64;
}

/// Sonda real do SO: volume via `sysinfo`; tamanho de diretório via walk de `std::fs`.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsDiskProbe;

impl DiskProbe for OsDiskProbe {
    fn volume_usage(&self, path: &Path) -> io::Result<(u64, u64)> {
        let disks = sysinfo::Disks::new_with_refreshed_list();
        // O volume do `path` = o disco cujo `mount_point` é o prefixo MAIS LONGO do caminho
        // (ex.: `/Users/...` casa `/` e `/Users` → vence `/Users`).
        let disk = disks
            .iter()
            .filter(|d| path.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("nenhum volume montado contém {}", path.display()),
                )
            })?;
        let total = disk.total_space();
        let used = total.saturating_sub(disk.available_space());
        Ok((used, total))
    }

    fn dir_size(&self, path: &Path) -> u64 {
        dir_size_walk(path)
    }
}

/// Soma recursiva do tamanho de um diretório, iterativa (sem estouro de pilha) e SEM seguir symlinks
/// (`DirEntry::metadata` não traversa link — evita laços). Entradas ilegíveis são puladas (medir é
/// best-effort; nunca panica).
fn dir_size_walk(root: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Constrói um [`ReclaimCandidate`] se o diretório existe e tem bytes (`None` se vazio/ausente).
/// Varredura REAL (toca o FS) — fora da decisão pura; o worker prova a decisão com simulado.
#[must_use]
pub fn scan_candidate(probe: &impl DiskProbe, path: &Path, kind: &str) -> Option<ReclaimCandidate> {
    let bytes = probe.dir_size(path);
    (bytes > 0).then(|| ReclaimCandidate {
        path: path.display().to_string(),
        bytes,
        kind: kind.to_string(),
    })
}

/// Monta um [`DiskSnapshot`] REAL para o escopo `scope` (volume em `volume_path`), varrendo
/// `candidate_dirs` (`(caminho, kind)`) como candidatos de poda. É a fronteira FS (probe) — o app
/// passa os caminhos reais; o core só decide. ZERO execução (só mede; apagar é do broker custodiado).
///
/// # Errors
/// Propaga falha de [`DiskProbe::volume_usage`] (volume não encontrado/erro de leitura).
pub fn probe_snapshot(
    probe: &impl DiskProbe,
    scope: &str,
    volume_path: &Path,
    candidate_dirs: &[(&Path, &str)],
) -> io::Result<DiskSnapshot> {
    let (used_bytes, total_bytes) = probe.volume_usage(volume_path)?;
    let candidates = candidate_dirs
        .iter()
        .filter_map(|(p, kind)| scan_candidate(probe, p, kind))
        .collect();
    Ok(DiskSnapshot {
        path: scope.to_string(),
        used_bytes,
        total_bytes,
        candidates,
    })
}

// ───────────────────────────────────────── testes ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Diretório temporário do event store / das varreduras; removido no `Drop`.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!(
                "lina-disk-{tag}-{}-{nanos}",
                std::process::id()
            ));
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

    fn candidate(path: &str, bytes: u64, kind: &str) -> ReclaimCandidate {
        ReclaimCandidate {
            path: path.to_string(),
            bytes,
            kind: kind.to_string(),
        }
    }

    fn kinds(store: &EventStore) -> Vec<String> {
        store
            .events()
            .expect("ler eventos")
            .into_iter()
            .map(|r| r.kind)
            .collect()
    }

    // ── classificação pura ──

    #[test]
    fn classify_thresholds() {
        assert_eq!(classify(96, 100).1, DiskPressure::Critical);
        assert_eq!(classify(95, 100).1, DiskPressure::Critical); // exatamente no limiar.
        assert_eq!(classify(88, 100).1, DiskPressure::Warn);
        assert_eq!(classify(85, 100).1, DiskPressure::Warn); // exatamente no limiar.
        assert_eq!(classify(50, 100).1, DiskPressure::Ok);
    }

    #[test]
    fn classify_zero_total_is_ok_no_panic() {
        // Volume desconhecido (total 0) NÃO sinaliza pressão e NÃO divide por zero.
        assert_eq!(classify(123, 0), (0.0, DiskPressure::Ok));
    }

    // ── critério (e): disco a 96% → DiskPressureSignaled{critical} + DiskReclaimProposed{>0} ──

    #[test]
    fn critical_disk_signals_and_proposes() {
        let tmp = TempDir::new("critical");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let snapshot = DiskSnapshot {
            path: "workspace_target".to_string(),
            used_bytes: 96,
            total_bytes: 100,
            candidates: vec![
                candidate("/ws/target", 30_000_000_000, "cargo_target"),
                candidate("/ws/.lina/dlq", 5_000_000, "dlq_archive"),
            ],
        };

        let outcome = evaluate(&snapshot, 1_000, &mut store).expect("evaluate");

        assert_eq!(outcome.pressure, DiskPressure::Critical);
        assert!(outcome.proposed, "critical com candidatos deve propor");

        let events = store.events().expect("eventos");
        let signaled = events
            .iter()
            .find(|r| r.kind == "DiskPressureSignaled")
            .expect("DiskPressureSignaled");
        assert_eq!(signaled.payload["threshold_crossed"], "critical");

        let proposed = events
            .iter()
            .find(|r| r.kind == "DiskReclaimProposed")
            .expect("DiskReclaimProposed");
        assert_eq!(
            proposed.payload["reclaimable_bytes"], 30_005_000_000_u64,
            "reclaimable = soma dos candidatos"
        );
    }

    #[test]
    fn warn_disk_signals_but_does_not_propose() {
        let tmp = TempDir::new("warn");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let snapshot = DiskSnapshot {
            path: "disk_total".to_string(),
            used_bytes: 88,
            total_bytes: 100,
            candidates: vec![candidate("/ws/target", 1_000, "cargo_target")],
        };

        let outcome = evaluate(&snapshot, 1_000, &mut store).expect("evaluate");

        assert_eq!(outcome.pressure, DiskPressure::Warn);
        assert!(!outcome.proposed, "warn NÃO propõe poda");
        let ks = kinds(&store);
        assert!(ks.iter().any(|k| k == "DiskPressureSignaled"));
        assert!(
            !ks.iter().any(|k| k == "DiskReclaimProposed"),
            "warn não pode propor"
        );
    }

    #[test]
    fn ok_disk_is_silent() {
        let tmp = TempDir::new("ok");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let snapshot = DiskSnapshot {
            path: "disk_total".to_string(),
            used_bytes: 40,
            total_bytes: 100,
            candidates: vec![],
        };
        let outcome = evaluate(&snapshot, 1_000, &mut store).expect("evaluate");
        assert_eq!(outcome.pressure, DiskPressure::Ok);
        assert!(kinds(&store).is_empty(), "Ok não apenda nada");
    }

    #[test]
    fn critical_without_candidates_signals_but_cannot_propose() {
        // Disco cheio mas SEM nada recuperável → sinaliza, mas não propõe (nada a apagar).
        let tmp = TempDir::new("crit-empty");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let snapshot = DiskSnapshot {
            path: "disk_total".to_string(),
            used_bytes: 99,
            total_bytes: 100,
            candidates: vec![],
        };
        let outcome = evaluate(&snapshot, 1_000, &mut store).expect("evaluate");
        assert_eq!(outcome.pressure, DiskPressure::Critical);
        assert!(!outcome.proposed);
        assert!(!kinds(&store).iter().any(|k| k == "DiskReclaimProposed"));
    }

    // ── projeção + a salvaguarda: approved_by é exibição, não autoridade ──

    #[test]
    fn projection_replays_pressure_and_pending_proposal() {
        let tmp = TempDir::new("proj");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let snapshot = DiskSnapshot {
            path: "workspace_target".to_string(),
            used_bytes: 96,
            total_bytes: 100,
            candidates: vec![candidate("/ws/target", 30_000_000_000, "cargo_target")],
        };
        evaluate(&snapshot, 1_000, &mut store).expect("evaluate");

        let budget = DiskBudget::replay(&store.events().expect("eventos"));
        assert_eq!(budget.pressure().expect("pressão").threshold, "critical");
        let proposal = budget.pending_proposal().expect("proposta pendente");
        assert_eq!(proposal.reclaimable_bytes, 30_000_000_000);
        assert!(proposal.approved_by.is_none());
        assert!(!budget.is_reclaim_executed());
        assert_eq!(budget.reclaimed_bytes(), 0);
    }

    #[test]
    fn forged_approved_by_is_inert_in_projection() {
        // MUTAÇÃO da salvaguarda: um agente FORJA `approved_by` apendando o evento direto no log,
        // SEM o gesto custodiado. A projeção NÃO pode tratar isso como poda executada.
        let tmp = TempDir::new("forged");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        // Há uma proposta legítima…
        let snapshot = DiskSnapshot {
            path: "workspace_target".to_string(),
            used_bytes: 96,
            total_bytes: 100,
            candidates: vec![candidate("/ws/target", 30_000_000_000, "cargo_target")],
        };
        evaluate(&snapshot, 1_000, &mut store).expect("evaluate");
        // …e um approved_by FORJADO (atacante se auto-aprovando no payload):
        store
            .append(&DomainEvent::DiskReclaimApproved {
                candidate_paths: vec!["/ws/target".to_string()],
                approved_by: "atacante-se-auto-aprovando".to_string(),
                approved_at_ms: 0,
            })
            .expect("append forjado");

        let budget = DiskBudget::replay(&store.events().expect("eventos"));
        // O approved_by aparece só como EXIBIÇÃO…
        assert_eq!(
            budget.pending_proposal().expect("proposta").approved_by.as_deref(),
            Some("atacante-se-auto-aprovando")
        );
        // …mas NADA foi executado: zero DiskReclaimExecuted, zero bytes (a autoridade é o gesto).
        assert!(
            !budget.is_reclaim_executed(),
            "forjar approved_by NÃO pode marcar poda executada"
        );
        assert_eq!(budget.reclaimed_bytes(), 0);
        assert!(
            !kinds(&store).iter().any(|k| k == "DiskReclaimExecuted"),
            "nenhum DiskReclaimExecuted nasce de um campo de payload"
        );
    }

    #[test]
    fn projection_marks_executed_only_from_executed_event() {
        let tmp = TempDir::new("exec");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let snapshot = DiskSnapshot {
            path: "workspace_target".to_string(),
            used_bytes: 96,
            total_bytes: 100,
            candidates: vec![candidate("/ws/target", 30_000_000_000, "cargo_target")],
        };
        evaluate(&snapshot, 1_000, &mut store).expect("evaluate");
        store
            .append(&DomainEvent::DiskReclaimApproved {
                candidate_paths: vec!["/ws/target".to_string()],
                approved_by: "@Fundador".to_string(),
                approved_at_ms: 0,
            })
            .expect("approved");
        store
            .append(&DomainEvent::DiskReclaimExecuted {
                reclaimed_bytes: 30_000_000_000,
                paths: vec!["/ws/target".to_string()],
                executed_at_ms: 0,
            })
            .expect("executed");

        let budget = DiskBudget::replay(&store.events().expect("eventos"));
        assert!(budget.is_reclaim_executed());
        assert_eq!(budget.reclaimed_bytes(), 30_000_000_000);
        // Proposta consumida pela poda.
        assert!(budget.pending_proposal().is_none());
    }

    // ── id estável + copy leiga ──

    #[test]
    fn reclaim_stable_id_is_order_independent() {
        let a = reclaim_stable_id(&["/b".to_string(), "/a".to_string()]);
        let b = reclaim_stable_id(&["/a".to_string(), "/b".to_string()]);
        assert_eq!(a, b, "id estável independe da ordem dos caminhos");
        assert_eq!(a, "disk-reclaim:/a|/b");
    }

    #[test]
    fn human_bytes_is_legible() {
        assert_eq!(human_bytes(30 * (1 << 30)), "30,0 GB");
        assert_eq!(human_bytes(3 * (1 << 20) + (1 << 19)), "3,5 MB");
        assert_eq!(human_bytes(512), "512 B");
    }

    // ── probe FS (varredura segura num tmpdir; NÃO toca cargo/rm) ──

    #[test]
    fn dir_size_walk_sums_files_recursively() {
        let tmp = TempDir::new("walk");
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).expect("subdir");
        std::fs::write(tmp.path().join("a.bin"), vec![0u8; 1000]).expect("a");
        std::fs::write(sub.join("b.bin"), vec![0u8; 2000]).expect("b");

        let probe = OsDiskProbe;
        assert_eq!(probe.dir_size(tmp.path()), 3000, "soma recursiva dos arquivos");
        // scan_candidate só materializa se há bytes.
        let cand = scan_candidate(&probe, tmp.path(), "cargo_target").expect("candidato");
        assert_eq!(cand.bytes, 3000);
        assert_eq!(cand.kind, "cargo_target");
        assert!(scan_candidate(&probe, &tmp.path().join("inexistente"), "x").is_none());
    }

    /// Probe SIMULADO (injeta used/total + tamanhos) — prova o fio probe→snapshot→evaluate sem
    /// depender do FS real do volume (lição #36: o worker não exercita o probe físico do disco).
    struct MockProbe {
        used: u64,
        total: u64,
    }
    impl DiskProbe for MockProbe {
        fn volume_usage(&self, _path: &Path) -> io::Result<(u64, u64)> {
            Ok((self.used, self.total))
        }
        fn dir_size(&self, path: &Path) -> u64 {
            if path.ends_with("target") {
                30_000_000_000
            } else {
                0
            }
        }
    }

    #[test]
    fn probe_snapshot_wires_volume_and_candidates() {
        let probe = MockProbe {
            used: 96,
            total: 100,
        };
        let target = PathBuf::from("/ws/target");
        let empty = PathBuf::from("/ws/empty");
        let snapshot = probe_snapshot(
            &probe,
            "workspace_target",
            Path::new("/ws"),
            &[(target.as_path(), "cargo_target"), (empty.as_path(), "x")],
        )
        .expect("probe");
        assert_eq!(snapshot.used_bytes, 96);
        assert_eq!(snapshot.total_bytes, 100);
        assert_eq!(snapshot.candidates.len(), 1, "só o target não-vazio vira candidato");
        assert_eq!(snapshot.reclaimable_bytes(), 30_000_000_000);
    }
}
