//! Teste de integração do nó-Gatilho (W5-4), espelhando o critério de aceite:
//!
//! `curl -X POST http://127.0.0.1:<porta>/hook/<hook_id> -H "X-Signature: <hmac>" -d '{...}'`
//!   → **202**, depois `WebhookReceived` aparece no event log E uma mensagem chega ao nó-alvo no
//!   bus; HMAC inválido → **401 sem publicar**.
//!
//! O cliente é um POST HTTP/1.1 cru por socket (como o `curl`), numa porta efêmera de `127.0.0.1`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use lina_core::{BusEvent, EventStore, Recipient, Supervisor};
use lina_secrets::{MockStore, SecretVault};
use lina_webhooks::{sign_hex, WebhookEngine};

/// Diretório temporário único (sem depender de uuid no crate de teste).
fn unique_tmp(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!("lina-wh-{tag}-{n}"));
    std::fs::create_dir_all(&p).expect("criar tempdir");
    p
}

/// Conta quantos `WebhookReceived` há no log (inspeção dos CAMPOS, não só presença).
fn count_webhook_received(store: &Arc<Mutex<EventStore>>) -> usize {
    let s = store.lock().unwrap_or_else(|p| p.into_inner());
    s.events()
        .map(|evs| evs.iter().filter(|e| e.kind == "WebhookReceived").count())
        .unwrap_or(0)
}

/// Espera (com timeout) o log conter `want` eventos `WebhookReceived` — o processamento é assíncrono.
async fn wait_for_webhook_received(store: &Arc<Mutex<EventStore>>, want: usize) -> bool {
    for _ in 0..60 {
        if count_webhook_received(store) >= want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// POST HTTP/1.1 cru pelo socket (espelha o `curl` do critério). Devolve (status_code, resposta).
async fn http_post(addr: SocketAddr, path: &str, sig: Option<&str>, body: &[u8]) -> (u16, String) {
    // Pequeno retry: o `axum::serve` pode estar subindo (o listener já está bound, então o kernel
    // aceita no backlog; o retry só cobre a janela inicial).
    let mut stream: Option<TcpStream> = None;
    for _ in 0..50 {
        match TcpStream::connect(addr).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    let mut stream = stream.expect("conectar ao servidor");

    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(s) = sig {
        req.push_str("X-Signature: ");
        req.push_str(s);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");

    stream
        .write_all(req.as_bytes())
        .await
        .expect("escrever head");
    stream.write_all(body).await.expect("escrever body");
    stream.flush().await.expect("flush");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("ler resposta");
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .expect("status line HTTP");
    (status, text)
}

/// Critério (caminho feliz): HMAC válido → 202 + `WebhookReceived` no log + `BusEvent::Message`
/// do nó-Gatilho (`from`) endereçada ao nó-alvo (`to`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_hmac_post_returns_202_logs_event_and_routes_to_target() {
    let tmp = unique_tmp("valid");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-valid", MockStore::new());

    let engine = WebhookEngine::new(store.clone(), sup.clone());
    let cfg = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    // Assina ANTES de firar para correlacionar a mensagem no bus.
    let mut rx = sup.subscribe();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;
    let sig = sign_hex(&cfg.secret, body);
    let path = format!("/hook/{}", cfg.hook_id);

    let (status, _resp) = http_post(addr, &path, Some(&sig), body).await;
    assert_eq!(status, 202, "HMAC válido deve responder 202");

    assert!(
        wait_for_webhook_received(&store, 1).await,
        "um WebhookReceived deve aparecer no event log"
    );

    // O bus deve receber uma mensagem do nó-Gatilho endereçada ao @Dev.
    let got = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(BusEvent::Message { from, to, .. }) => return Some((from, to)),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten();

    let (from, to) = got.expect("BusEvent::Message deve chegar ao bus");
    assert_eq!(from, trigger, "from da mensagem = nó-Gatilho");
    match to {
        Recipient::Node(id) => assert_eq!(id, dev, "mensagem endereçada ao @Dev"),
        other => panic!("esperava Recipient::Node(@Dev), veio {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Critério (rejeição): HMAC inválido → 401, SEM apendar `WebhookReceived` e SEM publicar no bus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_hmac_post_returns_401_without_publishing() {
    let tmp = unique_tmp("invalid");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-invalid", MockStore::new());

    let engine = WebhookEngine::new(store.clone(), sup.clone());
    let cfg = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    let mut rx = sup.subscribe();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(engine.serve(listener));

    let before = count_webhook_received(&store);
    let body = br#"{"order":42}"#;
    // Assinatura sintaticamente válida (32 bytes hex) mas ERRADA — não casa o HMAC do secret.
    let bad_sig = "00".repeat(32);
    let path = format!("/hook/{}", cfg.hook_id);

    let (status, _resp) = http_post(addr, &path, Some(&bad_sig), body).await;
    assert_eq!(status, 401, "HMAC inválido deve responder 401");

    // Dá tempo a um (indevido) processamento async — não deve haver nenhum.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        count_webhook_received(&store),
        before,
        "401 não pode apendar WebhookReceived"
    );
    assert!(
        rx.try_recv().is_err(),
        "401 não pode publicar mensagem no bus"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
