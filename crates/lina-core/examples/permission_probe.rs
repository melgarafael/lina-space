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
//! Exemplo (dev-only — usa `lina-hooks`/tokio das dev-dependencies; nada entra na lib):
//! `cargo run -p lina-core --example permission_probe`

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lina_core::permission_detect::{is_permission_prompt_line, HookSignal, PermissionDetector};
use lina_core::{AlacrittyBackend, EventStore, PtyCommand, PtyManager, VtBackend};
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
        "== F1-1-6 · probe de latência ==\nevent store: {}\n",
        store_dir.display()
    );

    phase_a_claude_real(&mut store).await?;
    phase_b_bash_read_fallback(&mut store)?;

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
