//! Receiver OTLP/HTTP **loopback-only** (F1-1-4). Recebe `POST /v1/metrics` (e
//! `/v1/logs`, absorvido) em `127.0.0.1:4318`, normaliza no schema de sessão e chama o
//! [`Sink`] **a cada export** — persist-on-receipt, SEM janela de batch própria (a
//! mitigação do critério 3: dados até o último export já estão persistidos quando o
//! terminal morre). Molde fiel ao `lina-webhooks` (W5-4): bind só loopback, teto de
//! corpo, enforce do invariante #2 no `serve`.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::post,
    Router,
};

use crate::{ensure_loopback, normalize_metrics, OtelError, Redactor};

/// Teto de corpo de um export OTLP (defensivo; exports de métricas são pequenos).
const MAX_BODY_BYTES: usize = 256 * 1024;

/// **Sink chamado a cada export normalizado** — `(session_id, OtelCost)`. O caller liga
/// isto ao upsert da `SessionProjection`/merge (persist-on-receipt). `Send + Sync` para
/// atravessar o handler async.
pub type Sink = Arc<dyn Fn(&str, lina_session_watch::OtelCost) + Send + Sync>;

#[derive(Clone)]
struct AppState {
    sink: Sink,
    redactor: Option<Arc<Redactor>>,
}

/// O collector OTLP local. Construído com um [`Sink`]; servido via [`serve`].
pub struct OtelReceiver {
    state: AppState,
}

impl OtelReceiver {
    /// Novo receiver. `redact` ativa o [`Redactor`] de PII (não-opcional em produção
    /// quando o collector está ligado — 13.5 item 3; a config força isso).
    #[must_use]
    pub fn new(sink: Sink, redact: bool) -> Self {
        Self {
            state: AppState {
                sink,
                redactor: redact.then(|| Arc::new(Redactor)),
            },
        }
    }

    /// Router axum (`POST /v1/metrics` + `/v1/logs`, teto de corpo). Exposto para
    /// composição/teste; produção usa [`serve`].
    pub fn router(&self) -> Router {
        router(self.state.clone())
    }

    /// Atende o `listener` (criado pelo chamador em `127.0.0.1`). **Enforce do invariante
    /// #2**: recusa servir fora de loopback. Consome o receiver.
    ///
    /// # Errors
    /// [`OtelError::NotLocal`] se o listener não é loopback; erro de serve do axum.
    pub async fn serve(self, listener: tokio::net::TcpListener) -> Result<(), OtelError> {
        serve(self.state, listener).await
    }
}

/// Router interno (o `AppState` é privado — exposto só via [`OtelReceiver::router`]).
fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/metrics", post(handle_metrics))
        // OTLP/logs é absorvido (200) mas não normalizado nesta story (métricas é o que
        // carrega tokens/custo); existir evita o CLI logar erro de exporter.
        .route("/v1/logs", post(handle_logs))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn serve(state: AppState, listener: tokio::net::TcpListener) -> Result<(), OtelError> {
    if let Ok(addr) = listener.local_addr() {
        ensure_loopback(addr)?;
    }
    axum::serve(listener, router(state).into_make_service())
        .await
        .map_err(|e| OtelError::NotLocal(format!("serve: {e}")))
}

async fn handle_metrics(State(state): State<AppState>, body: Bytes) -> StatusCode {
    let Ok(export) = serde_json::from_slice::<serde_json::Value>(&body) else {
        // Corpo inválido: 400, sem reter nada (a observabilidade nunca derruba o CLI).
        return StatusCode::BAD_REQUEST;
    };
    let costs = normalize_metrics(&export, state.redactor.as_deref());
    // Persist-on-receipt: chama o sink AGORA, por sessão (sem batch próprio).
    for (session_id, cost) in costs {
        (state.sink)(&session_id, cost);
    }
    StatusCode::OK
}

async fn handle_logs(body: Bytes) -> StatusCode {
    // Absorvido para o CLI não falhar o exporter de logs; não normalizamos nesta story.
    let _ = body;
    StatusCode::OK
}

// `AppState` é privado; exposto só via `OtelReceiver`. Re-export do tipo de erro do serve.
pub use crate::OtelError as ReceiverError;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Persist-on-receipt: o handler chama o sink SÍNCRONO no próprio request — dados do
    /// export ficam disponíveis imediatamente (mitigação do critério 3: sem batch nosso).
    #[tokio::test]
    async fn handler_persiste_no_recebimento() {
        let seen: Arc<Mutex<Vec<(String, u64)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let seen = Arc::clone(&seen);
            Arc::new(move |sid: &str, c: lina_session_watch::OtelCost| {
                seen.lock().unwrap().push((sid.to_owned(), c.tokens_in));
            })
        };
        let state = AppState {
            sink,
            redactor: None,
        };
        let export = serde_json::json!({
            "resourceMetrics": [{
                "resource": {"attributes":[{"key":"session.id","value":{"stringValue":"s"}}]},
                "scopeMetrics": [{
                    "metrics": [
                        {"name":"claude_code.token.usage","sum":{"dataPoints":[
                            {"attributes":[{"key":"type","value":{"stringValue":"input"}}],"asInt":"9"}
                        ]}}
                    ]
                }]
            }]
        });
        let code = handle_metrics(
            State(state),
            Bytes::from(serde_json::to_vec(&export).unwrap()),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(*seen.lock().unwrap(), vec![("s".to_string(), 9)]);
    }

    #[tokio::test]
    async fn corpo_invalido_e_400_sem_reter() {
        let sink: Sink = Arc::new(|_, _| panic!("sink não deve ser chamado com lixo"));
        let state = AppState {
            sink,
            redactor: None,
        };
        let code = handle_metrics(State(state), Bytes::from_static(b"nao e json")).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    /// Critério 4: bind só loopback; `serve` recusa um listener não-loopback. Verificamos
    /// pelo guard `ensure_loopback` (o serve o chama) + que o listener default é loopback.
    #[tokio::test]
    async fn bind_e_loopback_e_serve_recusa_fora_dele() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert!(
            listener.local_addr().unwrap().ip().is_loopback(),
            "o bind do collector é loopback (nada escuta fora de 127.0.0.1)"
        );
        // O guard recusa um endereço não-loopback (o que protege o serve).
        let non_local: std::net::SocketAddr = "10.0.0.5:4318".parse().unwrap();
        assert!(matches!(
            crate::ensure_loopback(non_local),
            Err(crate::OtelError::NotLocal(_))
        ));
        assert!(crate::ensure_loopback(listener.local_addr().unwrap()).is_ok());
    }
}
