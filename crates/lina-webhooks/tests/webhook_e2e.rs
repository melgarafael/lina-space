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
use lina_webhooks::{sign_hex, RateLimitConfig, WebhookEngine, WindowLimit};

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

/// W5-4-MED-a (budget do dono): requests com HMAC INVÁLIDO na MESMA rota NÃO podem consumir o
/// budget POR HOOK do dono. O `hook_id` viaja na URL (capability, não segredo); se o budget fosse
/// gasto antes do HMAC, o vazamento da rota viraria DoS do budget alheio só martelando corpo-lixo.
/// Prova: floda N > budget requests inválidos (todos 401) e, EM SEGUIDA, um request VÁLIDO ainda
/// passa (202) — o budget do dono permaneceu intacto.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hmac_invalido_nao_consome_budget_por_hook() {
    let tmp = unique_tmp("budget");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-budget", MockStore::new());

    // Budget POR HOOK pequeno (3) p/ o flood o estourar com folga; teto por IP largo (não é a
    // barreira aqui — todas as conexões de loopback compartilham o mesmo 127.0.0.1).
    let cfg = RateLimitConfig {
        ip_burst: WindowLimit {
            max_requests: 10_000,
            window: Duration::from_secs(60),
        },
        per_hook: WindowLimit {
            max_requests: 3,
            window: Duration::from_secs(60),
        },
    };
    let engine = WebhookEngine::with_config(store.clone(), sup.clone(), cfg);
    let hook = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;
    let path = format!("/hook/{}", hook.hook_id);
    let bad_sig = "00".repeat(32); // 32 bytes hex sintaticamente válidos, mas ERRADOS.

    // Flood de 8 inválidos (> budget 3). Cada um deve tomar 401 SEM tocar o budget por-hook.
    for i in 0..8 {
        let (status, _) = http_post(addr, &path, Some(&bad_sig), body).await;
        assert_eq!(
            status, 401,
            "flood #{i}: HMAC inválido → 401 (não pode consumir o budget do dono)"
        );
    }

    // O dono ainda tem budget: o request VÁLIDO passa.
    let good_sig = sign_hex(&hook.secret, body);
    let (status, _) = http_post(addr, &path, Some(&good_sig), body).await;
    assert_eq!(
        status, 202,
        "budget por-hook intacto: o request legítimo ainda passa após o flood de corpo-lixo"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// W5-4-MED-a (CPU): um flood BRUTO por IP é barrado ANTES de calcular o HMAC (camada 1 — protege a
/// CPU contra forçar o cálculo caro de HMAC em massa). Prova: com o teto por IP = 3, três requests
/// (mesmo sem assinatura → 401) esgotam a janela; o 4º, AINDA QUE traga um HMAC VÁLIDO, recebe 429
/// (não 202) — logo o gate de IP precede o HMAC — e nada é processado (nenhum WebhookReceived).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flood_bruto_por_ip_barra_antes_do_hmac() {
    let tmp = unique_tmp("burst");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-burst", MockStore::new());

    // Teto por IP pequeno (3); budget por-hook largo (não é a barreira deste teste).
    let cfg = RateLimitConfig {
        ip_burst: WindowLimit {
            max_requests: 3,
            window: Duration::from_secs(60),
        },
        per_hook: WindowLimit {
            max_requests: 10_000,
            window: Duration::from_secs(60),
        },
    };
    let engine = WebhookEngine::with_config(store.clone(), sup.clone(), cfg);
    let hook = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;
    let path = format!("/hook/{}", hook.hook_id);

    // 3 requests SEM assinatura → cada um passa o teto de IP (1,2,3) mas toma 401 (HMAC ausente).
    for i in 0..3 {
        let (status, _) = http_post(addr, &path, None, body).await;
        assert_eq!(
            status, 401,
            "request #{i} sem assinatura → 401 (mas conta no teto bruto por IP)"
        );
    }

    // 4º request COM HMAC VÁLIDO: o teto por IP já estourou → 429 ANTES do HMAC (senão seria 202).
    let good_sig = sign_hex(&hook.secret, body);
    let (status, _) = http_post(addr, &path, Some(&good_sig), body).await;
    assert_eq!(
        status, 429,
        "teto por IP estourado barra ANTES do HMAC — até um request válido recebe 429"
    );

    // E nada foi processado: o flood barrado não tocou o caminho de aceitação.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        count_webhook_received(&store),
        0,
        "flood barrado pelo teto de IP não processa nenhum disparo"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// W5-4-MED-b (durabilidade primeiro): no caminho feliz o `WebhookReceived` é apendado de forma
/// SÍNCRONA ANTES do `202` (invariante #4 — o log é a fonte da verdade; nada se perde num crash
/// entre o `202` e o append). Prova: assim que o cliente recebe o 202, o evento JÁ está no log —
/// SEM nenhuma espera. (Se o append fosse async, o log estaria vazio neste exato instante.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn append_e_sincrono_o_evento_esta_no_log_antes_do_202() {
    let tmp = unique_tmp("sync");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-sync", MockStore::new());

    let engine = WebhookEngine::new(store.clone(), sup.clone());
    let hook = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;
    let sig = sign_hex(&hook.secret, body);
    let path = format!("/hook/{}", hook.hook_id);

    let (status, _) = http_post(addr, &path, Some(&sig), body).await;
    assert_eq!(status, 202, "HMAC válido → 202");

    // SEM espera: o append é síncrono, então o fato já está durável no momento do 202.
    assert_eq!(
        count_webhook_received(&store),
        1,
        "WebhookReceived deve estar no log NO MOMENTO do 202 (append síncrono — durabilidade primeiro)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
