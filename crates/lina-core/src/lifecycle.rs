//! F1-0-3 · State-machine de lifecycle + heartbeat com progresso (ADR 0019).
//!
//! ## Decisão de design registrada — qual sinal decide cada estado
//! (exigência da story F1-0-3: "decisão de design registrada: qual sinal decide Busy/Idle")
//!
//! - **`Busy` ← OUTPUT do PTY**: delta de `cycle_count` entre amostras (os batches do pty-host
//!   W0-3 já filtram flush vazio, então "ciclo avançou" = "houve output novo"). Output é o sinal
//!   mais confiável de "trabalhando": não depende de regex de prompt nem de heurística de idle.
//! - **`Idle` ← FIM-DE-RESPOSTA EXPLÍCITO**: `EndDetector` (W0-10) / `prompt_ready_regex` do CLI
//!   Profile — hoje com `idle_ms` efetivo de **1500ms** no `claude-code.toml` (aviso Dev 02; o
//!   timing pertence ao detector, não a este módulo). `Idle` **NUNCA é inferido de silêncio**:
//!   silêncio em `Busy` é candidato a STALL, não a Idle — é exatamente essa distinção que
//!   elimina o falso-negativo "alive+hung" (ADR 0019 §4; padrão Sol Framework, 13.11).
//! - **`Blocked` ← gate humano/custódia** (W3-6): o chamador sinaliza; não acumula stall (§4).
//! - **`Dead` ← EOF/exit do PTY** (`TerminalState::Exited`) ou `mark_dead` do Supervisor.
//! - **`Ready` ← spawn concluído**, antes do primeiro trabalho.
//!
//! ## Invariantes (ADR 0019 §3-§5)
//! - O VEREDITO é projeção do log (inv#4): toda transição vira `NodeStatusChanged{from,to,reason}`
//!   e todo stall vira `NodeStalled` — as **amostras cruas são EFÊMERAS** (alto volume/baixo
//!   sinal, nunca apendadas — ADR 0003).
//! - **Anti-amplificação** (ADR 0005): evento só na TRANSIÇÃO — same-status é no-op;
//!   `NodeStalled` 1× por entrada em stall, re-armado por progresso.
//! - **O relógio de stall só corre em `Busy`**: `Blocked`/`Idle` não acumulam (§4) e qualquer
//!   transição zera o contador.
//! - PROGRESSO = `tail_hash` mudou entre amostras consecutivas **OU** ≥1 evento de domínio
//!   atribuível ao nó no período (`note_domain_activity`). Ciclo avançando com tail idêntico
//!   NÃO é progresso (§2 — literal por contrato).
//! - Disciplina de escrita: **toda mudança de status passa pelo engine** (choke point) — não
//!   chame `Supervisor::set_status` direto em caminho de produção; o roster vivo e o log andam
//!   juntos por aqui.
//!
//! A cadência de amostragem (`HEARTBEAT_SAMPLE_MS`, 2min) é dirigida pelo CHAMADOR (app/loop de
//! manutenção); o engine é puro sobre `(cycle_count, tail_hash)` — testável headless com relógio
//! nenhum (lição [[feature-filtra-ts-wallclock-vs-teste-relogio-fixo]]).

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::events::{DomainEvent, EventStore, StoreError};
use crate::{NodeId, NodeStatus, Supervisor, SupervisorError};

/// Cadência canônica de amostragem do heartbeat (ADR 0019 §1: 2 min).
pub const HEARTBEAT_SAMPLE_MS: u64 = 120_000;
/// Amostras consecutivas SEM progresso em `Busy` para o WARN `NodeStalled` (~6 min — §3).
pub const STALL_WARN_SAMPLES: u32 = 3;
/// Amostras para o circuit breaker (~12 min — §3). A AÇÃO do breaker é F1-0-7; a constante
/// vive aqui para o contrato ficar inteiro (consulte `stall_samples` para implementá-la).
pub const STALL_BREAKER_SAMPLES: u32 = 6;

/// Razões canônicas de transição (campo `reason` do `NodeStatusChanged`).
pub mod reason {
    /// Output novo no PTY (ciclo avançou) → `Busy`.
    pub const PTY_OUTPUT: &str = "pty_output";
    /// Fim-de-resposta explícito (`EndDetector` W0-10 / prompt-ready do perfil) → `Idle`.
    pub const END_OF_RESPONSE: &str = "end_of_response";
    /// EOF/exit do processo do terminal → `Dead`.
    pub const PTY_EXIT: &str = "pty_exit";
    /// Gate humano/custódia segurando o nó (W3-6) → `Blocked`.
    pub const CUSTODY_GATE: &str = "custody_gate";
    /// Spawn/registro concluído → `Ready`.
    pub const SPAWN: &str = "spawn";
    /// F1-0-8 (P4): morte PÓSTUMA registrada na reabertura do workspace — o processo
    /// do terminal morreu junto com a sessão anterior do app (kill -9/crash/fechar),
    /// e a reabertura é o primeiro momento em que o fato pode entrar no log.
    pub const APP_REOPENED: &str = "app_reopened";
}

/// F1-0-8 (P4) — fecha o ciclo de vida da geração ANTERIOR no log (nós-fantasma).
///
/// ## O problema (DIRECIONAMENTO P4 / pesquisa 13.2)
/// 3 reaberturas do app = 17 `NodeAdded` / 0 mortes no log → o replay "via" 17
/// terminais vivos; o roster do Supervisor só funcionava porque ignorava o replay —
/// tensão direta com o invariante #4 ("o event log é a fonte da verdade").
///
/// ## Decisão registrada: opção (a) — mortes PÓSTUMAS explícitas, por nó
/// Ao abrir o workspace (logo após `EventStore::open`/`open_or_recover`, ANTES de
/// registrar a nova geração), apenda `NodeStatusChanged{status:"dead", from:<último
/// status projetado>, reason:"app_reopened"}` para cada terminal da geração anterior
/// ainda "vivo" no log. Usa o evento EXISTENTE (zero variante nova) — o fato é
/// verdadeiro (os processos morreram com a sessão) e fica auditável POR NÓ:
/// `NodeAdded − mortes == nós vivos` fecha no próprio `log.jsonl`.
///
/// A alternativa (b) — snapshot/compactação — foi DESCARTADA por design: snapshot
/// materializa a projeção, e a projeção é exatamente o que está errado; congelar o
/// estado não acrescenta ao log o fato que falta (as mortes), e "compactar" eventos
/// seria reescrever história — anti-invariante #4 frontal. Um evento único de
/// geração (variante de (a)) também foi descartado: tornaria a morte de cada nó
/// IMPLÍCITA (regra não-local na projeção) e quebraria a aritmética por evento que
/// o critério de aceite exige. Nenhuma porta da §3 é fechada (mudança aditiva) →
/// sem ADR; decisão registrada aqui e no relatório da story.
///
/// ## Por que NÃO passa pelo `LifecycleEngine::transition`
/// O choke point do engine opera sobre o roster VIVO do Supervisor — e a geração
/// anterior não existe no processo novo (não há o que `mark_dead`). Esta é a única
/// escrita de status legítima fora do engine: é um fato PÓSTUMO, derivado do log
/// para o log. Nós sem status (notas, nós nunca spawnados) não têm ciclo de vida de
/// processo e atravessam intactos; nós já `dead` não morrem duas vezes (idempotente).
pub fn close_previous_generation(store: &mut EventStore) -> Result<Vec<NodeId>, StoreError> {
    let state = store.project()?;
    let mut closed = Vec::new();
    for (node, projected) in &state.nodes {
        let Some(last_status) = projected.status.as_deref() else {
            continue; // sem ciclo de vida (nota/nunca spawnou) — não é fantasma
        };
        if last_status == NodeStatus::Dead.as_str() {
            continue; // já fechado (idempotência entre reaberturas)
        }
        store.append(&DomainEvent::NodeStatusChanged {
            node: *node,
            status: NodeStatus::Dead.as_str().to_string(),
            from: last_status.to_string(),
            reason: reason::APP_REOPENED.to_string(),
        })?;
        closed.push(*node);
    }
    Ok(closed)
}

/// Erros do engine de lifecycle.
#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("supervisor: {0}")]
    Supervisor(#[from] SupervisorError),
    #[error("event store: {0}")]
    Store(#[from] StoreError),
    /// `Dead` é terminal: nó morto não transiciona (re-spawn = nó novo no roster).
    #[error("transição inválida: nó {0} está Dead (estado terminal)")]
    DeadIsTerminal(NodeId),
}

/// Resultado observável de UMA amostra do heartbeat (para o chamador e para os testes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleOutcome {
    /// Nó fora de `Busy` ganhou output novo → transicionou para `Busy` (`pty_output`).
    BecameBusy,
    /// `Busy` com progresso na janela (tail mudou ou evento de domínio) — contador zerado.
    Progress,
    /// `Busy` SEM progresso há N amostras consecutivas (ainda abaixo do warn, ou já warnado).
    NoProgress(u32),
    /// Esta amostra cruzou o threshold: `NodeStalled` foi emitido (1× — só nesta transição).
    StalledWarned,
    /// Nada a fazer (fora de `Busy` sem output novo; `Blocked`/`Dead` nunca acumulam — §4).
    Quiet,
}

/// Bookkeeping efêmero por nó (NUNCA persistido — ADR 0019 §5).
#[derive(Debug, Default)]
struct NodeTrack {
    last_cycle: Option<u64>,
    last_tail_hash: Option<[u8; 32]>,
    no_progress: u32,
    domain_activity: bool,
    stalled: bool,
}

/// O choke point de lifecycle: transições de status (roster + log juntos) e o heartbeat
/// com indicador de progresso (ADR 0019). Puro sobre amostras — sem thread própria.
#[derive(Debug)]
pub struct LifecycleEngine {
    warn_samples: u32,
    tracks: HashMap<NodeId, NodeTrack>,
}

impl Default for LifecycleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleEngine {
    /// Engine com os defaults canônicos do ADR 0019 (`STALL_WARN_SAMPLES`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_warn_samples(STALL_WARN_SAMPLES)
    }

    /// Threshold customizado (tunável via `RouterConfig::stall_warn_samples`).
    #[must_use]
    pub fn with_warn_samples(warn_samples: u32) -> Self {
        Self {
            warn_samples: warn_samples.max(1),
            tracks: HashMap::new(),
        }
    }

    /// Engine com os thresholds do `RouterConfig` (ADR 0019 §3: constantes tunáveis lá).
    #[must_use]
    pub fn from_config(cfg: &crate::RouterConfig) -> Self {
        Self::with_warn_samples(cfg.stall_warn_samples)
    }

    /// Transiciona `node` para `to`: atualiza o roster do Supervisor E apenda o evento —
    /// **fato antes do efeito** (o `NodeStatusChanged` é gravado antes da mutação do roster,
    /// mesmo padrão do append-antes-do-202 de W5-4). Devolve `Ok(false)` se já estava em `to`
    /// (no-op SEM evento — anti-amplificação ADR 0005). `Dead` é terminal: transicionar a
    /// partir dele é `Err(DeadIsTerminal)`.
    pub fn transition(
        &mut self,
        sup: &Supervisor,
        store: &mut EventStore,
        node: NodeId,
        to: NodeStatus,
        why: &str,
    ) -> Result<bool, LifecycleError> {
        let info =
            sup.get(node)
                .ok_or(LifecycleError::Supervisor(SupervisorError::NodeNotFound(
                    node,
                )))?;
        let from = info.status;
        if from == to {
            return Ok(false);
        }
        if from == NodeStatus::Dead {
            return Err(LifecycleError::DeadIsTerminal(node));
        }
        store.append(&DomainEvent::NodeStatusChanged {
            node,
            status: to.as_str().to_string(),
            from: from.as_str().to_string(),
            reason: why.to_string(),
        })?;
        if to == NodeStatus::Dead {
            sup.mark_dead(node)?;
        } else {
            sup.set_status(node, to)?;
        }
        // Transição zera o relógio de stall (§4) e re-arma o WARN.
        let t = self.tracks.entry(node).or_default();
        t.no_progress = 0;
        t.stalled = false;
        t.domain_activity = false;
        Ok(true)
    }

    /// Marca que houve ≥1 evento de domínio ATRIBUÍVEL ao nó desde a última amostra
    /// (`RouteDelivered` de/para o nó, `TokenUsageReported`, `PlanClaimed`… — ADR 0019 §2b).
    /// Consumido (e limpo) pela próxima `sample`.
    pub fn note_domain_activity(&mut self, node: NodeId) {
        self.tracks.entry(node).or_default().domain_activity = true;
    }

    /// UMA amostra do heartbeat para `node`: `(cycle_count, tail_hash)` capturados pelo
    /// chamador (cadência sugerida: `HEARTBEAT_SAMPLE_MS`). Efeitos possíveis: promover nó
    /// quieto com output novo a `Busy`, ou acumular/zerar o relógio de stall em `Busy` e
    /// emitir `NodeStalled` ao cruzar o threshold (1×). A amostra em si NUNCA vira evento.
    pub fn sample(
        &mut self,
        sup: &Supervisor,
        store: &mut EventStore,
        node: NodeId,
        cycle_count: u64,
        tail_hash: [u8; 32],
    ) -> Result<SampleOutcome, LifecycleError> {
        let info =
            sup.get(node)
                .ok_or(LifecycleError::Supervisor(SupervisorError::NodeNotFound(
                    node,
                )))?;
        // 1ª amostra de um nó: sem base de comparação → conta como progresso (conservador,
        // nunca stall instantâneo) e não promove a Busy (sem delta de ciclo conhecido).
        let (cycle_advanced, progressed) = {
            let t = self.tracks.entry(node).or_default();
            let cycle_advanced = t.last_cycle.is_some_and(|c| cycle_count > c);
            let hash_changed = t.last_tail_hash.is_none_or(|h| h != tail_hash);
            let progressed = hash_changed || t.domain_activity;
            t.last_cycle = Some(cycle_count);
            t.last_tail_hash = Some(tail_hash);
            t.domain_activity = false;
            (cycle_advanced, progressed)
        };

        match info.status {
            NodeStatus::Busy => {
                let warn = self.warn_samples;
                let t = self.tracks.entry(node).or_default();
                if progressed {
                    t.no_progress = 0;
                    t.stalled = false; // re-arma: um NOVO stall é uma nova transição → re-emite
                    Ok(SampleOutcome::Progress)
                } else {
                    t.no_progress = t.no_progress.saturating_add(1);
                    if t.no_progress >= warn && !t.stalled {
                        t.stalled = true;
                        store.append(&DomainEvent::NodeStalled { node, cycle_count })?;
                        Ok(SampleOutcome::StalledWarned)
                    } else {
                        Ok(SampleOutcome::NoProgress(t.no_progress))
                    }
                }
            }
            // §4: Blocked (gate humano já cuida) e Dead nunca acumulam o relógio.
            NodeStatus::Blocked | NodeStatus::Dead => Ok(SampleOutcome::Quiet),
            // Quieto com output novo → Busy (decisão de design no topo do módulo).
            NodeStatus::Starting | NodeStatus::Running | NodeStatus::Ready | NodeStatus::Idle => {
                if cycle_advanced {
                    self.transition(sup, store, node, NodeStatus::Busy, reason::PTY_OUTPUT)?;
                    Ok(SampleOutcome::BecameBusy)
                } else {
                    Ok(SampleOutcome::Quiet)
                }
            }
        }
    }

    /// Açúcar: fim-de-resposta explícito (`EndDetector` W0-10 / prompt-ready) → `Idle`.
    pub fn on_end_of_response(
        &mut self,
        sup: &Supervisor,
        store: &mut EventStore,
        node: NodeId,
    ) -> Result<bool, LifecycleError> {
        self.transition(sup, store, node, NodeStatus::Idle, reason::END_OF_RESPONSE)
    }

    /// Açúcar: EOF/exit do PTY → `Dead` (terminal).
    pub fn on_pty_exit(
        &mut self,
        sup: &Supervisor,
        store: &mut EventStore,
        node: NodeId,
    ) -> Result<bool, LifecycleError> {
        self.transition(sup, store, node, NodeStatus::Dead, reason::PTY_EXIT)
    }

    /// Amostras consecutivas sem progresso do nó (consulta para o breaker — F1-0-7 compara
    /// com `STALL_BREAKER_SAMPLES`/`RouterConfig`). `0` se nunca amostrado/zerado.
    #[must_use]
    pub fn stall_samples(&self, node: NodeId) -> u32 {
        self.tracks.get(&node).map_or(0, |t| t.no_progress)
    }

    /// O nó está atualmente em stall warnado (veredito vivo; o durável é o `NodeStalled` no log)?
    #[must_use]
    pub fn is_stalled(&self, node: NodeId) -> bool {
        self.tracks.get(&node).is_some_and(|t| t.stalled)
    }

    /// Hash canônico do tail do PTY (ADR 0019 §1): SHA-256 dos últimos ~80 **chars** do texto
    /// (tipicamente `VtBackend::last_nonempty_line` — o mesmo acessor do prompt-ready A2A).
    #[must_use]
    pub fn tail_hash(text: &str) -> [u8; 32] {
        let n = text.chars().count();
        let tail: String = text.chars().skip(n.saturating_sub(80)).collect();
        let mut h = Sha256::new();
        h.update(tail.as_bytes());
        h.finalize().into()
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-lc-{tag}-{}", Uuid::now_v7()));
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

    fn setup(tag: &str) -> (TempDir, Supervisor, EventStore, NodeId) {
        let tmp = TempDir::new(tag);
        let store = EventStore::open(tmp.path()).expect("open store");
        let sup = Supervisor::new();
        let node = sup.register("Terminal A", None, Box::new(std::io::sink()));
        (tmp, sup, store, node)
    }

    /// Tuplas `(from, to, reason)` dos `NodeStatusChanged` no log, em ordem.
    fn status_events(store: &EventStore) -> Vec<(String, String, String)> {
        store
            .events()
            .expect("events")
            .into_iter()
            .filter(|r| r.kind == "NodeStatusChanged")
            .map(|r| {
                (
                    r.payload["from"].as_str().unwrap_or_default().to_string(),
                    r.payload["status"].as_str().unwrap_or_default().to_string(),
                    r.payload["reason"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    fn stall_events(store: &EventStore) -> usize {
        store
            .events()
            .expect("events")
            .iter()
            .filter(|r| r.kind == "NodeStalled")
            .count()
    }

    /// ADR 0019 §3: os thresholds vivem TUNÁVEIS no `RouterConfig` (defaults conservadores)
    /// e o engine se constrói a partir dele.
    #[test]
    fn router_config_carries_adr0019_thresholds() {
        let cfg = crate::RouterConfig::default();
        assert_eq!(cfg.heartbeat_sample_ms, HEARTBEAT_SAMPLE_MS);
        assert_eq!(cfg.stall_warn_samples, STALL_WARN_SAMPLES);
        assert_eq!(cfg.stall_breaker_samples, STALL_BREAKER_SAMPLES);
        let eng = LifecycleEngine::from_config(&cfg);
        assert_eq!(eng.warn_samples, STALL_WARN_SAMPLES);
    }

    /// F1-0-3: `Ready` é canônico, disponível para trabalho, e `as_str` é estável
    /// (é a string persistida no log — mudar quebra replay de projeção).
    #[test]
    fn ready_is_available_and_as_str_is_canonical() {
        assert!(NodeStatus::Ready.is_available());
        for (st, s) in [
            (NodeStatus::Starting, "Starting"),
            (NodeStatus::Running, "Running"),
            (NodeStatus::Ready, "Ready"),
            (NodeStatus::Idle, "Idle"),
            (NodeStatus::Busy, "Busy"),
            (NodeStatus::Blocked, "Blocked"),
            (NodeStatus::Dead, "Dead"),
        ] {
            assert_eq!(st.as_str(), s);
        }
    }

    /// Transição atualiza o roster E apenda `NodeStatusChanged{from,to,reason}` com `ts`.
    #[test]
    #[serial]
    fn transition_updates_roster_and_appends_event() {
        let (_tmp, sup, mut store, node) = setup("transition");
        let initial = sup.get(node).expect("info").status;
        let mut eng = LifecycleEngine::new();

        let changed = eng
            .transition(&sup, &mut store, node, NodeStatus::Ready, reason::SPAWN)
            .expect("transition");
        assert!(changed);
        assert_eq!(sup.get(node).expect("info").status, NodeStatus::Ready);

        let evs = status_events(&store);
        assert_eq!(
            evs,
            vec![(
                initial.as_str().to_string(),
                "Ready".to_string(),
                reason::SPAWN.to_string()
            )]
        );
        let rec = store.events().expect("events").pop().expect("registro");
        assert!(rec.ts > 0, "evento de transição carrega timestamp");
    }

    /// Anti-amplificação (ADR 0005): same-status é no-op SEM evento.
    #[test]
    #[serial]
    fn same_status_is_noop_without_event() {
        let (_tmp, sup, mut store, node) = setup("noop");
        let mut eng = LifecycleEngine::new();
        assert!(eng
            .transition(&sup, &mut store, node, NodeStatus::Ready, reason::SPAWN)
            .expect("1ª"));
        assert!(!eng
            .transition(&sup, &mut store, node, NodeStatus::Ready, reason::SPAWN)
            .expect("2ª (no-op)"));
        assert_eq!(status_events(&store).len(), 1, "no-op não re-emite evento");
    }

    /// `Dead` é terminal: transicionar a partir dele falha explícito (sem evento novo).
    #[test]
    #[serial]
    fn dead_is_terminal() {
        let (_tmp, sup, mut store, node) = setup("dead");
        let mut eng = LifecycleEngine::new();
        eng.on_pty_exit(&sup, &mut store, node).expect("→ Dead");
        assert_eq!(sup.get(node).expect("info").status, NodeStatus::Dead);

        let res = eng.transition(&sup, &mut store, node, NodeStatus::Busy, reason::PTY_OUTPUT);
        assert!(matches!(res, Err(LifecycleError::DeadIsTerminal(_))));
        assert_eq!(sup.get(node).expect("info").status, NodeStatus::Dead);
        assert_eq!(status_events(&store).len(), 1, "só a transição para Dead");
    }

    /// ADR 0019 §3: `Busy` + 3 amostras consecutivas sem progresso → `NodeStalled` UMA vez;
    /// a amostra seguinte (ainda congelada) NÃO re-emite.
    #[test]
    #[serial]
    fn busy_frozen_tail_warns_once_at_threshold() {
        let (_tmp, sup, mut store, node) = setup("stall");
        let mut eng = LifecycleEngine::new();
        eng.transition(&sup, &mut store, node, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("→ Busy");
        let frozen = LifecycleEngine::tail_hash("❯ aguardando algo que nunca chega");

        // 1ª amostra: sem base → progresso (nunca stall instantâneo).
        assert_eq!(
            eng.sample(&sup, &mut store, node, 7, frozen).expect("s1"),
            SampleOutcome::Progress
        );
        assert_eq!(
            eng.sample(&sup, &mut store, node, 7, frozen).expect("s2"),
            SampleOutcome::NoProgress(1)
        );
        assert_eq!(
            eng.sample(&sup, &mut store, node, 7, frozen).expect("s3"),
            SampleOutcome::NoProgress(2)
        );
        assert_eq!(
            eng.sample(&sup, &mut store, node, 7, frozen).expect("s4"),
            SampleOutcome::StalledWarned,
            "3ª amostra sem progresso cruza o threshold"
        );
        assert_eq!(stall_events(&store), 1);
        assert!(eng.is_stalled(node));

        // Persistindo congelado: contador segue (p/ breaker F1-0-7), mas SEM re-emitir.
        assert_eq!(
            eng.sample(&sup, &mut store, node, 7, frozen).expect("s5"),
            SampleOutcome::NoProgress(4)
        );
        assert_eq!(stall_events(&store), 1, "NodeStalled é 1× na transição");
        assert_eq!(eng.stall_samples(node), 4);
    }

    /// Anti-falso-positivo (criterio 2): "thinking longo" com grid MUDANDO nunca stalla.
    #[test]
    #[serial]
    fn changing_tail_never_stalls() {
        let (_tmp, sup, mut store, node) = setup("nofp");
        let mut eng = LifecycleEngine::new();
        eng.transition(&sup, &mut store, node, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("→ Busy");
        for i in 0..6u32 {
            let out = eng
                .sample(
                    &sup,
                    &mut store,
                    node,
                    u64::from(i),
                    LifecycleEngine::tail_hash(&format!("pensando… passo {i}")),
                )
                .expect("sample");
            assert_eq!(out, SampleOutcome::Progress);
        }
        assert_eq!(stall_events(&store), 0);
        assert!(!eng.is_stalled(node));
    }

    /// ADR 0019 §2b: evento de domínio atribuível conta como progresso mesmo com tail congelado.
    #[test]
    #[serial]
    fn domain_activity_counts_as_progress() {
        let (_tmp, sup, mut store, node) = setup("domact");
        let mut eng = LifecycleEngine::new();
        eng.transition(&sup, &mut store, node, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("→ Busy");
        let frozen = LifecycleEngine::tail_hash("tail imóvel");

        eng.sample(&sup, &mut store, node, 1, frozen).expect("s1"); // baseline
        assert_eq!(
            eng.sample(&sup, &mut store, node, 1, frozen).expect("s2"),
            SampleOutcome::NoProgress(1)
        );
        eng.note_domain_activity(node); // ex.: RouteDelivered atribuído ao nó
        assert_eq!(
            eng.sample(&sup, &mut store, node, 1, frozen).expect("s3"),
            SampleOutcome::Progress,
            "evento de domínio zera o relógio"
        );
        assert_eq!(eng.stall_samples(node), 0);
        assert_eq!(stall_events(&store), 0);
    }

    /// ADR 0019 §4: o relógio SÓ corre em `Busy` — `Blocked`/`Idle` congelados não acumulam;
    /// e a transição (→Busy) zera o contador (recomeça do zero).
    #[test]
    #[serial]
    fn blocked_and_idle_do_not_accumulate_stall_clock() {
        let (_tmp, sup, mut store, node) = setup("clock");
        let mut eng = LifecycleEngine::new();
        let frozen = LifecycleEngine::tail_hash("parado");

        eng.transition(
            &sup,
            &mut store,
            node,
            NodeStatus::Blocked,
            reason::CUSTODY_GATE,
        )
        .expect("→ Blocked");
        for _ in 0..5 {
            assert_eq!(
                eng.sample(&sup, &mut store, node, 3, frozen).expect("s"),
                SampleOutcome::Quiet
            );
        }
        assert_eq!(eng.stall_samples(node), 0, "Blocked não acumula (§4)");
        assert_eq!(stall_events(&store), 0);

        eng.transition(&sup, &mut store, node, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("→ Busy");
        // pós-transição o contador está zerado; 3 amostras congeladas até o warn.
        assert_eq!(
            eng.sample(&sup, &mut store, node, 3, frozen).expect("b1"),
            SampleOutcome::NoProgress(1),
            "baseline de hash preservada através da transição"
        );
        eng.sample(&sup, &mut store, node, 3, frozen).expect("b2");
        assert_eq!(
            eng.sample(&sup, &mut store, node, 3, frozen).expect("b3"),
            SampleOutcome::StalledWarned
        );
        assert_eq!(stall_events(&store), 1);
    }

    /// Decisão de design: nó quieto (Ready/Idle) com output NOVO (ciclo avançou) → `Busy`,
    /// com o evento `from=Ready to=Busy reason=pty_output` no log.
    #[test]
    #[serial]
    fn quiet_node_with_new_output_becomes_busy() {
        let (_tmp, sup, mut store, node) = setup("busy");
        let mut eng = LifecycleEngine::new();
        eng.transition(&sup, &mut store, node, NodeStatus::Ready, reason::SPAWN)
            .expect("→ Ready");
        let h = LifecycleEngine::tail_hash("❯ ");

        assert_eq!(
            eng.sample(&sup, &mut store, node, 1, h).expect("s1"),
            SampleOutcome::Quiet,
            "1ª amostra: sem delta de ciclo conhecido"
        );
        assert_eq!(
            eng.sample(&sup, &mut store, node, 2, h).expect("s2"),
            SampleOutcome::BecameBusy
        );
        assert_eq!(sup.get(node).expect("info").status, NodeStatus::Busy);
        let evs = status_events(&store);
        assert_eq!(
            evs.last().expect("última transição"),
            &(
                "Ready".to_string(),
                "Busy".to_string(),
                reason::PTY_OUTPUT.to_string()
            )
        );
    }

    /// ADR 0019 §5: amostras são EFÊMERAS — N amostras produzem só vereditos no log
    /// (`NodeStatusChanged`/`NodeStalled`), nenhum evento de "heartbeat/sample".
    #[test]
    #[serial]
    fn samples_are_ephemeral_only_verdicts_are_logged() {
        let (_tmp, sup, mut store, node) = setup("ephemeral");
        let mut eng = LifecycleEngine::new();
        eng.transition(&sup, &mut store, node, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("→ Busy");
        for i in 0..10u64 {
            eng.sample(
                &sup,
                &mut store,
                node,
                i,
                LifecycleEngine::tail_hash(&format!("linha {i}")),
            )
            .expect("sample");
        }
        let kinds: Vec<String> = store
            .events()
            .expect("events")
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert!(
            kinds
                .iter()
                .all(|k| k == "NodeStatusChanged" || k == "NodeStalled"),
            "só vereditos entram no log (amostras nunca): {kinds:?}"
        );
        assert_eq!(
            kinds.len(),
            1,
            "10 amostras com progresso = só a transição inicial"
        );
    }

    /// Critério de aceite (4): replay do log reconstrói o último estado de cada nó IDÊNTICO
    /// ao roster vivo do Supervisor (inv#4) — inclusive o flag de stall consultável.
    #[test]
    #[serial]
    fn replay_reconstructs_roster_statuses_identically() {
        let tmp = TempDir::new("replay");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let sup = Supervisor::new();
        let mut eng = LifecycleEngine::new();

        let a = sup.register("A", None, Box::new(std::io::sink()));
        let b = sup.register("B", None, Box::new(std::io::sink()));
        let c = sup.register("C", None, Box::new(std::io::sink()));
        for n in [a, b, c] {
            store
                .append(&DomainEvent::NodeAdded {
                    node: n,
                    kind: "Terminal".into(),
                    x: 0.0,
                    y: 0.0,
                    requested_by: None,
                })
                .expect("NodeAdded");
            eng.transition(&sup, &mut store, n, NodeStatus::Ready, reason::SPAWN)
                .expect("→ Ready");
        }

        // A: ciclo completo de trabalho → Idle.
        eng.transition(&sup, &mut store, a, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("A busy");
        eng.on_end_of_response(&sup, &mut store, a).expect("A idle");

        // B: Busy + stall warnado (flag consultável na projeção).
        eng.transition(&sup, &mut store, b, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("B busy");
        let frozen = LifecycleEngine::tail_hash("B parado");
        for _ in 0..4 {
            eng.sample(&sup, &mut store, b, 9, frozen)
                .expect("B sample");
        }

        // C: morreu trabalhando.
        eng.transition(&sup, &mut store, c, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("C busy");
        eng.on_pty_exit(&sup, &mut store, c).expect("C dead");

        let projected = store.project().expect("project");
        for n in [a, b, c] {
            let live = sup.get(n).expect("roster vivo").status;
            let proj = projected.nodes.get(&n).expect("nó projetado");
            assert_eq!(
                proj.status.as_deref(),
                Some(live.as_str()),
                "replay deve bater com o roster vivo (inv#4) para o nó {n}"
            );
        }
        assert!(
            projected.nodes.get(&b).expect("B").stalled,
            "stall de B consultável na projeção"
        );
        assert!(!projected.nodes.get(&a).expect("A").stalled);
        assert_eq!(stall_events(&store), 1);
    }

    // ── F1-0-8 (P4): fechar o ciclo de vida no log — nós-fantasma de gerações anteriores ──

    /// Conta eventos do log por (kind, predicado no payload) — o "grep" do critério 2.
    fn count_events(
        store: &EventStore,
        kind: &str,
        pred: impl Fn(&serde_json::Value) -> bool,
    ) -> usize {
        store
            .events()
            .expect("events")
            .iter()
            .filter(|r| r.kind == kind && pred(&r.payload))
            .count()
    }

    /// Critérios 1+2 da story: 2 ciclos de "abre → mata → reabre"; o replay do log final
    /// produz roster idêntico ao Supervisor vivo (N nós, não 3N), e a aritmética de
    /// eventos `NodeAdded − mortes == nós vivos` fecha — o mesmo grep que revelou o P4.
    #[test]
    #[serial]
    fn reopening_closes_ghosts_of_previous_generations() {
        let tmp = TempDir::new("ghosts");
        let add_terminal = |store: &mut EventStore, sup: &Supervisor, name: &str| {
            let n = sup.register(name, None, Box::new(std::io::sink()));
            store
                .append(&DomainEvent::NodeAdded {
                    node: n,
                    kind: "Terminal".into(),
                    x: 0.0,
                    y: 0.0,
                    requested_by: None,
                })
                .expect("NodeAdded");
            n
        };

        // ── Geração 1: 3 terminais vivos + 1 nota (sem processo) — e o app "morre" (drop). ──
        let note;
        {
            let mut store = EventStore::open(tmp.path()).expect("open g1");
            let sup = Supervisor::new();
            let mut eng = LifecycleEngine::new();
            for name in ["A", "B", "C"] {
                let n = add_terminal(&mut store, &sup, name);
                eng.transition(&sup, &mut store, n, NodeStatus::Ready, reason::SPAWN)
                    .expect("→ Ready");
            }
            // Nó-NOTA: nunca recebe status — não é fantasma, deve atravessar intacto.
            note = sup.register("nota", None, Box::new(std::io::sink()));
            store
                .append(&DomainEvent::NodeAdded {
                    node: note,
                    kind: "Note".into(),
                    x: 1.0,
                    y: 1.0,
                    requested_by: None,
                })
                .expect("NodeAdded nota");
        } // kill -9 simulado: nenhum encerramento gracioso de domínio

        // ── Reabertura 1: fecha a geração anterior ANTES da nova; idempotente. ──
        {
            let mut store = EventStore::open(tmp.path()).expect("reopen 1");
            let closed = close_previous_generation(&mut store).expect("close g1");
            assert_eq!(closed.len(), 3, "fecha exatamente os 3 terminais da g1");
            let again = close_previous_generation(&mut store).expect("close 2x");
            assert!(again.is_empty(), "idempotente: nada vivo para fechar");

            let sup = Supervisor::new();
            let mut eng = LifecycleEngine::new();
            for name in ["D", "E"] {
                let n = add_terminal(&mut store, &sup, name);
                eng.transition(&sup, &mut store, n, NodeStatus::Ready, reason::SPAWN)
                    .expect("→ Ready");
            }
        } // morre de novo

        // ── Reabertura 2: fecha a g2; sobe a g3 (1 terminal, que fica Busy). ──
        let mut store = EventStore::open(tmp.path()).expect("reopen 2");
        let closed = close_previous_generation(&mut store).expect("close g2");
        assert_eq!(closed.len(), 2, "fecha exatamente os 2 terminais da g2");

        let sup = Supervisor::new();
        let mut eng = LifecycleEngine::new();
        let f = add_terminal(&mut store, &sup, "F");
        eng.transition(&sup, &mut store, f, NodeStatus::Ready, reason::SPAWN)
            .expect("→ Ready");
        eng.transition(&sup, &mut store, f, NodeStatus::Busy, reason::PTY_OUTPUT)
            .expect("→ Busy");

        // ── Critério 1: replay == roster vivo (1 terminal vivo, não 3N). ──
        let projected = store.project().expect("project");
        let alive: Vec<_> = projected
            .nodes
            .iter()
            .filter(|(_, n)| {
                n.status.is_some() && n.status.as_deref() != Some(NodeStatus::Dead.as_str())
            })
            .collect();
        assert_eq!(
            alive.len(),
            1,
            "replay vê 1 terminal vivo (não os fantasmas)"
        );
        assert_eq!(*alive[0].0, f);
        assert_eq!(
            alive[0].1.status.as_deref(),
            Some(sup.get(f).expect("roster").status.as_str()),
            "status do vivo bate com o Supervisor (inv#4)"
        );
        // A nota atravessou as gerações intacta (sem status, presente, nunca "morta").
        let n = projected.nodes.get(&note).expect("nota presente");
        assert_eq!(n.status, None, "nota não tem ciclo de vida de processo");

        // ── Critério 2: a aritmética do log fecha (o grep que revelou o P4). ──
        let added = count_events(&store, "NodeAdded", |_| true);
        let dead = count_events(&store, "NodeStatusChanged", |p| {
            p.get("status").and_then(|s| s.as_str()) == Some(NodeStatus::Dead.as_str())
        });
        let removed = count_events(&store, "NodeRemoved", |_| true);
        // 7 added (6 terminais + nota) − 5 mortes (3+2) − 0 removed = 2 vivos (F + nota).
        assert_eq!(added - dead - removed, 2, "NodeAdded − mortes == nós vivos");

        // As mortes póstumas carregam a razão canônica e o `from` projetado.
        let posthumous = count_events(&store, "NodeStatusChanged", |p| {
            p.get("reason").and_then(|s| s.as_str()) == Some(reason::APP_REOPENED)
                && p.get("from").and_then(|s| s.as_str()) == Some(NodeStatus::Ready.as_str())
        });
        assert_eq!(posthumous, 5, "toda morte póstuma registra reason+from");
    }
}
