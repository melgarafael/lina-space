//! F1-1-3 — **pipeline de hooks de CLI em tempo real, com degradação graciosa.**
//!
//! O canal: o spawn gera os hooks no CLI (Claude Code 1ª classe via handler **HTTP** do
//! `.claude/settings.json` — matriz 13.10 achado 1; a montagem do settings vive no
//! `lina-bootstrap`) → o CLI POSTa em `http://127.0.0.1:<porta>/hook/<token>/<kind>` →
//! o [`HookListener`] normaliza no contrato único [`HookEvent`] e publica num canal
//! tipado (`subscribe()` — emissão DESACOPLADA: nenhum `DomainEvent` nasce aqui; a
//! conversão p/ evento de domínio é fiação coordenada, mesmo padrão `CliDetection`).
//!
//! ## Doutrina de identidade (critério 2 da story; família ADR 0006/0007)
//! - A ATRIBUIÇÃO de um POST a um nó vem **exclusivamente do TOKEN opaco** criado por
//!   [`HookListener::register_node`] no spawn — **nenhum campo do payload** (escrito
//!   pelo agente/CLI) decide a que nó o evento pertence.
//! - TODA telemetria de hook nasce **`trusted: false`**: serve à timeline/observação,
//!   **jamais** a roteamento, permissão ou autoridade (o gate continua sendo o guard
//!   W3-6 — hooks aqui são observabilidade, não segurança).
//! - O token é atribuição de melhor esforço, não fronteira de SO: same-uid pode ler o
//!   settings de outro cwd (limite processual documentado — 13.14). Por isso o
//!   `trusted:false` é INCONDICIONAL, não dependente do token.
//!
//! ## Degradação graciosa (o centro da story)
//! CLI sem hooks (`capabilities.hooks=false` no TOML — inv#3) → [`HookPipeline`] em
//! [`HooksCapability::GridOnly`]: o aviso "atividade via grid apenas" sai **uma única
//! vez**, nenhum settings é gerado, nada quebra — as camadas seguintes (timeline,
//! dashboard) assumem a ausência sem erro. Kinds que não consumimos (os 27 do Claude
//! Code ≫ os 6 daqui) são aceitos e ignorados — o hook do CLI nunca recebe erro.
//!
//! Listener **LOOPBACK-ONLY** (inv#2): bind fixo em `127.0.0.1`, porta efêmera.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use lina_cli_profiles::CliProfile;
use tokio::sync::broadcast;

/// Os kinds de observabilidade que o pipeline CONSOME (subconjunto dos 27 do Claude
/// Code — 13.10; outros CLIs cobrem menos: capability por TOML, nunca paridade assumida).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    PreToolUse,
    PostToolUse,
    Notification,
    SubagentStart,
    SubagentStop,
    Stop,
}

impl HookKind {
    /// Parse do segmento de URL (o nome canônico do evento no CLI).
    #[must_use]
    pub fn from_path(s: &str) -> Option<Self> {
        match s {
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            "Notification" => Some(Self::Notification),
            "SubagentStart" => Some(Self::SubagentStart),
            "SubagentStop" => Some(Self::SubagentStop),
            "Stop" => Some(Self::Stop),
            _ => None,
        }
    }

    /// Nome canônico (o mesmo do settings/URL).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::Notification => "Notification",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::Stop => "Stop",
        }
    }

    /// Lista completa — usada pelo gerador de settings (lina-bootstrap) p/ registrar
    /// os handlers HTTP por evento.
    pub const ALL: [HookKind; 6] = [
        Self::PreToolUse,
        Self::PostToolUse,
        Self::Notification,
        Self::SubagentStart,
        Self::SubagentStop,
        Self::Stop,
    ];
}

/// **O contrato único** que o core consome (story: `HookEvent{node_id, kind, tool_name,
/// ts}` + identidade não-confiável). `ts` é o relógio de CHEGADA no listener (ms) — é
/// dele que a timeline deriva o "Running: Bash (X s)".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEvent {
    /// Nó atribuído pelo TOKEN de spawn (nunca por campo do payload).
    pub node_id: String,
    pub kind: HookKind,
    /// `tool_name` do payload (`PreToolUse`/`PostToolUse` do Claude Code — 13.10).
    pub tool_name: Option<String>,
    /// Id do subagente (`SubagentStart`/`Stop`) — telemetria NÃO-decisória (13.5 achado 8).
    pub subagent_id: Option<String>,
    /// Id do pai declarado no payload, quando houver (idem: não-decisório).
    pub parent_id: Option<String>,
    /// Chegada no listener, em ms desde epoch.
    pub ts: u64,
    /// SEMPRE `false` por contrato: hook é observabilidade; autoridade vem do guard/router.
    pub trusted: bool,
}

/// Normalização PURA payload→evento (testável sem rede). Lê só campos descritivos;
/// `node_id` vem do chamador (token), jamais do payload.
#[must_use]
pub fn normalize_payload(
    node_id: &str,
    kind: HookKind,
    payload: &serde_json::Value,
    now_ms: u64,
) -> HookEvent {
    let s = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(str::to_string);
    HookEvent {
        node_id: node_id.to_string(),
        kind,
        tool_name: s("tool_name"),
        subagent_id: s("agent_id").or_else(|| s("subagent_id")),
        parent_id: s("parent_id").or_else(|| s("parent_session_id")),
        ts: now_ms,
        trusted: false,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

struct ListenerState {
    /// token opaco → node_id (escrito SÓ por `register_node`, no spawn).
    tokens: Mutex<HashMap<String, String>>,
    tx: broadcast::Sender<HookEvent>,
}

/// Listener HTTP loopback-only dos hooks. `bind()` → porta efêmera; `register_node()`
/// no spawn de cada terminal; `subscribe()` para consumir o stream tipado.
pub struct HookListener {
    addr: SocketAddr,
    state: Arc<ListenerState>,
}

impl HookListener {
    /// Sobe o listener em `127.0.0.1:0` (porta efêmera — leia em [`Self::local_addr`]).
    /// Loopback é FIXO no código (inv#2): não há configuração que o exponha.
    ///
    /// # Errors
    /// Propaga erro de bind do SO.
    pub async fn bind() -> std::io::Result<Self> {
        let (tx, _) = broadcast::channel(1024);
        let state = Arc::new(ListenerState {
            tokens: Mutex::new(HashMap::new()),
            tx,
        });
        let app = Router::new()
            .route("/hook/:token/:kind", post(handle_hook))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "hook listener encerrou");
            }
        });
        Ok(Self { addr, state })
    }

    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Registra um nó no spawn e devolve o TOKEN opaco que o settings dele usará na
    /// URL — a ÚNICA fonte de atribuição (doutrina no topo do módulo).
    #[must_use]
    pub fn register_node(&self, node_id: &str) -> String {
        let token = uuid::Uuid::now_v7().simple().to_string();
        if let Ok(mut t) = self.state.tokens.lock() {
            t.insert(token.clone(), node_id.to_string());
        }
        token
    }

    /// Stream tipado dos eventos normalizados (emissão desacoplada — o consumidor
    /// decide o que projetar; nenhum `DomainEvent` nasce aqui).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<HookEvent> {
        self.state.tx.subscribe()
    }
}

async fn handle_hook(
    State(state): State<Arc<ListenerState>>,
    AxPath((token, kind)): AxPath<(String, String)>,
    body: String,
) -> StatusCode {
    // Atribuição EXCLUSIVA pelo token de spawn; desconhecido = 404 sem evento (nada de
    // telemetria órfã/forjada entra no canal).
    let node_id = match state.tokens.lock() {
        Ok(t) => match t.get(&token) {
            Some(n) => n.clone(),
            None => return StatusCode::NOT_FOUND,
        },
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };
    // Kind fora dos 6 consumidos: aceito e IGNORADO (forward-compat com os 27 do
    // Claude — o hook do CLI nunca recebe erro por algo que NÓS não consumimos).
    let Some(kind) = HookKind::from_path(&kind) else {
        return StatusCode::NO_CONTENT;
    };
    // Payload ilegível não derruba o hook: normaliza com campos ausentes (telemetria
    // degradada é melhor que CLI travado — os hooks rodam no caminho do agente).
    let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let event = normalize_payload(&node_id, kind, &payload, now_ms());
    // Sem assinantes ainda = ok (o app pode subscrever depois do 1º spawn).
    let _ = state.tx.send(event);
    StatusCode::OK
}

/// Capability efetiva de hooks de um CLI (declarada no TOML — inv#3; ADR 0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HooksCapability {
    /// CLI emite hooks instaláveis (Claude Code 1ª classe via handler HTTP).
    HttpHooks,
    /// CLI sem hooks: a atividade vem APENAS do grid (heurística camada 4 do ADR 0008).
    GridOnly,
}

/// Pipeline por terminal: decide a capability pelo perfil e carrega o aviso-1× da
/// degradação (story: "pipeline reporta ausência 1x, nada quebra").
#[derive(Debug)]
pub struct HookPipeline {
    capability: HooksCapability,
    grid_only_reported: AtomicBool,
}

impl HookPipeline {
    #[must_use]
    pub fn for_profile(profile: &CliProfile) -> Self {
        let capability = if profile.capabilities.has("hooks") {
            HooksCapability::HttpHooks
        } else {
            HooksCapability::GridOnly
        };
        Self {
            capability,
            grid_only_reported: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn capability(&self) -> HooksCapability {
        self.capability
    }

    /// Em `GridOnly`, devolve o aviso legível EXATAMENTE uma vez (depois `None` —
    /// nunca spamma o log a cada tick). Em `HttpHooks`, sempre `None`.
    #[must_use]
    pub fn grid_only_notice(&self) -> Option<String> {
        if self.capability == HooksCapability::GridOnly
            && !self.grid_only_reported.swap(true, Ordering::Relaxed)
        {
            return Some(
                "este CLI não emite hooks (capabilities.hooks=false) — atividade via grid \
                 apenas; timeline de tools indisponível, nada mais muda"
                    .to_string(),
            );
        }
        None
    }
}

/// Projeção pai→filhos dos `SubagentStart`/`SubagentStop` (critério 4). Identidade dos
/// ids é TELEMETRIA (payload do CLI — forjável): útil para desenhar a árvore, proibida
/// para decisão (13.5 achado 8, "agent session smuggling").
#[derive(Debug, Default)]
pub struct SubagentTree {
    /// nó (atribuído por token) → subagentes VIVOS, em ordem de chegada.
    children: HashMap<String, Vec<String>>,
}

impl SubagentTree {
    /// Aplica um evento à árvore (ignora kinds não-subagent).
    pub fn apply(&mut self, event: &HookEvent) {
        let Some(sub) = &event.subagent_id else {
            return;
        };
        match event.kind {
            HookKind::SubagentStart => {
                let kids = self.children.entry(event.node_id.clone()).or_default();
                if !kids.contains(sub) {
                    kids.push(sub.clone());
                }
            }
            HookKind::SubagentStop => {
                if let Some(kids) = self.children.get_mut(&event.node_id) {
                    kids.retain(|k| k != sub);
                }
            }
            _ => {}
        }
    }

    /// Subagentes vivos do nó, em ordem de chegada.
    #[must_use]
    pub fn children_of(&self, node_id: &str) -> Vec<&str> {
        self.children
            .get(node_id)
            .map(|v| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }
}
