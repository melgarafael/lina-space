//! F4-1-1 · Canal WhatsApp/Waha — transporte HTTP de saída (sessão por QR, custódia de sessão).
//!
//! Território do **Terminal B (Ultra Code)**, implementado contra o **ADR 0050** (aceito). Um cliente
//! HTTP fino para uma instância **WAHA** (WhatsApp HTTP API, `devlikeapro/waha`), falando REST com
//! `X-Api-Key` (o segredo custodiado, JAMAIS no env do agente). Os verbos do ciclo de vida:
//! - **conectar:** cria/inicia a sessão (`POST /api/sessions { name, start }`) → QR (`GET
//!   /api/{session}/auth/qr`) → status (`GET /api/sessions/{name}`, estados `WORKING`/…).
//! - **ler:** puxa mensagens de uma conversa DECLARADA (`GET /api/{session}/chats/{chatId}/messages`)
//!   on-demand. O conteúdo volta ao chamador como contexto, NUNCA ao log.
//! - **enviar:** `POST /api/sendText { session, chatId, text }` — chamado SÓ pelo `execute(&secret)` do
//!   broker (custódia ADR 0004), após o gate humano. Nunca direto pelo agente.
//! - **desconectar:** `POST /api/sessions/{name}/stop` → zera o cano.
//!
//! ## Doutrina dura (ADR 0050 · não regredir)
//! - O `X-Api-Key` vive no `lina-secrets` (escopo `channel:whatsapp`); chega aqui só por PARÂMETRO de
//!   cada verbo (lido server-side do cofre). NUNCA é lido do ambiente nem logado — o [`Debug`] de
//!   [`WahaHttpRequest`] redige a chave (teste `request_debug_nao_vaza_api_key`).
//! - Endpoint default-local (`http://127.0.0.1:3000`); sem canal ativo, zero socket. Endpoint remoto
//!   (https) é exposição opt-in que liga a feature `tls` do `ureq` — fora do escopo MVP, porta aberta.
//! - A ÚNICA superfície que toca a rede é a borda [`WahaHttp`] — injetável: prod [`UreqHttp`] + um mock
//!   de teste que CONTA requests (prova "exatamente 1 POST /api/sendText" sem rede, gate b do QA).
//! - Escopo MÍNIMO: `chat_id`/`text` passam DIRETO. SEM LID bridges, dedup, normalização, outbox ou
//!   retry do guia de produção (anti-creep, épico §V.4).

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

/// Endpoint Waha default — loopback, local-first (ADR 0050 §3). Endpoint remoto é exposição opt-in.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:3000";

/// O que pode dar errado ao falar com o Waha. Deriva `Clone + PartialEq + Eq` para os testes
/// compararem com `assert_eq!` e para quem manuseia o `WahaError` direto — NÃO porque se propague ao
/// wrapper `LeituraError<WahaError>` do `channel_read` (Terminal K): esse é só `Debug` (embrulha
/// `StoreError`, não-comparável) e compara via `matches!`. **Nunca** carrega o `api_key` nem o corpo
/// da resposta (que pode ter conteúdo de mensagem) — só o código de status.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum WahaError {
    /// Falha de transporte (conexão recusada, timeout, DNS) — o Waha não respondeu. Reusa a mensagem
    /// do cliente HTTP; o timeout curto garante que um Waha pendurado vira erro, não freeze (ADR 0046).
    #[error("transporte HTTP do Waha falhou: {0}")]
    Transport(String),
    /// O Waha respondeu, mas com status fora de 2xx. Só o código — o corpo pode conter conteúdo de
    /// mensagem e nunca entra no erro (que pode ser logado).
    #[error("Waha respondeu com status {status}")]
    Status { status: u16 },
    /// A resposta veio 2xx mas não casou o formato esperado (JSON ilegível, status de sessão
    /// desconhecido). É um erro honesto: não inventamos um estado que não soubemos ler.
    #[error("resposta do Waha ilegível: {0}")]
    Decode(String),
}

/// Método HTTP do escopo mínimo do Waha — só `GET`/`POST` (sem PUT/DELETE nesta onda).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

/// Uma request HTTP crua para o Waha. O [`Debug`] é MANUAL e **redige `api_key`**: o derive padrão
/// imprimiria o segredo, e um vazamento no log é a falha que a custódia (ADR 0004) existe para impedir.
pub struct WahaHttpRequest {
    pub method: HttpMethod,
    /// Caminho relativo à `base_url` (ex.: `"/api/sendText"`). O [`WahaClient`] o monta; a borda
    /// só o concatena com a `base_url`.
    pub path: String,
    /// Header `X-Api-Key` — NUNCA logado (ver o `Debug` manual abaixo).
    pub api_key: String,
    /// Corpo JSON do POST (`None` para GET).
    pub body: Option<serde_json::Value>,
}

impl std::fmt::Debug for WahaHttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WahaHttpRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("api_key", &"<redigido>")
            .field("has_body", &self.body.is_some())
            .finish()
    }
}

/// A resposta crua do Waha — status + bytes do corpo. O [`WahaClient`] interpreta conforme o verbo.
#[derive(Debug, Clone)]
pub struct WahaHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// **A única superfície que toca a rede** (ADR 0050 §4). Borda injetável: prod [`UreqHttp`] + um mock
/// de teste. Mockar no nível de I/O cru (não de domínio) prova que `send` faz *exatamente* 1
/// `POST /api/sendText` com o header certo — não só que `send()` foi chamado.
pub trait WahaHttp {
    /// Executa a request e devolve a resposta crua, ou um [`WahaError::Transport`] se a rede falhou.
    ///
    /// # Errors
    /// [`WahaError::Transport`] se o transporte falha (conexão/timeout). Status não-2xx **não** é erro
    /// aqui — volta em [`WahaHttpResponse::status`] para o [`WahaClient`] decidir.
    fn execute(&self, req: WahaHttpRequest) -> Result<WahaHttpResponse, WahaError>;
}

/// Estado de uma sessão Waha — espelha o guia (`GET /api/sessions/{name}` → `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Starting,
    ScanQrCode,
    Working,
    Failed,
    Stopped,
}

impl SessionStatus {
    /// Mapeia o `status` cru do Waha (case-insensitive, `-`→`_`). Um estado desconhecido é um
    /// [`WahaError::Decode`] — não fabricamos um estado que não soubemos interpretar (COD-4).
    ///
    /// # Errors
    /// [`WahaError::Decode`] se `raw` não é um dos estados conhecidos.
    fn from_waha(raw: &str) -> Result<Self, WahaError> {
        match raw.to_ascii_uppercase().replace('-', "_").as_str() {
            "STARTING" => Ok(Self::Starting),
            "SCAN_QR_CODE" => Ok(Self::ScanQrCode),
            "WORKING" => Ok(Self::Working),
            "FAILED" => Ok(Self::Failed),
            "STOPPED" => Ok(Self::Stopped),
            other => Err(WahaError::Decode(format!(
                "status de sessão desconhecido: {other}"
            ))),
        }
    }
}

/// Imagem do QR de pareamento (`GET /api/{session}/auth/qr`). Bytes opacos + mime — o app os renderiza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrImage {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Uma mensagem lida de uma conversa Waha — o conteúdo que volta ao chamador como CONTEXTO (nunca ao
/// log). Campos mínimos: quem mandou, quando, o texto. **Sem** `deny_unknown_fields`: o Waha manda
/// dezenas de campos (`id`, `fromMe`, `ack`, mídia…) que ignoramos — escopo mínimo, sem normalização.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WahaMessage {
    #[serde(default)]
    pub from: String,
    /// Unix timestamp (segundos) do Waha; `0` se ausente.
    #[serde(default, rename = "timestamp")]
    pub ts: i64,
    /// Texto da mensagem (`body` no Waha); vazio para mídia sem legenda.
    #[serde(default, rename = "body")]
    pub text: String,
}

/// Referência da mensagem enviada — o `id` que o Waha devolve no `POST /api/sendText`. `Option` porque
/// um 2xx é envio bem-sucedido mesmo se o id não for parseável: falhar o envio por causa do id seria
/// punir o sucesso (o id é informativo; o efeito externo já ocorreu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentRef {
    pub message_id: Option<String>,
}

/// O cliente do canal Waha: traduz os verbos do canal → requests HTTP, genérico sobre a borda
/// [`WahaHttp`] → testável sem rede. Guarda a `session` (nome da sessão Waha — o `session_ref`
/// custodiado, identificador opaco, JAMAIS o token). O `api_key` NÃO é guardado: chega por parâmetro
/// de cada verbo, lido server-side do cofre, para o cliente nunca o reter nem logar (ADR 0050 §3).
pub struct WahaClient<H: WahaHttp> {
    http: H,
    session: String,
}

impl<H: WahaHttp> WahaClient<H> {
    /// Monta o cliente com uma borda HTTP e o nome da sessão Waha (`session_ref`).
    pub fn new(http: H, session: impl Into<String>) -> Self {
        Self {
            http,
            session: session.into(),
        }
    }

    /// Cria e inicia a sessão Waha (`POST /api/sessions { name, start: true }`). Emite o gesto de
    /// conexão; o `ChannelConnected` quem apenda é a story de projeção (Terminal I), não o transporte.
    ///
    /// # Errors
    /// [`WahaError`] se o transporte falha ou o Waha responde fora de 2xx.
    pub fn connect(&self, api_key: &str) -> Result<(), WahaError> {
        let resp = self.http.execute(WahaHttpRequest {
            method: HttpMethod::Post,
            path: "/api/sessions".to_string(),
            api_key: api_key.to_string(),
            body: Some(serde_json::json!({ "name": self.session, "start": true })),
        })?;
        expect_success(&resp)
    }

    /// Busca o QR de pareamento (`GET /api/{session}/auth/qr`, PNG).
    ///
    /// # Errors
    /// [`WahaError`] se o transporte falha ou o Waha responde fora de 2xx.
    pub fn qr(&self, api_key: &str) -> Result<QrImage, WahaError> {
        let resp = self.http.execute(WahaHttpRequest {
            method: HttpMethod::Get,
            path: format!("/api/{}/auth/qr", self.session),
            api_key: api_key.to_string(),
            body: None,
        })?;
        expect_success(&resp)?;
        Ok(QrImage {
            bytes: resp.body,
            mime: "image/png".to_string(),
        })
    }

    /// Lê o estado da sessão (`GET /api/sessions/{name}`).
    ///
    /// # Errors
    /// [`WahaError`] se o transporte falha, o status é não-2xx, ou o estado é desconhecido.
    pub fn status(&self, api_key: &str) -> Result<SessionStatus, WahaError> {
        let resp = self.http.execute(WahaHttpRequest {
            method: HttpMethod::Get,
            path: format!("/api/sessions/{}", self.session),
            api_key: api_key.to_string(),
            body: None,
        })?;
        expect_success(&resp)?;
        let value: serde_json::Value =
            serde_json::from_slice(&resp.body).map_err(|e| WahaError::Decode(e.to_string()))?;
        let raw = value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        SessionStatus::from_waha(raw)
    }

    /// Lê até `limit` mensagens de uma conversa DECLARADA (`GET /api/{session}/chats/{chatId}/
    /// messages`). O conteúdo volta ao chamador como contexto; o gate de escopo + o rastro
    /// `ChannelMessageRead` são do `channel_read` (Terminal K), que chama este verbo pela borda dele.
    ///
    /// # Errors
    /// [`WahaError`] se o transporte falha, o status é não-2xx, ou o JSON é ilegível.
    pub fn read(
        &self,
        api_key: &str,
        chat_id: &str,
        limit: u32,
    ) -> Result<Vec<WahaMessage>, WahaError> {
        let resp = self.http.execute(WahaHttpRequest {
            method: HttpMethod::Get,
            path: format!(
                "/api/{}/chats/{chat_id}/messages?limit={limit}&downloadMedia=false",
                self.session
            ),
            api_key: api_key.to_string(),
            body: None,
        })?;
        expect_success(&resp)?;
        serde_json::from_slice(&resp.body).map_err(|e| WahaError::Decode(e.to_string()))
    }

    /// Envia uma mensagem de texto (`POST /api/sendText { session, chatId, text }`). Efeito externo
    /// irreversível: chamado SÓ pelo `execute(&secret)` do broker, após o gate humano (ADR 0004) —
    /// o transporte não conhece o gate, só executa quando lhe entregam o `api_key` do cofre.
    ///
    /// # Errors
    /// [`WahaError`] se o transporte falha ou o Waha responde fora de 2xx.
    pub fn send(&self, api_key: &str, chat_id: &str, text: &str) -> Result<SentRef, WahaError> {
        let resp = self.http.execute(WahaHttpRequest {
            method: HttpMethod::Post,
            path: "/api/sendText".to_string(),
            api_key: api_key.to_string(),
            body: Some(
                serde_json::json!({ "session": self.session, "chatId": chat_id, "text": text }),
            ),
        })?;
        expect_success(&resp)?;
        Ok(SentRef {
            message_id: extract_message_id(&resp.body),
        })
    }

    /// Encerra a sessão (`POST /api/sessions/{name}/stop`) — zera o cano. O `ChannelDisconnected`
    /// quem apenda é a projeção (Terminal I).
    ///
    /// # Errors
    /// [`WahaError`] se o transporte falha ou o Waha responde fora de 2xx.
    pub fn disconnect(&self, api_key: &str) -> Result<(), WahaError> {
        let resp = self.http.execute(WahaHttpRequest {
            method: HttpMethod::Post,
            path: format!("/api/sessions/{}/stop", self.session),
            api_key: api_key.to_string(),
            body: None,
        })?;
        expect_success(&resp)
    }
}

/// `Ok` sse o status é 2xx; senão [`WahaError::Status`] (sem o corpo, que pode ter conteúdo).
fn expect_success(resp: &WahaHttpResponse) -> Result<(), WahaError> {
    if (200..300).contains(&resp.status) {
        Ok(())
    } else {
        Err(WahaError::Status {
            status: resp.status,
        })
    }
}

/// Extrai o id da mensagem do corpo do `sendText`, lenientemente. O Waha tem 3+ formatos para o id
/// (string crua `<shortId>`, ou objeto `{ id, _serialized }`); aqui pegamos o que der sem inventar
/// normalização (anti-creep). `None` se ilegível — o envio (2xx) já foi um sucesso.
fn extract_message_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    match &value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(_) => value.get("id").and_then(|id| match id {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(_) => id
                .get("_serialized")
                .or_else(|| id.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(String::from),
            _ => None,
        }),
        _ => None,
    }
}

/// Implementação de produção da borda [`WahaHttp`] sobre `ureq` (síncrono, com timeout curto). A
/// chamada de rede roda off-critical-path na story do app (ADR 0046) — aqui só o transporte cru.
pub struct UreqHttp {
    agent: ureq::Agent,
    base_url: String,
}

impl UreqHttp {
    /// Cliente apontando para `base_url`, com timeout de conexão+leitura de 10s (ADR 0050 §1: Waha
    /// pendurado vira erro, não freeze).
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(10))
            .build();
        Self {
            agent,
            base_url: base_url.into(),
        }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

impl WahaHttp for UreqHttp {
    fn execute(&self, req: WahaHttpRequest) -> Result<WahaHttpResponse, WahaError> {
        let url = format!("{}{}", self.base_url, req.path);
        let request = match req.method {
            HttpMethod::Get => self.agent.get(&url),
            HttpMethod::Post => self.agent.post(&url),
        }
        .set("X-Api-Key", &req.api_key);

        let result = match &req.body {
            Some(body) => {
                let payload =
                    serde_json::to_string(body).map_err(|e| WahaError::Decode(e.to_string()))?;
                request
                    .set("Content-Type", "application/json")
                    .send_string(&payload)
            }
            None => request.call(),
        };

        match result {
            Ok(resp) => read_response(resp),
            // 4xx/5xx ainda é uma resposta do Waha — capturamos status+corpo para o cliente decidir.
            Err(ureq::Error::Status(_code, resp)) => read_response(resp),
            Err(ureq::Error::Transport(t)) => Err(WahaError::Transport(t.to_string())),
        }
    }
}

/// Lê status + corpo de uma resposta `ureq` para a forma crua [`WahaHttpResponse`].
fn read_response(resp: ureq::Response) -> Result<WahaHttpResponse, WahaError> {
    let status = resp.status();
    let mut body = Vec::new();
    resp.into_reader()
        .read_to_end(&mut body)
        .map_err(|e| WahaError::Transport(e.to_string()))?;
    Ok(WahaHttpResponse { status, body })
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Mock da borda HTTP que CONTA e GUARDA cada request, devolvendo uma resposta canônica. É a impl
    /// de teste da borda real [`WahaHttp`] (dependency injection projetada pelo ADR), não um mock do
    /// objeto sob teste — o que provamos é a request que o [`WahaClient`] MONTA.
    struct CountingHttp {
        requests: RefCell<Vec<WahaHttpRequest>>,
        response: WahaHttpResponse,
    }

    impl CountingHttp {
        fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                response: WahaHttpResponse {
                    status,
                    body: body.into(),
                },
            }
        }
        fn count(&self) -> usize {
            self.requests.borrow().len()
        }
    }

    impl WahaHttp for CountingHttp {
        fn execute(&self, req: WahaHttpRequest) -> Result<WahaHttpResponse, WahaError> {
            self.requests.borrow_mut().push(req);
            Ok(self.response.clone())
        }
    }

    /// Gate (b) do QA: `send` faz EXATAMENTE 1 `POST /api/sendText` com o `X-Api-Key` no header e o
    /// `chatId`/`text` no corpo. É a prova de que o transporte é o ÚNICO emissor e não dispara extras.
    #[test]
    fn send_faz_exatamente_um_post_sendtext_com_api_key() {
        let http = CountingHttp::new(201, br#"{"id":"3EB0C621994F26BCC61AC9"}"#.to_vec());
        let client = WahaClient::new(http, "lina-whatsapp");

        let sent = client
            .send("CHAVE-SECRETA", "5511999@c.us", "olá")
            .expect("envio 2xx");

        let http = client_http(&client);
        assert_eq!(
            http.count(),
            1,
            "exatamente 1 request — nem zero, nem extras"
        );
        let req = &http.requests.borrow()[0];
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.path, "/api/sendText");
        assert_eq!(req.api_key, "CHAVE-SECRETA", "o X-Api-Key vai no header");
        let body = req.body.as_ref().expect("POST carrega corpo");
        assert_eq!(body["chatId"], "5511999@c.us");
        assert_eq!(body["text"], "olá");
        assert_eq!(body["session"], "lina-whatsapp");
        assert_eq!(sent.message_id.as_deref(), Some("3EB0C621994F26BCC61AC9"));
    }

    /// O segredo NUNCA pode vazar pelo `Debug` da request (o derive padrão imprimiria o `api_key`).
    #[test]
    fn request_debug_nao_vaza_api_key() {
        let req = WahaHttpRequest {
            method: HttpMethod::Post,
            path: "/api/sendText".to_string(),
            api_key: "CHAVE-ULTRA-SECRETA-xyz".to_string(),
            body: Some(serde_json::json!({"chatId":"x","text":"y"})),
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("CHAVE-ULTRA-SECRETA-xyz"),
            "Debug vazou o api_key: {rendered}"
        );
        assert!(
            rendered.contains("<redigido>"),
            "o api_key deve aparecer redigido"
        );
    }

    /// COD-4: status não-2xx é ERRO, não sucesso silencioso. A request foi feita (1), mas o resultado
    /// é `Err(Status)` — o transporte não engole a falha do Waha.
    #[test]
    fn status_nao_2xx_vira_erro_nao_sucesso_silencioso() {
        let http = CountingHttp::new(500, b"erro interno".to_vec());
        let client = WahaClient::new(http, "lina-whatsapp");

        let result = client.send("CHAVE", "5511999@c.us", "oi");

        assert_eq!(result, Err(WahaError::Status { status: 500 }));
        assert_eq!(
            client_http(&client).count(),
            1,
            "a request foi feita, o erro é da resposta"
        );
    }

    /// `read` parseia o array de mensagens, ignorando os campos extras do Waha (escopo mínimo).
    #[test]
    fn read_parseia_mensagens_e_ignora_campos_extras() {
        let payload = br#"[
            {"id":"A","from":"5511@c.us","timestamp":1700000000,"body":"primeira","fromMe":false,"ack":2},
            {"id":"B","from":"5522@c.us","timestamp":1700000100,"body":"segunda","hasMedia":true}
        ]"#;
        let http = CountingHttp::new(200, payload.to_vec());
        let client = WahaClient::new(http, "lina-whatsapp");

        let msgs = client.read("CHAVE", "grupo@g.us", 50).expect("leitura 2xx");

        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].from, "5511@c.us");
        assert_eq!(msgs[0].ts, 1_700_000_000);
        assert_eq!(msgs[0].text, "primeira");
        assert_eq!(msgs[1].text, "segunda");

        let http = client_http(&client);
        let req = &http.requests.borrow()[0];
        assert_eq!(req.method, HttpMethod::Get);
        assert!(
            req.path
                .starts_with("/api/lina-whatsapp/chats/grupo@g.us/messages"),
            "path: {}",
            req.path
        );
        assert!(
            req.path.contains("limit=50"),
            "o limit vai na query: {}",
            req.path
        );
        assert!(req.body.is_none(), "GET não carrega corpo");
    }

    /// O manifesto declarado deste canal é VÁLIDO contra o schema (`deny_unknown_fields`, ref pinada).
    #[test]
    fn whatsapp_manifest_e_valido() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../channels/whatsapp/manifest.toml"
        ))
        .expect("lê o manifesto do canal");
        let manifest = crate::channel::ChannelManifest::parse(&src).expect("manifesto válido");
        assert_eq!(manifest.name, "whatsapp");
        assert_eq!(manifest.transport, "waha_http");
        assert_eq!(manifest.auth, "api_key");
        assert!(manifest.scopes.contains(&"messages:send".to_string()));
        assert!(manifest.scopes.contains(&"messages:read".to_string()));

        // Costura com o broker (ADR 0050 §5): o gate `channel_action_in_manifest` faz match EXATO do
        // slug da ação de `do:channel:whatsapp:<ação>`. Sem `send`/`read` declarados, o envio seria
        // negado por default-deny ANTES da custódia. Provamos contra a função REAL do broker para a
        // costura nunca regredir (e uma ação não-declarada continua negada).
        assert!(crate::broker::channel_action_in_manifest(&manifest, "send"));
        assert!(crate::broker::channel_action_in_manifest(&manifest, "read"));
        assert!(!crate::broker::channel_action_in_manifest(
            &manifest,
            "delete_everything"
        ));
    }

    /// Helper: o `WahaClient` não expõe o `http`; o teste o alcança por este acessor só-de-teste.
    fn client_http<H: WahaHttp>(client: &WahaClient<H>) -> &H {
        &client.http
    }
}
