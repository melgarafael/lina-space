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
//!   [`DomainEvent::WebhookReceived`] no [`EventStore`] — apendado de forma **SÍNCRONA ANTES do
//!   `202`** (durabilidade primeiro: se o append falha, respondemos `5xx`, nunca `202`). Só o efeito
//!   NÃO-durável (publicar no bus) roda fora do caminho da resposta. A config de cada hook é um
//!   [`DomainEvent::WebhookConfigured`] (event-sourcing).
//! - **Secret fora do disco.** O secret HMAC vive no Secret Vault (W0-7,
//!   [`SecretVault::ensure_webhook_secret`]) — nunca no log nem em claro.
//!
//! ## Defesa de taxa em DUAS camadas (W5-4-MED-a — ver [`RateLimitConfig`])
//! Como o `hook_id` viaja na URL (capability, não segredo), um gate que gaste um recurso escasso
//! ANTES do HMAC é manipulável por quem só conhece a rota. Por isso:
//! 1. **Teto bruto por IP** ANTES de tudo — protege a CPU contra flood (inclusive contra forçar o
//!    cálculo caro de HMAC em massa).
//! 2. **Budget por hook** SÓ DEPOIS do HMAC passar — protege o budget do dono legítimo; corpo-lixo
//!    sem o secret toma `401` e nunca consome o budget do hook.
//!
//! ## Segurança (gate humano — campos controláveis pelo atacante)
//! O POST externo só controla: o `hook_id` da URL, o header `X-Signature` e o corpo. NENHUM
//! desses decide identidade ou destino: `from` (nó-Gatilho) e `target_ref` são config do
//! servidor (fixados em [`WebhookEngine::configure_hook`]), não do request. O `hook_id` é uma
//! **capability** base32 não-enumerável (160 bits); o HMAC prova posse do secret compartilhado;
//! a divergência → **401 sem publicar**. `verify_slice` compara em tempo constante (`subtle`).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
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

/// Rate-limit em **DUAS camadas** (defesa em profundidade — W5-4-MED-a):
/// - **`ip_burst`** — teto BRUTO por IP de origem, checado ANTES do HMAC. Protege a CPU contra
///   flood (inclusive contra forçar o cálculo caro de HMAC). Generoso: só barra abuso massivo.
/// - **`per_hook`** — budget POR `hook_id`, checado SÓ DEPOIS do HMAC passar. Protege o budget do
///   dono legítimo: como o `hook_id` viaja na URL (não é segredo), corpo-lixo sem o secret toma
///   `401` e **nunca** consome este budget — caso contrário o vazamento da rota viraria DoS do
///   budget alheio.
///
/// Por construção `ip_burst.max_requests >= per_hook.max_requests` (o teto de CPU é mais largo que
/// o budget de um hook), mas a corretude não depende disso — as camadas são independentes.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Camada 1 (CPU): teto bruto por IP de origem, ANTES do HMAC.
    pub ip_burst: WindowLimit,
    /// Camada 2 (budget): teto por hook, DEPOIS do HMAC.
    pub per_hook: WindowLimit,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            // ~20 req/s por IP: largo para uso local legítimo (vários hooks compartilham o IP de
            // loopback), estreito o bastante para conter um flood de cálculo de HMAC.
            ip_burst: WindowLimit {
                max_requests: 1200,
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

/// Fronteira de durabilidade: o que o engine precisa de um log de eventos durável. Abstrair o
/// [`EventStore`] concreto aqui mantém o append do caminho da resposta **síncrono e falível** (o
/// handler decide `202` vs `5xx` pelo `Result`) e permite, nos testes, um log que falha de propósito
/// (prova observável do caminho `5xx` sem socket). Produção usa [`EventStoreLog`].
trait DurableLog: Send + Sync {
    /// Apende um evento de domínio de forma durável e SÍNCRONA. `Err` aborta o efeito (sem `202`).
    fn append(&self, event: &DomainEvent) -> Result<(), WebhookError>;
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
}

/// Estado compartilhado do servidor (clonado para dentro do router axum — tudo `Arc`).
#[derive(Clone)]
struct AppState {
    hooks: Arc<RwLock<HashMap<String, HookBinding>>>,
    store: Arc<dyn DurableLog>,
    supervisor: Arc<Supervisor>,
    /// Camada 1: contador bruto por IP de origem (protege CPU, antes do HMAC).
    rate_ip: Arc<Mutex<HashMap<IpAddr, RateWindow>>>,
    /// Camada 2: budget por `hook_id` (protege o dono, depois do HMAC).
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
        Self {
            state: AppState {
                hooks: Arc::new(RwLock::new(HashMap::new())),
                store: Arc::new(EventStoreLog(store)),
                supervisor,
                rate_ip: Arc::new(Mutex::new(HashMap::new())),
                rate_hook: Arc::new(Mutex::new(HashMap::new())),
                rate_cfg,
            },
        }
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
    /// o handler extrai [`ConnectInfo`] para o teto bruto por IP (camada 1). Sem o connect-info o
    /// extractor falha — por isso o `serve` é o caminho suportado.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/hook/:hook_id", post(handle_hook))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(self.state.clone())
    }

    /// Atende o `listener` (criado pelo chamador). Consome o engine. **Enforce do invariante #2**
    /// (local-first): recusa servir se o listener não estiver numa interface loopback — a lib não
    /// confia cegamente no chamador para nunca expor `0.0.0.0`. Injeta o [`ConnectInfo`] do peer
    /// (IP de origem) para a camada 1 do rate-limit.
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

/// Handler do webhook (`POST /hook/:hook_id`). Ordem das checagens é deliberada (W5-4-MED-a — duas
/// camadas de rate-limit; ver [`RateLimitConfig`]):
/// 1. `429` **teto bruto por IP** (camada 1) — ANTES de tudo; protege a CPU contra flood (inclusive
///    contra forçar o cálculo caro de HMAC em massa).
/// 2. `404` hook desconhecido — rota opaca (capability base32 não-enumerável).
/// 3. `401` HMAC ausente/inválido — prova posse do secret; sem ela, NÃO se publica nada.
/// 4. `429` **budget por hook** (camada 2) — SÓ APÓS o HMAC; corpo-lixo sem o secret (que toma 401)
///    nunca consome o budget do dono, então o vazamento do `hook_id` na URL não vira DoS de budget.
/// 5. **Append SÍNCRONO** do `WebhookReceived` (W5-4-MED-b — durabilidade primeiro, invariante #4):
///    se falha → `5xx` sem publicar; só com o fato durável é que respondemos `202` e disparamos o
///    efeito NÃO-durável (publicar no bus) fora do caminho da resposta.
async fn handle_hook(
    State(state): State<AppState>,
    Path(hook_id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // 1. Camada 1 (CPU): teto BRUTO por IP de origem, ANTES de qualquer trabalho caro (lookup,
    //    cópia do binding, HMAC). É a barreira mais externa contra flood.
    if !state.allow_ip_burst(peer.ip()) {
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

    // 5. DURABILIDADE PRIMEIRO (#4): apende o FATO de forma SÍNCRONA, ANTES de responder. O disco é
    //    local; segurar o request até o append durável é o preço da promessa do 202. Se o append
    //    falha, respondemos 5xx (NUNCA 202) e nada é publicado — sem perda silenciosa.
    let ts = now_millis();
    let ev = DomainEvent::WebhookReceived {
        hook_id: hook_id.clone(),
        ts,
        target_ref: binding.target_ref.clone(),
    };
    if let Err(e) = state.store.append(&ev) {
        tracing::error!(hook_id = %hook_id, error = %e, "append de WebhookReceived falhou; respondendo 5xx, nada publicado");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    // 6. Só o efeito NÃO-durável (publicar no bus) corre fora do caminho da resposta. Se ELE se
    //    perder num crash, o replay do log (fonte da verdade, #4) reconstrói — por isso pode ser
    //    assíncrono; a durabilidade já foi garantida no passo 5.
    tokio::spawn(async move {
        state.publish(&binding);
    });
    StatusCode::ACCEPTED
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

    /// Camada 1 (CPU): teto BRUTO por IP de origem. `true` se cabe na janela.
    fn allow_ip_burst(&self, ip: IpAddr) -> bool {
        allow_window(&self.rate_ip, ip, self.rate_cfg.ip_burst)
    }

    /// Camada 2 (budget): teto por hook. `true` se cabe na janela.
    fn allow_hook_budget(&self, hook_id: &str) -> bool {
        allow_window(&self.rate_hook, hook_id.to_string(), self.rate_cfg.per_hook)
    }
}

/// Rate-limit de janela fixa sobre um mapa `chave → janela`. `true` se a requisição cabe na janela
/// corrente. Genérico sobre a chave para servir as duas camadas (IP e `hook_id`) com uma só lógica.
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
    if now.duration_since(w.start) > limit.window {
        w.start = now;
        w.count = 0;
    }
    w.count += 1;
    w.count <= limit.max_requests
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
            rate_ip: Arc::new(Mutex::new(HashMap::new())),
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
}
