//! **F4-1 · gates (b)/(d)/(e)/(f) — envio é AÇÃO EXTERNA: nada sai sem o "sim" humano (ADR 0004).**
//!
//! A assimetria-doutrina de F4-1: **ler é fluxo** (contexto, sem gate), **enviar é efeito externo
//! irreversível** (gate humano + custódia de segredo). Esta suíte prova a metade dura — o ENVIO — na
//! camada que JÁ existe: o broker (`run_custody`). O gate de envio é genérico e já está pronto
//! (F4-0-3); a onda F4-1 só liga o transporte Waha REAL dentro do `execute(&secret)`. Por isso a
//! propriedade de segurança "0 envios sem o sim" **não depende** do `channel_waha` (ainda stub):
//! ela vive na fronteira do broker, e é aqui que o red-team a trava.
//!
//! O "mock Waha que conta POSTs" do gate (b) é modelado pelo `execute` do broker — o ÚNICO ponto que
//! falaria com o Waha real. Se ele não roda, nenhum POST sai. Cada teste de 0-POSTs traz a prova por
//! MUTAÇÃO (chamar o executor direto = simular a guarda DESLIGADA → o contador morde → o 0 não é vácuo).
//!
//! ## Gatilho do happy-path e2e (deferido ao integrador — "QA entrega o alvo, o integrador liga")
//! O teste e2e que prova **exatamente 1 POST `/api/sendText` com o `X-Api-Key` no header** exige
//! `channel_waha::send` (F4-1-1, Terminal B, stub vazio hoje). Referenciá-lo agora QUEBRARIA a
//! compilação do alvo de teste (símbolo inexistente) — não é um `#[ignore]` limpo, é um pacote
//! VERMELHO. Quando `channel_waha::send` existir, o integrador troca o `execute` mock por ele contra
//! um mock HTTP e afirma o header + POST único. A propriedade INEGOCIÁVEL (0 sem o sim) está provada
//! aqui, no broker, hoje.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use lina_core::{run_custody, BrokerOutcome, BrokerRequest, DomainEvent, EventStore};
use lina_secrets::{MockStore, SecretVault};

/// Diretório temporário do event store; removido no `Drop`. `thread::id` no nome → isolado sob
/// execução paralela do `cargo test` (lição [[lina-gpui-testes-flaky-e-catraca-comentario]]).
struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-f41-send-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("criar tempdir");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Cofre com o `X-Api-Key` do WhatsApp no escopo `channel:whatsapp` (espelha `set_channel_credential`
/// de F40-CRED). A chave dentro do escopo é o tipo de credencial (`api_key`).
fn vault_with_whatsapp_key() -> (SecretVault<MockStore>, String) {
    let token = lina_secrets::generate_webhook_secret().expect("gera o X-Api-Key");
    let vault = SecretVault::with_store("lina-space/ws-f41", MockStore::new());
    vault
        .set_channel_credential("whatsapp", "api_key", &token)
        .expect("semear o X-Api-Key do canal");
    (vault, token)
}

/// O pedido de ENVIO de uma mensagem de WhatsApp via canal (`lina do whatsapp send`). O escopo do
/// segredo é derivado do nome do canal (`channel:whatsapp`) **server-side**, nunca ditado pelo agente.
fn send_req(requester: &str) -> BrokerRequest {
    BrokerRequest::for_channel("whatsapp", "send", "api_key", requester)
}

fn kinds(store: &EventStore) -> Vec<String> {
    store
        .events()
        .expect("ler eventos")
        .into_iter()
        .map(|r| r.kind)
        .collect()
}

/// O log inteiro serializado — para provar que NENHUM segredo vaza para evento algum.
fn log_dump(store: &EventStore) -> String {
    let events = store.events().expect("eventos");
    serde_json::to_string(&events.iter().map(|r| &r.payload).collect::<Vec<_>>())
        .expect("serializar log")
}

// ───────────────────────────── GATE (b) · 0 POSTs sem o "sim" humano ─────────────────────────────

/// **GATE (b):** propor envio SEM confirmação → **0 POSTs ao Waha**. O `execute` (o único ponto que
/// falaria com o Waha) NUNCA roda; o cofre nem é consultado; só `ActionGated{ask}` + `BrokerDenied`.
#[test]
fn send_without_human_yes_makes_zero_waha_posts() {
    let tmp = TempDir::new("no-yes");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    let (vault, _token) = vault_with_whatsapp_key();
    let posts = Cell::new(0u32); // o "mock Waha": conta POSTs /api/sendText.

    let outcome = run_custody(&send_req("@Agente"), false, &vault, &mut store, |_secret| {
        posts.set(posts.get() + 1); // só rodaria se houvesse o "sim".
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
    assert_eq!(
        posts.get(),
        0,
        "NENHUM POST ao Waha pode sair sem o sim humano"
    );

    let events = store.events().expect("eventos");
    let gated = events
        .iter()
        .find(|r| r.kind == "ActionGated")
        .expect("ActionGated");
    assert_eq!(gated.payload["class"], "gated-hard-external");
    assert_eq!(gated.payload["decision"], "ask");
    assert!(
        events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "unconfirmed"),
        "envio sem o sim deve apendar BrokerDenied{{unconfirmed}}"
    );
    assert!(
        !events.iter().any(|r| r.kind == "BrokerExecuted"),
        "NUNCA pode haver BrokerExecuted (=envio) sem confirmação"
    );
}

/// COM o "sim" + segredo no escopo do canal → **exatamente 1 POST**, e o segredo passado ao transporte
/// vem DO COFRE (`channel:whatsapp`), nunca do agente — e jamais vaza para o log (invariante #2). É o
/// gate (b) pelo lado positivo: o envio só ocorre depois do gate humano, com a credencial custodiada.
#[test]
fn send_with_yes_and_secret_makes_exactly_one_waha_post() {
    let tmp = TempDir::new("yes-secret");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    let (vault, token) = vault_with_whatsapp_key();
    let posts = Cell::new(0u32);
    let seen: Cell<Option<String>> = Cell::new(None);

    let outcome = run_custody(&send_req("@Agente"), true, &vault, &mut store, |secret| {
        posts.set(posts.get() + 1);
        seen.set(Some(secret.to_string()));
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(outcome, BrokerOutcome::Executed);
    assert_eq!(posts.get(), 1, "exatamente 1 POST ao Waha após o sim");
    assert_eq!(
        seen.take().as_deref(),
        Some(token.as_str()),
        "o X-Api-Key passado ao transporte vem do escopo channel:whatsapp do cofre, não do agente"
    );

    assert!(store
        .events()
        .expect("eventos")
        .iter()
        .any(|r| r.kind == "BrokerExecuted" && r.payload["action"] == "send"));
    assert!(
        !log_dump(&store).contains(token.as_str()),
        "o X-Api-Key NÃO pode aparecer em nenhum evento do log"
    );
}

/// **GATE (b) — piso de custódia (mutação real):** mesmo COM o "sim", se o segredo NÃO está no escopo
/// `channel:whatsapp` do cofre, não há caminho de envio → **0 POSTs** (`DeniedNoSecret`). Sem
/// credencial guardada, o canal não dispara efeito externo — o "sim" sozinho não fabrica o segredo.
#[test]
fn send_with_yes_but_no_secret_makes_zero_posts() {
    let tmp = TempDir::new("yes-nosecret");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    // Cofre com um token de OUTRO canal — prova que o lookup é por escopo, não "qualquer segredo serve".
    let vault = SecretVault::with_store("lina-space/ws-f41", MockStore::new());
    vault
        .set_channel_credential("telegram", "api_key", "nao-e-deste-canal")
        .expect("semear outro escopo");
    let posts = Cell::new(0u32);

    let outcome = run_custody(&send_req("@Agente"), true, &vault, &mut store, |_secret| {
        posts.set(posts.get() + 1);
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(outcome, BrokerOutcome::DeniedNoSecret);
    assert_eq!(
        posts.get(),
        0,
        "sem o X-Api-Key no escopo do canal, 0 POSTs mesmo com o sim"
    );
    assert!(kinds(&store).iter().any(|k| k == "BrokerDenied"));
    assert!(!kinds(&store).iter().any(|k| k == "BrokerExecuted"));
}

/// **Prova por MUTAÇÃO (padrão-ouro): o contador de POSTs NÃO é vácuo.** Se a guarda fosse desligada
/// (i.e., o executor rodasse sem passar pelo gate), o "mock Waha" PEGARIA o POST. Chamar o executor
/// direto sobe o contador — provando que os testes de 0-POSTs acima de fato observariam um envio.
#[test]
fn mutation_proves_the_post_counter_would_catch_a_send() {
    let posts = Cell::new(0u32);
    let execute = |_secret: &str| -> Result<(), String> {
        posts.set(posts.get() + 1);
        Ok(())
    };
    // Caminho-bypass simulado (a guarda "desligada"): o transporte é chamado diretamente.
    execute("X-Api-Key-qualquer").expect("executor");
    assert_eq!(
        posts.get(),
        1,
        "o mock Waha conta POSTs — um envio real seria observado (o 0 dos testes acima não é vácuo)"
    );
}

// ───────────────── GATES (d)/(e) · campos de payload NUNCA decidem autoridade (mutação) ──────────

/// **GATE (e):** `approved_by`/`broker_exec_ref` de um `ChannelMessageSent` são rótulos de
/// PROVENIÊNCIA, JAMAIS autoridade. Forjar o evento de envio DIRETO no log (sem passar pelo broker) é
/// inerte: o único produtor de um envio REAL é `run_custody` sob `confirmed=true` + segredo. O campo de
/// payload não dispara POST algum — espelha `broker::forged_approved_by_in_log_executes_nothing`.
#[test]
fn forged_channel_message_sent_in_log_sends_nothing() {
    let tmp = TempDir::new("forged-sent");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    let (vault, _token) = vault_with_whatsapp_key();

    // O agente forja o evento de "enviado" escrevendo o campo direto no log:
    store
        .append(&DomainEvent::ChannelMessageSent {
            channel: "whatsapp".to_string(),
            conversation_ref: "grupo:Vendas".to_string(),
            broker_exec_ref: "forjado-pelo-agente".to_string(),
            approved_by: "@Fundador".to_string(), // proveniência forjada — NÃO é autoridade.
        })
        .expect("append do evento forjado");

    // Nenhum envio nasce de um campo de payload: não há BrokerExecuted, e o caminho real (run_custody)
    // SEM o "sim" segue negando — o evento forjado não substitui o gate.
    assert!(
        !kinds(&store).iter().any(|k| k == "BrokerExecuted"),
        "forjar ChannelMessageSent NÃO produz envio — só o gate via run_custody o produz"
    );
    let posts = Cell::new(0u32);
    let outcome = run_custody(&send_req("@Fundador"), false, &vault, &mut store, |_| {
        posts.set(posts.get() + 1);
        Ok(())
    })
    .expect("run_custody");
    assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
    assert_eq!(posts.get(), 0, "o evento forjado não abre caminho de envio");
}

/// **GATES (d)+(e):** um `ChannelConnected{session_ref}` forjado no log NÃO concede envio. `session_ref`
/// é REFERÊNCIA custodiada, jamais autoridade: mesmo "conectado" (forjado) e COM o "sim", sem o
/// `X-Api-Key` no cofre não há POST (`DeniedNoSecret`). A autoridade do envio é a custódia (segredo +
/// gate), nunca o estado de sessão que um campo de log afirma. É o substrato de (d) "desconectar zera o
/// cano": se nem um connected forjado abre o envio, o estado real de conexão também não é a autoridade.
#[test]
fn forged_connected_state_does_not_grant_send_without_secret() {
    let tmp = TempDir::new("forged-connected");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    // Cofre VAZIO no escopo do canal (a conexão "existe" no log, mas o segredo não).
    let vault = SecretVault::with_store("lina-space/ws-f41", MockStore::new());

    store
        .append(&DomainEvent::ChannelConnected {
            channel: "whatsapp".to_string(),
            session_ref: "sessao-FORJADA-pelo-agente".to_string(),
            scope: "waha".to_string(),
        })
        .expect("append do connected forjado");

    let posts = Cell::new(0u32);
    let outcome = run_custody(&send_req("@Agente"), true, &vault, &mut store, |_| {
        posts.set(posts.get() + 1);
        Ok(())
    })
    .expect("run_custody");

    assert_eq!(
        outcome,
        BrokerOutcome::DeniedNoSecret,
        "session_ref forjado não é autoridade — sem segredo no cofre, 0 envio mesmo com o sim"
    );
    assert_eq!(posts.get(), 0);
}

/// **GATE (e):** o `requester` (identidade AUTO-DECLARADA do pedido, A3 pendente) nunca decide
/// autoridade. Um agente que se diz `@Fundador` no pedido continua barrado sem o "sim" — 0 POSTs.
#[test]
fn self_declared_requester_is_not_authority() {
    let tmp = TempDir::new("requester");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    let (vault, _token) = vault_with_whatsapp_key();
    let posts = Cell::new(0u32);

    // Identidade forjada no campo de pedido — não há caminho de auto-aprovação por identidade.
    let outcome = run_custody(
        &send_req("@Fundador-mentira"),
        false,
        &vault,
        &mut store,
        |_| {
            posts.set(posts.get() + 1);
            Ok(())
        },
    )
    .expect("run_custody");

    assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
    assert_eq!(
        posts.get(),
        0,
        "o requester auto-declarado não fabrica autoridade — o gate humano confirma SEMPRE"
    );
}
