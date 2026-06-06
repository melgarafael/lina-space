//! Gate F1-0-2 — prova contra **claude REAL** (critérios 2/3 da story, versão headless):
//! com o perfil de entrega REAL (`profiles/claude-code.toml`), um `deliver_a2a` disparado
//! com o alvo **OCUPADO** (força-busy, mesmo protocolo §4.4 da baseline) ESPERA o prompt
//! real e injeta LIMPO — o marcador não fica preso na região de input (regra t+3s/t+8s
//! da baseline F1-0-1). A baseline com o `demo_profile()` media **18/20 presos (90%)**;
//! o alvo do DEPOIS é **0/N**.
//!
//! `#[ignore]`: exige o binário `claude` autenticado na máquina e ~2-4 min de parede —
//! roda sob demanda (não no CI):
//! ```sh
//! cargo test -p lina-core --test gate_f102_real_claude -- --ignored --nocapture
//! ```
//! Workspace 100% isolado em `/tmp` — o canvas vivo nunca é tocado.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lina_core::bench::{Grid, LoadHarness};
use lina_core::{
    deliver_a2a, CliProfile, DeliveryOutcome, InjectPolicy, NodeId, Supervisor, WriteOp,
};

const COLS: u16 = 120;
const ROWS: u16 = 40;
const ATTEMPTS: usize = 3;

/// Linhas não-vazias do grid (diagnóstico de timeout/diálogos — a PRONTIDÃO usa o
/// sensor oficial `GridSense::input_region_text`, o mesmo da entrega).
fn grid_rows(grid: &Grid) -> Vec<String> {
    let g = grid.lock().expect("lock grid");
    let (_, nrows) = g.dims();
    (0..nrows)
        .map(|i| g.row_text(i))
        .filter(|r| !r.trim().is_empty())
        .collect()
}

fn region(grid: &Grid) -> String {
    use lina_core::GridSense;
    grid.input_region_text()
}

fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Espera o claude chegar ao prompt REAL (settle de 2s) respondendo diálogos de
/// primeira execução com Enter — mesma receita calibrada do harness da baseline.
fn wait_claude_ready(
    sup: &Arc<Supervisor>,
    node: NodeId,
    grid: &Grid,
    profile: &CliProfile,
    timeout: Duration,
) -> Result<(), String> {
    const DIALOG_MARKERS: &[&str] = &[
        "trust the files",
        "Do you trust",
        "Yes, proceed",
        "Press Enter",
        "to continue",
    ];
    let prompt_re = regex::Regex::new(&profile.prompt_ready_regex)
        .map_err(|e| format!("regex do perfil: {e}"))?;
    let deadline = Instant::now() + timeout;
    let mut first_match: Option<Instant> = None;
    let mut last_enter = Instant::now() - Duration::from_secs(10);
    while Instant::now() < deadline {
        let reg = region(grid);
        let ready = prompt_re.is_match(&reg)
            && !profile
                .busy_markers
                .iter()
                .any(|m| reg.contains(m.as_str()));
        if ready {
            match first_match {
                Some(t0) if t0.elapsed() >= Duration::from_secs(2) => return Ok(()),
                Some(_) => {}
                None => first_match = Some(Instant::now()),
            }
            std::thread::sleep(Duration::from_millis(400));
            continue;
        }
        first_match = None;
        let joined = grid_rows(grid).join("\n");
        if DIALOG_MARKERS.iter().any(|m| joined.contains(m))
            && last_enter.elapsed() > Duration::from_secs(2)
        {
            let _ = sup.enqueue_write(node, WriteOp::Submit { from: None });
            last_enter = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let tail: Vec<String> = grid_rows(grid).into_iter().rev().take(10).collect();
    Err(format!(
        "timeout esperando o prompt; tail do grid: {tail:?}"
    ))
}

/// Digita uma linha como HUMANO (mesma fila serial do teclado do app) — arma o busy.
fn type_line(sup: &Arc<Supervisor>, node: NodeId, text: &str) {
    sup.enqueue_write(node, WriteOp::HumanKeys(text.as_bytes().to_vec()))
        .expect("HumanKeys");
    std::thread::sleep(Duration::from_millis(250));
    sup.enqueue_write(node, WriteOp::Submit { from: None })
        .expect("Submit");
}

#[test]
#[ignore = "exige `claude` autenticado; ~2-4 min; rodar com --ignored --nocapture"]
fn real_claude_busy_target_delivers_clean_with_real_profile() {
    // Perfil REAL — o mesmo arquivo que produção carregará (vigência impressa no log).
    let profile_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/claude-code.toml");
    let profile = CliProfile::load_file(&profile_path).expect("carregar claude-code.toml real");
    eprintln!(
        "[f102] perfil REAL: regex={:?} submit_delay_ms={} ready_timeout_ms={:?} busy_markers={:?} idle_ms={:?}",
        profile.prompt_ready_regex,
        profile.submit_delay_ms,
        profile.ready_timeout_ms,
        profile.busy_markers,
        profile.idle_ms
    );

    let ws = std::env::temp_dir().join(format!("lina-f102-real-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(ws.join("agent")).expect("ws isolado");

    let mut harness = LoadHarness::new(COLS, ROWS);
    let cmd = lina_core::PtyCommand::new("claude")
        .cwd(ws.join("agent"))
        .env("TERM", "xterm-256color");
    let target = harness
        .spawn_terminal("Alvo", "worker", cmd)
        .expect("spawn claude real");
    let grid = Arc::clone(harness.live().last().expect("vivo").grid());
    let sup = Arc::clone(harness.supervisor());
    let from = sup.register("Probe", None, Box::new(std::io::sink()));

    wait_claude_ready(&sup, target, &grid, &profile, Duration::from_secs(90))
        .expect("claude deve chegar ao prompt real");
    eprintln!("[f102] claude pronto no prompt real");

    let stop = Arc::new(AtomicBool::new(false));
    let mut clean = 0usize;
    let mut stuck = 0usize;

    for k in 0..ATTEMPTS {
        // 1) Força-busy (§4.4): tarefa que faz o claude streamar por vários segundos.
        type_line(
            &sup,
            target,
            "Conte de 1 a 25, um número por linha, devagar, sem usar ferramentas.",
        );
        // Ocupado DE VERDADE = o próprio sensor de prontidão diz "não pronto".
        let prompt_re = regex::Regex::new(&profile.prompt_ready_regex).expect("regex");
        let became_busy = poll_until(Duration::from_secs(30), || {
            let reg = region(&grid);
            // "não pronto" = sem linha-prompt OU com busy marker presente.
            !prompt_re.is_match(&reg)
                || profile
                    .busy_markers
                    .iter()
                    .any(|m| reg.contains(m.as_str()))
        });
        assert!(became_busy, "força-busy: o alvo deve sair da prontidão");
        eprintln!("[f102] tentativa {k}: alvo OCUPADO no instante da entrega");

        // 2) Entrega com o perfil REAL — deve ESPERAR o prompt e injetar limpo.
        let marker = format!("PROBE-F102-{k}-{}", std::process::id());
        let payload = format!("{marker} — mensagem de teste do gate; NÃO responda, ignore.");
        let t0 = Instant::now();
        let out = deliver_a2a(
            &sup,
            target,
            from,
            &payload,
            &profile,
            &grid,
            InjectPolicy::AllowAll,
        );
        let waited = t0.elapsed();
        match out {
            Ok(DeliveryOutcome::Injected { ready, .. }) => {
                assert!(ready, "pós-fix só se injeta com prontidão verificada");
                // 3) Regra da baseline: marcador na REGIÃO DE INPUT em t+3s e t+8s = preso.
                std::thread::sleep(Duration::from_secs(3));
                let at3 = region(&grid).contains(&marker);
                std::thread::sleep(Duration::from_secs(5));
                let at8 = region(&grid).contains(&marker);
                if at3 && at8 {
                    stuck += 1;
                    eprintln!(
                        "[f102] tentativa {k}: PRESO (esperou {waited:?}); região t+8s: {:?}",
                        region(&grid)
                    );
                } else {
                    clean += 1;
                    eprintln!("[f102] tentativa {k}: LIMPO (esperou {waited:?} até o prompt real)");
                }
            }
            Ok(other) => panic!("desfecho inesperado: {other:?}"),
            Err(e) => panic!("entrega falhou (budget {waited:?}): {e}"),
        }
        // settle entre tentativas (o claude termina de processar o probe entregue).
        std::thread::sleep(Duration::from_secs(4));
        let _ = wait_claude_ready(&sup, target, &grid, &profile, Duration::from_secs(60));
    }
    stop.store(true, Ordering::Relaxed);

    eprintln!("[f102] TAXA DEPOIS: {stuck}/{ATTEMPTS} presos · {clean}/{ATTEMPTS} limpos (baseline ANTES: 18/20 presos)");
    assert_eq!(
        stuck, 0,
        "critério 3 da F1-0-2: texto-colado deve cair a 0/N com o perfil real"
    );
    let _ = std::fs::remove_dir_all(&ws);
}
