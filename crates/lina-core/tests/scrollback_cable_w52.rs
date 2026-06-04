//! Gate **W5-2 FIAÇÃO** — o CABO `append-on-scroll` do pty-host REAL ao `ScrollbackStore`.
//!
//! Percorre o caminho de produção ponta a ponta (não lógica isolada):
//! `PtyHost::spawn` → thread de leitura → `flush` → `VtBackend::advance` (captura) →
//! `VtBackend::take_scrollback` → `ScrollbackStore::push_line`.
//!
//! Prova os critérios de aceite da story:
//! - (c) a RAM do painel estabiliza no `cap` (ring do motor VT == cap);
//! - (d) linhas ALÉM do `cap` são recuperadas **byte-idênticas** do Store (inclusive paginadas
//!   em disco e após **reabertura** do processo);
//! - (e) **zero perda**: `[linhas no Store] ++ [linhas na janela viva]` == toda a saída produzida,
//!   na ordem, sem gap (o invariante #4 — "o event log é a fonte da verdade", scrollback é
//!   projeção durável, nada some).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lina_core::scrollback::{ScrollbackConfig, ScrollbackStore};
use lina_core::{NodeId, PtyCommand, PtyHost, TerminalState};

const T: Duration = Duration::from_secs(10);

/// Espera-POR-CONDIÇÃO com timeout (poll), nunca sleep fixo de asserção.
fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Diretório temporário único (best-effort cleanup no Drop).
struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("lina-w52cable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("criar tempdir");
        Self(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// As linhas NÃO-vazias do viewport (janela viva), em ordem de cima p/ baixo.
fn viewport_lines(host: &PtyHost, node: NodeId) -> Vec<String> {
    host.with_grid(node, |vt| {
        let (_, rows) = vt.dims();
        (0..rows)
            .map(|r| vt.row_text(r).trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

#[test]
fn pty_host_cables_scrollback_to_store_with_zero_loss() {
    const N: usize = 500; // 25× o cap → muitas linhas saem da janela viva para o disco
    let cap = 20usize;
    let cols = 80u16;
    let rows = 24u16;

    let tmp = TempDir::new("zeroloss");
    let store = Arc::new(Mutex::new(
        ScrollbackStore::open(
            &tmp.0,
            ScrollbackConfig {
                cap,
                flush_batch: 8,
            },
        )
        .expect("open store"),
    ));

    let mut host = PtyHost::new();
    // O cabo: liga o Store ANTES do spawn (o backend nasce capturando, com cap = store.cap()).
    host.set_scrollback_store(Arc::clone(&store));

    // Emite linha-1 .. linha-N via shell POSIX puro (sem dep de `seq`). Saída < 1 MiB → flui sem
    // backpressure (não precisa de ack). Ao terminar, EOF → estado Exited.
    let script = format!("i=1; while [ $i -le {N} ]; do echo \"linha-$i\"; i=$((i+1)); done");
    let node = host
        .spawn(PtyCommand::new("sh").arg("-c").arg(script), cols, rows)
        .expect("spawn");
    let panel = node.to_string();

    // O comando terminou e a thread de leitura processou TODO o output (incl. o flush de EOF).
    assert!(
        poll_until(T, || host.state(node) == Some(TerminalState::Exited)),
        "o terminal deveria terminar (EOF) após emitir as {N} linhas"
    );

    // (c) RAM do painel estabiliza NO cap: o ring do motor VT ficou exatamente em `cap`.
    let ring = host.with_grid(node, |vt| vt.scrollback_len()).unwrap_or(0);
    assert_eq!(
        ring, cap,
        "ring de scrollback em RAM deveria estabilizar no cap"
    );

    // Achata o write-behind no disco para medir a paginação.
    {
        let mut s = store.lock().expect("lock store");
        s.flush(&panel).expect("flush");
    }
    let total = store.lock().expect("lock").total_lines(&panel);

    // Saíram da janela viva pelo menos N - rows linhas (no máx. `rows` ficam visíveis).
    assert!(
        total >= (N as u64) - u64::from(rows),
        "esperava ≥ {} linhas no Store, veio {total}",
        N - rows as usize
    );
    assert!(
        total <= N as u64,
        "Store não pode ter mais linhas que o emitido"
    );

    // (d) PAGINAÇÃO: o excedente além do cap foi de fato ao disco (não só ao cache da cauda).
    {
        let mut s = store.lock().expect("lock");
        s.checkpoint().expect("checkpoint");
        let disk = s.disk_rows(&panel).expect("disk_rows");
        assert!(
            disk >= total - cap as u64,
            "disco não recebeu o excedente paginado: {disk} (total {total}, cap {cap})"
        );
    }

    // (d) byte-idêntico do Store: uma linha bem antiga (paginada em disco, fora do cache da cauda).
    {
        let s = store.lock().expect("lock");
        // idx 0 = "linha-1"; idx k = "linha-{k+1}". idx 4 está paginado (< total - cap).
        assert!(
            4 < total - cap as u64,
            "idx de teste precisa estar paginado"
        );
        assert_eq!(
            s.line(&panel, 4).expect("line").as_deref(),
            Some("linha-5"),
            "linha paginada NÃO recuperada byte-idêntica do Store"
        );
    }

    // (e) ZERO PERDA — a prova mais forte: Store (prefixo cronológico, atravessando disco→cauda)
    // concatenado com a janela viva reconstrói EXATAMENTE linha-1 .. linha-N, em ordem, sem gap.
    let store_lines = store
        .lock()
        .expect("lock")
        .range(&panel, 0, total)
        .expect("range");
    let live = viewport_lines(&host, node);
    let mut reconstructed = store_lines;
    reconstructed.extend(live);
    let expected: Vec<String> = (1..=N).map(|i| format!("linha-{i}")).collect();
    assert_eq!(
        reconstructed, expected,
        "perda/gap/reordenação: Store + janela viva não reconstroem toda a saída"
    );

    // (d') DURABILIDADE pós-reabertura (inv#4): fecha o Store e reabre do disco — a linha paginada
    // segue byte-idêntica (o histórico sobrevive ao fim do processo que o produziu).
    let total_before = total;
    drop(store); // fecha a conexão (Drop faz flush_all)
    host.kill(node).ok();

    let s2 = ScrollbackStore::open(
        &tmp.0,
        ScrollbackConfig {
            cap,
            flush_batch: 8,
        },
    )
    .expect("reopen store");
    assert_eq!(
        s2.total_lines(&panel),
        total_before,
        "total perdido na reabertura"
    );
    assert_eq!(
        s2.line(&panel, 4).expect("line").as_deref(),
        Some("linha-5"),
        "recuperação pós-reabertura não byte-idêntica"
    );
}
