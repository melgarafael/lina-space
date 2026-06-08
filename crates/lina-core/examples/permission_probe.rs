//! F1-1-6 · Probe de LATÊNCIA REAL da detecção de permissão (critérios 1 e 3).
//!
//! **Fase A (hook primário, critério 1):** spawna um **Claude Code real** num PTY
//! (`lina-pty`), com `.claude/settings.json` apontando os hooks de observabilidade
//! (formato idêntico ao do `lina-bootstrap` F1-1-3) para um `HookListener` REAL
//! (`lina-hooks`). Um prompt que exige Bash dispara o pedido de permissão de verdade →
//! mede-se **chegada da `Notification` → `PermissionAsked` apendado no log** (e, como
//! cross-check, o instante em que o prompt ficou visível nos bytes do PTY).
//!
//! **Fase B (fallback de grid, critério 3):** spawna um script bash com `read` de
//! y/n (CLI sem hook) → grid VT real (`AlacrittyBackend` atrás da trait) → fallback
//! detecta → mede **prompt visível no grid → evento apendado** (inclui a espera de
//! idle, que é constitutiva do mecanismo anti-#28174).
//!
//! **Fase C (R2b — caixa de escolha REAL, critério de aceite F1-1-7):** spawna um
//! **Claude Code real SEM hooks** (cwd isolado — isola o caminho de GRID, a única via
//! que pode emitir) e o leva a renderizar uma **caixa de múltipla escolha**
//! (`AskUserQuestion`). Prova que `scan_choice`/[`is_choice_chrome_line`] dispara
//! contra a saída REAL do CLI — não contra um rodapé sintético — e **captura o rodapé
//! de chrome REAL** para validar a âncora (calibrada na screenshot do fundador
//! 2026-06-07, nunca antes batida programaticamente). Cross-check de precisão embutido:
//! o detector NÃO pode emitir durante a prosa/thinking ANTES da caixa (emissão precoce
//! = FP real capturado contra Claude, contado na telemetria).
//!
//! Exemplo (dev-only — usa `lina-hooks`/tokio das dev-dependencies; nada entra na lib).
//! Todas as fases: `cargo run -p lina-core --example permission_probe`
//! Só uma fase (ex.: a C, que exige só `claude` autenticado, ~1-2 min):
//! `cargo run -p lina-core --example permission_probe -- c`

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lina_core::permission_detect::{
    is_choice_chrome_line, is_permission_prompt_line, HookSignal, PermissionDetector,
};
use lina_core::{AlacrittyBackend, EventStore, PromptKind, PtyCommand, PtyManager, VtBackend};
use lina_hooks::{HookKind, HookListener};

const COLS: u16 = 100;
const ROWS: u16 = 30;
/// Fase A: teto de espera pelo pedido real do Claude (boot + turno + gate).
const CLAUDE_DEADLINE_S: u64 = 120;
/// Fase B: cadência de amostragem do grid e janela de idle do fallback.
const SAMPLE_MS: u64 = 100;
const IDLE_MS: u64 = 1_200;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Single-quote shell-safe (mesmo padrão do bootstrap: `'…'` com escape de `'`).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Settings de observabilidade — MESMO shape do `lina-bootstrap`
/// (`hook_settings_json_with_observability`): handler HTTP assíncrono por kind.
fn observability_settings(port: u16, token: &str) -> anyhow::Result<String> {
    let mut hooks = serde_json::Map::new();
    for kind in ["PreToolUse", "PostToolUse", "Notification", "Stop"] {
        let url = format!("http://127.0.0.1:{port}/hook/{token}/{kind}");
        // Espelha o wiring de produção (bootstrap F1-1-3): http + async em todos.
        // MEDIDO (Claude Code 2.1.168, 4 rodadas): a Notification chega ~5,8s após o
        // diálogo ficar visível — atraso INTRÍNSECO do CLI; nem `async:false` nem
        // `messageIdleNotifThresholdMs:500` o mudam (ambos testados, sem efeito).
        // → decisão de design p/ a onda: fallback de grid também em CLIs com hook
        //   (~1,5s) com dedupe entre camadas, ou aceitar ~6s — registrado no relatório.
        hooks.insert(
            kind.to_string(),
            serde_json::json!([{ "hooks": [{ "type": "http", "url": url, "async": true }] }]),
        );
    }
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({ "hooks": hooks }),
    )?)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let store_dir = std::env::temp_dir().join(format!("lina-probe-f116-{}", uuid::Uuid::now_v7()));
    let mut store = EventStore::open(&store_dir)?;
    println!(
        "== F1-1-6/7 · probe de latência ==\nevent store: {}\n",
        store_dir.display()
    );

    // Seletor de fases: sem args roda todas; `-- a`, `-- b`, `-- c` (ou `all`) selecionam.
    // Permite rodar a Fase C (Claude+caixa de escolha, R2b) isolada das fases A/B.
    let sel: Vec<String> = std::env::args().skip(1).map(|s| s.to_lowercase()).collect();
    let run = |p: &str| sel.is_empty() || sel.iter().any(|s| s == p || s == "all");

    if run("a") {
        phase_a_claude_real(&mut store).await?;
    }
    if run("b") {
        phase_b_bash_read_fallback(&mut store)?;
    }
    if run("c") {
        phase_c_choice_grid(&mut store)?;
    }

    // Replay não duplica (evidência adicional do critério 4 com eventos REAIS).
    let count = |s: &EventStore| -> anyhow::Result<usize> {
        Ok(s.events()?
            .into_iter()
            .filter(|r| r.kind == "PermissionAsked")
            .count())
    };
    let before = count(&store)?;
    store.project()?;
    store.project()?;
    println!(
        "\nreplay: {before} PermissionAsked no log antes e {} depois de 2 projeções",
        count(&store)?
    );
    Ok(())
}

// ───────────────────────── fase A · Claude real + hooks ─────────────────────────

async fn phase_a_claude_real(store: &mut EventStore) -> anyhow::Result<()> {
    println!("— FASE A: Claude Code real, detecção primária (hook) —");
    let listener = HookListener::bind().await?;
    let token = listener.register_node("probe-claude");
    let port = listener.local_addr().port();
    let mut rx = listener.subscribe();

    // cwd temporário com os hooks de observabilidade instalados.
    let cwd = std::env::temp_dir().join(format!("lina-probe-cwd-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(cwd.join(".claude"))?;
    std::fs::write(
        cwd.join(".claude/settings.json"),
        observability_settings(port, &token)?,
    )?;

    let mut manager = PtyManager::new();
    // `curl` NÃO está no allowlist global da máquina (auditado no probe-run) — dispara
    // o gate com certeza. `--permission-mode default` NEUTRALIZA um eventual
    // `defaultMode: bypassPermissions` do settings do usuário (achado REAL desta
    // máquina: com bypass, permissão NUNCA é pedida e a detecção não tem o que ver —
    // nota operacional para a fiação do spawn no app).
    let prompt = "Use the Bash tool to run exactly this command: curl -s https://example.com \
                  -o /dev/null && echo probe-done . Do not ask me anything, just run it.";
    // Wrapper bash: LIMPA as env vars de sessão aninhada (o probe roda de dentro de
    // outra sessão de agente; o claude filho precisa se ver standalone).
    let wrapped = format!(
        "unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT CLAUDE_CODE_SESSION_ID CLAUDE_CODE_EXECPATH; \
         exec claude --permission-mode default {}",
        shell_quote(prompt)
    );
    let cmd = PtyCommand::new("/bin/bash")
        .arg("-lc")
        .arg(wrapped)
        .env("TERM", "xterm-256color")
        .cwd(&cwd);
    manager.spawn("probe-claude", cmd, COLS, ROWS)?;
    let writer = Arc::new(Mutex::new(manager.take_writer("probe-claude")?));

    // Reader → GRID VT (não bytes crus!): o TUI do Claude posiciona palavra-a-palavra
    // com escapes de cursor, então frases NUNCA aparecem contíguas no stream — só o
    // grid parseado reconstrói o texto legível (a mesma razão de o fallback da story
    // ler pela trait, não por OCR/stream). Dump cru mantido p/ diagnóstico.
    let grid: Arc<Mutex<AlacrittyBackend>> =
        Arc::new(Mutex::new(AlacrittyBackend::new(COLS, ROWS)));
    {
        let mut reader = manager.clone_reader("probe-claude")?;
        let g = Arc::clone(&grid);
        std::thread::spawn(move || {
            let mut dump =
                std::fs::File::create(std::env::temp_dir().join("f116-claude-pty-dump.log")).ok();
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Some(f) = dump.as_mut() {
                            let _ = f.write_all(&buf[..n]);
                        }
                        lock(&g).advance(&buf[..n]);
                    }
                }
            }
        });
    }

    let mut det = PermissionDetector::new();
    let deadline = Instant::now() + Duration::from_secs(CLAUDE_DEADLINE_S);
    let mut trust_answered = false;
    let mut prompt_seen: Option<Instant> = None;
    let mut hooks_log: Vec<String> = Vec::new();
    let mut outcome: Option<(u64, Instant)> = None; // (ts chegada notification, instante append)

    while Instant::now() < deadline {
        {
            let g = lock(&grid);
            // Diálogo de confiança de pasta nova: Enter aceita o default e segue.
            if !trust_answered && g.row_text_contains("trust this folder") {
                drop(g);
                lock(&writer).write_all(b"\r")?;
                trust_answered = true;
                println!("  (trust dialog respondido com Enter)");
            } else {
                // Cross-check: instante em que o DIÁLOGO de permissão ficou visível.
                // Needle = headline do diálogo ("Do you want to proceed?"); needles
                // genéricos ("permission") casam com a barra de status do TUI e
                // adiantam o carimbo (medição v4 deu 5.8s falsos).
                if prompt_seen.is_none() && g.row_text_contains("Do you want") {
                    prompt_seen = Some(Instant::now());
                }
            }
        }
        let ev = match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Err(_) => continue,
            Ok(Err(_)) => continue,
            Ok(Ok(ev)) => ev,
        };
        hooks_log.push(format!("{} tool={:?}", ev.kind.as_str(), ev.tool_name));
        let signal = match ev.kind {
            HookKind::PreToolUse => Some(HookSignal::PreToolUse {
                tool: ev.tool_name.as_deref(),
                detail: None,
            }),
            HookKind::PostToolUse => Some(HookSignal::PostToolUse),
            HookKind::Notification => Some(HookSignal::Notification),
            HookKind::Stop => Some(HookSignal::TurnEnd),
            _ => None,
        };
        let Some(signal) = signal else { continue };
        if let Some(ask) = det.observe_hook(&ev.node_id, signal, ev.ts) {
            store.append(&ask.to_event())?;
            let appended_at = Instant::now();
            let hook_to_event_ms = now_ms().saturating_sub(ev.ts);
            println!("  hooks recebidos até aqui: {hooks_log:?}");
            println!(
                "  PermissionAsked REAL: tool={:?} stable_id={}",
                ask.tool, ask.stable_id
            );
            println!("  LATÊNCIA hook(Notification)→evento no log: {hook_to_event_ms} ms");
            if let Some(seen) = prompt_seen {
                let d = appended_at.saturating_duration_since(seen);
                println!(
                    "  cross-check prompt visível no grid→evento: {} ms",
                    d.as_millis()
                );
            }
            outcome = Some((ev.ts, appended_at));
            break;
        }
    }

    // Recusa o pedido (Esc) e encerra o Claude — o probe só mede, nunca aprova.
    let _ = lock(&writer).write_all(b"\x1b");
    std::thread::sleep(Duration::from_millis(300));
    let _ = manager.kill("probe-claude", Duration::from_secs(3));
    let _ = std::fs::remove_dir_all(&cwd);

    match outcome {
        Some(_) => Ok(()),
        None => {
            println!(
                "  SEM detecção dentro de {CLAUDE_DEADLINE_S}s. hooks recebidos: {hooks_log:?}"
            );
            anyhow::bail!("fase A falhou: nenhum PermissionAsked com Claude real")
        }
    }
}

// ───────────────────── fase B · bash read y/n via fallback ─────────────────────

fn phase_b_bash_read_fallback(store: &mut EventStore) -> anyhow::Result<()> {
    println!("\n— FASE B: script bash com read y/n (CLI sem hook), fallback de grid —");
    let mut manager = PtyManager::new();
    let script = r#"echo preparando ambiente; sleep 1; read -p "Continue? (y/n) " ans; echo; echo "resposta: $ans"; sleep 1"#;
    let cmd = PtyCommand::new("/bin/bash").arg("-c").arg(script);
    manager.spawn("probe-bash", cmd, COLS, ROWS)?;
    let mut writer = manager.take_writer("probe-bash")?;

    let grid: Arc<Mutex<AlacrittyBackend>> =
        Arc::new(Mutex::new(AlacrittyBackend::new(COLS, ROWS)));
    {
        let mut reader = manager.clone_reader("probe-bash")?;
        let g = Arc::clone(&grid);
        std::thread::spawn(move || {
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => lock(&g).advance(&buf[..n]),
                }
            }
        });
    }

    let mut det = PermissionDetector::new();
    let mut prompt_visible_at: Option<Instant> = None;
    let mut last_change = (0_u64, Instant::now());
    let deadline = Instant::now() + Duration::from_secs(30);

    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(SAMPLE_MS));
        let guard = lock(&grid);
        let vt: &dyn VtBackend = &*guard;
        // idle = grid sem mudança por IDLE_MS (fingerprint barato da última linha+cursor).
        let fp = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            vt.last_nonempty_line().hash(&mut h);
            let s = vt.screen();
            (s.cursor.col, s.cursor.line).hash(&mut h);
            h.finish()
        };
        if fp != last_change.0 {
            last_change = (fp, Instant::now());
        }
        if prompt_visible_at.is_none() && is_permission_prompt_line(&vt.last_nonempty_line()) {
            prompt_visible_at = Some(Instant::now());
        }
        let idle = last_change.1.elapsed() >= Duration::from_millis(IDLE_MS);
        if let Some(ask) = det.observe_grid("probe-bash", vt, idle, now_ms()) {
            drop(guard);
            store.append(&ask.to_event())?;
            let total = prompt_visible_at
                .map(|t| t.elapsed().as_millis())
                .unwrap_or_default();
            println!(
                "  PermissionAsked via fallback: evidence=grid detail={:?} stable_id={}",
                ask.detail, ask.stable_id
            );
            println!(
                "  LATÊNCIA prompt visível no grid→evento no log: {total} ms \
                 (inclui a janela de idle de {IDLE_MS} ms, constitutiva do anti-FP)"
            );
            // Destrava o script de verdade: responde y (prova de que era interativo).
            writer.write_all(b"y\r")?;
            std::thread::sleep(Duration::from_millis(600));
            let answered = lock(&grid).row_text_contains("resposta: y");
            println!("  script destravou após o y: {answered}");
            let _ = manager.kill("probe-bash", Duration::from_secs(2));
            return Ok(());
        }
    }
    let _ = manager.kill("probe-bash", Duration::from_secs(2));
    anyhow::bail!("fase B falhou: fallback não detectou o read y/n em 30s")
}

// ─────────────── fase C · Claude real + caixa de escolha (grid/chrome, R2b) ───────────────

/// Desfecho de UMA amostra da Fase C (nomeia a tupla — clippy::type_complexity).
struct ChoiceHit {
    ask: lina_core::permission_detect::PermissionAsk,
    /// O rodapé de chrome estava visível no MESMO frame da emissão? (`false` = a prosa
    /// disparou a âncora antes da caixa — FP de precisão capturado contra Claude real.)
    chrome_visible: bool,
    /// `(row, texto)` do rodapé casado — evidência forense do chrome REAL no log.
    footer: Option<(usize, String)>,
}

/// Fase C: um **Claude Code real SEM hooks** renderiza uma caixa de múltipla escolha
/// (`AskUserQuestion`); prova que o caminho de GRID/CHROME ([`is_choice_chrome_line`] +
/// `scan_choice`, dentro de `PermissionDetector::observe_grid`) dispara contra a saída
/// REAL — e captura o **rodapé de chrome REAL** para validar a âncora. O critério de
/// aceite da story é exatamente este: o detector dispara a fila a partir de saída REAL
/// de CLI (não do modo demo `LINA_ATTENTION_DEMO`).
fn phase_c_choice_grid(store: &mut EventStore) -> anyhow::Result<()> {
    println!("\n— FASE C: Claude real, caixa de escolha via GRID/CHROME (R2b) —");
    // cwd isolado SEM `.claude/settings.json` de hooks: o único caminho que pode emitir
    // aqui é o fallback de grid — isola a prova da âncora de chrome (sem a camada 1).
    let cwd = std::env::temp_dir().join(format!("lina-probe-c-cwd-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&cwd)?;

    let mut manager = PtyManager::new();
    // Imperativo sobre usar a TOOL (não só "perguntar"): a caixa navegável só nasce do
    // AskUserQuestion. `--permission-mode default` neutraliza um eventual bypass do
    // settings do usuário (achado da Fase A) — AskUserQuestion não pede aprovação, mas
    // o flag mantém o ambiente do probe idêntico ao da detecção real.
    let prompt = "Use the AskUserQuestion tool right now to ask me to pick my favorite \
                  color. Provide exactly three options: Vermelho, Verde, Azul. Ask only \
                  this one question and wait for my selection — do not write any other \
                  text and do not answer it yourself.";
    let wrapped = format!(
        "unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT CLAUDE_CODE_SESSION_ID CLAUDE_CODE_EXECPATH; \
         exec claude --permission-mode default {}",
        shell_quote(prompt)
    );
    let cmd = PtyCommand::new("/bin/bash")
        .arg("-lc")
        .arg(wrapped)
        .env("TERM", "xterm-256color")
        .cwd(&cwd);
    manager.spawn("probe-choice", cmd, COLS, ROWS)?;
    let writer = Arc::new(Mutex::new(manager.take_writer("probe-choice")?));

    let grid: Arc<Mutex<AlacrittyBackend>> =
        Arc::new(Mutex::new(AlacrittyBackend::new(COLS, ROWS)));
    let dump_path = std::env::temp_dir().join("f117-choice-pty-dump.log");
    {
        let mut reader = manager.clone_reader("probe-choice")?;
        let g = Arc::clone(&grid);
        let dp = dump_path.clone();
        std::thread::spawn(move || {
            let mut dump = std::fs::File::create(dp).ok();
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Some(f) = dump.as_mut() {
                            let _ = f.write_all(&buf[..n]);
                        }
                        lock(&g).advance(&buf[..n]);
                    }
                }
            }
        });
    }

    let mut det = PermissionDetector::new();
    let deadline = Instant::now() + Duration::from_secs(CLAUDE_DEADLINE_S);
    let mut trust_answered = false;
    let mut question_seen: Option<Instant> = None;
    let mut chrome_seen: Option<Instant> = None;
    let mut last_change = (0_u64, Instant::now());
    let mut early_emit = false;

    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(SAMPLE_MS));

        // Um único lock por amostra: idle, trust, chrome, pergunta e detecção coerentes.
        let mut do_trust = false;
        let mut question_now = false;
        let mut chrome_now = false;
        let mut emitted: Option<ChoiceHit> = None;
        {
            let g = lock(&grid);
            let vt: &dyn VtBackend = &*g;
            let (_, rows) = vt.dims();

            // idle = grid sem mudança por IDLE_MS (fingerprint barato: última linha+cursor).
            let fp = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                vt.last_nonempty_line().hash(&mut h);
                let s = vt.screen();
                (s.cursor.col, s.cursor.line).hash(&mut h);
                h.finish()
            };
            if fp != last_change.0 {
                last_change = (fp, Instant::now());
            }
            let idle = last_change.1.elapsed() >= Duration::from_millis(IDLE_MS);

            // Trust dialog (pasta nova): Enter aceita — NÃO é a caixa-alvo.
            let trust = (0..rows).any(|r| {
                let t = vt.row_text(r).to_lowercase();
                t.contains("trust") && (t.contains("folder") || t.contains("files"))
            });
            if trust && !trust_answered {
                do_trust = true;
            } else {
                question_now = (0..rows).any(|r| {
                    let t = vt.row_text(r).to_lowercase();
                    t.contains("cor favorita") || t.contains("favorite color")
                });
                let footer = (0..rows).find_map(|r| {
                    let t = vt.row_text(r);
                    is_choice_chrome_line(&t).then(|| (r, t.trim().to_string()))
                });
                chrome_now = footer.is_some();
                if let Some(ask) = det.observe_grid("probe-choice", vt, idle, now_ms()) {
                    emitted = Some(ChoiceHit {
                        ask,
                        chrome_visible: chrome_now,
                        footer,
                    });
                }
            }
        }

        if do_trust {
            let _ = lock(&writer).write_all(b"\r");
            trust_answered = true;
            last_change = (0, Instant::now()); // reseta idle pós-resposta
            println!("  (trust dialog respondido com Enter)");
            continue;
        }
        if question_now && question_seen.is_none() {
            question_seen = Some(Instant::now());
        }
        // Instante em que o RODAPÉ ficou visível — separa a latência do DETECTOR (chrome
        // estável → evento ≈ idle + 1 tick) da latência de RENDER do Claude (pergunta →
        // chrome). Sem isso, o número total confundiria "detector lento" com "TUI lenta".
        if chrome_now && chrome_seen.is_none() {
            chrome_seen = Some(Instant::now());
        }

        if let Some(ChoiceHit {
            ask,
            chrome_visible,
            footer,
        }) = emitted
        {
            // Emissão SEM o rodapé de chrome na tela = a prosa/lista disparou a âncora:
            // FP REAL capturado contra Claude (o que o cross-check da Fase C existe para
            // pegar). Conta na telemetria e segue medindo — não aborta.
            if !chrome_visible {
                early_emit = true;
                det.record_false_positive();
                println!(
                    "  ⚠ FP (precisão): emissão sem rodapé de chrome — kind={:?} detail={:?}",
                    ask.kind, ask.detail
                );
                continue;
            }
            store.append(&ask.to_event())?;
            let now = Instant::now();
            let lat_question = question_seen.map(|t| now.duration_since(t).as_millis());
            let lat_chrome = chrome_seen.map(|t| now.duration_since(t).as_millis());
            println!(
                "  PermissionAsked via GRID/CHROME: kind={:?} evidence={:?}",
                ask.kind, ask.evidence
            );
            println!("  detail (pergunta no toast): {:?}", ask.detail);
            if let Some((row, line)) = footer.as_ref() {
                println!("  RODAPÉ DE CHROME REAL casado (row {row}): {line:?}");
            }
            // Duas medições honestas: o detector é responsável só pela 2ª (chrome→evento).
            println!(
                "  LATÊNCIA detector (rodapé estável→evento): {} ms (idle {IDLE_MS} ms + ticks — a parte do MECANISMO)",
                lat_chrome.map_or_else(|| "n/d".into(), |m| m.to_string())
            );
            println!(
                "  LATÊNCIA UX total (pergunta visível→evento): {} ms (inclui o RENDER da caixa pelo Claude)",
                lat_question.map_or_else(|| "n/d".into(), |m| m.to_string())
            );
            println!(
                "  telemetria: emitted_choice={} false_positives={} (early_emit={early_emit})",
                det.telemetry().emitted_choice,
                det.telemetry().false_positives
            );

            // O probe só MEDE: recusa (Esc) e encerra — nunca seleciona.
            let _ = lock(&writer).write_all(b"\x1b");
            std::thread::sleep(Duration::from_millis(300));
            let _ = manager.kill("probe-choice", Duration::from_secs(3));
            let _ = std::fs::remove_dir_all(&cwd);

            if ask.kind != PromptKind::Choice {
                anyhow::bail!("fase C: emitiu mas kind={:?} (esperado Choice)", ask.kind);
            }
            return Ok(());
        }
    }

    // Sem detecção dentro do prazo: dump das últimas linhas para diagnóstico/recalibração
    // (a âncora pode não casar o chrome real desta versão — é o que a Fase C revela).
    let tail: Vec<String> = {
        let g = lock(&grid);
        let (_, rows) = g.dims();
        (0..rows)
            .map(|r| g.row_text(r))
            .filter(|t| !t.trim().is_empty())
            .collect()
    };
    println!("  SEM detecção em {CLAUDE_DEADLINE_S}s (early_emit={early_emit}).");
    println!("  dump cru do PTY: {}", dump_path.display());
    println!("  últimas linhas do grid (para calibrar a âncora is_choice_chrome_line):");
    for l in tail.iter().rev().take(18).rev() {
        println!("    | {l}");
    }
    let _ = manager.kill("probe-choice", Duration::from_secs(3));
    let _ = std::fs::remove_dir_all(&cwd);
    anyhow::bail!("fase C falhou: nenhuma caixa de escolha detectada com Claude real")
}

/// Pequena extensão local: o grid contém o texto em alguma linha do viewport?
trait GridContains {
    fn row_text_contains(&self, needle: &str) -> bool;
}
impl GridContains for AlacrittyBackend {
    fn row_text_contains(&self, needle: &str) -> bool {
        let (_, rows) = self.dims();
        (0..rows).any(|r| self.row_text(r).contains(needle))
    }
}
