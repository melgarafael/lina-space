//! Entrega A2A faseada (W0-9) + contrato de fim-de-resposta (W0-10).
//!
//! ## W0-9 — injeção no PTY vivo, FASEADA (a correção load-bearing do debate)
//! Entregar texto de A dentro da sessão viva de B **nunca** é `texto + \r` num write
//! só (Ink/REPL tratam `\r` programático ≠ Enter físico). A sequência correta, via a
//! **fila serial** do terminal (1 dono do writer):
//! 1. `wait_ready` — espera o grid de B ficar pronto (`prompt_ready_regex`);
//! 2. `AgentText` = `build_paste(texto)` — bracketed-paste **se** `mode().bracketed_paste`,
//!    com o payload **sanitizado** (sem `ESC[201~`, CVE-2021-31701) e **sem `\r`**;
//! 3. espera `submit_delay` (~0.3s, do CLI Profile);
//! 4. `Submit` (`0x0D`) como `WriteOp` **separado** — é o que de fato submete.
//!
//! `delivery = session_resume` é o fallback headless (stub documentado na Onda 0).
//!
//! ## W0-10 — fim-de-resposta: `result/sentinela > idle do grid > timeout`
//! [`EndDetector`] decide deterministicamente quando o turno de B terminou, na ordem:
//! evento `result` do NDJSON (caminho-ouro) > idle do **grid já parseado** (damage
//! estável por `idle_ms` + prompt pronto, nunca OCR) > **timeout duro** (rede de
//! segurança, sempre presente). No timeout, devolve `truncated: true` — **nunca**
//! truncamento silencioso. Usa relógio LÓGICO (ms) → testes determinísticos sem `sleep`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lina_cli_profiles::{CliProfile, Delivery, EndSignal};
use lina_vt::{TermMode, VtBackend};
use regex::Regex;
use thiserror::Error;

use crate::{lock, NodeId, Supervisor, SupervisorError, WriteOp, Writer};

/// Início e fim de uma colagem bracketed-paste (xterm).
const PASTE_BEGIN: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const PASTE_END_STR: &str = "\x1b[201~";
const PASTE_BEGIN_STR: &str = "\x1b[200~";

/// Tempo máximo esperando o prompt ficar pronto antes de injetar mesmo assim.
const READY_TIMEOUT: Duration = Duration::from_secs(2);
/// Intervalo de polling do grid no `wait_ready`.
const READY_POLL: Duration = Duration::from_millis(10);
/// Timeout para adquirir o lock lógico do PTY do alvo.
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout duro default de um `ask` A2A (rede de segurança do W0-10).
pub const DEFAULT_ASK_TIMEOUT_MS: u64 = 120_000;
/// Janela de quietude default que conta como "idle" (W0-10).
pub const DEFAULT_IDLE_MS: u64 = 500;

/// Erros da entrega A2A.
#[derive(Debug, Error)]
pub enum A2aError {
    /// O `prompt_ready_regex` do CLI Profile não compila.
    #[error("prompt_ready_regex inválido: {0}")]
    BadRegex(String),
    /// Política de injeção negou este par (allow-list por nó).
    #[error("injeção de {from} em {target} negada pela allow-list")]
    InjectionDenied { from: NodeId, target: NodeId },
    /// Erro vindo do supervisor (alvo inexistente, lock, etc.).
    #[error(transparent)]
    Supervisor(#[from] SupervisorError),
}

// ───────────────────────────── leitura do grid (sensing) ─────────────────────────────

/// Leitura `Send + Sync` do grid parseado do alvo — desacopla a entrega A2A do dono
/// concreto do `VtBackend` (o pty-host) e permite mock em teste.
pub trait GridSense: Send + Sync {
    fn mode(&self) -> TermMode;
    fn last_nonempty_line(&self) -> String;
}

/// O grid do pty-host vive atrás de `Arc<Mutex<Box<dyn VtBackend>>>`; lê sob lock.
impl GridSense for Arc<Mutex<Box<dyn VtBackend>>> {
    fn mode(&self) -> TermMode {
        lock(self).mode()
    }
    fn last_nonempty_line(&self) -> String {
        lock(self).last_nonempty_line()
    }
}

// ───────────────────────────── construção do payload ─────────────────────────────

/// Remove os marcadores de bracketed-paste (`ESC[200~`/`ESC[201~`) do payload. O
/// `ESC[201~` é o vetor do CVE-2021-31701 (fecharia o bloco e escaparia da colagem);
/// removemos os dois por segurança — o wrapper legítimo é adicionado por [`build_paste`].
#[must_use]
pub fn sanitize_paste(text: &str) -> String {
    text.replace(PASTE_END_STR, "").replace(PASTE_BEGIN_STR, "")
}

/// Bytes do passo "colar texto" da fila serial. Em bracketed-paste mode, embrulha em
/// `ESC[200~ … ESC[201~`; senão, texto puro. SEMPRE sanitiza e **nunca** inclui `\r`
/// (o Enter é um [`WriteOp::Submit`] separado — a correção load-bearing).
#[must_use]
pub fn build_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let clean = sanitize_paste(text);
    if bracketed {
        let mut out = Vec::with_capacity(clean.len() + PASTE_BEGIN.len() + PASTE_END.len());
        out.extend_from_slice(PASTE_BEGIN);
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(PASTE_END);
        out
    } else {
        clean.into_bytes()
    }
}

fn wait_ready(grid: &dyn GridSense, prompt: &Regex, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if prompt.is_match(&grid.last_nonempty_line()) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(READY_POLL);
    }
}

/// Resultado da entrega A2A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Injetado no PTY vivo (faseado). `ready` = prompt detectado; `bracketed` = modo do alvo.
    Injected { ready: bool, bracketed: bool },
    /// `delivery = session_resume`: fallback headless (stub W0-9; em produção seria
    /// `claude --resume <id>` / `codex exec resume`).
    SessionResume,
}

/// Política de allow-list de injeção (gancho de segurança — A2A é vetor de
/// prompt-injection agente↔agente). Default permissivo no MVP headless, mas presente.
#[derive(Debug, Clone, Copy)]
pub enum InjectPolicy<'a> {
    /// Qualquer nó pode injetar em qualquer nó (default).
    AllowAll,
    /// Só os pares `(from, target)` listados podem injetar.
    AllowOnly(&'a [(NodeId, NodeId)]),
    /// Todos podem, exceto os pares `(from, target)` listados.
    Deny(&'a [(NodeId, NodeId)]),
}

impl InjectPolicy<'_> {
    fn allows(&self, from: NodeId, target: NodeId) -> bool {
        match self {
            InjectPolicy::AllowAll => true,
            InjectPolicy::AllowOnly(pairs) => pairs.contains(&(from, target)),
            InjectPolicy::Deny(pairs) => !pairs.contains(&(from, target)),
        }
    }
}

/// **W0-9.** Entrega `text` de `from` ao terminal vivo `target`, faseado, pela fila
/// serial do supervisor. `grid` é a leitura do grid do alvo (mode + prompt). `policy`
/// é a allow-list (use `InjectPolicy::AllowAll` no MVP).
pub fn deliver_a2a(
    sup: &Supervisor,
    target: NodeId,
    from: NodeId,
    text: &str,
    profile: &CliProfile,
    grid: &dyn GridSense,
    policy: InjectPolicy<'_>,
) -> Result<DeliveryOutcome, A2aError> {
    if !policy.allows(from, target) {
        return Err(A2aError::InjectionDenied { from, target });
    }

    if profile.delivery == Delivery::SessionResume {
        // Fallback headless: em produção, retoma a sessão (`--resume`). Stub na Onda 0
        // (sem CLI real no teste) — NÃO toca na fila do PTY.
        tracing::info!(%target, "delivery=session_resume: fallback headless (stub W0-9)");
        return Ok(DeliveryOutcome::SessionResume);
    }

    // 1) wait_ready: o prompt do alvo está pronto? (compila o regex 1x).
    let prompt =
        Regex::new(&profile.prompt_ready_regex).map_err(|e| A2aError::BadRegex(e.to_string()))?;
    let ready = wait_ready(grid, &prompt, READY_TIMEOUT);

    // 2) bracketed só se o alvo está em BRACKETED_PASTE mode.
    let bracketed = grid.mode().bracketed_paste;
    let paste = build_paste(text, bracketed); // sanitiza ESC[201~; sem `\r`

    // 3) sequência FASEADA sob o lock serial (humano/outro agente nunca entram no meio):
    //    AgentText → submit_delay → Submit (Enter separado).
    let _guard = sup.lock_pty(target, Writer::Agent(from), LOCK_TIMEOUT)?;
    sup.enqueue_write(target, WriteOp::AgentText { from, bytes: paste })?;
    std::thread::sleep(profile.submit_delay());
    sup.enqueue_write(target, WriteOp::Submit { from: Some(from) })?;
    // _guard solto aqui → libera o terminal.

    Ok(DeliveryOutcome::Injected { ready, bracketed })
}

// ───────────────────────────── W0-10: fim-de-resposta ─────────────────────────────

/// Como um turno de resposta terminou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOutcome {
    /// Evento `result`/sentinela determinístico no NDJSON (caminho-ouro).
    Completed,
    /// Idle do grid: silêncio por `idle_ms` + prompt pronto (fallback).
    IdleSettled,
    /// Timeout duro (rede de segurança) — a resposta é `truncated`.
    TimedOut,
}

/// Resultado do contrato de fim-de-resposta. `truncated` é **explícito** — nunca há
/// truncamento silencioso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndResult {
    pub outcome: EndOutcome,
    pub truncated: bool,
}

/// `true` se a linha é o evento de fim do NDJSON (objeto JSON com `type == event_type`).
/// Guard: linha não-JSON ou sem `type` → `false` (não fecha o turno por engano).
#[must_use]
pub fn line_is_end_event(line: &str, event_type: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|s| s == event_type)
        })
        .unwrap_or(false)
}

/// **W0-10.** Decisor de fim-de-resposta com relógio LÓGICO (ms). Contrato:
/// `result (on_line) > idle (on_tick) > timeout (on_tick)`. O loop chamador checa
/// `on_line` a cada linha de saída e `on_tick` periodicamente.
#[derive(Debug, Clone)]
pub struct EndDetector {
    end_signal: EndSignal,
    started_ms: u64,
    ask_timeout_ms: u64,
    idle_ms: u64,
    last_change_ms: u64,
    seen_output: bool,
}

impl EndDetector {
    /// Cria o decisor a partir do CLI Profile e do tempo lógico inicial (ms).
    #[must_use]
    pub fn new(profile: &CliProfile, now_ms: u64) -> Self {
        Self {
            end_signal: profile.end_signal.clone(),
            started_ms: now_ms,
            ask_timeout_ms: profile.ask_timeout_ms.unwrap_or(DEFAULT_ASK_TIMEOUT_MS),
            idle_ms: profile.idle_ms.unwrap_or(DEFAULT_IDLE_MS),
            last_change_ms: now_ms,
            seen_output: false,
        }
    }

    /// Caminho-ouro: alimenta uma linha de saída do CLI. Fecha o turno se for o evento
    /// `result`/sentinela (prioridade máxima). Também marca atividade para a histerese.
    pub fn on_line(&mut self, line: &str) -> Option<EndResult> {
        if let EndSignal::StreamJson { event_type } = &self.end_signal {
            if line_is_end_event(line, event_type) {
                return Some(EndResult {
                    outcome: EndOutcome::Completed,
                    truncated: false,
                });
            }
        }
        if !line.trim().is_empty() {
            self.seen_output = true;
        }
        None
    }

    /// Tick periódico com o estado do grid. `changed` = houve dano novo desde o tick
    /// anterior (lido do `damaged_rows()`/`reset_damage()` — NÃO OCR); `ready` = a
    /// última linha casa o `prompt_ready_regex`. Fecha por idle (silêncio + prompt) ou,
    /// como rede de segurança sempre presente, por **timeout duro** (`truncated`).
    pub fn on_tick(&mut self, now_ms: u64, changed: bool, ready: bool) -> Option<EndResult> {
        // Timeout duro — guard contra `result` vazio/hang (#7124/#25629).
        if now_ms.saturating_sub(self.started_ms) >= self.ask_timeout_ms {
            return Some(EndResult {
                outcome: EndOutcome::TimedOut,
                truncated: true,
            });
        }
        if changed {
            self.last_change_ms = now_ms;
            self.seen_output = true;
            return None; // grid ainda mudando
        }
        let quiet_ms = now_ms.saturating_sub(self.last_change_ms);
        if quiet_ms >= self.idle_ms && self.seen_output && ready {
            return Some(EndResult {
                outcome: EndOutcome::IdleSettled,
                truncated: false,
            });
        }
        None
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lina_vt::AlacrittyBackend;
    use serial_test::serial;
    use std::time::Duration as Dur;

    // ---- helpers ----

    fn profile_toml(delivery: &str, submit_ms: u64, end: &str) -> CliProfile {
        let src = format!(
            r#"
            id = "fake"
            program = "fake"
            delivery = "{delivery}"
            submit_delay_ms = {submit_ms}
            prompt_ready_regex = '\$'
            idle_ms = 100
            ask_timeout_ms = 100000
            {end}
        "#
        );
        CliProfile::from_toml_str(&src, "<test>").expect("profile de teste deve parsear")
    }

    fn idle_profile() -> CliProfile {
        profile_toml("pty_inject", 5, "[end_signal]\nkind = \"idle\"")
    }

    fn grid(prompt: &str, bracketed: bool) -> Arc<Mutex<Box<dyn VtBackend>>> {
        let mut b = AlacrittyBackend::new(40, 6);
        if bracketed {
            b.advance(b"\x1b[?2004h"); // habilita BRACKETED_PASTE mode
        }
        b.advance(b"\x1b[2J\x1b[H");
        b.advance(prompt.as_bytes());
        Arc::new(Mutex::new(Box::new(b) as Box<dyn VtBackend>))
    }

    fn poll_until(timeout: Dur, mut cond: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        loop {
            if cond() {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Dur::from_millis(5));
        }
    }

    // ---- W0-9 ----

    /// A sanitização remove o `ESC[201~` malicioso; o wrapper legítimo fica intacto.
    #[test]
    fn sanitize_removes_paste_terminators() {
        let evil = "hello\x1b[201~ rm -rf /";
        assert_eq!(sanitize_paste(evil), "hello rm -rf /");

        let bytes = build_paste(evil, true);
        // Exatamente UM ESC[201~ (o terminador do wrapper), não o do payload.
        let n_end = bytes
            .windows(PASTE_END.len())
            .filter(|w| *w == PASTE_END)
            .count();
        assert_eq!(n_end, 1, "só o terminador do wrapper deve sobrar");
        assert!(bytes.starts_with(PASTE_BEGIN) && bytes.ends_with(PASTE_END));
        // Nunca um `\r` no texto colado (o Enter é um Submit separado).
        assert!(!bytes.contains(&b'\r'));
    }

    /// `build_paste` sem bracketed = texto puro (sanitizado), sem wrapper, sem `\r`.
    #[test]
    fn build_paste_plain_when_not_bracketed() {
        let bytes = build_paste("echo oi", false);
        assert_eq!(bytes, b"echo oi");
        assert!(!bytes.contains(&b'\r'));
    }

    /// Critério (a): a injeção produz a SEQUÊNCIA faseada correta — `AgentText`
    /// (bracketed, sanitizado, sem `\r`) e DEPOIS `Submit` (`0x0D`) separado.
    #[test]
    #[serial]
    fn delivery_produces_phased_sequence() {
        let sup = Supervisor::new();
        let from = sup.register("@A", None, Box::new(std::io::sink()));
        let target = sup.register("@B", Some("dev".into()), Box::new(std::io::sink()));
        let g = grid("$ ", true); // prompt pronto ("$") + bracketed mode

        let out = deliver_a2a(
            &sup,
            target,
            from,
            "revise o diff\x1b[201~",
            &idle_profile(),
            &g,
            InjectPolicy::AllowAll,
        )
        .expect("deliver_a2a");
        assert_eq!(
            out,
            DeliveryOutcome::Injected {
                ready: true,
                bracketed: true
            }
        );

        assert!(poll_until(Dur::from_secs(5), || sup
            .applied_ops(target)
            .len()
            == 2));
        let ops = sup.applied_ops(target);
        match (&ops[0], &ops[1]) {
            (WriteOp::AgentText { from: f, bytes }, WriteOp::Submit { from: Some(s) }) => {
                assert_eq!(f, &from);
                assert_eq!(s, &from);
                assert!(bytes.starts_with(PASTE_BEGIN) && bytes.ends_with(PASTE_END));
                assert!(!bytes.contains(&b'\r'), "texto colado nunca tem CR");
                // o ESC[201~ malicioso do payload foi removido (sobra só o do wrapper).
                let n_end = bytes
                    .windows(PASTE_END.len())
                    .filter(|w| *w == PASTE_END)
                    .count();
                assert_eq!(n_end, 1);
            }
            other => panic!("sequência faseada errada: {other:?}"),
        }
    }

    /// Sem bracketed mode, o `AgentText` é texto puro (sem wrapper).
    #[test]
    #[serial]
    fn delivery_plain_text_when_target_not_in_bracketed_mode() {
        let sup = Supervisor::new();
        let from = sup.register("@A", None, Box::new(std::io::sink()));
        let target = sup.register("@B", None, Box::new(std::io::sink()));
        let g = grid("$ ", false);

        let out = deliver_a2a(
            &sup,
            target,
            from,
            "oi",
            &idle_profile(),
            &g,
            InjectPolicy::AllowAll,
        )
        .expect("deliver");
        assert_eq!(
            out,
            DeliveryOutcome::Injected {
                ready: true,
                bracketed: false
            }
        );
        assert!(poll_until(Dur::from_secs(5), || sup
            .applied_ops(target)
            .len()
            == 2));
        let ops = sup.applied_ops(target);
        if let WriteOp::AgentText { bytes, .. } = &ops[0] {
            assert_eq!(bytes, b"oi");
        } else {
            panic!("op[0] deveria ser AgentText");
        }
    }

    /// Critério (a): duas injeções concorrentes no MESMO terminal saem serializadas,
    /// sem interleave — cada `AgentText` é imediatamente seguido pelo seu `Submit`.
    #[test]
    #[serial]
    fn concurrent_deliveries_stay_serial() {
        let sup = Arc::new(Supervisor::new());
        let a = sup.register("@A", None, Box::new(std::io::sink()));
        let b = sup.register("@B", None, Box::new(std::io::sink()));
        let target = sup.register("@T", None, Box::new(std::io::sink()));
        let g = grid("$ ", true);
        let prof = idle_profile();

        let mut handles = Vec::new();
        for from in [a, b] {
            let sup = Arc::clone(&sup);
            let g = Arc::clone(&g);
            let prof = prof.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..15 {
                    deliver_a2a(
                        &sup,
                        target,
                        from,
                        &format!("t{i}"),
                        &prof,
                        &g,
                        InjectPolicy::AllowAll,
                    )
                    .expect("deliver concorrente");
                }
            }));
        }
        for h in handles {
            h.join().expect("join");
        }

        let expected = 2 * 15 * 2; // 2 agentes × 15 injeções × (AgentText + Submit)
        assert!(poll_until(Dur::from_secs(5), || sup
            .applied_ops(target)
            .len()
            == expected));
        let ops = sup.applied_ops(target);
        for (i, op) in ops.iter().enumerate() {
            if let WriteOp::AgentText { from, .. } = op {
                assert!(
                    matches!(ops.get(i + 1), Some(WriteOp::Submit { from: Some(s) }) if s == from),
                    "AgentText deve ser imediatamente seguido pelo seu Submit (sem interleave)"
                );
            }
        }
    }

    /// `delivery = session_resume` é o fallback (stub) — não toca na fila do PTY.
    #[test]
    #[serial]
    fn session_resume_is_a_fallback_stub() {
        let sup = Supervisor::new();
        let from = sup.register("@A", None, Box::new(std::io::sink()));
        let target = sup.register("@B", None, Box::new(std::io::sink()));
        let g = grid("$ ", true);
        let prof = profile_toml("session_resume", 5, "[end_signal]\nkind = \"idle\"");

        let out = deliver_a2a(&sup, target, from, "oi", &prof, &g, InjectPolicy::AllowAll)
            .expect("deliver");
        assert_eq!(out, DeliveryOutcome::SessionResume);
        // nenhum WriteOp foi enfileirado no alvo.
        std::thread::sleep(Dur::from_millis(50));
        assert!(sup.applied_ops(target).is_empty());
    }

    /// A allow-list nega injeção de um par bloqueado (gancho de segurança W0-9).
    #[test]
    #[serial]
    fn inject_policy_denies_blocked_pair() {
        let sup = Supervisor::new();
        let from = sup.register("@A", None, Box::new(std::io::sink()));
        let target = sup.register("@B", None, Box::new(std::io::sink()));
        let g = grid("$ ", true);
        let denied = [(from, target)];

        let res = deliver_a2a(
            &sup,
            target,
            from,
            "oi",
            &idle_profile(),
            &g,
            InjectPolicy::Deny(&denied),
        );
        assert!(matches!(res, Err(A2aError::InjectionDenied { .. })));
        assert!(sup.applied_ops(target).is_empty());
    }

    // ---- W0-10 ----

    /// Caminho-ouro: o evento `result` do NDJSON fecha o turno; linhas comuns não.
    #[test]
    fn end_by_stream_json_result() {
        let prof = profile_toml(
            "pty_inject",
            5,
            "[end_signal]\nkind = \"stream_json\"\nevent_type = \"result\"",
        );
        let mut det = EndDetector::new(&prof, 0);

        assert_eq!(det.on_line("{\"type\":\"assistant\",\"text\":\"…\"}"), None);
        assert_eq!(det.on_line("não é json"), None);
        assert_eq!(
            det.on_line("{\"type\":\"result\",\"subtype\":\"success\"}"),
            Some(EndResult {
                outcome: EndOutcome::Completed,
                truncated: false
            })
        );
    }

    /// Idle do grid: silêncio por `idle_ms` + prompt pronto fecha o turno (truncated=false).
    #[test]
    fn end_by_grid_idle() {
        let prof = idle_profile(); // idle_ms=100
        let mut det = EndDetector::new(&prof, 0);

        assert_eq!(det.on_tick(0, true, false), None); // saída chegando
        assert_eq!(det.on_tick(50, false, true), None); // quieto só 50ms (< 100)
        assert_eq!(
            det.on_tick(200, false, true),
            Some(EndResult {
                outcome: EndOutcome::IdleSettled,
                truncated: false
            }),
            "quieto 200ms (>=100) + prompt pronto → idle"
        );
    }

    /// Idle NÃO fecha enquanto o prompt não está pronto (processo travado no meio da
    /// saída) — distingue "assentou no prompt" de "pendurado".
    #[test]
    fn idle_requires_prompt_ready() {
        let prof = idle_profile();
        let mut det = EndDetector::new(&prof, 0);
        det.on_tick(0, true, false); // viu saída
        assert_eq!(
            det.on_tick(500, false, false),
            None,
            "quieto, mas sem prompt → não fecha"
        );
    }

    /// Timeout duro com `result` vazio + processo pendurado: fecha por timeout com
    /// `truncated: true` — NUNCA silencioso.
    #[test]
    fn end_by_hard_timeout_is_truncated() {
        // ask_timeout curto; sinal stream_json (mas nenhum result válido chega).
        let prof = profile_toml(
            "pty_inject",
            5,
            "[end_signal]\nkind = \"stream_json\"\nevent_type = \"result\"",
        );
        let mut det = EndDetector::new(&prof, 0);
        // O "result vazio" não casa (sem type==result); processo pendura.
        assert_eq!(det.on_line("{\"type\":\"\"}"), None);
        // grid pendurado (sem prompt) → idle não fecha; só o timeout.
        assert_eq!(det.on_tick(50_000, false, false), None);
        let r = det
            .on_tick(120_001, false, false)
            .expect("o timeout duro deve fechar o turno");
        assert_eq!(r.outcome, EndOutcome::TimedOut);
        assert!(
            r.truncated,
            "timeout deve marcar truncated=true (nunca silencioso)"
        );
    }

    /// O `result` tem prioridade sobre idle (é checado por linha, antes do tick).
    #[test]
    fn result_takes_priority_over_idle() {
        let prof = profile_toml(
            "pty_inject",
            5,
            "[end_signal]\nkind = \"stream_json\"\nevent_type = \"result\"",
        );
        let mut det = EndDetector::new(&prof, 0);
        // mesmo com o grid já quieto e pronto, uma linha result fecha como Completed.
        det.on_tick(0, true, true);
        let r = det.on_line("{\"type\":\"result\"}").expect("result fecha");
        assert_eq!(r.outcome, EndOutcome::Completed);
        assert!(!r.truncated);
    }

    /// `wait_ready` casa o `prompt_ready_regex` na última linha do grid.
    #[test]
    fn wait_ready_matches_prompt() {
        let re = Regex::new(r"\$").expect("regex");
        let ready = grid("$ ", false);
        assert!(wait_ready(&ready, &re, Dur::from_millis(50)));

        let not_ready = grid("trabalhando...", false);
        assert!(!wait_ready(&not_ready, &re, Dur::from_millis(50)));
    }
}
