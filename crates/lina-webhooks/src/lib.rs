//! `lina-webhooks` — Engine de Webhooks do nó-Gatilho (story **W5-4**).
//!
//! O mínimo do **nó-Gatilho**: um servidor `axum` local que recebe um `POST` externo, valida
//! **HMAC**, responde **202** e publica o evento no **Workspace Bus** como mais um produtor — sem
//! abrir nenhuma porta do produto de 5 anos.
//!
//! ## Invariantes honrados (`CLAUDE.md`)
//! - **#2 Local-first.** O listener faz bind SOMENTE em `127.0.0.1` (o chamador cria o
//!   [`tokio::net::TcpListener`]; recomenda-se `127.0.0.1:0`). Nunca `0.0.0.0`.
//! - **#4 O event log é a fonte da verdade.** Cada disparo válido vira um
//!   [`DomainEvent::WebhookReceived`] no [`EventStore`] ANTES de qualquer efeito no bus; a
//!   config de cada hook é um [`DomainEvent::WebhookConfigured`] (event-sourcing).
//! - **Secret fora do disco.** O secret HMAC vive no Secret Vault (W0-7,
//!   [`SecretVault::ensure_webhook_secret`]) — nunca no log nem em claro.
//!
//! ## Segurança (gate humano — campos controláveis pelo atacante)
//! O POST externo só controla: o `hook_id` da URL, o header `X-Signature` e o corpo. NENHUM
//! desses decide identidade ou destino: `from` (nó-Gatilho) e `target_ref` são config do
//! servidor (fixados em [`WebhookEngine::configure_hook`]), não do request. O `hook_id` é uma
//! **capability** base32 não-enumerável (160 bits); o HMAC prova posse do secret compartilhado;
//! a divergência → **401 sem publicar**. `verify_slice` compara em tempo constante (`subtle`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
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

/// Política de rate-limit por hook (janela fixa). Default: 120 req / 60s por `hook_id`.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Máximo de requisições aceitas dentro de uma janela.
    pub max_requests: u32,
    /// Duração da janela.
    pub window: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 120,
            window: Duration::from_secs(60),
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

/// Janela de rate-limit de um hook.
#[derive(Debug)]
struct RateWindow {
    start: Instant,
    count: u32,
}

/// Estado compartilhado do servidor (clonado para dentro do router axum — tudo `Arc`).
#[derive(Clone)]
struct AppState {
    hooks: Arc<RwLock<HashMap<String, HookBinding>>>,
    store: Arc<Mutex<EventStore>>,
    supervisor: Arc<Supervisor>,
    rate: Arc<Mutex<HashMap<String, RateWindow>>>,
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

    /// Novo engine com rate-limit customizado.
    #[must_use]
    pub fn with_config(
        store: Arc<Mutex<EventStore>>,
        supervisor: Arc<Supervisor>,
        rate_cfg: RateLimitConfig,
    ) -> Self {
        Self {
            state: AppState {
                hooks: Arc::new(RwLock::new(HashMap::new())),
                store,
                supervisor,
                rate: Arc::new(Mutex::new(HashMap::new())),
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
        lock(&self.state.store)
            .append(&DomainEvent::WebhookConfigured {
                hook_id: hook_id.clone(),
                target_ref: target_ref.to_string(),
            })
            .map_err(|e| WebhookError::Store(e.to_string()))?;

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
    pub fn router(&self) -> Router {
        Router::new()
            .route("/hook/:hook_id", post(handle_hook))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(self.state.clone())
    }

    /// Atende o `listener` (criado pelo chamador). Consome o engine. **Enforce do invariante #2**
    /// (local-first): recusa servir se o listener não estiver numa interface loopback — a lib não
    /// confia cegamente no chamador para nunca expor `0.0.0.0`.
    pub async fn serve(self, listener: tokio::net::TcpListener) -> Result<(), WebhookError> {
        if let Ok(addr) = listener.local_addr() {
            ensure_local(addr)?;
        }
        let app = self.router();
        axum::serve(listener, app)
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

/// Handler do webhook (`POST /hook/:hook_id`). Ordem das checagens é deliberada:
/// `404` hook desconhecido → `429` rate-limit → `401` HMAC ausente/inválido → `202` aceito.
/// O **202 é imediato**; o processamento (log + bus) roda FORA do caminho da resposta.
async fn handle_hook(
    State(state): State<AppState>,
    Path(hook_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // 1. Hook conhecido? 404 opaco — a rota é uma capability base32 não-enumerável.
    let binding = match rlock(&state.hooks).get(&hook_id) {
        Some(b) => b.clone(),
        None => return StatusCode::NOT_FOUND,
    };

    // 2. Rate-limit POR hook (só hooks conhecidos entram no mapa → ele fica limitado às rotas
    //    configuradas; um atacante batendo em rotas aleatórias toma 404 e não cria entrada).
    if !state.allow_rate(&hook_id) {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    // 3. HMAC OBRIGATÓRIO. Ausência OU divergência → 401 SEM publicar nada (gate de publicação).
    let Some(sig) = headers.get("x-signature").and_then(|v| v.to_str().ok()) else {
        return StatusCode::UNAUTHORIZED;
    };
    if !verify_hex(&binding.secret, &body, sig) {
        return StatusCode::UNAUTHORIZED;
    }

    // 4. 202 IMEDIATO; processa o disparo fora do caminho da resposta (invariante async).
    let ts = now_millis();
    tokio::spawn(async move {
        state.process(&hook_id, ts, &binding);
    });
    StatusCode::ACCEPTED
}

impl AppState {
    /// Processa um disparo VÁLIDO fora do caminho da resposta: registra `WebhookReceived` no log
    /// (fonte da verdade, #4) e, SÓ ENTÃO, publica a mensagem ao alvo no Workspace Bus — o efeito no
    /// bus deriva do fato persistido; se o append falha, nada é publicado.
    fn process(&self, hook_id: &str, ts: u64, binding: &HookBinding) {
        let ev = DomainEvent::WebhookReceived {
            hook_id: hook_id.to_string(),
            ts,
            target_ref: binding.target_ref.clone(),
        };
        if let Err(e) = lock(&self.store).append(&ev) {
            tracing::error!(hook_id, error = %e, "falha ao apendar WebhookReceived; não publica no bus");
            return;
        }
        // Mensagem do nó-Gatilho (`from`, fixado na config) endereçada ao `target_ref`. `from`/alvo
        // NUNCA vêm do request — só o servidor os define (sem impersonação via campo controlável).
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

    /// Rate-limit de janela fixa por hook. `true` se a requisição cabe na janela corrente.
    fn allow_rate(&self, hook_id: &str) -> bool {
        let now = Instant::now();
        let mut map = lock(&self.rate);
        let w = map.entry(hook_id.to_string()).or_insert(RateWindow {
            start: now,
            count: 0,
        });
        if now.duration_since(w.start) > self.rate_cfg.window {
            w.start = now;
            w.count = 0;
        }
        w.count += 1;
        w.count <= self.rate_cfg.max_requests
    }
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
}
