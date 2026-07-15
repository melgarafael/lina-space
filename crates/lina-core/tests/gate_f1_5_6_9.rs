//! Gate **F1-5-6 + F1-5-9** — durabilidade de flush (idle-drain + signal handler) e
//! retenção configurável de 30 dias (despacho `r2-dados.md`; fonte `ondas-5-6.md`
//! linhas 100-108 e 130-138).
//!
//! F1-5-6: (a) SIGTERM com pendentes → zero perda byte-idêntica (processo FILHO real);
//! (b) idle ≤2s → disco, visível a leitor EXTERNO; (c) regressão: gates W5-2 seguem
//! verdes (rodam na mesma suíte); (d) output torrencial NÃO dispara o idle-drain;
//! métrica de linhas pendentes observável.
//!
//! F1-5-9: (a) relógio injetável — antigas somem, recentes ficam, `idx` monotônico
//! sobrevive ao DELETE (inclusive expiração TOTAL + reabertura); (b) o `.db`
//! ESTABILIZA sob workload contínuo (anti-"Warp 41GB", medição registrada);
//! (c) fixture do schema ANTIGO abre sem perda (migração idempotente); (d) leitura
//! pós-expiração sinaliza "expirado" (`expired_before`), nunca erro.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lina_core::scrollback::{FlushGuard, FlushGuardConfig, ScrollbackConfig, ScrollbackStore};

const DAY_MS: u64 = 86_400_000;

/// Espera-por-condição com timeout (poll) — idioma de `scrollback_cable_w52.rs`.
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

/// Diretório temporário único; removido no Drop.
struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "lina-gate-f1569-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
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

fn cfg(cap: usize, flush_batch: usize, retention_days: u32) -> ScrollbackConfig {
    ScrollbackConfig {
        cap,
        flush_batch,
        retention_days,
    }
}

// ═══════════════════════ F1-5-6 (a) — SIGTERM zero perda ═══════════════════════

/// As linhas que o FILHO empurra (≈500B pendentes) — compartilhado entre o filho e a
/// asserção byte-idêntica do pai.
fn child_lines() -> Vec<String> {
    (0..10u32)
        .map(|i| format!("pendente-{i:02}-{}", "x".repeat(38)))
        .collect()
}

/// MODO FILHO do teste de sinal (no-op fora do re-exec): abre o store, deixa ~500B
/// PENDENTES (flush_batch alto — nada vai ao disco), liga o FlushGuard com sinais e
/// espera ser morto. O ÚNICO caminho que salva as linhas é o handler de SIGTERM →
/// `flush_all` (o `Drop` não roda sob morte por sinal — não há unwinding).
#[test]
fn signal_child_mode() {
    let Some(dir) = std::env::var_os("LINA_F1569_CHILD_DIR") else {
        return; // execução normal da suíte: no-op
    };
    let dir = PathBuf::from(dir);
    let store = ScrollbackStore::open(&dir, cfg(100, 10_000, 30)).expect("filho: open");
    let store = Arc::new(Mutex::new(store));
    {
        let mut s = store.lock().expect("lock");
        for l in child_lines() {
            s.push_line("sinal", l).expect("push");
        }
        assert_eq!(s.pending_lines("sinal"), 10, "pendência montada");
        assert_eq!(
            s.disk_rows("sinal").expect("rows"),
            0,
            "nada no disco ainda"
        );
    }
    // idle_for ENORME: o idle-drain NUNCA salva; só o caminho de sinal pode.
    let _guard = FlushGuard::start(
        Arc::clone(&store),
        FlushGuardConfig {
            idle_for: Duration::from_secs(3600),
            tick: Duration::from_millis(25),
            handle_signals: true,
        },
    )
    .expect("subir o flush guard");
    std::fs::write(dir.join("child-ready"), b"ok").expect("marker");
    // Morre por sinal; o teto de 120s é só rede de segurança contra env vazada
    // num shell de dev (exit code distinto, nunca confundível com sucesso).
    for _ in 0..1_200 {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::process::exit(86);
}

/// Modo filho da regressão do coordenador: um callback público entra em panic em
/// passadas consecutivas. O coordenador precisa conter o panic e continuar apto a
/// observar/reemitir SIGTERM; caso contrário o handler deixaria o processo imortal.
#[test]
fn coordinator_panic_child_mode() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let Some(dir) = std::env::var_os("LINA_F1569_PANIC_CHILD_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mut store = ScrollbackStore::open(&dir, cfg(100, 10_000, 30)).expect("filho: open");
    let clock_calls = Arc::new(AtomicUsize::new(0));
    store.set_clock({
        let clock_calls = Arc::clone(&clock_calls);
        move || {
            clock_calls.fetch_add(1, Ordering::SeqCst);
            panic!("panic injetado no relógio do scrollback")
        }
    });
    let store = Arc::new(Mutex::new(store));
    let _guard = FlushGuard::start(
        Arc::clone(&store),
        FlushGuardConfig {
            idle_for: Duration::from_secs(3_600),
            tick: Duration::from_millis(10),
            handle_signals: true,
        },
    )
    .expect("subir coordenador");
    assert!(
        poll_until(Duration::from_secs(5), || clock_calls
            .load(Ordering::SeqCst)
            >= 2),
        "o worker não continuou depois do primeiro panic"
    );
    let stats = lina_core::scrollback::flush_coordinator_stats();
    assert_eq!(stats.threads, 1, "thread real segue viva depois do panic");
    assert!(stats.handlers_installed, "handlers seguem observáveis");
    std::fs::write(dir.join("panic-child-ready"), b"ok").expect("marker");
    for _ in 0..1_200 {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::process::exit(87);
}

/// **Critério (a)**: processo REAL com ~500B pendentes recebe `SIGTERM` → reabrir o
/// store de fora → zero perda, byte-idêntica. (Estilo 13.16; o filho é este mesmo
/// binário de teste re-executado em modo filho.)
#[cfg(unix)]
#[test]
fn a_sigterm_com_pendentes_zero_perda_byte_identica() {
    let tmp = TempDir::new("sigterm");
    let dir = tmp.path().join("sb");
    std::fs::create_dir_all(&dir).expect("dir");

    let exe = std::env::current_exe().expect("exe do teste");
    let mut child = std::process::Command::new(exe)
        .args([
            "signal_child_mode",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LINA_F1569_CHILD_DIR", &dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn do filho");

    let marker = dir.join("child-ready");
    assert!(
        poll_until(Duration::from_secs(20), || marker.exists()),
        "filho não sinalizou pronto"
    );
    // SAFETY: kill(2) com pid do filho que ESTE teste criou; só envia o sinal.
    unsafe {
        assert_eq!(
            libc::kill(child.id() as i32, libc::SIGTERM),
            0,
            "kill falhou"
        );
    }
    assert!(
        poll_until(Duration::from_secs(10), || matches!(
            child.try_wait(),
            Ok(Some(_))
        )),
        "filho não terminou após SIGTERM"
    );

    // Reabertura EXTERNA: as 10 linhas pendentes foram salvas pelo handler.
    let store = ScrollbackStore::open(&dir, cfg(100, 10_000, 30)).expect("reabrir");
    assert_eq!(
        store.range("sinal", 0, 100).expect("range"),
        child_lines(),
        "perda ou corrupção: o flush de sinal não salvou byte-idêntico"
    );
}

/// Regressão: panic de um store não pode matar a thread global e transformar os
/// handlers permanentes em um sumidouro de SIGTERM.
#[cfg(unix)]
#[test]
fn workspace_reliability_coordinator_survives_store_panic_and_reemits_sigterm() {
    use std::os::unix::process::ExitStatusExt;

    let tmp = TempDir::new("coordinator-panic");
    let dir = tmp.path().join("sb");
    std::fs::create_dir_all(&dir).expect("dir");
    let exe = std::env::current_exe().expect("exe do teste");
    let mut child = std::process::Command::new(exe)
        .args([
            "coordinator_panic_child_mode",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LINA_F1569_PANIC_CHILD_DIR", &dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn do filho");

    assert!(
        poll_until(Duration::from_secs(20), || dir
            .join("panic-child-ready")
            .exists()),
        "filho não provou contenção do panic"
    );
    // SAFETY: kill(2) somente no processo filho criado por este teste.
    unsafe {
        assert_eq!(libc::kill(child.id() as i32, libc::SIGTERM), 0);
    }
    assert!(
        poll_until(Duration::from_secs(10), || matches!(
            child.try_wait(),
            Ok(Some(_))
        )),
        "coordenador engoliu SIGTERM depois do panic"
    );
    let status = child.wait().expect("status do filho");
    assert_eq!(
        status.signal(),
        Some(libc::SIGTERM),
        "o processo precisa preservar a semântica do sinal original: {status:?}"
    );
}

/// (revisão) `nohup`/background-shell: SIGHUP herdado como `SIG_IGN` é RESPEITADO —
/// o guard não instala handler para sinal ignorado por herança (convenção POSIX);
/// os demais sinais seguem drenando. O filho ignora SIGHUP, sobrevive a ele, e morre
/// (drenando) no SIGTERM.
#[cfg(unix)]
#[test]
fn sighup_ignorado_por_heranca_continua_ignorado() {
    use std::os::unix::process::CommandExt;
    let tmp = TempDir::new("sigign");
    let dir = tmp.path().join("sb");
    std::fs::create_dir_all(&dir).expect("dir");

    let exe = std::env::current_exe().expect("exe do teste");
    let mut cmd = std::process::Command::new(exe);
    cmd.args([
        "signal_child_mode",
        "--exact",
        "--nocapture",
        "--test-threads=1",
    ])
    .env("LINA_F1569_CHILD_DIR", &dir)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    // SAFETY: pre_exec roda no filho entre fork e exec; só seta SIGHUP=SIG_IGN
    // (async-signal-safe) — simula exatamente o `nohup`.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn do filho");

    let marker = dir.join("child-ready");
    assert!(
        poll_until(Duration::from_secs(20), || marker.exists()),
        "filho não sinalizou pronto"
    );
    // SAFETY: kill(2) no filho deste teste.
    unsafe {
        assert_eq!(libc::kill(child.id() as i32, libc::SIGHUP), 0);
    }
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        matches!(child.try_wait(), Ok(None)),
        "nohup quebrado: SIGHUP ignorado por herança MATOU o filho"
    );
    // O SIGHUP ignorado também NÃO flushou nada (handler não instalado p/ ele).
    {
        let external =
            rusqlite::Connection::open(dir.join("scrollback.db")).expect("leitor externo");
        external
            .busy_timeout(Duration::from_millis(3000))
            .expect("busy_timeout");
        let n: i64 = external
            .query_row("SELECT COUNT(*) FROM scrollback", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(n, 0, "SIGHUP ignorado não deveria drenar");
    }
    // Os DEMAIS sinais seguem funcionando: SIGTERM drena e mata.
    unsafe {
        assert_eq!(libc::kill(child.id() as i32, libc::SIGTERM), 0);
    }
    assert!(
        poll_until(Duration::from_secs(10), || matches!(
            child.try_wait(),
            Ok(Some(_))
        )),
        "filho não terminou após SIGTERM"
    );
    let store = ScrollbackStore::open(&dir, cfg(100, 10_000, 30)).expect("reabrir");
    assert_eq!(
        store.range("sinal", 0, 100).expect("range"),
        child_lines(),
        "SIGTERM continua drenando byte-idêntico"
    );
}

// ═══════════════════════ F1-5-6 (b) — idle ≤2s → disco ═══════════════════════

/// (revisão) O DEFAULT de produção respeita o teto do critério: pior caso
/// `idle_for + tick ≤ 2s` — mudar o default para fora da faixa quebra AQUI.
#[test]
fn default_do_guard_respeita_o_teto_de_2s_do_criterio() {
    let d = FlushGuardConfig::default();
    assert!(
        d.idle_for + d.tick <= Duration::from_secs(2),
        "default do guard ({:?} + {:?}) estoura o '≤2s' do critério (b)",
        d.idle_for,
        d.tick
    );
    assert!(
        d.idle_for >= Duration::from_secs(1),
        "a story fixa a faixa 1-2s para o idle-drain"
    );
}

/// **Critério (b)**: output para → em ≤2s o pendente está no DISCO, visível a um
/// leitor EXTERNO (outra conexão SQLite, como o `lina check` de outro processo).
/// Também prova a métrica: `last_pending` registra as linhas no momento do drain.
#[test]
fn b_idle_drain_persiste_em_ate_2s_visivel_a_leitor_externo() {
    let tmp = TempDir::new("idle");
    let store = ScrollbackStore::open(tmp.path(), cfg(100, 10_000, 30)).expect("open");
    let store = Arc::new(Mutex::new(store));
    let db_path = store.lock().expect("lock").db_path().to_path_buf();

    {
        let mut s = store.lock().expect("lock");
        for i in 0..5u32 {
            s.push_line("T", format!("idle-{i}")).expect("push");
        }
        assert_eq!(s.pending_lines("T"), 5);
    }
    // Leitor externo ANTES do drain: nada no disco (controle não-vácuo).
    let external = rusqlite::Connection::open(&db_path).expect("leitor externo");
    external
        .busy_timeout(Duration::from_millis(3000))
        .expect("busy_timeout");
    let count = |c: &rusqlite::Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM scrollback WHERE panel = 'T'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    assert_eq!(
        count(&external),
        0,
        "controle: pendente ainda fora do disco"
    );

    let _guard = FlushGuard::start(
        Arc::clone(&store),
        FlushGuardConfig {
            idle_for: Duration::from_millis(100),
            tick: Duration::from_millis(20),
            handle_signals: false,
        },
    )
    .expect("subir o flush guard");
    // ≤2s (o critério da story) para o leitor EXTERNO enxergar as 5 linhas.
    assert!(
        poll_until(Duration::from_secs(2), || count(&external) == 5),
        "idle-drain não persistiu em ≤2s (leitor externo vê {})",
        count(&external)
    );

    let s = store.lock().expect("lock");
    assert_eq!(s.pending_lines("T"), 0, "pendência zerada após o drain");
    let stats = s.drain_stats();
    assert!(stats.idle_drains >= 1, "métrica: drain contabilizado");
    assert_eq!(
        stats.last_pending, 5,
        "métrica: linhas pendentes no momento do flush"
    );
}

// ═══════════════════════ F1-5-6 (d) — torrencial não dispara ═══════════════════════

/// **Critério (d)**: sob output torrencial CONTÍNUO o idle-drain NÃO dispara (não é
/// flush-por-linha disfarçado). Controle positivo: parou o output → o drain dispara.
#[test]
fn d_output_torrencial_nao_dispara_idle_drain() {
    let tmp = TempDir::new("torrent");
    let store = ScrollbackStore::open(tmp.path(), cfg(1_000, 100_000, 30)).expect("open");
    let store = Arc::new(Mutex::new(store));

    let _guard = FlushGuard::start(
        Arc::clone(&store),
        FlushGuardConfig {
            idle_for: Duration::from_millis(400),
            tick: Duration::from_millis(20),
            handle_signals: false,
        },
    )
    .expect("subir o flush guard");

    // Torrente que OUTLASTA o idle_for (revisão): 140 linhas × ~5ms ≈ ≥700ms de
    // atividade contínua > 400ms de idle_for — uma implementação degenerada que
    // dispara num timer fixo desde o start falharia no assert do MEIO da torrente.
    for i in 0..140u32 {
        store
            .lock()
            .expect("lock")
            .push_line("F", format!("torrente-{i}"))
            .expect("push");
        if i == 100 {
            // Já passamos de 500ms de torrente (> idle_for) e NADA drenou.
            let s = store.lock().expect("lock");
            assert_eq!(
                s.drain_stats().idle_drains,
                0,
                "idle-drain disparou NO MEIO da torrente (timer fixo, não ociosidade)"
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    {
        let s = store.lock().expect("lock");
        assert_eq!(
            s.drain_stats().idle_drains,
            0,
            "idle-drain disparou DURANTE a torrente (virou flush-por-linha)"
        );
        assert_eq!(
            s.disk_rows("F").expect("rows"),
            0,
            "nada deveria ter ido ao disco durante a torrente (flush_batch alto)"
        );
        assert_eq!(s.pending_lines("F"), 140);
    }

    // Controle positivo: output PAROU → o drain dispara e persiste tudo.
    assert!(
        poll_until(Duration::from_secs(2), || {
            store.lock().expect("lock").pending_lines("F") == 0
        }),
        "drain não disparou após o fim da torrente"
    );
    let s = store.lock().expect("lock");
    assert!(s.drain_stats().idle_drains >= 1);
    assert_eq!(
        s.disk_rows("F").expect("rows"),
        140,
        "tudo no disco após o idle"
    );
}

/// (revisão) Fiação do PtyHost: `start_flush_guard` sem store → `Ok(false)`; com o
/// store ligado → o guard via HOST drena painel ocioso (o hunk do lib.rs deixa de
/// ser código morto no gate — mutação que o deletar quebra AQUI).
#[test]
fn guard_via_ptyhost_drena_e_sem_store_devolve_false() {
    use lina_core::PtyHost;
    let mut sem_store = PtyHost::new();
    assert!(
        matches!(
            sem_store.start_flush_guard(FlushGuardConfig::default()),
            Ok(false)
        ),
        "sem store ligado não há o que proteger"
    );

    let tmp = TempDir::new("viahost");
    let store = Arc::new(Mutex::new(
        ScrollbackStore::open(tmp.path(), cfg(100, 10_000, 30)).expect("open"),
    ));
    let mut host = PtyHost::new();
    host.set_scrollback_store(Arc::clone(&store));
    assert!(
        matches!(
            host.start_flush_guard(FlushGuardConfig {
                idle_for: Duration::from_millis(100),
                tick: Duration::from_millis(20),
                handle_signals: false,
            }),
            Ok(true)
        ),
        "com store ligado o guard sobe"
    );
    store
        .lock()
        .expect("lock")
        .push_line("H", "via-host")
        .expect("push");
    assert!(
        poll_until(Duration::from_secs(2), || {
            store.lock().expect("lock").pending_lines("H") == 0
        }),
        "o guard subido PELO PtyHost não drenou"
    );
}

// ═══════════════════════ F1-5-9 (a) — expiração + idx monotônico ═══════════════════════

/// **Critério (a)**: com relógio INJETÁVEL, linhas além de `retention_days` somem;
/// recentes ficam; `idx` segue monotônico — inclusive com expiração TOTAL do painel
/// e REABERTURA pós-limpeza (o `MAX(idx)` morreu; a meta durável preserva a sequência).
#[test]
fn ret_a_expira_antigas_mantem_recentes_idx_monotonico_pos_reabertura() {
    let tmp = TempDir::new("reta");
    let c = cfg(4, 2, 30);
    let t0 = 1_750_000_000_000u64;
    {
        let mut s = ScrollbackStore::open(tmp.path(), c).expect("open");
        s.set_clock(move || t0);
        for i in 0..10u32 {
            s.push_line("P", format!("velha-{i}")).expect("push");
        }
        s.flush("P").expect("flush"); // ts = t0
        for i in 0..5u32 {
            s.push_line("Q", format!("toda-velha-{i}")).expect("push");
        }
        s.flush("Q").expect("flush");

        // 31 dias depois: P ganha linhas novas; Q fica só com as velhas.
        let t1 = t0 + 31 * DAY_MS;
        s.set_clock(move || t1);
        for i in 0..3u32 {
            s.push_line("P", format!("nova-{i}")).expect("push");
        }
        s.flush("P").expect("flush"); // ts = t1

        let report = s.run_retention().expect("retention");
        assert_eq!(report.deleted, 15, "10 de P + 5 de Q expiradas");

        // P: antigas sumiram; recentes ficaram; piso de expiração sinalizado.
        assert_eq!(
            s.line("P", 0).expect("line"),
            None,
            "expirada → None, não erro"
        );
        assert_eq!(s.range("P", 0, 10).expect("range"), Vec::<String>::new());
        assert_eq!(s.expired_before("P"), 10, "piso de expiração de P");
        assert_eq!(s.line("P", 10).expect("line").as_deref(), Some("nova-0"));
        // idx segue monotônico no painel vivo.
        s.push_line("P", "nova-3").expect("push");
        assert_eq!(s.total_lines("P"), 14);
    } // drop (flush_all)

    // Reabertura PÓS-LIMPEZA: Q expirou INTEIRO (MAX(idx) sumiu do disco) — a
    // numeração NÃO pode regredir: a próxima linha de Q é idx 5, nunca 0.
    let mut s2 = ScrollbackStore::open(tmp.path(), c).expect("reopen");
    assert_eq!(
        s2.total_lines("Q"),
        5,
        "sequência de Q preservada pela meta durável"
    );
    assert_eq!(s2.expired_before("Q"), 5);
    assert_eq!(s2.line("Q", 0).expect("line"), None);
    s2.push_line("Q", "renascida").expect("push");
    assert_eq!(
        s2.line("Q", 5).expect("line").as_deref(),
        Some("renascida"),
        "idx de Q continuou de onde parou (5), não reusou 0"
    );
    assert_eq!(s2.total_lines("P"), 14, "P também reidratado");
}

// ═══════════════════════ F1-5-9 (b) — .db estabiliza ═══════════════════════

/// **Critério (b)** — a propriedade anti-"Warp 41GB": sob workload contínuo com
/// retenção ativa, o tamanho do `scrollback.db` ESTABILIZA (não cresce monotônico).
/// Medição direta de bytes registrada (eprintln) para o relatório do Maestro.
/// Nota: o `checkpoint()` aqui é INSTRUMENTO de medição (WAL → .db); em produção o
/// `-wal` é limitado pelo auto-checkpoint default do SQLite (~1000 páginas).
#[test]
fn ret_b_tamanho_db_estabiliza_sob_workload_continuo() {
    let tmp = TempDir::new("retb");
    let mut s = ScrollbackStore::open(tmp.path(), cfg(50, 64, 2)).expect("open");
    let t0 = 1_750_000_000_000u64;

    let mut sizes = Vec::new();
    for day in 0..12u64 {
        let now = t0 + day * DAY_MS;
        s.set_clock(move || now);
        for i in 0..2_000u32 {
            s.push_line("W", format!("dia{day:02}-linha{i:04}-{}", "y".repeat(40)))
                .expect("push");
        }
        s.flush("W").expect("flush");
        s.run_retention().expect("retention");
        s.checkpoint().expect("checkpoint");
        let len = std::fs::metadata(s.db_path()).expect("metadata").len();
        sizes.push(len);
    }
    eprintln!("[F1-5-9 ret_b] tamanhos diários do scrollback.db (bytes): {sizes:?}");

    // Cresce no warmup (controle: a retenção ainda não corta nada nos 2 primeiros dias)...
    assert!(
        sizes[2] > sizes[0],
        "controle: o arquivo deveria crescer antes da retenção cortar"
    );
    // ...e ESTABILIZA depois: nenhuma medição do regime (dia ≥4) excede 115% da menor.
    let regime = &sizes[4..];
    let min = *regime.iter().min().expect("min");
    let max = *regime.iter().max().expect("max");
    assert!(
        max * 100 <= min * 115,
        "scrollback.db NÃO estabilizou (min={min}, max={max}) — anti-Warp falhou"
    );
}

// ═══════════════════════ F1-5-9 (c) — fixture do schema antigo ═══════════════════════

/// **Critério (c)**: um `scrollback.db` da VERSÃO ANTERIOR (schema sem `ts`/meta —
/// fixture criada com o SQL original da W5-2) abre sem perda; a migração é
/// idempotente (reabrir N vezes não duplica nem corrompe).
#[test]
fn ret_c_fixture_schema_antigo_abre_sem_perda_e_migracao_e_idempotente() {
    let tmp = TempDir::new("retc");
    let db = tmp.path().join("scrollback.db");
    {
        // Fixture: o schema EXATO da versão anterior (scrollback.rs pré-F1-5-9).
        let conn = rusqlite::Connection::open(&db).expect("fixture");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scrollback (
                panel TEXT    NOT NULL,
                idx   INTEGER NOT NULL,
                text  TEXT    NOT NULL,
                PRIMARY KEY (panel, idx)
            );",
        )
        .expect("schema antigo");
        let mut stmt = conn
            .prepare("INSERT INTO scrollback (panel, idx, text) VALUES (?1, ?2, ?3)")
            .expect("prep");
        for i in 0..5i64 {
            stmt.execute(rusqlite::params!["legado", i, format!("antiga-{i}")])
                .expect("insert");
        }
    }

    let c = cfg(4, 2, 30);
    let mut s = ScrollbackStore::open(tmp.path(), c).expect("abrir fixture migrando");
    assert_eq!(
        s.total_lines("legado"),
        5,
        "nenhuma linha perdida na migração"
    );
    let exp: Vec<String> = (0..5).map(|i| format!("antiga-{i}")).collect();
    assert_eq!(
        s.range("legado", 0, 5).expect("range"),
        exp,
        "byte-idêntico"
    );
    // Linhas migradas ganham o ts da migração (relógio corrente) → NÃO expiram já.
    let t_now = 1_750_000_000_000u64;
    s.set_clock(move || t_now);
    assert_eq!(s.run_retention().expect("retention").deleted, 0);
    s.push_line("legado", "antiga-5").expect("push continua");
    assert_eq!(
        s.line("legado", 5).expect("line").as_deref(),
        Some("antiga-5")
    );
    drop(s);

    // Idempotência: reabrir roda a migração de novo sem erro/perda/duplicação.
    let s2 = ScrollbackStore::open(tmp.path(), c).expect("reabrir migrado");
    assert_eq!(s2.total_lines("legado"), 6);
    assert_eq!(s2.disk_rows("legado").expect("rows"), 6);
}

// ═══════════════════════ F1-5-9 (d) — "expirado", não erro ═══════════════════════

/// **Critério (d)**: leitura numa janela expirada responde VAZIO + piso sinalizado
/// (`expired_before`) — nunca erro, nunca dado fantasma.
#[test]
fn ret_d_leitura_pos_expiracao_sinaliza_expirado_nao_erro() {
    let tmp = TempDir::new("retd");
    let mut s = ScrollbackStore::open(tmp.path(), cfg(2, 2, 7)).expect("open");
    let t0 = 1_750_000_000_000u64;
    s.set_clock(move || t0);
    for i in 0..6u32 {
        s.push_line("X", format!("x-{i}")).expect("push");
    }
    s.flush("X").expect("flush");
    let t1 = t0 + 8 * DAY_MS;
    s.set_clock(move || t1);
    s.run_retention().expect("retention");

    // Janela 100% expirada: Ok(vazio), nunca Err; piso aponta o corte.
    assert_eq!(
        s.range("X", 0, 6).expect("não é erro"),
        Vec::<String>::new()
    );
    assert_eq!(s.line("X", 2).expect("não é erro"), None);
    assert_eq!(
        s.expired_before("X"),
        6,
        "o sinal 'histórico expirado' para UI/API"
    );
    // total não regride: a história continua do 6.
    assert_eq!(s.total_lines("X"), 6);
}

/// (revisão) A cadência do job é DIÁRIA de fato: a 2ª chamada no mesmo "dia" do
/// relógio injetável devolve `None` (throttle); +1 dia → roda de novo.
#[test]
fn ret_f_cadencia_diaria_do_job_e_throttled() {
    let tmp = TempDir::new("retf");
    let mut s = ScrollbackStore::open(tmp.path(), cfg(4, 2, 30)).expect("open");
    let t0 = 1_750_000_000_000u64;
    s.set_clock(move || t0);
    assert!(
        s.maybe_run_retention().expect("1ª").is_some(),
        "no boot o job roda na primeira oportunidade"
    );
    assert!(
        s.maybe_run_retention().expect("2ª").is_none(),
        "mesmo dia → throttled (sem full-scan por tick)"
    );
    let t1 = t0 + DAY_MS;
    s.set_clock(move || t1);
    assert!(
        s.maybe_run_retention().expect("3ª").is_some(),
        "+1 dia → roda de novo"
    );
}

/// (revisão) Painel LEGADO (fixture pré-F1-5-9, sem linha na meta) que expira
/// INTEIRO sem nenhum flush novo: o piso "expirado" aparece e a sequência de `idx`
/// NÃO regride na reabertura — a migração semeia a meta dos painéis órfãos.
#[test]
fn ret_g_fixture_legada_expira_inteira_sem_flush_novo_idx_nao_regride() {
    let tmp = TempDir::new("retg");
    let db = tmp.path().join("scrollback.db");
    {
        let conn = rusqlite::Connection::open(&db).expect("fixture");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scrollback (
                panel TEXT    NOT NULL,
                idx   INTEGER NOT NULL,
                text  TEXT    NOT NULL,
                PRIMARY KEY (panel, idx)
            );",
        )
        .expect("schema antigo");
        let mut stmt = conn
            .prepare("INSERT INTO scrollback (panel, idx, text) VALUES (?1, ?2, ?3)")
            .expect("prep");
        for i in 0..5i64 {
            stmt.execute(rusqlite::params!["orfao", i, format!("velha-{i}")])
                .expect("insert");
        }
    }

    let c = cfg(4, 2, 30);
    {
        let mut s = ScrollbackStore::open(tmp.path(), c).expect("abrir migrando");
        // As migradas ganham o ts REAL da migração; pula o relógio 40 dias à frente.
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("relógio")
            .as_millis() as u64
            + 40 * DAY_MS;
        s.set_clock(move || future);
        let report = s.run_retention().expect("retention");
        assert_eq!(report.deleted, 5, "as 5 legadas expiraram");
        assert_eq!(
            s.expired_before("orfao"),
            5,
            "o piso 'expirado' aparece MESMO sem flush novo (meta semeada na migração)"
        );
    } // drop

    let mut s2 = ScrollbackStore::open(tmp.path(), c).expect("reabrir pós-expiração total");
    assert_eq!(
        s2.total_lines("orfao"),
        5,
        "sequência preservada: o painel NÃO sumiu com o MAX(idx)"
    );
    s2.push_line("orfao", "renascida").expect("push");
    assert_eq!(
        s2.line("orfao", 5).expect("line").as_deref(),
        Some("renascida"),
        "idx continuou do 5 — nunca regride a 0"
    );
}

/// (revisão) A migração cria o índice em `ts`: o DELETE diário e o UPDATE da migração
/// não fazem full-scan segurando o Mutex global (BAIXA-iii não piora).
#[test]
fn ret_h_migracao_cria_indice_em_ts() {
    let tmp = TempDir::new("reth");
    let s = ScrollbackStore::open(tmp.path(), cfg(4, 2, 30)).expect("open");
    drop(s);
    let conn = rusqlite::Connection::open(tmp.path().join("scrollback.db")).expect("conn");
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='scrollback_ts'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(
        n, 1,
        "índice scrollback_ts ausente — retenção vira full-scan"
    );
}

// ═══════════════════════ F1-5-9 — job diário NO idle-drain (F1-5-6 ∩ F1-5-9) ═══════════════════════

/// O job diário roda NO MESMO idle-drain (thread única — decisão da story): com o
/// guard ligado e o relógio 31 dias à frente, as linhas antigas somem SEM chamada
/// manual de `run_retention`.
#[test]
fn ret_e_job_diario_roda_dentro_do_idle_drain() {
    let tmp = TempDir::new("rete");
    let t0 = 1_750_000_000_000u64;
    let store = {
        let mut s = ScrollbackStore::open(tmp.path(), cfg(4, 2, 30)).expect("open");
        s.set_clock(move || t0);
        for i in 0..6u32 {
            s.push_line("J", format!("j-{i}")).expect("push");
        }
        s.flush("J").expect("flush");
        let t1 = t0 + 31 * DAY_MS;
        s.set_clock(move || t1);
        Arc::new(Mutex::new(s))
    };

    let _guard = FlushGuard::start(
        Arc::clone(&store),
        FlushGuardConfig {
            idle_for: Duration::from_millis(50),
            tick: Duration::from_millis(20),
            handle_signals: false,
        },
    )
    .expect("subir o flush guard");
    assert!(
        poll_until(Duration::from_secs(2), || {
            store.lock().expect("lock").expired_before("J") == 6
        }),
        "o job diário não rodou dentro do idle-drain"
    );
    assert_eq!(
        store.lock().expect("lock").line("J", 0).expect("line"),
        None,
        "linha expirada removida pelo job do guard"
    );
}
