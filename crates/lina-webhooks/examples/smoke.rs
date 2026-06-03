//! Smoke do nó-Gatilho (W5-4) com `curl` REAL: sobe o engine numa porta efêmera de `127.0.0.1`,
//! configura um hook, dispara um `curl` assinado (→202) e um adulterado (→401), e inspeciona o
//! event log + o bus. Roda com:
//!
//! ```text
//! cargo run -p lina-webhooks --example smoke
//! ```

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lina_core::{BusEvent, EventStore, Supervisor};
use lina_secrets::{MockStore, SecretVault};
use lina_webhooks::{sign_hex, WebhookEngine};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let tmp = std::env::temp_dir().join(format!("lina-wh-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("tempdir");

    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/smoke", MockStore::new());

    let engine = WebhookEngine::new(store.clone(), sup.clone());
    let cfg = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    let mut rx = sup.subscribe();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let body = r#"{"order":42}"#;
    let sig = sign_hex(&cfg.secret, body.as_bytes());
    let url = format!("http://{addr}/hook/{}", cfg.hook_id);

    println!("== nó-Gatilho no ar em {addr} (bind 127.0.0.1) ==");
    println!("hook_id (base32, não-enumerável): {}", cfg.hook_id);
    println!("secret HMAC (no Secret Vault):    {}\n", cfg.secret);

    // --- 1) curl VÁLIDO → 202 ---
    println!("$ curl -X POST {url} -H \"X-Signature: {sig}\" -d '{body}'");
    let code_ok = curl_status(&url, Some(&sig), body);
    println!("  → HTTP {code_ok}  (esperado 202)\n");

    // O processamento é assíncrono; espera o WebhookReceived cair no log.
    let mut logged = false;
    for _ in 0..40 {
        if webhook_received_count(&store) >= 1 {
            logged = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("  event log → WebhookReceived presente: {logged}");
    if let Some(rec) = last_webhook_received(&store) {
        println!("  registro: kind={} payload={}", rec.0, rec.1);
    }
    let bussed = matches!(
        tokio::time::timeout(Duration::from_secs(2), rx.recv()).await,
        Ok(Ok(BusEvent::Message { .. }))
    );
    println!("  bus → BusEvent::Message ao alvo: {bussed}\n");

    // --- 2) curl INVÁLIDO → 401, sem publicar ---
    let before = webhook_received_count(&store);
    let bad = "00".repeat(32);
    println!("$ curl -X POST {url} -H \"X-Signature: {bad}\" -d '{body}'");
    let code_bad = curl_status(&url, Some(&bad), body);
    println!("  → HTTP {code_bad}  (esperado 401)");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = webhook_received_count(&store);
    println!(
        "  event log → WebhookReceived inalterado: {} (antes {before}, depois {after})\n",
        before == after
    );

    let ok = code_ok == "202" && logged && bussed && code_bad == "401" && before == after;
    println!("== SMOKE {} ==", if ok { "OK" } else { "FALHOU" });
    let _ = std::fs::remove_dir_all(&tmp);
    if !ok {
        std::process::exit(1);
    }
}

/// Dispara um `curl` real e devolve o código HTTP como string (ex.: "202").
fn curl_status(url: &str, sig: Option<&str>, body: &str) -> String {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-s",
        "-o",
        "/dev/null",
        "-w",
        "%{http_code}",
        "-X",
        "POST",
        url,
    ]);
    if let Some(s) = sig {
        cmd.args(["-H", &format!("X-Signature: {s}")]);
    }
    cmd.args(["-H", "Content-Type: application/json", "-d", body]);
    match cmd.output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(e) => format!("erro-curl: {e}"),
    }
}

fn webhook_received_count(store: &Arc<Mutex<EventStore>>) -> usize {
    let s = store.lock().unwrap_or_else(|p| p.into_inner());
    s.events()
        .map(|evs| evs.iter().filter(|e| e.kind == "WebhookReceived").count())
        .unwrap_or(0)
}

fn last_webhook_received(store: &Arc<Mutex<EventStore>>) -> Option<(String, String)> {
    let s = store.lock().unwrap_or_else(|p| p.into_inner());
    s.events().ok()?.into_iter().rev().find_map(|e| {
        (e.kind == "WebhookReceived").then(|| (e.kind.clone(), e.payload.to_string()))
    })
}
