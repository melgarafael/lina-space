//! F1-5-8 — critérios headless da API de consulta paginada do histórico.
//!
//! (a) tail paginado byte-idêntico, nunca excede o limite; (b) pedido adversarial de 10^9
//! linhas → página máxima + cursor; (c) search encontra linha JÁ no disco; (d) export json
//! round-trip íntegro; (e) leitura cross emite o evento auditável (e fora do Espaço é
//! NEGADA); (f) janela expirada → "expirado", não erro (consome `expired_before`).

use lina_core::history::{self, ExportFormat, HistoryError, HistoryLimits, HistoryPage};
use lina_core::scrollback::{ScrollbackConfig, ScrollbackStore};
use lina_core::{EventStore, NodeId};

const PANEL: &str = "Terminal A";

/// Tempdir manual (sem dep externa) — mesmo padrão dos gates F1-0.
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-hist158-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("criar tempdir");
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Store com cap de RAM pequeno (12) e flush rápido — histórico vai ao DISCO cedo,
/// provando que a API lê além da janela viva.
fn store_com_500_linhas(tmp: &TempDir) -> ScrollbackStore {
    let cfg = ScrollbackConfig {
        cap: 12,
        flush_batch: 8,
        retention_days: 30,
    };
    let mut store = ScrollbackStore::open(tmp.path(), cfg).expect("abrir store");
    for i in 0..500 {
        store
            .push_line(PANEL, format!("linha-{i:04}"))
            .expect("push");
    }
    store.flush_all().expect("flush");
    store
}

fn node(n: u8) -> NodeId {
    NodeId::from_u128(n as u128)
}

// (a) — paginado, byte-idêntico, nunca excede o limite (histórico 500 ≫ cap RAM 12).
#[test]
fn a_tail_paginado_byte_identico_nunca_excede() {
    let tmp = TempDir::new();
    let store = store_com_500_linhas(&tmp);
    let limits = HistoryLimits::default();

    let p1 = history::tail(&store, PANEL, Some(50), 0, &limits).expect("tail p1");
    assert_eq!(p1.lines.len(), 50, "página devolve exatamente o pedido");
    assert_eq!(p1.lines.last().expect("última"), "linha-0499");
    assert_eq!(p1.lines.first().expect("primeira"), "linha-0450");
    assert_eq!(p1.start, 450);

    // Continuação pelo cursor: a página anterior NÃO se repete, byte-idêntico ao range.
    let off2 = p1.next_cursor.expect("há mais histórico");
    let p2 = history::tail(&store, PANEL, Some(50), off2, &limits).expect("tail p2");
    assert_eq!(p2.lines.first().expect("primeira p2"), "linha-0400");
    assert_eq!(p2.lines.last().expect("última p2"), "linha-0449");

    // Byte-idêntico ao conteúdo empurrado (inclui a região SÓ-disco, idx < 488-12).
    let inicio = history::tail(&store, PANEL, Some(10), 490, &limits).expect("tail início");
    assert_eq!(
        inicio.lines,
        (0..10).map(|i| format!("linha-{i:04}")).collect::<Vec<_>>()
    );
}

// (b) — adversarial: pedir 10^9 linhas devolve a página MÁXIMA + cursor, nunca mais.
#[test]
fn b_pedido_adversarial_clampa_no_teto() {
    let tmp = TempDir::new();
    let store = store_com_500_linhas(&tmp);
    let limits = HistoryLimits {
        max_page: 100,
        ..HistoryLimits::default()
    };
    let page = history::tail(&store, PANEL, Some(1_000_000_000), 0, &limits).expect("tail");
    assert_eq!(page.lines.len(), 100, "clampado no max_page — NUNCA mais");
    assert!(page.next_cursor.is_some(), "continuação via cursor");

    // O mesmo teto vale para o search (limit E varredura bounded).
    let hits = history::search(&store, PANEL, "linha", Some(1_000_000_000), None, &limits)
        .expect("search");
    assert!(hits.hits.len() <= 100, "hits clampados no teto");
}

// (c) — search encontra linha que JÁ está no disco (fora da janela viva de RAM).
#[test]
fn c_search_encontra_linha_no_disco() {
    let tmp = TempDir::new();
    let store = store_com_500_linhas(&tmp);
    let limits = HistoryLimits::default();
    // cap RAM = 12 → "linha-0007" só existe no disco.
    let page = history::search(&store, PANEL, "^linha-0007$", None, None, &limits).expect("search");
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].idx, 7);
    assert_eq!(page.hits[0].line, "linha-0007");

    // Regex inválido é erro EXPLÍCITO (não pânico, não vazio silencioso).
    let err = history::search(&store, PANEL, "[inval", None, None, &limits);
    assert!(matches!(err, Err(HistoryError::BadRegex(_))));
}

// (d) — export json round-trip íntegro (parse de volta == página exportada).
#[test]
fn d_export_json_round_trip() {
    let tmp = TempDir::new();
    let store = store_com_500_linhas(&tmp);
    let limits = HistoryLimits::default();
    let (json, _next) =
        history::export(&store, PANEL, ExportFormat::Json, 100, 160, &limits).expect("export");
    let de: HistoryPage = serde_json::from_str(&json).expect("round-trip");
    assert_eq!(de.start, 100);
    assert_eq!(de.lines.len(), 60);
    assert_eq!(de.lines[0], "linha-0100");
    assert_eq!(de.lines[59], "linha-0159");

    // txt: linhas cruas \n — sem ANSI (linearização W0-2 upstream), sem envelope.
    let (txt, _) =
        history::export(&store, PANEL, ExportFormat::Txt, 100, 102, &limits).expect("txt");
    assert_eq!(txt, "linha-0100\nlinha-0101");
}

// (e) — leitura cross emite o evento auditável; fora do Espaço é NEGADA (default-deny).
#[test]
fn e_leitura_cross_audita_e_nega_fora_do_espaco() {
    let tmp = TempDir::new();
    let store = store_com_500_linhas(&tmp);
    let limits = HistoryLimits::default();
    let mut events = EventStore::open(tmp.path().join("events")).expect("event store");

    let leitor = node(1);
    let dono = node(2);
    let estranho = node(9);
    let membros = [leitor, dono];

    // Membro do MESMO Espaço lê o painel do colega → entrega + evento no log.
    let page = history::tail_cross(
        &mut events,
        &membros,
        leitor,
        dono,
        &store,
        PANEL,
        Some(5),
        0,
        &limits,
    )
    .expect("cross permitido");
    assert_eq!(page.lines.len(), 5);
    let recs = events.events().expect("ler log");
    let audit: Vec<_> = recs
        .iter()
        .filter(|r| r.kind == "HistoryReadCross")
        .collect();
    assert_eq!(audit.len(), 1, "UMA leitura cross = UM evento auditável");
    assert_eq!(
        audit[0].payload.get("query").and_then(|v| v.as_str()),
        Some("tail")
    );

    // Fora do Espaço: NEGADA (default-deny) e NADA é lido nem auditado como permitido.
    let err = history::search_cross(
        &mut events,
        &membros,
        estranho,
        dono,
        &store,
        PANEL,
        "linha",
        None,
        None,
        &limits,
    );
    assert!(matches!(err, Err(HistoryError::CrossDenied { .. })));
    let recs = events.events().expect("reler log");
    assert_eq!(
        recs.iter().filter(|r| r.kind == "HistoryReadCross").count(),
        1,
        "negada não audita como leitura feita"
    );

    // Same-owner (ler o próprio painel) passa SEM evento — só cross é sinal.
    history::tail_cross(
        &mut events,
        &membros,
        dono,
        dono,
        &store,
        PANEL,
        Some(1),
        0,
        &limits,
    )
    .expect("same-owner sempre passa");
    let recs = events.events().expect("reler log 2");
    assert_eq!(
        recs.iter().filter(|r| r.kind == "HistoryReadCross").count(),
        1,
        "same-owner não polui a auditoria"
    );

    // export CROSS é leitura — a porta de MAIOR volume de exfiltração também audita
    // (uniformidade da fronteira: nenhuma das 3 ops escapa do rastro).
    let (_json, _next) = history::export_cross(
        &mut events,
        &membros,
        leitor,
        dono,
        &store,
        PANEL,
        ExportFormat::Json,
        0,
        5,
        &limits,
    )
    .expect("export cross permitido");
    let recs = events.events().expect("reler log 3");
    let exports = recs
        .iter()
        .filter(|r| {
            r.kind == "HistoryReadCross"
                && r.payload.get("query").and_then(|v| v.as_str()) == Some("export")
        })
        .count();
    assert_eq!(exports, 1, "export cross = UM evento auditável 'export'");
    let total_cross = recs.iter().filter(|r| r.kind == "HistoryReadCross").count();
    assert_eq!(
        total_cross, 2,
        "tail + export = 2 leituras cross registradas"
    );

    // E o estranho também é barrado no export (default-deny uniforme nas 3 ops).
    let err = history::export_cross(
        &mut events,
        &membros,
        estranho,
        dono,
        &store,
        PANEL,
        ExportFormat::Txt,
        0,
        5,
        &limits,
    );
    assert!(matches!(err, Err(HistoryError::CrossDenied { .. })));
    let recs = events.events().expect("reler log 4");
    assert_eq!(
        recs.iter().filter(|r| r.kind == "HistoryReadCross").count(),
        2,
        "export negado não vira rastro de leitura feita"
    );
}

// (f) — janela expirada responde "expirado", NUNCA erro (consome expired_before da F1-5-9).
#[test]
fn f_janela_expirada_e_expirado_nao_erro() {
    let tmp = TempDir::new();
    let cfg = ScrollbackConfig {
        cap: 12,
        flush_batch: 8,
        retention_days: 1,
    };
    let mut store = ScrollbackStore::open(tmp.path(), cfg).expect("abrir store");
    // Relógio injetável: as primeiras 100 linhas nascem "há 3 dias"…
    let antigo = 1_000_000_000_u64;
    store.set_clock(move || antigo);
    for i in 0..100 {
        store
            .push_line(PANEL, format!("velha-{i:03}"))
            .expect("push");
    }
    store.flush_all().expect("flush antigas");
    // …e as 50 seguintes nascem "agora" (3 dias depois).
    let agora = antigo + 3 * 24 * 60 * 60 * 1000;
    store.set_clock(move || agora);
    for i in 0..50 {
        store
            .push_line(PANEL, format!("nova-{i:03}"))
            .expect("push");
    }
    store.flush_all().expect("flush novas");
    store.run_retention().expect("retenção");
    let expired = store.expired_before(PANEL);
    assert!(expired >= 100, "as 100 antigas expiraram (piso={expired})");

    let limits = HistoryLimits::default();
    // Janela INTEIRA abaixo do piso: Ok + vazia + expired=true (nunca Err).
    let page = history::tail(&store, PANEL, Some(10), 60, &limits).expect("não é erro");
    assert!(page.expired, "sinaliza 'expirado'");
    assert!(page.lines.is_empty());
    assert_eq!(page.expired_before, expired);

    // Janela PARCIAL (pede 200, só 50 vivas): devolve as vivas + expired=true.
    let page = history::tail(&store, PANEL, Some(200), 0, &limits).expect("parcial ok");
    assert_eq!(page.lines.len(), 50, "só o que vive");
    assert!(page.expired, "e avisa que o resto expirou");
    assert!(page.next_cursor.is_none(), "não há mais nada atrás do piso");

    // search: cursor abaixo do piso é CONSUMIDO pelo piso (varre só o vivo).
    let hits = history::search(&store, PANEL, "velha", None, Some(0), &limits).expect("search");
    assert!(hits.hits.is_empty(), "expirado não é encontrável");
    assert_eq!(hits.expired_before, expired);
}
