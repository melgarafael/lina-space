//! `lina-webhooks` — Engine de Webhooks do nó-Gatilho (story **W5-4**).
//!
//! O mínimo do **nó-Gatilho**: um servidor `axum` local que recebe um `POST` externo, valida
//! **HMAC** e publica o evento no **Workspace Bus** como mais um produtor — sem abrir nenhuma porta
//! do produto de 5 anos.
//!
//! ## Invariantes honrados (`CLAUDE.md`)
//! - **#2 Local-first.** O listener faz bind SOMENTE em `127.0.0.1` (o chamador cria o
//!   [`tokio::net::TcpListener`]; recomenda-se `127.0.0.1:0`). Nunca `0.0.0.0`.
//! - **#4 O event log é a fonte da verdade.** Cada disparo válido vira um
//!   [`DomainEvent::WebhookReceived`] no [`EventStore`] — o handler **AGUARDA o append ANTES do
//!   `202`** (durabilidade primeiro: se o append falha, respondemos `5xx`, nunca `202`; o append
//!   roda numa thread de bloqueio via `spawn_blocking` para não prender a worker async — MEDIA-2,
//!   F1-6-2). Só o efeito NÃO-durável (publicar no bus) roda fora do caminho da resposta. A config
//!   de cada hook é um [`DomainEvent::WebhookConfigured`] (event-sourcing) e, **no boot,
//!   [`WebhookEngine::replay_configured`] reconstrói as ligações varrendo o log** (F1-6-1 /
//!   MEDIA-5): restart não derruba hooks — a sequência de boot é `new` → `replay_configured` →
//!   `serve`.
//! - **Secret fora do disco.** O secret HMAC vive no Secret Vault (W0-7,
//!   [`SecretVault::ensure_webhook_secret`]) — nunca no log nem em claro.
//!
//! ## Defesa de taxa em TRÊS camadas (W5-4-MED-a + F1-6-4/MEDIA-1 — ver [`RateLimitConfig`])
//! Como o `hook_id` viaja na URL (capability, não segredo), um gate que gaste um recurso escasso
//! COMPARTILHADO antes do HMAC é manipulável por quem só conhece (ou chuta) rotas — e em
//! loopback/atrás de túnel todo request chega do MESMO IP, então teto "por IP" era na prática
//! global (starvation, MEDIA-1). Por isso:
//! 1. **Balde pré-auth POR CHAVE-DE-ROTA** ANTES de tudo — anti-starvation: lixo na rota X só
//!    esgota o balde de X (contabilidade idêntica p/ rota existente/inexistente; sem oráculo;
//!    memória O(cap)).
//! 2. **Teto GLOBAL generoso** em seguida — backstop de CPU contra flood de HMAC em massa
//!    distribuído por muitas rotas (nenhum campo do cliente decide identidade/orçamento; IP é
//!    só observabilidade).
//! 3. **Budget por hook** SÓ DEPOIS do HMAC passar — protege o budget do dono legítimo; corpo-lixo
//!    sem o secret toma `401` e nunca consome o budget do hook.
//!
//! ## Segurança (gate humano — campos controláveis pelo atacante)
//! O POST externo só controla: o `hook_id` da URL, o header `X-Signature` e o corpo. NENHUM
//! desses decide identidade ou destino: `from` (nó-Gatilho) e `target_ref` são config do
//! servidor (fixados em [`WebhookEngine::configure_hook`]), não do request. O `hook_id` é uma
//! **capability** base32 não-enumerável (160 bits); o HMAC prova posse do secret compartilhado;
//! a divergência → **401 sem publicar**. `verify_slice` compara em tempo constante (`subtle`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

use lina_core::{
    parse_target, A2aEnvelope, DomainEvent, EventStore, NodeId, Recipient, RolePolicy, Supervisor,
    TargetSpec,
};
use lina_secrets::{SecretStore, SecretVault};

type HmacSha256 = Hmac<Sha256>;

/// Escopo do hook no Secret Vault (a `key` é o `hook_id`).
const SECRET_SCOPE: &str = "webhook";

/// Erros do engine de webhooks. Sem `unwrap()`/`expect()` em produção: todo caminho falível vira
/// uma destas variantes (regra do `CLAUDE.md`).
#[derive(Debug, Error)]
pub enum WebhookError {
    /// Falha ao obter/gravar o secret HMAC no Secret Vault (W0-7).
    #[error("erro de secret do webhook: {0}")]
    Secret(String),
    /// Falha ao apendar o evento (config/recepção) no event store.
    #[error("erro do event store: {0}")]
    Store(String),
    /// Falha ao obter entropia do SO para o `hook_id`.
    #[error("falha ao gerar hook_id: {0}")]
    Random(String),
    /// Falha do `axum::serve` ao atender o listener.
    #[error("erro ao servir o listener axum: {0}")]
    Serve(String),
    /// Recusa de servir: o listener NÃO está em interface loopback (invariante #2 local-first).
    #[error("bind não-local recusado (local-first; use 127.0.0.1): {0}")]
    NotLocal(String),
}

/// Limite de janela fixa: no máximo `max_requests` requisições por `window`.
#[derive(Debug, Clone, Copy)]
pub struct WindowLimit {
    /// Máximo de requisições aceitas dentro de uma janela.
    pub max_requests: u32,
    /// Duração da janela.
    pub window: Duration,
}

/// Rate-limit em **TRÊS camadas** (defesa em profundidade — W5-4-MED-a + F1-6-4/MEDIA-1):
/// - **`pre_auth_per_route`** — balde pré-autenticação **POR CHAVE-DE-ROTA** (o `hook_id` cru da
///   URL, **exista ou não** — contabilidade idêntica nos dois casos, sem oráculo de existência).
///   É a camada **anti-starvation** do cenário túnel (F1-6-4): em loopback/atrás de túnel TODO
///   request chega do mesmo IP, então qualquer teto por-IP é na prática GLOBAL e corpo-lixo
///   barato derrubaria TODOS os hooks para 429 (MEDIA-1). Particionar o orçamento pela rota
///   pedida mata a starvation: lixo na rota X só esgota o balde de X. A chave é controlada pelo
///   cliente, mas isso é seguro POR CONSTRUÇÃO: escolher a chave só decide QUAL balde o próprio
///   chamador consome — nunca amplia orçamento nem toca o balde de outra rota.
/// - **`pre_auth_global`** — teto GLOBAL generoso, depois do balde de rota: o backstop de CPU
///   contra flood massivo de cálculo de HMAC/lookup (herda o papel que o antigo teto "por IP"
///   exercia de fato). Só conta requests que PASSARAM o balde de rota (os que vão de fato gastar
///   lookup+HMAC) — lixo já barrado na rota não consome o teto de ninguém.
/// - **`per_hook`** — budget POR hook, checado SÓ DEPOIS do HMAC passar. Protege o budget do
///   dono legítimo: corpo-lixo sem o secret toma `401` e **nunca** consome este budget
///   (W5-4-MED-a, contrato do `a993340`).
///
/// O peer IP segue extraído (observabilidade nos 429 globais) mas NÃO é mais chave de orçamento —
/// neste listener (loopback-only, invariante #2) ele nunca discriminou clientes.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Camada 1a (anti-starvation): balde pré-auth por chave-de-rota, ANTES do 404/HMAC.
    pub pre_auth_per_route: WindowLimit,
    /// Camada 1b (CPU backstop): teto GLOBAL generoso pré-HMAC, após o balde de rota.
    pub pre_auth_global: WindowLimit,
    /// Camada 2 (budget do dono): teto por hook, DEPOIS do HMAC.
    pub per_hook: WindowLimit,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            // ~4 req/s sustentados POR ROTA: folga ampla para webhook legítimo (≥2× o budget
            // pós-HMAC do dono) e teto barato para corpo-lixo numa rota (vazada ou chutada).
            pre_auth_per_route: WindowLimit {
                max_requests: 240,
                window: Duration::from_secs(60),
            },
            // Backstop de CPU: só um flood distribuído por MUITAS rotas distintas chega aqui —
            // e aí o que se protege é a máquina (HMAC em massa), não o orçamento de um hook.
            // RESIDUAL ASSUMIDO (red-team F1-6-4): quando ESTE teto dispara, TODOS os hooks
            // tomam 429 na janela — disponibilidade cede para proteger CPU. Volumetria que
            // atravessa o túnel se mitiga NA camada do túnel (rate-limit do provedor), não aqui.
            pre_auth_global: WindowLimit {
                max_requests: 2400,
                window: Duration::from_secs(60),
            },
            // Budget por hook do dono — preservado mesmo sob flood de corpo-lixo na mesma rota.
            per_hook: WindowLimit {
                max_requests: 120,
                window: Duration::from_secs(60),
            },
        }
    }
}

/// Cap do mapa de baldes por rota: memória O(cap) INDEPENDENTE do tráfego. Acima do cap (após
/// reciclar janelas expiradas), chave nova NÃO aloca balde — o request segue só sob o teto
/// global. Flood de chaves aleatórias não compra alocação ilimitada (e não toca balde alheio).
const MAX_ROUTE_BUCKETS: usize = 4096;

/// Chave de rota maior que isso não ganha balde próprio (hook_id legítimo tem 32 chars; URLs
/// gigantes não viram âncora de memória). Regra ESTRUTURAL por tamanho — não revela existência.
const MAX_ROUTE_KEY_LEN: usize = 64;

/// Throttle da varredura de expirados no mapa de baldes: a varredura O(cap) sob o lock roda no
/// máximo 1× por este intervalo — o custo POR REQUEST fica O(1) mesmo sob spray de chaves com o
/// mapa cheio (critério "custo de lookup constante" da F1-6-4). Entre varreduras, chave nova sem
/// vaga segue ADMITIDA sob o teto global (nunca negada por falta de balde).
const PURGE_MIN_INTERVAL: Duration = Duration::from_millis(500);

/// Baldes pré-auth por chave-de-rota + relógio da última varredura de expirados (throttle).
struct RouteBuckets {
    map: HashMap<String, RateWindow>,
    last_purge: Instant,
}

impl RouteBuckets {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            // Nasce "vencido" para a 1ª varredura não ser adiada à toa; `checked_sub` cobre
            // plataformas onde Instant não recua (aí só a 1ª janela de throttle é paga).
            last_purge: Instant::now()
                .checked_sub(PURGE_MIN_INTERVAL)
                .unwrap_or_else(Instant::now),
        }
    }
}

/// Config devolvida ao registrar um hook: a rota opaca (`hook_id`) e o `secret` HMAC que o
/// integrador externo usa para assinar o corpo. O `secret` NÃO é re-derivável da rota — guarde-o
/// no momento da configuração (ele também vive no Secret Vault).
#[derive(Debug, Clone)]
pub struct HookConfig {
    /// Identificador opaco base32 (160 bits) — compõe a rota `/hook/<hook_id>`.
    pub hook_id: String,
    /// Secret HMAC (hex, 256 bits) compartilhado com o integrador externo.
    pub secret: String,
}

/// Resumo do replay de boot ([`WebhookEngine::replay_configured`]): quantos hooks foram religados
/// e quantos foram pulados por secret ausente no vault (perda sinalizada, nunca silenciosa).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReplaySummary {
    /// Hooks religados (rota + `target_ref` reconstruídos do log; secret relido do vault).
    pub rebound: usize,
    /// Hooks NÃO religados porque o secret sumiu do vault (warn no log de tracing; o replay não
    /// recria secret em silêncio — isso mascararia a perda e quebraria o integrador externo).
    pub skipped_no_secret: usize,
    /// Hooks NÃO religados por ERRO do backend do vault (keyring indisponível etc.) — por hook,
    /// para um erro transitório num hook não derrubar o boot inteiro (warn no log de tracing).
    pub skipped_vault_error: usize,
}

/// Ligação interna de um hook: secret (cache em memória do que está no Vault), identidade de
/// origem (`from` = nó-Gatilho, alocado pelo Supervisor) e `target_ref` (endereço A2A do destino).
#[derive(Debug, Clone)]
struct HookBinding {
    secret: String,
    from: NodeId,
    target_ref: String,
}

/// Janela de rate-limit (janela fixa).
#[derive(Debug)]
struct RateWindow {
    start: Instant,
    count: u32,
}

/// Config de hook lida do log no replay de boot (projeção de um `WebhookConfigured`).
#[derive(Debug, Clone)]
struct ConfiguredHook {
    hook_id: String,
    target_ref: String,
}

/// Fronteira do log de eventos: o que o engine precisa de um log durável — apendar fatos
/// (caminho da resposta) e reler as configs no boot (replay F1-6-1). Abstrair o [`EventStore`]
/// concreto aqui mantém o append do caminho da resposta **bloqueante e falível** (o handler decide
/// `202` vs `5xx` pelo `Result`) e permite, nos testes, um log que falha ou demora de propósito
/// (prova observável dos caminhos `5xx`/worker-livre sem disco real). Produção usa [`EventStoreLog`].
trait DurableLog: Send + Sync {
    /// Apende um evento de domínio de forma durável e BLOQUEANTE. `Err` aborta o efeito (sem `202`).
    /// O chamador async deve envolver em `spawn_blocking` (MEDIA-2) — a função em si pode tocar disco.
    fn append(&self, event: &DomainEvent) -> Result<(), WebhookError>;

    /// Relê do log, em ordem de `seq`, as configs de hook (`WebhookConfigured`) para o replay de
    /// boot. Só leitura — replay nunca apenda.
    fn configured_hooks(&self) -> Result<Vec<ConfiguredHook>, WebhookError>;
}

/// Ligação de produção: o [`EventStore`] real (escritor único do log, serializado pelo `Mutex`).
struct EventStoreLog(Arc<Mutex<EventStore>>);

impl DurableLog for EventStoreLog {
    fn append(&self, event: &DomainEvent) -> Result<(), WebhookError> {
        lock(&self.0)
            .append(event)
            .map(|_seq| ())
            .map_err(|e| WebhookError::Store(e.to_string()))
    }

    fn configured_hooks(&self) -> Result<Vec<ConfiguredHook>, WebhookError> {
        let records = lock(&self.0)
            .events()
            .map_err(|e| WebhookError::Store(e.to_string()))?;
        let mut out = Vec::new();
        for r in records {
            if r.kind != "WebhookConfigured" {
                continue;
            }
            // Parse tolerante (degradação honesta): um payload de shape inesperado vira warn+skip,
            // nunca derruba o boot inteiro. NOTA DE COSTURA: o upcast tipado do core
            // (`DomainEvent::from_record`) é `pub(crate)` — quando o core expuser um helper
            // público de replay, este parse manual migra para lá (hoje `WebhookConfigured` é v1,
            // sem upcast definido, então os caminhos são equivalentes).
            let hook_id = r.payload.get("hook_id").and_then(|v| v.as_str());
            let target_ref = r.payload.get("target_ref").and_then(|v| v.as_str());
            match (hook_id, target_ref) {
                (Some(h), Some(t)) => out.push(ConfiguredHook {
                    hook_id: h.to_string(),
                    target_ref: t.to_string(),
                }),
                _ => tracing::warn!(
                    seq = r.seq,
                    "WebhookConfigured com payload inesperado; ignorado no replay"
                ),
            }
        }
        Ok(out)
    }
}

/// Estado compartilhado do servidor (clonado para dentro do router axum — tudo `Arc`).
#[derive(Clone)]
struct AppState {
    hooks: Arc<RwLock<HashMap<String, HookBinding>>>,
    store: Arc<dyn DurableLog>,
    supervisor: Arc<Supervisor>,
    /// Camada 1a (anti-starvation, F1-6-4): balde pré-auth por CHAVE-DE-ROTA (exista ou não),
    /// com cap de entradas ([`MAX_ROUTE_BUCKETS`]) e varredura com throttle — memória O(cap),
    /// custo por-request O(1), sem oráculo de existência.
    rate_route: Arc<Mutex<RouteBuckets>>,
    /// Camada 1b (CPU backstop): janela GLOBAL única pré-HMAC.
    rate_global: Arc<Mutex<RateWindow>>,
    /// Camada 2: budget por `hook_id` (protege o dono, depois do HMAC) — só hooks EXISTENTES
    /// chegam aqui (pós-HMAC), então o mapa é limitado pelo nº de hooks configurados.
    rate_hook: Arc<Mutex<HashMap<String, RateWindow>>>,
    rate_cfg: RateLimitConfig,
}

/// Engine de Webhooks: dono das ligações de hook + referências ao event store e ao supervisor.
pub struct WebhookEngine {
    state: AppState,
}

impl WebhookEngine {
    /// Novo engine com rate-limit padrão. `store` e `supervisor` são compartilhados (em produção,
    /// os mesmos do app — escritor único do log, serializado pelo `Mutex`).
    #[must_use]
    pub fn new(store: Arc<Mutex<EventStore>>, supervisor: Arc<Supervisor>) -> Self {
        Self::with_config(store, supervisor, RateLimitConfig::default())
    }

    /// Novo engine com rate-limit customizado (ver [`RateLimitConfig`] — duas camadas).
    #[must_use]
    pub fn with_config(
        store: Arc<Mutex<EventStore>>,
        supervisor: Arc<Supervisor>,
        rate_cfg: RateLimitConfig,
    ) -> Self {
        Self::with_log(Arc::new(EventStoreLog(store)), supervisor, rate_cfg)
    }

    /// Construtor interno sobre a fronteira [`DurableLog`] — caminho único de produção
    /// (`with_config`) e dos testes que injetam um log que falha/demora de propósito.
    fn with_log(
        store: Arc<dyn DurableLog>,
        supervisor: Arc<Supervisor>,
        rate_cfg: RateLimitConfig,
    ) -> Self {
        Self {
            state: AppState {
                hooks: Arc::new(RwLock::new(HashMap::new())),
                store,
                supervisor,
                rate_route: Arc::new(Mutex::new(RouteBuckets::new())),
                rate_global: Arc::new(Mutex::new(RateWindow {
                    start: Instant::now(),
                    count: 0,
                })),
                rate_hook: Arc::new(Mutex::new(HashMap::new())),
                rate_cfg,
            },
        }
    }

    /// **Replay de boot (F1-6-1 / MEDIA-5).** Reconstrói as ligações de hook varrendo os
    /// `WebhookConfigured` do event log (fonte da verdade, invariante #4) — chame **uma vez no
    /// boot, antes de [`WebhookEngine::serve`]**: após um restart os hooks voltam a responder
    /// `202` sem reconfigurar (antes deste replay, sumiam até alguém reconfigurar → `404`).
    ///
    /// Semântica:
    /// - **Fold last-wins por `hook_id`** em ordem de log: re-configurar um hook atualiza o
    ///   `target_ref`. (Ponto único preparado para um futuro evento de remoção — hoje o schema
    ///   não tem remoção de hook, então nada "renasce" porque nada morre.)
    /// - **O secret NUNCA vem do log** (lá não está — invariante "nada em claro no disco"): é
    ///   relido do Secret Vault por `hook_id`. Secret ausente no vault → o hook **NÃO é religado**
    ///   (sem ele não há verificação HMAC possível) e **NÃO criamos secret novo em silêncio**
    ///   (mascararia a perda e quebraria a assinatura do integrador) — vira `warn` + contagem em
    ///   [`ReplaySummary::skipped_no_secret`].
    /// - Replay é só-leitura: não apenda evento algum.
    ///
    /// `from` é o nó-Gatilho DESTE boot (o `NodeId` muda a cada processo — quem chama registra o
    /// nó no Supervisor e o passa aqui, como em [`WebhookEngine::configure_hook`]).
    ///
    /// **Contrato de erro:** `Err` SÓ vem da leitura do log (`configured_hooks`) — e nesse caso
    /// nada foi religado (sem estado parcial). Problema de vault é POR HOOK (mesma filosofia
    /// skip-e-conta do secret ausente): warn + contagem em
    /// [`ReplaySummary::skipped_vault_error`] — um keyring transitoriamente indisponível num
    /// hook não derruba o boot inteiro; o chamador decide pelo summary.
    pub fn replay_configured<S: SecretStore>(
        &self,
        vault: &SecretVault<S>,
        from: NodeId,
    ) -> Result<ReplaySummary, WebhookError> {
        let configured = self.state.store.configured_hooks()?;

        // Fold last-wins por hook_id (ordem de seq do log).
        let mut latest: HashMap<String, String> = HashMap::new();
        for c in configured {
            latest.insert(c.hook_id, c.target_ref);
        }

        // Resolve os secrets ANTES de tocar o lock de hooks: o vault de produção é keyring
        // (IPC com o SO, potencialmente lento) — nunca segurar write-lock através de IPC.
        let mut summary = ReplaySummary::default();
        let mut rebound = Vec::new();
        for (hook_id, target_ref) in latest {
            match vault.get(SECRET_SCOPE, &hook_id) {
                Ok(Some(secret)) => rebound.push((
                    hook_id,
                    HookBinding {
                        secret,
                        from,
                        target_ref,
                    },
                )),
                Ok(None) => {
                    tracing::warn!(
                        hook_id = %hook_id,
                        "replay: secret ausente no vault; hook NÃO religado (não recriamos secret em silêncio)"
                    );
                    summary.skipped_no_secret += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        hook_id = %hook_id,
                        error = %e,
                        "replay: erro do vault; hook NÃO religado (perda sinalizada, não silenciosa)"
                    );
                    summary.skipped_vault_error += 1;
                }
            }
        }

        let mut hooks = wlock(&self.state.hooks);
        for (hook_id, binding) in rebound {
            hooks.insert(hook_id, binding);
            summary.rebound += 1;
        }
        Ok(summary)
    }

    /// Configura um hook: gera um `hook_id` base32 não-enumerável, garante o secret HMAC no Secret
    /// Vault, registra a config no log (`WebhookConfigured` — event-sourcing, SEM o segredo) e
    /// instala a ligação em memória. `from` é o nó-Gatilho (alocado pelo Supervisor); `target_ref`
    /// é o endereço A2A do destino (nome de nó, `role:…` ou `*`). Devolve a [`HookConfig`].
    pub fn configure_hook<S: SecretStore>(
        &self,
        vault: &SecretVault<S>,
        from: NodeId,
        target_ref: &str,
    ) -> Result<HookConfig, WebhookError> {
        let hook_id = gen_hook_id()?;
        let secret = vault
            .ensure_webhook_secret(SECRET_SCOPE, &hook_id)
            .map_err(|e| WebhookError::Secret(e.to_string()))?;

        // Event-sourcing: a config (rota + alvo) é um fato do log. O secret NÃO entra aqui (#2).
        self.state.store.append(&DomainEvent::WebhookConfigured {
            hook_id: hook_id.clone(),
            target_ref: target_ref.to_string(),
        })?;

        wlock(&self.state.hooks).insert(
            hook_id.clone(),
            HookBinding {
                secret: secret.clone(),
                from,
                target_ref: target_ref.to_string(),
            },
        );

        Ok(HookConfig { hook_id, secret })
    }

    /// Router axum do engine (rota `POST /hook/:hook_id` + teto de corpo defensivo de 64 KiB).
    ///
    /// **Sirva via [`WebhookEngine::serve`]** (ou `into_make_service_with_connect_info::<SocketAddr>()`):
    /// o handler extrai [`ConnectInfo`] do peer para OBSERVABILIDADE dos 429 globais (desde
    /// F1-6-4 o IP não é mais chave de orçamento — em loopback ele nunca discriminou clientes).
    /// Sem o connect-info o extractor falha — por isso o `serve` é o caminho suportado.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/hook/:hook_id", post(handle_hook))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(self.state.clone())
    }

    /// Atende o `listener` (criado pelo chamador). Consome o engine. **Enforce do invariante #2**
    /// (local-first): recusa servir se o listener não estiver numa interface loopback — a lib não
    /// confia cegamente no chamador para nunca expor `0.0.0.0`. Injeta o [`ConnectInfo`] do peer
    /// (IP de origem) para observabilidade do rate-limit (não é chave de orçamento — F1-6-4).
    pub async fn serve(self, listener: tokio::net::TcpListener) -> Result<(), WebhookError> {
        if let Ok(addr) = listener.local_addr() {
            ensure_local(addr)?;
        }
        let app = self.router();
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(|e| WebhookError::Serve(e.to_string()))
    }
}

/// Guard do invariante #2: só interface loopback é aceita (nunca `0.0.0.0`/IP de rede).
fn ensure_local(addr: std::net::SocketAddr) -> Result<(), WebhookError> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(WebhookError::NotLocal(addr.to_string()))
    }
}

/// Handler do webhook (`POST /hook/:hook_id`). Ordem das checagens é deliberada (W5-4-MED-a +
/// F1-6-4/MEDIA-1 — três camadas de rate-limit; ver [`RateLimitConfig`]):
/// 1. `429` **balde pré-auth por CHAVE-DE-ROTA** (camada 1a) — ANTES de tudo; anti-starvation:
///    lixo na rota X só esgota o balde de X; contabilidade idêntica p/ rota existente/inexistente
///    (sem oráculo); depois `429` **teto GLOBAL generoso** (camada 1b) — backstop de CPU contra
///    flood de HMAC em massa (nenhum campo do cliente decide identidade/orçamento).
/// 2. `404` hook desconhecido — rota opaca (capability base32 não-enumerável).
/// 3. `401` HMAC ausente/inválido — prova posse do secret; sem ela, NÃO se publica nada.
/// 4. `429` **budget por hook** (camada 2) — SÓ APÓS o HMAC; corpo-lixo sem o secret (que toma 401)
///    nunca consome o budget do dono, então o vazamento do `hook_id` na URL não vira DoS de budget.
/// 5. **Append AGUARDADO** do `WebhookReceived` (W5-4-MED-b — durabilidade primeiro, invariante
///    #4; o disco roda em `spawn_blocking`, MEDIA-2): se falha → `5xx` sem publicar; só com o
///    fato durável é que respondemos `202` e disparamos o efeito NÃO-durável (publicar no bus)
///    fora do caminho da resposta. Append + disparo rodam numa task PRÓPRIA que sobrevive à
///    desconexão do cliente — sem janela "durável-mas-não-publicado" por drop do handler.
async fn handle_hook(
    State(state): State<AppState>,
    Path(hook_id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // 1a. Camada anti-starvation (F1-6-4/MEDIA-1): balde pré-auth POR CHAVE-DE-ROTA, ANTES de
    //     qualquer trabalho caro (lookup, cópia do binding, HMAC). Contabilidade IDÊNTICA para
    //     rota existente/inexistente (sem oráculo); custo O(1); lixo na rota X só esgota o balde
    //     de X — nunca o de um hook legítimo concorrente.
    if !state.allow_route_budget(&hook_id) {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    // 1b. Backstop GLOBAL de CPU: só conta o que PASSOU o balde de rota (o que de fato vai gastar
    //     lookup+HMAC). Estourar aqui exige flood distribuído por muitas rotas — protege a
    //     máquina, e o peer fica no log (observabilidade; IP NÃO é chave de orçamento).
    if !state.allow_global_budget() {
        tracing::warn!(peer = %peer, "teto global pré-auth estourado; 429 (CPU backstop)");
        return StatusCode::TOO_MANY_REQUESTS;
    }

    // 2. Hook conhecido? 404 opaco — a rota é uma capability base32 não-enumerável.
    let binding = match rlock(&state.hooks).get(&hook_id) {
        Some(b) => b.clone(),
        None => return StatusCode::NOT_FOUND,
    };

    // 3. HMAC OBRIGATÓRIO. Ausência OU divergência → 401 SEM publicar nada (gate de publicação).
    let Some(sig) = headers.get("x-signature").and_then(|v| v.to_str().ok()) else {
        return StatusCode::UNAUTHORIZED;
    };
    if !verify_hex(&binding.secret, &body, sig) {
        return StatusCode::UNAUTHORIZED;
    }

    // 4. Camada 2 (budget): SÓ APÓS o HMAC provar posse do secret. Assim um flood de corpo-lixo sem
    //    o secret (barrado no passo 3) NUNCA consome o budget do hook do dono legítimo.
    if !state.allow_hook_budget(&hook_id) {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    // 5. DURABILIDADE PRIMEIRO (#4): o handler AGUARDA o append do FATO ANTES de responder. O
    //    append toca disco (bloqueante), então roda em `spawn_blocking` — thread de bloqueio do
    //    Tokio — para NÃO prender a worker async sob disco lento/cheio/NFS (MEDIA-2, F1-6-2). A
    //    SEMÂNTICA não muda, só a thread: 202 SÓ após o append durável; falha (ou abort da tarefa
    //    de append) → 5xx (NUNCA 202) e nada é publicado — sem perda silenciosa.
    //
    //    O DESFECHO (append + disparo do efeito) roda numa task PRÓPRIA que o handler aguarda:
    //    `.await` é ponto de cancelamento — se o cliente desconectar durante o append (provável
    //    exatamente sob disco lento: timeout do integrador), o future do handler é dropado, mas a
    //    task sobrevive e completa append E publicação. Sem ela, o estado "durável-mas-não-
    //    publicado" seria alcançável por mera desconexão, não só por crash.
    let ts = now_millis();
    let ev = DomainEvent::WebhookReceived {
        hook_id: hook_id.clone(),
        ts,
        target_ref: binding.target_ref.clone(),
    };
    let hook_for_log = hook_id.clone();
    let outcome = tokio::spawn(async move {
        let store = Arc::clone(&state.store);
        let appended = match tokio::task::spawn_blocking(move || store.append(&ev)).await {
            Ok(res) => res,
            // Tarefa de append abortada (panic no pool de bloqueio): trate como falha de
            // durabilidade — o fato NÃO está garantido no log, então nunca 202.
            Err(join_err) => Err(WebhookError::Store(format!(
                "tarefa de append abortada: {join_err}"
            ))),
        };
        match appended {
            Ok(()) => {
                // 6. Só o efeito NÃO-durável (publicar no bus) corre fora do caminho da resposta —
                //    a durabilidade já foi garantida no passo 5: o FATO está no log. Se este efeito
                //    se perder num crash, o log durável é a PROVA do disparo e permite
                //    reconstrução; a re-entrega automática (retries/dead-letter) é Fase 2 (W5-4) —
                //    hoje ninguém re-publica sozinho.
                tokio::spawn(async move {
                    state.publish(&binding);
                });
                Ok(())
            }
            Err(e) => {
                // Log AQUI (na task), não no handler: se o cliente já desconectou, o handler
                // morreu — a falha de durabilidade precisa de rastro mesmo sem ninguém ouvindo.
                tracing::error!(hook_id = %hook_for_log, error = %e, "append de WebhookReceived falhou; respondendo 5xx, nada publicado");
                Err(e)
            }
        }
    });
    match outcome.await {
        Ok(Ok(())) => StatusCode::ACCEPTED,
        Ok(Err(_)) => StatusCode::INTERNAL_SERVER_ERROR,
        Err(join_err) => {
            tracing::error!(hook_id = %hook_id, error = %join_err, "tarefa de desfecho do webhook abortada; respondendo 5xx");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

impl AppState {
    /// Efeito NÃO-durável: publica a mensagem do nó-Gatilho ao alvo no Workspace Bus. `from`/alvo
    /// vêm da config (nunca do request) — sem impersonação via campo controlável.
    fn publish(&self, binding: &HookBinding) {
        match resolve_recipient(&self.supervisor, &binding.target_ref) {
            Some(to) => {
                let env = A2aEnvelope::new(binding.from, to, Some("webhook".to_string()));
                if self
                    .supervisor
                    .route(&env, RolePolicy::FirstIdle)
                    .is_empty()
                {
                    tracing::warn!(target = %binding.target_ref, "webhook registrado; alvo sem nó vivo p/ entrega");
                }
            }
            None => {
                tracing::warn!(target = %binding.target_ref, "webhook registrado; target_ref não resolveu");
            }
        }
    }

    /// Camada 1a (anti-starvation): balde pré-auth por CHAVE-DE-ROTA. `true` se cabe na janela
    /// — ou se a chave não ganhou balde (cap/tamanho), caso em que só o teto global freia.
    fn allow_route_budget(&self, route_key: &str) -> bool {
        allow_route_window(
            &self.rate_route,
            route_key,
            self.rate_cfg.pre_auth_per_route,
        )
    }

    /// Camada 1b (CPU backstop): janela GLOBAL única pré-HMAC. `true` se cabe na janela.
    fn allow_global_budget(&self) -> bool {
        let mut w = lock(&self.rate_global);
        tick_window(&mut w, Instant::now(), self.rate_cfg.pre_auth_global)
    }

    /// Camada 2 (budget): teto por hook. `true` se cabe na janela.
    fn allow_hook_budget(&self, hook_id: &str) -> bool {
        allow_window(&self.rate_hook, hook_id.to_string(), self.rate_cfg.per_hook)
    }
}

/// Avança uma janela fixa: reinicia se expirou, conta a requisição, diz se coube no limite.
fn tick_window(w: &mut RateWindow, now: Instant, limit: WindowLimit) -> bool {
    if now.duration_since(w.start) > limit.window {
        w.start = now;
        w.count = 0;
    }
    w.count += 1;
    w.count <= limit.max_requests
}

/// Rate-limit de janela fixa sobre um mapa `chave → janela`. `true` se a requisição cabe na
/// janela corrente. Usado pela camada pós-HMAC (`rate_hook`), cujo domínio de chaves é limitado
/// pelos hooks CONFIGURADOS (só rota existente + HMAC válido chega lá) — sem cap necessário.
fn allow_window<K: Eq + std::hash::Hash>(
    map: &Mutex<HashMap<K, RateWindow>>,
    key: K,
    limit: WindowLimit,
) -> bool {
    let now = Instant::now();
    let mut m = lock(map);
    let w = m.entry(key).or_insert(RateWindow {
        start: now,
        count: 0,
    });
    tick_window(w, now, limit)
}

/// Balde pré-auth por chave-de-rota (F1-6-4) com memória LIMITADA — as garantias, em ordem:
/// 1. **Sem oráculo:** chave existente e inexistente percorrem o MESMO caminho (lookup O(1) no
///    mapa + mesma janela/limite) — o orçamento de lixo não revela se o hook existe.
/// 2. **Memória O(cap) e custo por-request O(1):** chave nova só aloca balde se houver vaga
///    ([`MAX_ROUTE_BUCKETS`]) e se a chave for plausível ([`MAX_ROUTE_KEY_LEN`]); a varredura de
///    expirados que abre vaga é O(cap), mas roda com THROTTLE ([`PURGE_MIN_INTERVAL`]) — spray
///    de chaves com o mapa cheio não compra varredura por request. Sem vaga → `true` SEM alocar:
///    o request segue apenas sob o teto global (backstop) — flood de chaves aleatórias não
///    compra memória nem toca balde alheio.
/// 3. **Partição por construção:** a chave é escolhida pelo cliente, mas só decide QUAL balde o
///    próprio chamador consome — nunca amplia orçamento nem drena o de outra rota.
fn allow_route_window(buckets: &Mutex<RouteBuckets>, route_key: &str, limit: WindowLimit) -> bool {
    let now = Instant::now();
    let mut b = lock(buckets);
    if let Some(w) = b.map.get_mut(route_key) {
        return tick_window(w, now, limit);
    }
    if route_key.len() > MAX_ROUTE_KEY_LEN {
        return true; // chave implausível não vira âncora de memória; teto global cobre
    }
    if b.map.len() >= MAX_ROUTE_BUCKETS {
        if now.duration_since(b.last_purge) < PURGE_MIN_INTERVAL {
            return true; // sem vaga e sem varredura AGORA (throttle): O(1); teto global cobre
        }
        b.last_purge = now;
        b.map
            .retain(|_, w| now.duration_since(w.start) <= limit.window);
        if b.map.len() >= MAX_ROUTE_BUCKETS {
            return true; // mapa cheio de janelas VIVAS: sem balde novo; teto global cobre
        }
    }
    let mut w = RateWindow {
        start: now,
        count: 0,
    };
    let allowed = tick_window(&mut w, now, limit);
    b.map.insert(route_key.to_string(), w);
    allowed
}

/// Resolve o `target_ref` (endereço A2A) em um [`Recipient`], reusando o parser canônico do core
/// (`*` → broadcast, `role:…` → papel late-bound, senão nome de nó vivo).
fn resolve_recipient(sup: &Supervisor, target_ref: &str) -> Option<Recipient> {
    match parse_target(target_ref) {
        TargetSpec::Broadcast => Some(Recipient::Broadcast),
        TargetSpec::Role(r) => Some(Recipient::Role(r)),
        TargetSpec::Name(n) => sup.node_by_name(&n).map(Recipient::Node),
    }
}

// ───────────────────────────── helpers ─────────────────────────────

/// Assinatura canônica que o integrador externo computa sobre o corpo: HMAC-SHA256(secret, body)
/// em hex. É o `X-Signature` esperado pelo engine. (Exposta para clientes e testes.)
#[must_use]
pub fn sign_hex(secret: &str, body: &[u8]) -> String {
    match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mut mac) => {
            mac.update(body);
            to_hex(&mac.finalize().into_bytes())
        }
        // HMAC aceita chave de qualquer tamanho → este braço é inalcançável; devolver vazio (em vez
        // de panic, regra do CLAUDE.md) nunca casa uma assinatura válida.
        Err(_) => String::new(),
    }
}

/// Verifica o `X-Signature` (hex) contra HMAC-SHA256(secret, body). `verify_slice` compara em
/// **tempo constante** (`subtle`) — sem oráculo de timing. Header malformado/hex inválido → `false`.
fn verify_hex(secret: &str, body: &[u8], sig_hex: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let Some(sig) = from_hex(sig_hex) else {
        return false;
    };
    mac.verify_slice(&sig).is_ok()
}

/// Decodifica uma string hex em bytes; `None` se o comprimento for ímpar ou houver dígito inválido.
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Gera um `hook_id` opaco: 20 bytes (160 bits) de entropia do SO em base32 (RFC4648, 32 chars).
fn gen_hook_id() -> Result<String, WebhookError> {
    let mut bytes = [0u8; 20];
    getrandom::getrandom(&mut bytes).map_err(|e| WebhookError::Random(e.to_string()))?;
    Ok(base32_encode(&bytes))
}

/// Base32 RFC4648 (alfabeto maiúsculo, sem padding). Mantém o acumulador podado a cada byte para
/// nunca estourar o inteiro.
fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for &b in data {
        buffer = (buffer << 8) | u64::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
        buffer &= (1u64 << bits) - 1;
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Recupera o guard de um `Mutex` mesmo se envenenado (sem propagar panic — `CLAUDE.md`).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn rlock<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|p| p.into_inner())
}

fn wlock<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const B32_ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    #[test]
    fn hook_id_e_opaco_estavel_no_formato_e_unico() {
        let a = gen_hook_id().expect("gera a");
        let b = gen_hook_id().expect("gera b");
        assert_eq!(a.len(), 32, "20 bytes (160 bits) em base32 = 32 chars");
        assert!(
            a.chars().all(|c| B32_ALPHABET.contains(c)),
            "deve usar o alfabeto base32 RFC4648 (não-enumerável, sem padding)"
        );
        assert_ne!(a, b, "dois hook_id não podem colidir (entropia do SO)");
    }

    #[test]
    fn hmac_roundtrip_verifica_a_propria_assinatura() {
        let secret = "deadbeefcafe0123";
        let body = br#"{"event":"order.created","id":42}"#;
        let sig = sign_hex(secret, body);
        assert!(
            verify_hex(secret, body, &sig),
            "a assinatura computada pelo cliente deve verificar"
        );
    }

    #[test]
    fn hmac_rejeita_corpo_adulterado() {
        let secret = "s3cr3t-de-hook";
        let sig = sign_hex(secret, b"corpo-original");
        assert!(
            !verify_hex(secret, b"corpo-ADULTERADO", &sig),
            "corpo trocado (mesmo tamanho ou não) não pode verificar"
        );
    }

    #[test]
    fn hmac_rejeita_secret_errado() {
        let sig = sign_hex("secret-A", b"payload");
        assert!(
            !verify_hex("secret-B", b"payload", &sig),
            "quem não tem o secret não forja assinatura"
        );
    }

    #[test]
    fn hmac_rejeita_assinatura_malformada() {
        let (secret, body) = ("k", b"p" as &[u8]);
        assert!(!verify_hex(secret, body, "zz"), "hex inválido → false");
        assert!(
            !verify_hex(secret, body, "abc"),
            "comprimento ímpar → false"
        );
        assert!(
            !verify_hex(secret, body, ""),
            "vazio → false (tag de 32 bytes não casa)"
        );
    }

    #[test]
    fn from_hex_casos_de_borda() {
        assert_eq!(from_hex("00ff10"), Some(vec![0x00, 0xff, 0x10]));
        assert_eq!(from_hex(""), Some(vec![]));
        assert_eq!(from_hex("0"), None, "comprimento ímpar");
        assert_eq!(from_hex("zz"), None, "dígito não-hex");
    }

    #[test]
    fn resolve_recipient_mapeia_broadcast_role_e_nome() {
        let sup = Supervisor::new();
        let dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));

        assert!(matches!(
            resolve_recipient(&sup, "*"),
            Some(Recipient::Broadcast)
        ));
        assert!(matches!(
            resolve_recipient(&sup, "role:dev"),
            Some(Recipient::Role(r)) if r == "dev"
        ));
        assert!(matches!(
            resolve_recipient(&sup, "@Dev"),
            Some(Recipient::Node(id)) if id == dev
        ));
        assert!(
            resolve_recipient(&sup, "@Fantasma").is_none(),
            "nome sem nó vivo não resolve"
        );
    }

    #[test]
    fn ensure_local_so_aceita_loopback() {
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
        assert!(ensure_local(SocketAddr::from((Ipv4Addr::LOCALHOST, 8080))).is_ok());
        assert!(ensure_local(SocketAddr::from((Ipv6Addr::LOCALHOST, 8080))).is_ok());
        assert!(
            matches!(
                ensure_local(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080))),
                Err(WebhookError::NotLocal(_))
            ),
            "0.0.0.0 deve ser recusado (invariante #2 local-first)"
        );
        assert!(
            matches!(
                ensure_local(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 5), 80))),
                Err(WebhookError::NotLocal(_))
            ),
            "IP de rede deve ser recusado"
        );
    }

    /// W5-4-MED-b (caminho de falha): se o append durável do `WebhookReceived` FALHA, o handler
    /// responde `5xx` (NUNCA `202`) e NADA é publicado no bus — durabilidade vem antes da resposta
    /// (invariante #4). Injeta um [`DurableLog`] que sempre falha (determinístico, sem socket).
    #[tokio::test]
    async fn append_falho_responde_5xx_e_nao_publica() {
        use std::net::{Ipv4Addr, SocketAddr};

        /// Log durável que sempre falha — simula disco cheio/erro de IO no append.
        struct FailingLog;
        impl DurableLog for FailingLog {
            fn append(&self, _event: &DomainEvent) -> Result<(), WebhookError> {
                Err(WebhookError::Store("falha simulada de IO no append".into()))
            }
            fn configured_hooks(&self) -> Result<Vec<ConfiguredHook>, WebhookError> {
                Ok(Vec::new())
            }
        }

        let sup = Arc::new(Supervisor::new());
        let from = sup.register(
            "@Trigger",
            Some("trigger".into()),
            Box::new(std::io::sink()),
        );
        let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
        let mut rx = sup.subscribe();

        // Hook registrado em memória com um secret conhecido (o request abaixo passa o HMAC → chega
        // ao append, que então falha).
        let secret = "secret-de-teste";
        let hook_id = "TESTHOOKID".to_string();
        let mut hooks = HashMap::new();
        hooks.insert(
            hook_id.clone(),
            HookBinding {
                secret: secret.to_string(),
                from,
                target_ref: "@Dev".to_string(),
            },
        );

        let state = AppState {
            hooks: Arc::new(RwLock::new(hooks)),
            store: Arc::new(FailingLog),
            supervisor: sup.clone(),
            rate_route: Arc::new(Mutex::new(RouteBuckets::new())),
            rate_global: Arc::new(Mutex::new(RateWindow {
                start: Instant::now(),
                count: 0,
            })),
            rate_hook: Arc::new(Mutex::new(HashMap::new())),
            rate_cfg: RateLimitConfig::default(),
        };

        let body = Bytes::from_static(br#"{"order":42}"#);
        let sig = sign_hex(secret, &body);
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().expect("sig vira HeaderValue"));

        let code = handle_hook(
            State(state),
            Path(hook_id),
            ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 40000))),
            headers,
            body,
        )
        .await;

        assert_eq!(
            code,
            StatusCode::INTERNAL_SERVER_ERROR,
            "append falho → 5xx (NUNCA 202): o fato não ficou durável, então não prometemos recepção"
        );

        // O efeito no bus só pode ocorrer APÓS o append durável — que falhou. Nada pode ter saído.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            rx.try_recv().is_err(),
            "append falho não pode publicar mensagem no bus (sem 202, sem efeito)"
        );
    }

    /// F1-6-2 / MEDIA-2: append LENTO (2s, bloqueante — disco lento/cheio/NFS simulado) com **UMA
    /// única worker thread** no runtime:
    /// (a) um request a OUTRO hook responde normal DURANTE a espera — a worker não fica presa
    ///     (é o ponto do MEDIA: sem `spawn_blocking`, o `thread::sleep` do append congelaria a
    ///     única worker e TUDO — inclusive este cliente — só andaria após os 2s);
    /// (b) o 202 do hook lento SÓ chega APÓS o append concluir (a semântica síncrona-pré-202 de
    ///     `a993340` é preservada — só a thread muda).
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn append_lento_nao_prende_a_worker_e_o_202_so_chega_apos_o_append() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const SLOW_HOOK: &str = "SLOWHOOKAAAAAAAAAAAAAAAAAAAAAAAA";
        const FAST_HOOK: &str = "FASTHOOKAAAAAAAAAAAAAAAAAAAAAAAA";
        const APPEND_DELAY: Duration = Duration::from_secs(2);

        /// Log durável cujo append do hook lento BLOQUEIA a thread por `APPEND_DELAY` (como um
        /// disco real travado faria) e registra cada append concluído.
        struct SlowLog {
            entered_slow: Arc<AtomicBool>,
            appended: Arc<Mutex<Vec<String>>>,
        }
        impl DurableLog for SlowLog {
            fn append(&self, event: &DomainEvent) -> Result<(), WebhookError> {
                if let DomainEvent::WebhookReceived { hook_id, .. } = event {
                    if hook_id == SLOW_HOOK {
                        self.entered_slow.store(true, Ordering::SeqCst);
                        std::thread::sleep(APPEND_DELAY);
                    }
                    lock(&self.appended).push(hook_id.clone());
                }
                Ok(())
            }
            fn configured_hooks(&self) -> Result<Vec<ConfiguredHook>, WebhookError> {
                Ok(Vec::new())
            }
        }

        /// POST HTTP/1.1 cru (como o `curl` do critério). Devolve o status code.
        async fn post(addr: SocketAddr, hook_id: &str, sig: &str, body: &[u8]) -> u16 {
            let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
            let req = format!(
                "POST /hook/{hook_id} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nX-Signature: {sig}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(req.as_bytes()).await.expect("head");
            stream.write_all(body).await.expect("body");
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.expect("resposta");
            String::from_utf8_lossy(&buf)
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|c| c.parse().ok())
                .expect("status line")
        }

        let entered_slow = Arc::new(AtomicBool::new(false));
        let appended = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(SlowLog {
            entered_slow: entered_slow.clone(),
            appended: appended.clone(),
        });

        let sup = Arc::new(Supervisor::new());
        let from = sup.register(
            "@Trigger",
            Some("trigger".into()),
            Box::new(std::io::sink()),
        );
        let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));

        let engine = WebhookEngine::with_log(log, sup, RateLimitConfig::default());
        for (id, secret) in [(SLOW_HOOK, "secret-slow"), (FAST_HOOK, "secret-fast")] {
            wlock(&engine.state.hooks).insert(
                id.to_string(),
                HookBinding {
                    secret: secret.to_string(),
                    from,
                    target_ref: "@Dev".to_string(),
                },
            );
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(engine.serve(listener));

        let body: &[u8] = br#"{"order":42}"#;
        let sig_slow = sign_hex("secret-slow", body);
        let sig_fast = sign_hex("secret-fast", body);

        let t0 = Instant::now();
        let slow_task = tokio::spawn(async move { post(addr, SLOW_HOOK, &sig_slow, body).await });

        // Espera o request lento ENTRAR no append (aí a thread de bloqueio já está dormindo 2s).
        // Sem o `spawn_blocking`, este próprio loop congelaria junto com a worker — e o assert de
        // latência abaixo falharia (tudo só andaria após os 2s).
        for _ in 0..300 {
            if entered_slow.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            entered_slow.load(Ordering::SeqCst),
            "o request lento deve ter entrado no append antes do request rápido (concorrência real)"
        );

        // (a) Worker LIVRE durante o append lento: outro hook responde normal, bem antes dos 2s.
        // Margem deliberadamente folgada p/ CI carregada (caminho-feliz ≈ dezenas de ms; caminho
        // QUEBRADO ≥ 2s — a folga de 500ms ainda discrimina com clareza).
        let fast_status = post(addr, FAST_HOOK, &sig_fast, body).await;
        let fast_done = t0.elapsed();
        assert_eq!(
            fast_status, 202,
            "hook rápido deve responder 202 normalmente"
        );
        assert!(
            fast_done < Duration::from_millis(1500),
            "request a OUTRO hook deve responder DURANTE a espera do append lento (worker não \
             presa — MEDIA-2); levou {fast_done:?}"
        );
        assert!(
            !slow_task.is_finished(),
            "o 202 do hook lento NÃO pode chegar antes do append terminar (ordem preservada)"
        );

        // (b) Semântica síncrona-pré-202 preservada: o 202 lento SÓ chega após o append (≥2s) e,
        // no instante da resposta, o evento JÁ está no log.
        let slow_status = slow_task.await.expect("join do request lento");
        let slow_done = t0.elapsed();
        assert_eq!(slow_status, 202, "hook lento responde 202 após o append");
        assert!(
            slow_done >= APPEND_DELAY,
            "o handler deve AGUARDAR o append (202 só após durável); levou {slow_done:?}"
        );
        assert!(
            lock(&appended).iter().any(|h| h == SLOW_HOOK),
            "no instante do 202 o WebhookReceived do hook lento JÁ está no log"
        );
    }

    /// F1-6-2 (janela de cancelamento): se o INTEGRADOR desconecta durante o append lento
    /// (timeout de cliente — provável exatamente sob disco lento), o desfecho NÃO se perde: o
    /// append completa E a publicação chega ao bus mesmo sem ninguém esperando a resposta.
    /// (`.await` é ponto de cancelamento: sem a task própria de desfecho, o drop do future do
    /// handler na desconexão engoliria o `tokio::spawn(publish)` em silêncio — estado
    /// "durável-mas-não-publicado" por mera desconexão.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn desconexao_do_cliente_durante_append_nao_perde_a_publicacao() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::io::AsyncWriteExt;

        const HOOK: &str = "DROPHOOKAAAAAAAAAAAAAAAAAAAAAAAA";
        const APPEND_DELAY: Duration = Duration::from_secs(1);

        struct SlowLog {
            entered: Arc<AtomicBool>,
            appended: Arc<AtomicBool>,
        }
        impl DurableLog for SlowLog {
            fn append(&self, _event: &DomainEvent) -> Result<(), WebhookError> {
                self.entered.store(true, Ordering::SeqCst);
                std::thread::sleep(APPEND_DELAY);
                self.appended.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn configured_hooks(&self) -> Result<Vec<ConfiguredHook>, WebhookError> {
                Ok(Vec::new())
            }
        }

        let entered = Arc::new(AtomicBool::new(false));
        let appended = Arc::new(AtomicBool::new(false));
        let log = Arc::new(SlowLog {
            entered: entered.clone(),
            appended: appended.clone(),
        });

        let sup = Arc::new(Supervisor::new());
        let from = sup.register(
            "@Trigger",
            Some("trigger".into()),
            Box::new(std::io::sink()),
        );
        let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
        let mut rx = sup.subscribe();

        let engine = WebhookEngine::with_log(log, sup, RateLimitConfig::default());
        wlock(&engine.state.hooks).insert(
            HOOK.to_string(),
            HookBinding {
                secret: "secret-drop".to_string(),
                from,
                target_ref: "@Dev".to_string(),
            },
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(engine.serve(listener));

        // Cliente: envia o request completo e FECHA o socket assim que o append entra — sem
        // nunca ler a resposta (simula timeout/desistência do integrador).
        let body: &[u8] = br#"{"order":42}"#;
        let sig = sign_hex("secret-drop", body);
        let req = format!(
            "POST /hook/{HOOK} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nX-Signature: {sig}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        stream.write_all(req.as_bytes()).await.expect("head");
        stream.write_all(body).await.expect("body");
        stream.flush().await.expect("flush");

        for _ in 0..300 {
            if entered.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            entered.load(Ordering::SeqCst),
            "o request deve ter entrado no append antes da desconexão"
        );
        drop(stream); // desconexão DURANTE o append

        // Dá tempo do append terminar + o desfecho publicar.
        tokio::time::sleep(APPEND_DELAY + Duration::from_millis(700)).await;
        assert!(
            appended.load(Ordering::SeqCst),
            "o append deve completar mesmo com o cliente desconectado (spawn_blocking é eager)"
        );

        // O EFEITO não pode se perder: a mensagem do nó-Gatilho tem que chegar ao bus.
        let mut publicado = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, lina_core::BusEvent::Message { .. }) {
                publicado = true;
                break;
            }
        }
        assert!(
            publicado,
            "a publicação no bus NÃO pode se perder quando o cliente desconecta durante o \
             append (desfecho roda em task própria, imune ao drop do handler)"
        );
    }

    /// F1-6-4 (memória O(cap) + custo O(1)): o mapa de baldes por rota NUNCA cresce além do cap
    /// — chave nova com o mapa cheio de janelas vivas NÃO aloca (e é admitida: o teto global é o
    /// freio dela); janelas EXPIRADAS são recicladas para abrir vaga, mas a varredura O(cap)
    /// roda com throttle ([`PURGE_MIN_INTERVAL`]). Usa o helper direto (o cap real,
    /// `MAX_ROUTE_BUCKETS`, só muda a constante — a mecânica é a mesma).
    #[test]
    fn baldes_por_rota_tem_memoria_limitada_e_reciclam_expirados() {
        let buckets = Mutex::new(RouteBuckets::new());
        // Janela mais curta que o throttle: após dormir > PURGE_MIN_INTERVAL, tudo expirou E a
        // varredura está liberada — provamos reciclagem sem relógio falso.
        let limit = WindowLimit {
            max_requests: 100,
            window: Duration::from_millis(30),
        };

        // Enche o mapa até o cap (chaves curtas e distintas).
        for i in 0..MAX_ROUTE_BUCKETS {
            assert!(allow_route_window(&buckets, &format!("K{i}"), limit));
        }
        assert_eq!(lock(&buckets).map.len(), MAX_ROUTE_BUCKETS, "mapa no cap");

        // Mapa cheio de janelas VIVAS: chave nova é admitida SEM alocar balde (a 1ª tentativa
        // pode até varrer — throttle nasce vencido — mas nada expirou ainda; as seguintes nem
        // varrem). Duas tentativas seguidas provam os dois caminhos (com e sem varredura).
        for tentativa in 0..2 {
            assert!(
                allow_route_window(&buckets, "CHAVE-NOVA-SEM-VAGA", limit),
                "sem vaga (tentativa {tentativa}), a chave nova segue só sob o teto global"
            );
        }
        assert_eq!(
            lock(&buckets).map.len(),
            MAX_ROUTE_BUCKETS,
            "cap respeitado: nada alocado acima do limite (flood de chaves não compra memória)"
        );

        // Janelas expiram E o throttle libera → a próxima chave nova RECICLA e ganha balde.
        std::thread::sleep(PURGE_MIN_INTERVAL + Duration::from_millis(100));
        assert!(allow_route_window(&buckets, "CHAVE-POS-EXPIRACAO", limit));
        let b = lock(&buckets);
        assert!(
            b.map.len() <= MAX_ROUTE_BUCKETS && b.map.contains_key("CHAVE-POS-EXPIRACAO"),
            "expirados reciclados e a chave nova ganhou balde (len={})",
            b.map.len()
        );
    }

    /// F1-6-4 (âncora de memória): chave de rota implausível (maior que hook_id legítimo) não
    /// ganha balde — segue admitida sob o teto global. Regra ESTRUTURAL por tamanho, não por
    /// existência: não há sinal de oráculo aqui.
    #[test]
    fn chave_de_rota_gigante_nao_vira_ancora_de_memoria() {
        let buckets = Mutex::new(RouteBuckets::new());
        let limit = WindowLimit {
            max_requests: 3,
            window: Duration::from_secs(60),
        };
        let gigante = "X".repeat(MAX_ROUTE_KEY_LEN + 1);
        for _ in 0..10 {
            assert!(
                allow_route_window(&buckets, &gigante, limit),
                "chave gigante é admitida (teto global cobre), nunca negada por este balde"
            );
        }
        assert!(
            lock(&buckets).map.is_empty(),
            "chave gigante NÃO aloca balde (memória não cresce com URLs enormes)"
        );
    }

    /// F1-6-4 (simetria anti-oráculo, nível unitário): a mecânica do balde é IDÊNTICA para
    /// qualquer chave — mesmo limite, mesmo onset de recusa — não importa se a rota existe no
    /// engine (o helper nem tem acesso ao roster de hooks; a prova E2E está no teste de
    /// integração `orcamento_de_lixo_nao_revela_existencia_de_hook`).
    #[test]
    fn balde_por_rota_e_simetrico_para_qualquer_chave() {
        let buckets = Mutex::new(RouteBuckets::new());
        let limit = WindowLimit {
            max_requests: 3,
            window: Duration::from_secs(60),
        };
        let onset = |key: &str| -> usize {
            for i in 1..=10 {
                if !allow_route_window(&buckets, key, limit) {
                    return i;
                }
            }
            11
        };
        assert_eq!(
            onset("ROTA-A"),
            onset("ROTA-B"),
            "onset de recusa idêntico para chaves distintas (mesma janela/limite)"
        );
    }
}
