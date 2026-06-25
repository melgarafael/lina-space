//! **F4-1 · gate (c) + doutrina de auditoria — ler é FLUXO, mas escopado e sem vazar conteúdo.**
//!
//! A outra metade da assimetria de F4-1: **ler é fluxo** (contexto de entrada, sem efeito externo →
//! sem gate), **mas** default-deny por grupo + auditado + **conteúdo NUNCA no log** (só metadados).
//! Esta suíte trava as duas propriedades na camada que JÁ existe:
//!
//! - **(c) default-deny de grupos:** a decisão de produção é `ToolScopeSet::is_declared` (F4-0-4) — o
//!   substrato que o wrapper `channel_read::pode_ler` (Terminal K, stub hoje) vai consumir. Um grupo
//!   NÃO declarado é invisível → leitura recusada. Provado aqui pela projeção REAL (não por mock).
//! - **doutrina de log (D2/D3):** os 4 eventos F4-1 carregam só REFERÊNCIAS e CONTAGEM — `session_ref`
//!   nunca é o token; `conversation_ref`/`count` nunca trazem o texto da mensagem. Re-derivado no
//!   red-team por inspeção do payload serializado (espelha `..._carry_no_content_or_secret` da largada).
//!
//! ## Gatilho do happy-path e2e (deferido ao integrador — "QA entrega o alvo, o integrador liga")
//! O teste do wrapper `channel_read::pode_ler(scope_projetado, conversation_ref) -> bool` + "leitura de
//! grupo declarado retorna conteúdo + apenda ChannelMessageRead (sem texto)" exige `channel_read`
//! preenchido (Terminal K, stub vazio hoje). Referenciá-lo agora QUEBRARIA a compilação do alvo
//! (símbolo inexistente). A propriedade de SEGURANÇA de (c) — não-declarado ⇒ recusa — está provada
//! aqui, na projeção `ToolScopeSet` que o `pode_ler` consome; o integrador adiciona o teste do wrapper
//! (que é um adaptador fino sobre `is_declared`) quando a frente ligar.

use std::path::{Path, PathBuf};

use lina_core::tool_scope::{declare_tool_scope, revoke_tool_scope, ToolScopeSet};
use lina_core::{DomainEvent, EventStore};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-f41-read-{tag}-{}-{:?}",
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

// ───────────────────────────── GATE (c) · default-deny de grupos (projeção REAL) ────────────────

/// **GATE (c):** um grupo de WhatsApp NÃO declarado é INVISÍVEL à IA — a decisão real
/// (`ToolScopeSet::is_declared`, que o `pode_ler` consome) o nega. Só o grupo declarado pelo leigo
/// (`grupo:Vendas`) é legível; o não-declarado (`grupo:Diretoria`) → recusa. Provado pela projeção de
/// produção a partir do log real (emit → replay), não por um mock da decisão.
#[test]
fn undeclared_whatsapp_group_is_denied_by_real_tool_scope() {
    let tmp = TempDir::new("default-deny");
    let mut store = EventStore::open(tmp.path()).expect("open store");

    // O leigo declara SÓ o grupo de Vendas para o projeto "loja".
    declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
    let scope = ToolScopeSet::replay(&store).expect("replay");

    // Declarado → legível (a leitura é permitida).
    assert!(
        scope.is_declared("loja", "whatsapp", "grupo:Vendas"),
        "grupo declarado é legível como contexto"
    );
    // NÃO declarado → recusa (a leitura é negada antes de tocar o transporte).
    assert!(
        !scope.is_declared("loja", "whatsapp", "grupo:Diretoria"),
        "grupo NÃO declarado é invisível à IA (default-deny) — ler é recusado"
    );
    // E o gating é por projeto: outro projeto não enxerga a declaração de "loja".
    assert!(
        !scope.is_declared("outra-loja", "whatsapp", "grupo:Vendas"),
        "a declaração não vaza entre projetos"
    );
}

/// **GATE (c) — o cano de leitura zera por evento:** revogar o grupo o torna invisível no próximo
/// replay (sem restart). É a contrapartida de "desconectar zera o cano", no eixo da leitura: tirar a
/// declaração corta o acesso de contexto imediatamente.
#[test]
fn revoking_group_makes_it_invisible_next_turn() {
    let tmp = TempDir::new("revoke");
    let mut store = EventStore::open(tmp.path()).expect("open store");

    declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
    assert!(ToolScopeSet::replay(&store).expect("replay").is_declared(
        "loja",
        "whatsapp",
        "grupo:Vendas"
    ));

    revoke_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("revoga");
    assert!(
        !ToolScopeSet::replay(&store)
            .expect("replay")
            .is_declared("loja", "whatsapp", "grupo:Vendas"),
        "grupo revogado some da projeção no próximo turno (sem restart) — leitura volta a ser negada"
    );
}

// ───────────────── doutrina D2/D3 · conteúdo e token FORA do log (red-team dos eventos) ──────────

/// Serializa o payload do evento COMO no log (a tag interna `event` precisa estar presente para o
/// fold determinístico decodificar). Devolve `(kind, json_do_payload)`.
fn logged(event: &DomainEvent) -> (String, String) {
    let tmp = TempDir::new("log-roundtrip");
    let mut store = EventStore::open(tmp.path()).expect("open store");
    store.append(event).expect("append");
    let rec = store
        .events()
        .expect("eventos")
        .into_iter()
        .next()
        .expect("um evento");
    (
        rec.kind,
        serde_json::to_string(&rec.payload).expect("serializa payload"),
    )
}

/// **D2 — `session_ref` é REFERÊNCIA, jamais o token:** `ChannelConnected`/`ChannelDisconnected`
/// carregam a conta/referência da sessão custodiada, nunca o `X-Api-Key`. Um token-agulha NÃO aparece
/// no payload (porque ele nunca entra no evento — vive só no cofre).
#[test]
fn connected_events_carry_session_reference_never_the_token() {
    let token_needle = "WAHA-TOKEN-SECRETO-NUNCA-NO-LOG";

    let (kind, payload) = logged(&DomainEvent::ChannelConnected {
        channel: "whatsapp".to_string(),
        session_ref: "conta:whatsapp/principal".to_string(), // REFERÊNCIA, não o token.
        scope: "waha".to_string(),
    });
    assert_eq!(kind, "ChannelConnected");
    assert!(
        payload.contains("conta:whatsapp/principal"),
        "a referência da sessão está no log (auditável): {payload}"
    );
    assert!(
        !payload.contains(token_needle),
        "o token de sessão JAMAIS pode aparecer no evento: {payload}"
    );

    let (kind, payload) = logged(&DomainEvent::ChannelDisconnected {
        channel: "whatsapp".to_string(),
        session_ref: "conta:whatsapp/principal".to_string(),
    });
    assert_eq!(kind, "ChannelDisconnected");
    assert!(!payload.contains(token_needle));
}

/// **D2/D3 — leitura e envio auditam METADADOS, nunca o conteúdo:** `ChannelMessageRead` carrega
/// `conversation_ref`/`count`/`read_by_node` e `ChannelMessageSent` carrega `conversation_ref`/refs —
/// **o TEXTO da mensagem nunca entra no log** (não há sequer um campo para ele). Provado mostrando que
/// um conteúdo-agulha está ausente do payload, e que o metadado esperado (a contagem) está presente.
#[test]
fn read_and_sent_events_log_only_metadata_never_message_content() {
    let content_needle = "texto-confidencial-da-conversa-do-cliente";

    let (kind, payload) = logged(&DomainEvent::ChannelMessageRead {
        channel: "whatsapp".to_string(),
        conversation_ref: "grupo:Vendas".to_string(),
        count: 12,
        read_by_node: "@Agente".to_string(),
    });
    assert_eq!(kind, "ChannelMessageRead");
    assert!(
        payload.contains("grupo:Vendas") && payload.contains("12"),
        "metadados (ref do grupo + contagem) estão no log: {payload}"
    );
    assert!(
        !payload.contains(content_needle),
        "o TEXTO das mensagens lidas JAMAIS pode aparecer no log: {payload}"
    );

    let (kind, payload) = logged(&DomainEvent::ChannelMessageSent {
        channel: "whatsapp".to_string(),
        conversation_ref: "grupo:Vendas".to_string(),
        broker_exec_ref: "exec:abc123".to_string(),
        approved_by: "@Fundador".to_string(),
    });
    assert_eq!(kind, "ChannelMessageSent");
    assert!(
        payload.contains("exec:abc123"),
        "a correlação com o BrokerExecuted (prova de que passou pelo gate) está no log: {payload}"
    );
    assert!(
        !payload.contains(content_needle),
        "o TEXTO da mensagem enviada JAMAIS pode aparecer no log: {payload}"
    );
}
