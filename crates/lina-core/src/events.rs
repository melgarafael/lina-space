//! Event Store (W0-5) + Recuperação pós-crash visível (W0-6).
//!
//! **Invariante do `CLAUDE.md`:** o event log é a **fonte da verdade** do domínio.
//! O estado projetado (roster, posições, contadores) é uma *projeção reconstruível*
//! por replay; nunca a autoridade.
//!
//! ## Estratificação (ver [[20 - Arquitetura Tecnica]] §4)
//! - **SQLite WAL** (`events`): log append-only imutável — NUNCA `UPDATE`/`DELETE`.
//!   Projeção rápida e queryável.
//! - **`log.jsonl`** append-only: **espelho redundante** do log (autoridade de
//!   disaster-recovery). Se o `.db` corromper, o estado é reconstruído 100% daqui.
//! - **`snapshots/snap-<seq>.json`**: estado projetado materializado (+ versão) que
//!   encurta o replay.
//!
//! ## Determinismo
//! O reducer é puro e o estado usa `BTreeMap` (ordem estável) → `serde_json` canônico
//! → `fingerprint` idêntico em replays repetidos.
//!
//! ## Upcasting versionado
//! Cada tipo de evento tem `event_version`. No replay, um payload de versão antiga
//! passa por [`upcast`] antes de desserializar — o schema evolui sem quebrar o log.

use std::collections::BTreeMap;
use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lina_host::{HostEvent, NodeId, UiHost};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::plan::Plan;

/// Erros do event store / recuperação.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ───────────────────────────── catálogo de eventos do domínio ─────────────────────────────

/// W3-7 (ADR 0003): motivo TIPADO de uma recusa de roteamento — projetável e assertável do log.
/// Serializa em `snake_case` (`"hop_limit"`, `"loop_detected"`, …) para casar a tabela do ADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    Duplicate,
    HopLimit,
    UnknownSender,
    NoTarget,
    BlockedByAutonomy,
    FanoutGated,
    BudgetExceeded,
    Deadlock,
    LoopDetected,
}

/// W3-7b (ADR 0002): motivo do fechamento de um `await`. Serializa em `snake_case`
/// (`"replied"`/`"timeout"`/`"deadlock"`). `Deadlock` existe para completude do contrato, mas o
/// `AwaitClosed{Deadlock}` NÃO é emitido — a pré-checagem recusa ANTES de abrir o ticket (ADR 0002 §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitReason {
    Replied,
    Timeout,
    Deadlock,
}

/// Catálogo canônico de eventos do domínio (subconjunto da [[20 - Arquitetura Tecnica]]
/// §4.2 suficiente para a Onda 0). Internamente *tagged* por `event` → o nome da
/// variante é o `kind` persistido.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum DomainEvent {
    WorkspaceCreated {
        name: String,
    },
    NodeAdded {
        node: NodeId,
        kind: String,
        x: f64,
        y: f64,
    },
    NodeMoved {
        node: NodeId,
        x: f64,
        y: f64,
    },
    NodeRenamed {
        node: NodeId,
        name: String,
    },
    NodeRoleAssigned {
        node: NodeId,
        role: String,
    },
    NodeStatusChanged {
        node: NodeId,
        status: String,
    },
    NodeRemoved {
        node: NodeId,
    },
    TerminalSpawned {
        node: NodeId,
        cli: String,
    },
    TerminalExited {
        node: NodeId,
    },
    BusMessageSent {
        id: String,
        from: NodeId,
        to: String,
    },
    /// W3-4: um colega anunciou presença (`lina handshake`). Meta/observabilidade — o
    /// roster vivo é do Supervisor; aqui só registramos a entrada no log (`agent.joined`).
    Handshake {
        node: NodeId,
        name: String,
    },
    /// W3-4: o supervisor ROTEOU uma mensagem da mailbox (passou os guardrails 0-4) e a
    /// despachou para `to` (alvo resolvido). `id` = id da mensagem (dedupe/anti-loop).
    MessageRouted {
        id: String,
        from: NodeId,
        to: String,
        intent: String,
        /// W3-7 (A2): turno de origem propagado pelo supervisor — filtro do grafo anti-loop (§2.1).
        #[serde(default)]
        root_cause_id: String,
        /// W3-7 (A2): profundidade da cadeia neste salto (cresce a cada hop herdado).
        #[serde(default)]
        hops: u8,
        /// W3-7 (§2.1): alvo resolvido (NodeId) — aresta `from→to_node` do grafo de handoffs.
        /// `None` para broadcast (não entra no anti-loop por ciclo).
        #[serde(default)]
        to_node: Option<NodeId>,
    },
    /// W3-4: a mensagem `id` foi entregue ao PTY do alvo (`deliver_a2a` faseado concluiu).
    MessageDelivered {
        id: String,
        to: NodeId,
    },
    /// W3-7 (ADR 0003): recusa de guardrail no roteamento — o log é o livro-razão das recusas.
    /// `from`/`to` são NOMES (cobre `unknown_sender`, em que `from` não resolve a `NodeId`).
    /// `duplicate` NÃO é apendado (anti-amplificação A4 — ver `Router::route_message`).
    RouteBlocked {
        id: String,
        reason: BlockReason,
        from: String,
        to: String,
    },
    /// W3-7b (ADR 0002): `A --await--> B` aprovado → a aresta `waiter→target` foi ABERTA no
    /// wait-for-graph (persiste até o reply/timeout). `id` = id da mensagem-pergunta (chave do reply).
    AwaitOpened {
        id: String,
        waiter: NodeId,
        target: NodeId,
        root_cause_id: String,
    },
    /// W3-7b (ADR 0002): o ciclo de `await` `id` fechou (`Replied` pelo `reply_to` do alvo, ou
    /// `Timeout` pelo sweep). Libera a aresta do wait-for-graph (`end_await`).
    AwaitClosed {
        id: String,
        reason: AwaitReason,
    },
    /// W3-5: um item foi semeado no plano compartilhado (`status:todo`, `@owner:?`).
    PlanItemAdded {
        item: String,
        desc: String,
    },
    /// W3-5: uma decisão viva foi adicionada ao plano.
    PlanDecisionAdded {
        text: String,
    },
    /// W3-5: `lina plan claim` aplicado pelo supervisor. `id`=id da mensagem de origem;
    /// `item`=ref do item; `by`=quem reivindicou. Projeta `@owner:by` + `status:doing`.
    PlanClaimed {
        id: String,
        item: String,
        by: String,
    },
    /// W3-5: `lina plan check` aplicado pelo supervisor. Projeta `status:done` (campos como acima).
    PlanChecked {
        id: String,
        item: String,
        by: String,
    },
    NoteUpdated {
        name: String,
        hash: String,
    },
    SnapshotTaken {
        seq: u64,
    },
}

impl DomainEvent {
    /// O `kind` persistido — idêntico à tag interna do serde.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            DomainEvent::WorkspaceCreated { .. } => "WorkspaceCreated",
            DomainEvent::NodeAdded { .. } => "NodeAdded",
            DomainEvent::NodeMoved { .. } => "NodeMoved",
            DomainEvent::NodeRenamed { .. } => "NodeRenamed",
            DomainEvent::NodeRoleAssigned { .. } => "NodeRoleAssigned",
            DomainEvent::NodeStatusChanged { .. } => "NodeStatusChanged",
            DomainEvent::NodeRemoved { .. } => "NodeRemoved",
            DomainEvent::TerminalSpawned { .. } => "TerminalSpawned",
            DomainEvent::TerminalExited { .. } => "TerminalExited",
            DomainEvent::BusMessageSent { .. } => "BusMessageSent",
            DomainEvent::Handshake { .. } => "Handshake",
            DomainEvent::MessageRouted { .. } => "MessageRouted",
            DomainEvent::MessageDelivered { .. } => "MessageDelivered",
            DomainEvent::RouteBlocked { .. } => "RouteBlocked",
            DomainEvent::AwaitOpened { .. } => "AwaitOpened",
            DomainEvent::AwaitClosed { .. } => "AwaitClosed",
            DomainEvent::PlanItemAdded { .. } => "PlanItemAdded",
            DomainEvent::PlanDecisionAdded { .. } => "PlanDecisionAdded",
            DomainEvent::PlanClaimed { .. } => "PlanClaimed",
            DomainEvent::PlanChecked { .. } => "PlanChecked",
            DomainEvent::NoteUpdated { .. } => "NoteUpdated",
            DomainEvent::SnapshotTaken { .. } => "SnapshotTaken",
        }
    }

    /// Versão CORRENTE do schema deste tipo. `NodeAdded` está em v2 (ganhou o campo
    /// `kind`); os demais em v1. O replay sobe versões antigas via [`upcast`].
    #[must_use]
    pub fn current_version(&self) -> u32 {
        match self {
            DomainEvent::NodeAdded { .. } => 2,
            _ => 1,
        }
    }

    /// Reconstrói um evento a partir de um registro persistido: aplica [`upcast`] e
    /// então desserializa na forma corrente.
    fn from_record(
        kind: &str,
        version: u32,
        payload: serde_json::Value,
    ) -> Result<Self, StoreError> {
        let upcasted = upcast(kind, version, payload);
        Ok(serde_json::from_value(upcasted)?)
    }
}

/// Upcasting versionado: transforma um payload de versão antiga na forma corrente.
/// Aditivo e puro — base do replay à prova de evolução de schema.
fn upcast(kind: &str, version: u32, mut payload: serde_json::Value) -> serde_json::Value {
    // NodeAdded v1 → v2: não tinha o campo `kind` (tipo do nó). Default `Terminal`.
    if kind == "NodeAdded" && version < 2 {
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("kind")
                .or_insert_with(|| serde_json::Value::String("Terminal".into()));
        }
    }
    payload
}

/// Registro persistido de um evento (forma comum ao SQLite e ao espelho JSONL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: u64,
    pub ts: u64,
    pub kind: String,
    pub version: u32,
    pub payload: serde_json::Value,
}

// ───────────────────────────── estado projetado + reducer ─────────────────────────────

/// Nó no estado projetado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedNode {
    pub kind: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub x: f64,
    pub y: f64,
    pub cli: Option<String>,
}

/// Estado projetado do domínio. `BTreeMap` garante ordem estável → fingerprint estável.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectedState {
    pub workspace_name: Option<String>,
    pub nodes: BTreeMap<NodeId, ProjectedNode>,
    pub bus_messages: u64,
    /// W3-5: plano compartilhado projetado do log (`#[serde(default)]` → snapshots antigos
    /// desserializam com plano vazio). É a fonte do `.lina/plan.md` (escritor único: supervisor).
    #[serde(default)]
    pub plan: Plan,
}

impl ProjectedState {
    /// Fingerprint determinístico (FNV-1a sobre o JSON canônico ordenado). Replays do
    /// mesmo log produzem o MESMO fingerprint.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let json = serde_json::to_string(self).unwrap_or_default();
        fnv1a(json.as_bytes())
    }
}

/// Reducer puro: aplica um evento ao estado projetado.
pub fn apply(state: &mut ProjectedState, event: &DomainEvent) {
    match event {
        DomainEvent::WorkspaceCreated { name } => {
            state.workspace_name = Some(name.clone());
            // O cabeçalho do plano espelha o nome do workspace (projeção → `# Plano — <name>`).
            state.plan.workspace = name.clone();
        }
        DomainEvent::NodeAdded { node, kind, x, y } => {
            state.nodes.insert(
                *node,
                ProjectedNode {
                    kind: kind.clone(),
                    name: None,
                    role: None,
                    status: None,
                    x: *x,
                    y: *y,
                    cli: None,
                },
            );
        }
        DomainEvent::NodeMoved { node, x, y } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.x = *x;
                n.y = *y;
            }
        }
        DomainEvent::NodeRenamed { node, name } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.name = Some(name.clone());
            }
        }
        DomainEvent::NodeRoleAssigned { node, role } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.role = Some(role.clone());
            }
        }
        DomainEvent::NodeStatusChanged { node, status } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.status = Some(status.clone());
            }
        }
        DomainEvent::NodeRemoved { node } => {
            state.nodes.remove(node);
        }
        DomainEvent::TerminalSpawned { node, cli } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.cli = Some(cli.clone());
                n.status = Some("Running".into());
            }
        }
        DomainEvent::TerminalExited { node } => {
            if let Some(n) = state.nodes.get_mut(node) {
                n.status = Some("Dead".into());
            }
        }
        // W3-4: roteamento conta como mensagem do bus (mesma contagem de BusMessageSent).
        DomainEvent::BusMessageSent { .. } | DomainEvent::MessageRouted { .. } => {
            state.bus_messages += 1;
        }
        // W3-5: o plano é uma projeção do log — fatos já validados quando logados, aplicados
        // incondicionalmente no replay (a validação vive no comando, não na projeção).
        DomainEvent::PlanItemAdded { item, desc } => {
            state.plan.apply_item_added(item.clone(), desc.clone());
        }
        DomainEvent::PlanDecisionAdded { text } => state.plan.apply_decision_added(text.clone()),
        DomainEvent::PlanClaimed { item, by, .. } => state.plan.apply_claimed(item, by),
        DomainEvent::PlanChecked { item, by, .. } => state.plan.apply_checked(item, by),
        // Handshake/entrega/recusa/notas/snapshot são META (observabilidade no log) — sem efeito
        // na projeção do canvas; o roster vivo é do Supervisor, não da projeção.
        DomainEvent::Handshake { .. }
        | DomainEvent::MessageDelivered { .. }
        | DomainEvent::RouteBlocked { .. }
        | DomainEvent::AwaitOpened { .. }
        | DomainEvent::AwaitClosed { .. }
        | DomainEvent::NoteUpdated { .. }
        | DomainEvent::SnapshotTaken { .. } => {}
    }
}

// ───────────────────────────── Event Store ─────────────────────────────

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    seq     INTEGER PRIMARY KEY,
    ts      INTEGER NOT NULL,
    kind    TEXT    NOT NULL,
    version INTEGER NOT NULL,
    payload TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS snapshots (
    seq   INTEGER PRIMARY KEY,
    ts    INTEGER NOT NULL,
    state TEXT    NOT NULL
);
";

/// **W3-7b §4.3 — reconciliação do path do espelho.** O design e o gate da Onda 3 verificam
/// `.lina/events/log.jsonl`; o código histórico escrevia `<store>/bus.jsonl`. DECISÃO: o espelho
/// JSONL chama-se **`log.jsonl`** (alinha com o gate + design; o `EventStore` já é aberto em
/// `.lina/events/`). Um único nome canônico — escritor (`open`) E leitor (`rebuild_from_jsonl`)
/// usam esta const; nada de 3º nome. (Stores de dev com `bus.jsonl` antigo ficam órfãos — sem
/// migração no MVP, pois não há dados persistentes de produção.)
const JSONL_FILE: &str = "log.jsonl";

/// Event store de um workspace: SQLite WAL (`events`) + espelho `log.jsonl` (§4.3) +
/// snapshots. Single-thread (a `Connection` do rusqlite não é `Sync`); o supervisor
/// futuro a embrulha sob lock.
pub struct EventStore {
    conn: Connection,
    dir: PathBuf,
    db_path: PathBuf,
    jsonl_path: PathBuf,
    snapshots_dir: PathBuf,
    next_seq: u64,
}

impl EventStore {
    /// Abre (ou cria) o event store em `dir`. **Não** faz recuperação — use
    /// [`EventStore::open_or_recover`] para a abertura resiliente a corrupção (W0-6).
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let snapshots_dir = dir.join("snapshots");
        std::fs::create_dir_all(&snapshots_dir)?;
        let db_path = dir.join("lina.db");
        let jsonl_path = dir.join(JSONL_FILE);

        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        let next_seq: i64 =
            conn.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM events", [], |r| {
                r.get(0)
            })?;

        Ok(Self {
            conn,
            dir,
            db_path,
            jsonl_path,
            snapshots_dir,
            next_seq: next_seq as u64,
        })
    }

    /// Abertura RESILIENTE (W0-6): roda `PRAGMA integrity_check`; se o `.db` estiver
    /// corrompido, **preserva** o arquivo (`lina.db.corrupt-<ts>`), reconstrói do
    /// `log.jsonl` e emite `Recovering` → `Recovered` no `UiHost` — nunca silencioso.
    pub fn open_or_recover(dir: impl AsRef<Path>, ui: &mut dyn UiHost) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("lina.db");

        if !db_path.exists() || !db_is_corrupt(&db_path) {
            return Self::open(&dir); // novo ou saudável → abertura normal, sem recovery
        }

        // Corrompido (lição Codex #21750): recuperação VISÍVEL.
        ui.on_event(HostEvent::Recovering);
        let preserved = preserve_corrupt(&db_path)?;
        tracing::warn!(corrupt = %preserved.display(), "banco corrompido preservado; reconstruindo do JSONL");
        let store = Self::rebuild_from_jsonl(&dir)?;
        ui.on_event(HostEvent::Recovered);
        Ok(store)
    }

    /// Reconstrói o `.db` (novo) reaplicando o espelho `log.jsonl`. Linhas ilegíveis
    /// são postas em **quarentena** (warn + segue) — não engole erro silenciosamente.
    fn rebuild_from_jsonl(dir: &Path) -> Result<Self, StoreError> {
        let mut store = Self::open(dir)?; // `lina.db` novo+schema (o corrompido já foi renomeado)
        let jsonl = dir.join(JSONL_FILE);
        if !jsonl.exists() {
            return Ok(store);
        }
        let file = std::fs::File::open(&jsonl)?;
        let reader = std::io::BufReader::new(file);
        for (lineno, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(line = lineno, "linha JSONL ilegível, quarentena: {e}");
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<EventRecord>(&line) {
                Ok(rec) => store.db_insert(&rec)?,
                Err(e) => tracing::warn!(line = lineno, "linha JSONL corrompida, quarentena: {e}"),
            }
        }
        Ok(store)
    }

    /// Anexa um evento do domínio (versão CORRENTE) ao log: INSERT no SQLite +
    /// linha no espelho JSONL append-only. Devolve o `seq` atribuído.
    pub fn append(&mut self, event: &DomainEvent) -> Result<u64, StoreError> {
        let payload = serde_json::to_value(event)?;
        self.insert_raw(event.kind(), event.current_version(), payload)
    }

    /// Insere um registro CRU (`kind`/`version`/`payload`) — usado por [`append`], por
    /// importação e para gravar versões ANTIGAS (migração / teste de upcaster). Escreve
    /// no SQLite e no espelho JSONL.
    pub fn insert_raw(
        &mut self,
        kind: &str,
        version: u32,
        payload: serde_json::Value,
    ) -> Result<u64, StoreError> {
        let rec = EventRecord {
            seq: self.next_seq,
            ts: now_millis(),
            kind: kind.to_string(),
            version,
            payload,
        };
        self.db_insert(&rec)?;
        append_jsonl(&self.jsonl_path, &rec)?;
        Ok(rec.seq)
    }

    /// INSERT apenas no SQLite (sem tocar no JSONL) — usado pelo rebuild, em que o
    /// JSONL é a FONTE (re-escrevê-lo duplicaria o espelho).
    fn db_insert(&mut self, rec: &EventRecord) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO events (seq, ts, kind, version, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rec.seq as i64,
                rec.ts as i64,
                rec.kind,
                rec.version as i64,
                serde_json::to_string(&rec.payload)?
            ],
        )?;
        if rec.seq + 1 > self.next_seq {
            self.next_seq = rec.seq + 1;
        }
        Ok(())
    }

    /// Estado projetado = último snapshot + replay dos eventos após ele (com upcasting).
    /// Determinístico.
    pub fn project(&self) -> Result<ProjectedState, StoreError> {
        let (mut state, from_seq) = match self.latest_snapshot()? {
            Some((seq, st)) => (st, seq),
            None => (ProjectedState::default(), 0),
        };
        let mut stmt = self
            .conn
            .prepare("SELECT kind, version, payload FROM events WHERE seq > ?1 ORDER BY seq ASC")?;
        let rows = stmt.query_map([from_seq as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (kind, version, payload_str) = row?;
            let payload: serde_json::Value = serde_json::from_str(&payload_str)?;
            let event = DomainEvent::from_record(&kind, version as u32, payload)?;
            apply(&mut state, &event);
        }
        Ok(state)
    }

    /// Materializa um snapshot do estado projetado (tabela + arquivo espelho em
    /// `snapshots/`, para resiliência §4.6). Devolve o `seq` coberto.
    pub fn take_snapshot(&mut self) -> Result<u64, StoreError> {
        let state = self.project()?;
        let seq = self.next_seq.saturating_sub(1);
        let state_json = serde_json::to_string(&state)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO snapshots (seq, ts, state) VALUES (?1, ?2, ?3)",
            params![seq as i64, now_millis() as i64, state_json],
        )?;
        std::fs::write(
            self.snapshots_dir.join(format!("snap-{seq}.json")),
            &state_json,
        )?;
        Ok(seq)
    }

    fn latest_snapshot(&self) -> Result<Option<(u64, ProjectedState)>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT seq, state FROM snapshots ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .ok();
        match row {
            Some((seq, state_json)) => {
                let state: ProjectedState = serde_json::from_str(&state_json)?;
                Ok(Some((seq as u64, state)))
            }
            None => Ok(None),
        }
    }

    /// Nº de eventos no log (queries só-leitura).
    pub fn event_count(&self) -> Result<u64, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Registros do log em ordem de `seq` (só-leitura). Permite inspecionar os CAMPOS de um evento
    /// no registro (ex.: asserir `id`/`item`/`by` de um `PlanClaimed`), não só sua presença.
    pub fn events(&self) -> Result<Vec<EventRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, ts, kind, version, payload FROM events ORDER BY seq ASC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, ts, kind, version, payload_str) = row?;
            out.push(EventRecord {
                seq: seq as u64,
                ts: ts as u64,
                kind,
                version: version as u32,
                payload: serde_json::from_str(&payload_str)?,
            });
        }
        Ok(out)
    }

    /// Caminho do diretório do workspace.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Caminho do banco SQLite.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

// ───────────────────────────── helpers ─────────────────────────────

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Anexa um registro de evento ao espelho JSONL (append-only, uma linha por evento).
fn append_jsonl(path: &Path, rec: &EventRecord) -> Result<(), StoreError> {
    let line = serde_json::to_string(rec)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

/// `true` se o `.db` está corrompido: `integrity_check` falha/≠ "ok", OU uma varredura
/// completa da tabela `events` (se existir) erra ("disk image is malformed").
fn db_is_corrupt(path: &Path) -> bool {
    let Ok(conn) = Connection::open(path) else {
        return true;
    };
    let integrity_bad = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .map(|s| s != "ok")
        .unwrap_or(true);
    if integrity_bad {
        return true;
    }
    // `integrity_check` pode passar com corrupção localizada; varre a tabela para
    // forçar a leitura das páginas. Tabela ausente = db novo/vazio (não é corrupção).
    let has_events = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='events'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .is_ok();
    if !has_events {
        return false;
    }
    let scan_ok = match conn.prepare("SELECT seq, kind, version, payload FROM events") {
        Ok(mut stmt) => match stmt.query_map([], |r| r.get::<_, i64>(0)) {
            Ok(rows) => rows.collect::<Result<Vec<i64>, _>>().is_ok(),
            Err(_) => false,
        },
        Err(_) => false,
    };
    !scan_ok
}

/// Renomeia o `.db` corrompido para `<nome>.corrupt-<ts>` (PRESERVA, nunca apaga) e
/// remove os arquivos `-wal`/`-shm` órfãos para não reanexar lixo ao banco novo.
fn preserve_corrupt(db_path: &Path) -> Result<PathBuf, StoreError> {
    let file_name = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("lina.db");
    let corrupt = db_path.with_file_name(format!("{file_name}.corrupt-{}", now_nanos()));
    std::fs::rename(db_path, &corrupt)?;
    for ext in ["-wal", "-shm"] {
        let mut side = db_path.as_os_str().to_os_string();
        side.push(ext);
        let _ = std::fs::remove_file(PathBuf::from(side));
    }
    Ok(corrupt)
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use uuid::Uuid;

    /// Diretório temporário único; removido no `Drop` (best-effort).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-evt-{tag}-{}", Uuid::now_v7()));
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

    /// Host de UI gravador para checar `Recovering`/`Recovered`.
    #[derive(Default)]
    struct RecordingUi {
        events: Vec<HostEvent>,
    }
    impl UiHost for RecordingUi {
        fn on_event(&mut self, event: HostEvent) {
            self.events.push(event);
        }
    }

    fn seed(store: &mut EventStore) -> (NodeId, NodeId) {
        let dev = Uuid::now_v7();
        let qa = Uuid::now_v7();
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "App X".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeAdded {
                node: dev,
                kind: "Terminal".into(),
                x: 10.0,
                y: 20.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeAdded {
                node: qa,
                kind: "Terminal".into(),
                x: 200.0,
                y: 50.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeMoved {
                node: dev,
                x: 15.0,
                y: 25.0,
            })
            .unwrap();
        store
            .append(&DomainEvent::TerminalSpawned {
                node: dev,
                cli: "claude".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::NodeRoleAssigned {
                node: dev,
                role: "dev".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::BusMessageSent {
                id: "msg_1".into(),
                from: dev,
                to: "role:qa".into(),
            })
            .unwrap();
        store
            .append(&DomainEvent::BusMessageSent {
                id: "msg_2".into(),
                from: qa,
                to: "node:dev".into(),
            })
            .unwrap();
        (dev, qa)
    }

    /// W0-5 (i): grava uma sequência, faz replay e o estado projetado bate.
    #[test]
    #[serial]
    fn replay_projects_expected_state() {
        let tmp = TempDir::new("replay");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let (dev, qa) = seed(&mut store);

        let state = store.project().expect("project");
        assert_eq!(state.workspace_name.as_deref(), Some("App X"));
        assert_eq!(state.nodes.len(), 2);
        assert_eq!(state.bus_messages, 2);

        let dev_node = state.nodes.get(&dev).expect("dev no estado");
        assert_eq!((dev_node.x, dev_node.y), (15.0, 25.0)); // NodeMoved aplicado
        assert_eq!(dev_node.cli.as_deref(), Some("claude")); // TerminalSpawned
        assert_eq!(dev_node.role.as_deref(), Some("dev"));
        assert_eq!(dev_node.status.as_deref(), Some("Running"));
        assert!(state.nodes.contains_key(&qa));
    }

    /// W0-5 (ii): replay 2x do mesmo log dá estado byte-idêntico (fingerprint igual).
    #[test]
    #[serial]
    fn replay_is_deterministic() {
        let tmp = TempDir::new("determinism");
        let mut store = EventStore::open(tmp.path()).expect("open");
        seed(&mut store);

        let f1 = store.project().expect("project 1").fingerprint();
        let f2 = store.project().expect("project 2").fingerprint();
        assert_eq!(
            f1, f2,
            "replays do mesmo log devem ter fingerprint idêntico"
        );

        // E com snapshot: snapshot + 0 eventos novos = mesmo estado.
        store.take_snapshot().expect("snapshot");
        let f3 = store.project().expect("project pós-snapshot").fingerprint();
        assert_eq!(f1, f3, "snapshot + replay vazio = mesmo estado");
    }

    /// W0-5 (iii): um evento com `event_version` ANTIGO passa pelo upcaster e
    /// reproduz sem erro (NodeAdded v1, sem o campo `kind`, vira `Terminal`).
    #[test]
    #[serial]
    fn legacy_event_version_is_upcasted_on_replay() {
        let tmp = TempDir::new("upcast");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let legacy_node = Uuid::now_v7();

        // payload v1: SEM `kind` (o campo só existe a partir da v2).
        let v1_payload = serde_json::json!({
            "event": "NodeAdded",
            "node": legacy_node,
            "x": 1.0,
            "y": 2.0
        });
        store
            .insert_raw("NodeAdded", 1, v1_payload)
            .expect("insert legado v1");

        // replay não pode errar e o nó deve ganhar kind=Terminal via upcaster.
        let state = store.project().expect("replay com upcasting");
        let n = state.nodes.get(&legacy_node).expect("nó legado projetado");
        assert_eq!(n.kind, "Terminal", "upcaster deve injetar kind=Terminal");
        assert_eq!((n.x, n.y), (1.0, 2.0));
    }

    /// W0-6: corrompe o `.db` (lixo no meio), reabre via `open_or_recover` e prova:
    /// corrompido PRESERVADO, estado reconstruído do JSONL IDÊNTICO, e o par
    /// `Recovering`→`Recovered` emitido.
    #[test]
    #[serial]
    fn corrupt_db_recovers_from_jsonl_visibly() {
        let tmp = TempDir::new("recover");

        // 1) Estado bom + snapshot; captura o fingerprint pré-corrupção.
        let pre_fp = {
            let mut store = EventStore::open(tmp.path()).expect("open");
            seed(&mut store);
            store.take_snapshot().expect("snapshot");
            let fp = store.project().expect("project").fingerprint();
            // checkpoint para materializar tudo no .db principal antes de corromper.
            store
                .conn
                .pragma_update(None, "wal_checkpoint", "TRUNCATE")
                .ok();
            fp
        }; // store dropado → conexão fechada

        // 2) Corrompe o .db: lixo no MEIO do arquivo (lição Codex #21750).
        let db_path = tmp.path().join("lina.db");
        corrupt_middle(&db_path);
        assert!(
            db_is_corrupt(&db_path),
            "o .db deveria ser detectado corrompido"
        );

        // 3) Reabre resiliente, com UI gravando os eventos de recuperação.
        let mut ui = RecordingUi::default();
        let store2 = EventStore::open_or_recover(tmp.path(), &mut ui).expect("open_or_recover");

        // 4a) o corrompido foi PRESERVADO (não apagado).
        let corrupt_files: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(
            corrupt_files.len(),
            1,
            "exatamente 1 arquivo *.corrupt-* preservado"
        );

        // 4b) estado reconstruído do JSONL é IDÊNTICO ao pré-corrupção.
        let post_fp = store2
            .project()
            .expect("project pós-recovery")
            .fingerprint();
        assert_eq!(post_fp, pre_fp, "reconstrução do JSONL deve ser idêntica");

        // 4c) recuperação VISÍVEL: par Recovering → Recovered.
        assert!(
            matches!(
                ui.events.as_slice(),
                [HostEvent::Recovering, HostEvent::Recovered]
            ),
            "deveria emitir exatamente Recovering depois Recovered; veio {:?}",
            ui.events
        );
    }

    /// `open_or_recover` num db SAUDÁVEL não dispara recuperação (sem eventos de UI).
    #[test]
    #[serial]
    fn healthy_open_does_not_recover() {
        let tmp = TempDir::new("healthy");
        {
            let mut store = EventStore::open(tmp.path()).expect("open");
            seed(&mut store);
        }
        let mut ui = RecordingUi::default();
        let _store = EventStore::open_or_recover(tmp.path(), &mut ui).expect("open_or_recover");
        assert!(
            ui.events.is_empty(),
            "db saudável não deve emitir Recovering/Recovered"
        );
    }

    /// Sobrescreve um trecho contíguo no MEIO do arquivo com lixo (0xEE).
    fn corrupt_middle(path: &Path) {
        use std::io::{Seek, SeekFrom, Write};
        let len = std::fs::metadata(path).expect("metadata").len();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("abrir p/ corromper");
        let start = len / 4;
        let span = (len / 2).max(1);
        file.seek(SeekFrom::Start(start)).expect("seek");
        let garbage = vec![0xEE_u8; span as usize];
        file.write_all(&garbage).expect("escrever lixo");
        file.flush().expect("flush");
    }
}
