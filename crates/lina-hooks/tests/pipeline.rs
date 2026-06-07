//! Gate F1-1-3 (headless) — pipeline de hooks em tempo real:
//! - critério 1 (par PreToolUse/PostToolUse → timeline "Running: Bash (X s)") com payload
//!   byte-fiel ao do Claude Code (matriz 13.10 achado 1);
//! - critério 2 (red-team: `node_id` forjado no payload NÃO muda atribuição — identidade
//!   vem do TOKEN de spawn; telemetria é `trusted=false` por contrato);
//! - critério 3 (capabilities.hooks=false → degradação graciosa, aviso 1×, zero erro);
//! - critério 4 (SubagentStart/Stop → árvore pai→filho, identidade não-decisória);
//! - critério 5 (listener LOOPBACK-only).
//!
//! O critério 1 "com 1 Claude real na tela" é a validação do gate da onda (F1-1-5).

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::Duration;

use lina_cli_profiles::CliProfile;
use lina_hooks::{HookKind, HookListener, HookPipeline, HooksCapability, SubagentTree};

/// POST HTTP/1.1 cru via TcpStream (sem client novo na árvore) → status-line.
fn post(addr: std::net::SocketAddr, path: &str, body: &str) -> String {
    let mut s = TcpStream::connect(addr).expect("conectar no listener");
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("enviar request");
    let mut resp = String::new();
    let _ = s.read_to_string(&mut resp);
    resp.lines().next().unwrap_or_default().to_string()
}

async fn recv_event(
    rx: &mut tokio::sync::broadcast::Receiver<lina_hooks::HookEvent>,
) -> lina_hooks::HookEvent {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("evento dentro do timeout")
        .expect("canal vivo")
}

/// Payload REAL do `PreToolUse` do Claude Code (campos da matriz 13.10 achado 1).
fn pretooluse_payload(extra: &str) -> String {
    format!(
        r#"{{"session_id":"sess-abc","transcript_path":"/tmp/t.jsonl","cwd":"/tmp/agent",
            "permission_mode":"default","tool_name":"Bash",
            "tool_input":{{"command":"cargo test"}}{extra}}}"#
    )
}

/// Critério 5: o listener nasce em loopback e SÓ em loopback.
#[tokio::test(flavor = "multi_thread")]
async fn listener_binds_loopback_only() {
    let l = HookListener::bind().await.expect("bind");
    assert!(
        l.local_addr().ip().is_loopback(),
        "listener deve escutar APENAS em loopback (inv#2): {}",
        l.local_addr()
    );
}

/// Critério 1 (headless): o par PreToolUse/PostToolUse com payload REAL vira o par de
/// `HookEvent` normalizado de que a timeline "Running: Bash (X s)" deriva.
#[tokio::test(flavor = "multi_thread")]
async fn pre_and_post_tooluse_normalize_for_timeline() {
    let l = HookListener::bind().await.expect("bind");
    let token = l.register_node("QA");
    let mut rx = l.subscribe();

    let st = post(
        l.local_addr(),
        &format!("/hook/{token}/PreToolUse"),
        &pretooluse_payload(""),
    );
    assert!(st.contains("200") || st.contains("204"), "status: {st}");
    let pre = recv_event(&mut rx).await;
    assert_eq!(pre.node_id, "QA");
    assert_eq!(pre.kind, HookKind::PreToolUse);
    assert_eq!(pre.tool_name.as_deref(), Some("Bash"));
    // Seam F1-1-6 (aditivo): o argumento LITERAL da tool atravessa o pipeline —
    // payload sem os campos novos segue parseando (None; provado no unit do crate).
    assert_eq!(
        pre.tool_input.as_deref(),
        Some(r#"{"command":"cargo test"}"#),
        "tool_input compacto lossless p/ o detail do toast"
    );
    assert!(pre.ts > 0, "ts de chegada para a duração da timeline");
    assert!(
        !pre.trusted,
        "telemetria de hook NUNCA é confiável p/ decisão"
    );

    post(
        l.local_addr(),
        &format!("/hook/{token}/PostToolUse"),
        &pretooluse_payload(""),
    );
    let post_ev = recv_event(&mut rx).await;
    assert_eq!(post_ev.kind, HookKind::PostToolUse);
    assert_eq!(post_ev.tool_name.as_deref(), Some("Bash"));
    assert!(
        post_ev.ts >= pre.ts,
        "o par ordenado é o que dá o 'Running: Bash (X s)'"
    );
}

/// Critério 2 (red-team): payload forjado com `node_id` de OUTRO nó não muda a
/// atribuição (que vem do TOKEN de spawn) nem ganha confiança.
#[tokio::test(flavor = "multi_thread")]
async fn forged_node_id_in_payload_does_not_change_attribution() {
    let l = HookListener::bind().await.expect("bind");
    let token_qa = l.register_node("QA");
    let _token_maestro = l.register_node("@Maestro");
    let mut rx = l.subscribe();

    post(
        l.local_addr(),
        &format!("/hook/{token_qa}/PreToolUse"),
        &pretooluse_payload(r#","node_id":"@Maestro","node":"@Maestro""#),
    );
    let ev = recv_event(&mut rx).await;
    assert_eq!(
        ev.node_id, "QA",
        "atribuição vem do token de spawn, NUNCA de campo escrito pelo agente"
    );
    assert!(!ev.trusted);
}

/// Token desconhecido → recusado SEM evento (nada de telemetria órfã/forjada).
#[tokio::test(flavor = "multi_thread")]
async fn unknown_token_rejected_without_event() {
    let l = HookListener::bind().await.expect("bind");
    let _t = l.register_node("QA");
    let mut rx = l.subscribe();

    let st = post(
        l.local_addr(),
        "/hook/token-inexistente/PreToolUse",
        &pretooluse_payload(""),
    );
    assert!(st.contains("404"), "token desconhecido = 404: {st}");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "nenhum evento pode nascer de token desconhecido"
    );
}

/// Kind que não consumimos (ex.: `FileChanged` dos 27 do Claude) é aceito e IGNORADO —
/// forward-compat sem erro (degradação, nunca quebra o hook do CLI).
#[tokio::test(flavor = "multi_thread")]
async fn unknown_kind_accepted_and_ignored() {
    let l = HookListener::bind().await.expect("bind");
    let token = l.register_node("QA");
    let mut rx = l.subscribe();

    let st = post(
        l.local_addr(),
        &format!("/hook/{token}/FileChanged"),
        r#"{"path":"/tmp/x"}"#,
    );
    assert!(
        st.contains("200") || st.contains("204") || st.contains("202"),
        "kind desconhecido não pode dar erro ao CLI: {st}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "kind não-consumido não vira evento"
    );
}

/// Critério 4: `SubagentStart`/`SubagentStop` viram árvore pai→filho na projeção —
/// identidade dos ids tratada como NÃO-confiável (telemetria, jamais autorização).
#[tokio::test(flavor = "multi_thread")]
async fn subagent_start_stop_builds_nested_tree() {
    let l = HookListener::bind().await.expect("bind");
    let token = l.register_node("QA");
    let mut rx = l.subscribe();
    let mut tree = SubagentTree::default();

    post(
        l.local_addr(),
        &format!("/hook/{token}/SubagentStart"),
        r#"{"session_id":"sess-abc","agent_id":"sub-1","agent_type":"Task"}"#,
    );
    let start = recv_event(&mut rx).await;
    assert_eq!(start.kind, HookKind::SubagentStart);
    assert_eq!(start.subagent_id.as_deref(), Some("sub-1"));
    tree.apply(&start);
    assert_eq!(
        tree.children_of("QA"),
        vec!["sub-1"],
        "filho aninhado sob o nó pai (atribuído pelo token)"
    );

    post(
        l.local_addr(),
        &format!("/hook/{token}/SubagentStop"),
        r#"{"session_id":"sess-abc","agent_id":"sub-1"}"#,
    );
    let stop = recv_event(&mut rx).await;
    tree.apply(&stop);
    assert!(
        tree.children_of("QA").is_empty(),
        "Stop encerra o filho na árvore"
    );
}

/// Critério 3: CLI com `capabilities.hooks=false` → pipeline degrada GRACIOSAMENTE:
/// capability GridOnly, aviso legível exatamente 1× ("atividade via grid apenas"),
/// nenhum fragmento de settings, zero erro.
#[test]
fn grid_only_profile_degrades_gracefully() {
    let toml_src = r#"
        id = "opencode"
        program = "opencode"
        delivery = "pty_inject"
        prompt_ready_regex = "> "
        [capabilities]
        hooks = false
        [end_signal]
        kind = "idle"
    "#;
    let profile = CliProfile::from_toml_str(toml_src, "<test>").expect("parsear");
    let pipeline = HookPipeline::for_profile(&profile);

    assert_eq!(pipeline.capability(), HooksCapability::GridOnly);
    let first = pipeline.grid_only_notice();
    assert!(
        first.as_deref().is_some_and(|m| m.contains("grid")),
        "aviso legível na 1ª consulta: {first:?}"
    );
    assert!(
        pipeline.grid_only_notice().is_none(),
        "aviso é 1× — não spamma o log a cada tick"
    );

    // E um perfil COM hooks fica habilitado (o claude-code real declara hooks=true).
    let claude = CliProfile::load_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/claude-code.toml"),
    )
    .expect("perfil real");
    assert_eq!(
        HookPipeline::for_profile(&claude).capability(),
        HooksCapability::HttpHooks
    );
}
