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

    // Budget POR HOOK pequeno (3) p/ o flood o estourar com folga; camadas pré-auth largas (não
    // são a barreira deste teste — aqui se prova só o budget pós-HMAC do dono).
    let cfg = RateLimitConfig {
        pre_auth_per_route: WindowLimit {
            max_requests: 10_000,
            window: Duration::from_secs(60),
        },
        pre_auth_global: WindowLimit {
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

/// W5-4-MED-a (CPU) reencarnado pós-F1-6-4: o backstop GLOBAL pré-auth barra ANTES de calcular o
/// HMAC. Prova: com o teto global = 3 (e balde por rota largo), três requests sem assinatura
/// esgotam a janela global; o 4º, AINDA QUE traga um HMAC VÁLIDO, recebe 429 (não 202) — logo o
/// gate global precede o HMAC — e nada é processado (nenhum `WebhookReceived`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn teto_global_pre_auth_barra_antes_do_hmac() {
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

    // Teto GLOBAL pequeno (3); demais camadas largas (não são a barreira deste teste).
    let cfg = RateLimitConfig {
        pre_auth_per_route: WindowLimit {
            max_requests: 10_000,
            window: Duration::from_secs(60),
        },
        pre_auth_global: WindowLimit {
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

    // 3 requests SEM assinatura → passam o balde de rota e o global (1,2,3), mas tomam 401.
    for i in 0..3 {
        let (status, _) = http_post(addr, &path, None, body).await;
        assert_eq!(
            status, 401,
            "request #{i} sem assinatura → 401 (mas conta no teto global pré-auth)"
        );
    }

    // 4º request COM HMAC VÁLIDO: o teto global já estourou → 429 ANTES do HMAC (senão seria 202).
    let good_sig = sign_hex(&hook.secret, body);
    let (status, _) = http_post(addr, &path, Some(&good_sig), body).await;
    assert_eq!(
        status, 429,
        "teto global estourado barra ANTES do HMAC — até um request válido recebe 429"
    );

    // E nada foi processado: o flood barrado não tocou o caminho de aceitação.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        count_webhook_received(&store),
        0,
        "flood barrado pelo teto global não processa nenhum disparo"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// F1-6-4 / MEDIA-1, critério (a) — FIM DA STARVATION: flood de corpo-lixo sem secret, tanto em
/// rota INEXISTENTE quanto na rota EXISTENTE do hook A sem HMAC, **não** derruba para 429 um hook
/// legítimo CONCORRENTE (B). No design antigo (teto por IP — efetivamente GLOBAL em loopback e
/// atrás de túnel), este mesmo flood esgotava o orçamento compartilhado e B tomava 429 (auto-DoS).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flood_de_lixo_nao_derruba_hook_legitimo_concorrente() {
    let tmp = unique_tmp("starv");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-starv", MockStore::new());

    // Balde por rota apertado (5) p/ o flood estourar rápido; global largo o bastante p/ provar
    // que NÃO é ele quem segura a linha (o lixo barrado na rota nem o consome).
    let cfg = RateLimitConfig {
        pre_auth_per_route: WindowLimit {
            max_requests: 5,
            window: Duration::from_secs(60),
        },
        pre_auth_global: WindowLimit {
            max_requests: 10_000,
            window: Duration::from_secs(60),
        },
        per_hook: WindowLimit {
            max_requests: 120,
            window: Duration::from_secs(60),
        },
    };
    let engine = WebhookEngine::with_config(store.clone(), sup.clone(), cfg);
    let hook_a = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook A");
    let hook_b = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook B");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;

    // Flood 1: 20 lixos numa rota INEXISTENTE (32 chars, shape de hook_id) — só esgota o balde DELA.
    let rota_falsa = "/hook/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    for _ in 0..20 {
        let (status, _) = http_post(addr, rota_falsa, None, body).await;
        assert!(
            status == 404 || status == 429,
            "lixo em rota inexistente → 404 (dentro do balde) ou 429 (balde estourado); veio {status}"
        );
    }

    // Flood 2: 20 lixos SEM assinatura na rota EXISTENTE de A — só esgota o balde de A.
    let path_a = format!("/hook/{}", hook_a.hook_id);
    for _ in 0..20 {
        let (status, _) = http_post(addr, &path_a, None, body).await;
        assert!(
            status == 401 || status == 429,
            "lixo na rota de A → 401 (dentro do balde) ou 429 (balde de A estourado); veio {status}"
        );
    }

    // O CRITÉRIO: o hook B, legítimo e concorrente, passa ileso — 202.
    let path_b = format!("/hook/{}", hook_b.hook_id);
    let sig_b = sign_hex(&hook_b.secret, body);
    let (status, _) = http_post(addr, &path_b, Some(&sig_b), body).await;
    assert_eq!(
        status, 202,
        "40 requests de corpo-lixo NÃO podem derrubar o hook legítimo concorrente (MEDIA-1 fechado)"
    );

    // Residual DOCUMENTADO (e travado aqui): quem CONHECE a rota de A nega o próprio A na janela
    // corrente (balde pré-auth de A esgotado) — mas o budget pós-HMAC do dono segue intacto e
    // nenhuma outra rota é afetada. Mitigação: janela vira / rotacionar o hook.
    let sig_a = sign_hex(&hook_a.secret, body);
    let (status, _) = http_post(addr, &path_a, Some(&sig_a), body).await;
    assert_eq!(
        status, 429,
        "residual conhecido: o balde pré-auth da PRÓPRIA rota floodada fica esgotado na janela"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// F1-6-4 critério (b) — SEM ORÁCULO: o orçamento de lixo não revela existência de `hook_id`. O
/// onset do 429 pré-auth é IDÊNTICO para rota existente e inexistente (mesma janela, mesmo
/// limite, mesmo caminho O(1)). A diferença 404×401 dentro do balde é o contrato pré-existente
/// da capability (160 bits não-enumeráveis — provados em `hook_id_e_opaco_…`), não um sinal novo
/// introduzido pelo rate-limit.
///
/// Sobre a dimensão TEMPO do critério: medir timing em CI é flaky por natureza, então a prova
/// aqui é ESTRUTURAL + comportamental — o rate-limit novo usa o MESMO lookup O(1) no MESMO mapa
/// para qualquer chave (ver `allow_route_window`), não acrescentando NENHUM ramo dependente de
/// existência. O diferencial de tempo que sobra (HMAC só roda em rota existente com assinatura
/// presente) é o contrato 404/401 pré-existente do W5-4 — e atravessá-lo por enumeração exigiria
/// varrer um espaço de 2^160 capabilities, inviável com ou sem oráculo de timing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orcamento_de_lixo_nao_revela_existencia_de_hook() {
    let tmp = unique_tmp("oracle");
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let vault = SecretVault::with_store("lina-space/test-oracle", MockStore::new());

    let cfg = RateLimitConfig {
        pre_auth_per_route: WindowLimit {
            max_requests: 3,
            window: Duration::from_secs(60),
        },
        pre_auth_global: WindowLimit {
            max_requests: 10_000,
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
    let serie = |path: String| async move {
        let mut out = Vec::new();
        for _ in 0..6 {
            let (status, _) = http_post(addr, &path, None, body).await;
            out.push(status);
        }
        out
    };

    let existente = serie(format!("/hook/{}", hook.hook_id)).await;
    let inexistente = serie("/hook/BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string()).await;

    assert_eq!(
        existente,
        vec![401, 401, 401, 429, 429, 429],
        "rota existente: 401 dentro do balde (HMAC ausente), 429 a partir do 4º"
    );
    assert_eq!(
        inexistente,
        vec![404, 404, 404, 429, 429, 429],
        "rota inexistente: 404 dentro do balde, 429 a partir do 4º"
    );
    let onset = |s: &[u16]| s.iter().position(|&c| c == 429);
    assert_eq!(
        onset(&existente),
        onset(&inexistente),
        "onset do 429 IDÊNTICO — o orçamento de lixo não discrimina existência (sem oráculo)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// F1-6-1 / MEDIA-5 (replay no boot): configura hook → derruba e re-sobe o engine →
/// (1) SEM replay, HMAC válido toma **404** (o gap que a story fecha — prova não-vacuosa);
/// (2) COM [`WebhookEngine::replay_configured`], o MESMO `curl` toma **202 sem reconfigurar**;
/// (3) HMAC inválido pós-replay → **401 sem publicar** (o replay não afrouxou nada).
/// O vault (keyring) sobrevive ao restart na vida real — aqui o MESMO `vault` atravessa as vidas.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_no_boot_religa_hook_apos_restart() {
    let tmp = unique_tmp("replay");
    let vault = SecretVault::with_store("lina-space/test-replay", MockStore::new());

    // ── Vida 1: configura o hook e "morre" (drop). O fato fica no log; o secret, no vault.
    let store1 = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup1 = Arc::new(Supervisor::new());
    let trigger1 = sup1.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let engine1 = WebhookEngine::new(store1.clone(), sup1);
    let cfg = engine1
        .configure_hook(&vault, trigger1, "@Dev")
        .expect("configurar hook");
    drop(engine1);
    drop(store1);

    let body = br#"{"order":42}"#;
    let sig = sign_hex(&cfg.secret, body);
    let path = format!("/hook/{}", cfg.hook_id);

    // ── Vida 2 SEM replay: o gap de hoje — o hook "sumiu" (404) mesmo com HMAC válido.
    {
        let store2 = Arc::new(Mutex::new(EventStore::open(&tmp).expect("reabrir store")));
        let sup2 = Arc::new(Supervisor::new());
        let _trigger2 = sup2.register(
            "@Trigger",
            Some("trigger".into()),
            Box::new(std::io::sink()),
        );
        let engine2 = WebhookEngine::new(store2, sup2);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(engine2.serve(listener));

        let (status, _) = http_post(addr, &path, Some(&sig), body).await;
        assert_eq!(
            status, 404,
            "sem replay o hook some após restart — é o MEDIA-5 que a story fecha (prova não-vacuosa)"
        );
    }

    // ── Vida 3 COM replay: o MESMO curl agora toma 202, sem reconfigurar nada.
    let store3 = Arc::new(Mutex::new(EventStore::open(&tmp).expect("reabrir store")));
    let sup3 = Arc::new(Supervisor::new());
    let trigger3 = sup3.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let dev3 = sup3.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));
    let engine3 = WebhookEngine::new(store3.clone(), sup3.clone());

    let summary = engine3
        .replay_configured(&vault, trigger3)
        .expect("replay de boot");
    assert_eq!(summary.rebound, 1, "1 hook religado a partir do log");
    assert_eq!(summary.skipped_no_secret, 0);

    let mut rx = sup3.subscribe();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine3.serve(listener));

    let before = count_webhook_received(&store3);
    let (status, _) = http_post(addr, &path, Some(&sig), body).await;
    assert_eq!(
        status, 202,
        "pós-replay o HMAC válido volta a 202 SEM reconfigurar (critério a da F1-6-1)"
    );
    assert!(
        wait_for_webhook_received(&store3, before + 1).await,
        "o disparo pós-replay vira WebhookReceived no log"
    );

    // O efeito no bus segue vivo: mensagem do nó-Gatilho chega endereçada ao alvo religado.
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
    let (from, to) = got.expect("BusEvent::Message deve chegar ao bus pós-replay");
    assert_eq!(from, trigger3, "from = nó-Gatilho DESTE boot");
    match to {
        Recipient::Node(id) => assert_eq!(id, dev3, "target_ref religado resolve no @Dev vivo"),
        other => panic!("esperava Recipient::Node(@Dev), veio {other:?}"),
    }

    // (b) HMAC inválido pós-replay → 401 sem publicar (o replay não afrouxou o gate).
    let count_before_bad = count_webhook_received(&store3);
    let bad_sig = "00".repeat(32);
    let (status, _) = http_post(addr, &path, Some(&bad_sig), body).await;
    assert_eq!(status, 401, "HMAC inválido pós-replay → 401 (não-vacuoso)");
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        count_webhook_received(&store3),
        count_before_bad,
        "401 pós-replay não pode apendar WebhookReceived"
    );
    assert!(
        rx.try_recv().is_err(),
        "401 pós-replay não pode publicar mensagem no bus (\"sem publicar\" literal)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// F1-6-1 (perda de secret sinalizada): se o secret sumiu do vault (keyring zerado), o replay
/// NÃO religa o hook (sem secret não há verificação HMAC) e NÃO recria secret em silêncio —
/// recriar mascararia a perda e quebraria a assinatura do integrador. O hook responde 404 e o
/// [`lina_webhooks::ReplaySummary`] conta o pulo.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_sem_secret_no_vault_nao_religa_nem_recria() {
    let tmp = unique_tmp("replay-nosecret");

    // Vida 1 configura com um vault que MORRE junto (secret perdido — keyring zerado).
    let cfg = {
        let vault_perdido = SecretVault::with_store("lina-space/test-replay-ns1", MockStore::new());
        let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
        let sup = Arc::new(Supervisor::new());
        let trigger = sup.register(
            "@Trigger",
            Some("trigger".into()),
            Box::new(std::io::sink()),
        );
        let engine = WebhookEngine::new(store, sup);
        engine
            .configure_hook(&vault_perdido, trigger, "@Dev")
            .expect("configurar hook")
    };

    // Vida 2: vault NOVO (vazio). O replay deve pular o hook — e jamais inventar secret novo.
    let vault_novo = SecretVault::with_store("lina-space/test-replay-ns2", MockStore::new());
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("reabrir store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let engine = WebhookEngine::new(store, sup);
    let summary = engine
        .replay_configured(&vault_novo, trigger)
        .expect("replay de boot");
    assert_eq!(summary.rebound, 0, "sem secret não há religação");
    assert_eq!(
        summary.skipped_no_secret, 1,
        "a perda é CONTADA (sinalizada), nunca silenciosa"
    );
    assert!(
        vault_novo
            .get("webhook", &cfg.hook_id)
            .expect("consultar vault")
            .is_none(),
        "o replay NÃO pode recriar secret em silêncio no vault"
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;
    let sig = sign_hex(&cfg.secret, body);
    let (status, _) = http_post(addr, &format!("/hook/{}", cfg.hook_id), Some(&sig), body).await;
    assert_eq!(
        status, 404,
        "hook com secret perdido NÃO renasce — responde 404 até reconfiguração explícita"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// F1-6-1 critério (c): nenhum secret em claro nos ARTEFATOS do log (SQLite + espelho
/// `log.jsonl` + WAL). Configura E dispara (cobre `WebhookConfigured` e `WebhookReceived`) e
/// então varre os bytes de TODO arquivo do diretório do store atrás do secret.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nenhum_secret_em_claro_nos_artefatos_do_log() {
    fn arquivos(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    arquivos(&p, out);
                } else {
                    out.push(p);
                }
            }
        }
    }
    fn contem(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
    }

    let tmp = unique_tmp("scan");
    let vault = SecretVault::with_store("lina-space/test-scan", MockStore::new());
    let store = Arc::new(Mutex::new(EventStore::open(&tmp).expect("open store")));
    let sup = Arc::new(Supervisor::new());
    let trigger = sup.register(
        "@Trigger",
        Some("trigger".into()),
        Box::new(std::io::sink()),
    );
    let _dev = sup.register("@Dev", Some("dev".into()), Box::new(std::io::sink()));

    let engine = WebhookEngine::new(store.clone(), sup);
    let cfg = engine
        .configure_hook(&vault, trigger, "@Dev")
        .expect("configurar hook");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(engine.serve(listener));

    let body = br#"{"order":42}"#;
    let sig = sign_hex(&cfg.secret, body);
    let (status, _) = http_post(addr, &format!("/hook/{}", cfg.hook_id), Some(&sig), body).await;
    assert_eq!(status, 202);
    assert!(wait_for_webhook_received(&store, 1).await);

    // Varredura nos ARTEFATOS: o secret (hex, 256 bits) não pode aparecer em byte algum.
    let mut paths = Vec::new();
    arquivos(&tmp, &mut paths);
    assert!(
        paths
            .iter()
            .any(|p| p.file_name().is_some_and(|n| n == "log.jsonl")),
        "o espelho log.jsonl deve existir — sem ele este scan seria vácuo (nome mudou?)"
    );
    for p in &paths {
        let bytes = std::fs::read(p).unwrap_or_default();
        assert!(
            !contem(&bytes, cfg.secret.as_bytes()),
            "secret em claro encontrado no artefato {} — invariante 'nada em claro no disco' violada",
            p.display()
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

/// W5-4-MED-b (durabilidade primeiro): no caminho feliz o `WebhookReceived` é apendado — e
/// AGUARDADO (a semântica síncrona-pré-202; desde F1-6-2 o disco roda em `spawn_blocking`, mas o
/// handler espera o resultado) — ANTES do `202` (invariante #4 — o log é a fonte da verdade; nada
/// se perde num crash entre o `202` e o append). Prova: assim que o cliente recebe o 202, o
/// evento JÁ está no log — SEM nenhuma espera. (Se o append fosse fire-and-forget, o log poderia
/// estar vazio neste exato instante.)
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
