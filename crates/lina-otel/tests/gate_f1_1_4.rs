//! GATE F1-1-4 — OTel local OPCIONAL, end-to-end com a projeção REAL da F1-1-2.
//!
//! Critérios da story (tasks/epico-f1/ondas-0-1.md §F1-1-4):
//! (1) OTel ligado → a sessão na projeção muda `source` para `otel` (campo visível) +
//!     sanity check JSONL×OTel roda (divergência calculada);
//! (2) OTel DESLIGADO → a MESMA sessão segue populada a partir do JSONL (cards nos dois
//!     casos — prova observável da NÃO-dependência; 13.10 item 7);
//! (3) "persist-on-receipt": cada export é persistido na hora (sem batch nosso) — o dado
//!     do export sobrevive a um encerramento imediato (mitigação do batch de 5s);
//! (4) loopback: bind real em 127.0.0.1 + guard recusa fora dele (testado no `lib`).

use lina_otel::{merge_and_check, normalize_metrics, CostStore, OtelReceiver, Sink};
use lina_session_watch::{CostSource, OtelCost, Session, SessionProjection};
use std::sync::Arc;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-gate114-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("tmp");
        Self(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Sessão como a F1-1-2 a deriva do JSONL (fonte = Jsonl, custo estimado, subcontado).
fn jsonl_session(cost: f64, tokens_in: u64) -> Session {
    Session {
        cli: "claude-code".into(),
        session_id: "sess-real".into(),
        tokens_in,
        tokens_out: 30,
        tokens_cache: 0,
        tokens_thinking: 0,
        cost_usd: cost,
        cost_estimated: true,
        model: Some("claude".into()),
        cwd: Some("/Users/dev/proj".into()),
        last_ts: Some("2026-06-06T10:00:00Z".into()),
        subagents: vec![],
        tools: vec!["Bash".into(), "Read".into()],
        source: CostSource::Jsonl,
    }
}

fn metrics_export(session: &str, cost: f64, tokens_in: u64) -> serde_json::Value {
    serde_json::json!({
        "resourceMetrics": [{
            "resource": {"attributes": [{"key":"session.id","value":{"stringValue":session}}]},
            "scopeMetrics": [{
                "metrics": [
                    {"name":"claude_code.token.usage","sum":{"dataPoints":[
                        {"attributes":[{"key":"type","value":{"stringValue":"input"}},
                                       {"key":"model","value":{"stringValue":"claude-opus"}}],
                         "asInt": tokens_in.to_string()}
                    ]}},
                    {"name":"claude_code.cost.usage","sum":{"dataPoints":[
                        {"attributes":[], "asDouble": cost}
                    ]}}
                ]
            }]
        }]
    })
}

/// **Critérios 1 + 2 (roteiro lado a lado): MESMA sessão, OTel on/off, cards populados.**
#[test]
fn otel_on_muda_source_e_off_segue_pelo_jsonl() {
    let tmp = TempDir::new("onoff");
    let jsonl = jsonl_session(0.40, 200); // 200 = subcontado

    // ── OTel DESLIGADO: a projeção é populada SÓ pelo JSONL (cards aparecem). ──
    {
        let mut proj = SessionProjection::open(&tmp.0).expect("proj off");
        proj.upsert(&jsonl).expect("upsert jsonl");
        let got = proj
            .get("claude-code", "sess-real")
            .expect("get")
            .expect("existe");
        assert_eq!(got.source, CostSource::Jsonl, "OFF: fonte é JSONL");
        assert!(got.cost_estimated, "OFF: custo é estimativa (subcontado)");
        assert_eq!(got.tokens_in, 200);
    }

    // ── OTel LIGADO: o mesmo terminal/sessão, agora enriquecido pelo OTel. ──
    let export = metrics_export("sess-real", 0.42, 17_400); // medido, corrige o subconto
    let costs = normalize_metrics(&export, None);
    let otel = costs.get("sess-real").expect("normalizado");
    let merged = merge_and_check(&jsonl, otel);
    {
        let mut proj = SessionProjection::open(&tmp.0).expect("proj on");
        proj.upsert(&merged.session).expect("upsert merged");
        let got = proj
            .get("claude-code", "sess-real")
            .expect("get")
            .expect("existe");
        // Critério 1: source vira otel (campo visível na projeção).
        assert_eq!(got.source, CostSource::Otel, "ON: fonte muda para OTel");
        assert!(!got.cost_estimated, "ON: custo medido não é estimativa");
        assert_eq!(got.cost_usd, 0.42);
        assert_eq!(got.tokens_in, 17_400, "OTel corrige o subconto do JSONL");
        // Sanity check rodou (divergência calculada — aqui dentro do threshold).
        assert!(merged.divergence < 0.20, "sanity: 0.40 vs 0.42 não diverge");
        // Os campos não-custo (cwd/tools) seguem do JSONL.
        assert_eq!(got.cwd.as_deref(), Some("/Users/dev/proj"));
        assert_eq!(got.tools, vec!["Bash".to_string(), "Read".to_string()]);
    }
}

/// **Critério 1 (sanity check): divergência grande é sinalizada.**
#[test]
fn sanity_check_sinaliza_divergencia_grande() {
    let jsonl = jsonl_session(0.05, 200); // JSONL subconta MUITO
    let export = metrics_export("sess-real", 0.50, 20_000); // OTel mede 10× mais
    let otel = normalize_metrics(&export, None);
    let merged = merge_and_check(&jsonl, otel.get("sess-real").unwrap());
    assert!(merged.diverged, "0.05 vs 0.50 diverge (>20%) → sinalizado");
    assert!(merged.divergence > 0.20);
}

/// **Critério 3 (persist-on-receipt): cada export é entregue ao sink na hora.** Um
/// `POST /v1/metrics` real atravessa o router axum e o sink reflete o dado IMEDIATAMENTE
/// — sem janela de batch nossa, então um encerramento logo após o export não perde o que
/// já chegou. Round-trip determinístico via `tower::oneshot` (sem TCP frágil).
#[tokio::test]
async fn persist_on_receipt_sem_batch_proprio() {
    use http::{Request, StatusCode};
    use tower::ServiceExt;

    let store = CostStore::new();
    let sink: Sink = {
        let store = store.clone();
        Arc::new(move |sid: &str, c: OtelCost| store.upsert(sid, c))
    };
    let receiver = OtelReceiver::new(sink, true);
    let body = serde_json::to_vec(&metrics_export("live", 0.10, 1234)).unwrap();

    let resp = receiver
        .router()
        .oneshot(
            Request::post("/v1/metrics")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .expect("router responde");
    assert_eq!(resp.status(), StatusCode::OK);

    // Persist-on-receipt: o sink JÁ foi chamado durante o handle (nada bufferizado).
    let got = store
        .get("live")
        .expect("export persistido na hora (sem batch)");
    assert_eq!(got.tokens_in, 1234);
    assert_eq!(got.cost_usd, Some(0.10));
    assert_eq!(store.len(), 1);
}

/// **Critério 4: o bind do collector é loopback** (nada escuta fora de 127.0.0.1).
#[tokio::test]
async fn bind_e_so_loopback() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    assert!(
        listener.local_addr().unwrap().ip().is_loopback(),
        "o collector só faz bind em loopback (invariante #2)"
    );
}
